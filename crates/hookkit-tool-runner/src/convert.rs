//! Conversion from the layered Pkl schema to crate-private execution specs.

use crate::spec::{
    CheckScope, CommandArgTemplate, ExitCodePolicy, FileSelection, InvocationGranularity,
    PhaseMode, ToolMessages, ToolPhase, ToolSpec, ToolWorkflow,
};
use crate::util::invalid_data;
use hookkit_pkl_config::schema as pkl;
use std::collections::BTreeSet;

/// Resolve the `run` list to ordered tool specs.
pub(crate) fn resolve_run_order(
    config: &pkl::RunnerConfig,
) -> hookkit_core::Result<Vec<&pkl::ToolSpec>> {
    let mut tools = Vec::with_capacity(config.run.len());
    for id in &config.run {
        let Some(spec) = config.tools.get(id) else {
            return Err(invalid_data(format!(
                "run references unknown tool `{id}`; define it under `tools` or remove it from `run`"
            )));
        };
        tools.push(spec);
    }
    Ok(tools)
}

/// Convert a Pkl-shaped tool spec to the runtime execution type.
pub(crate) fn convert_tool_spec(spec: &pkl::ToolSpec, global_exclude: &[String]) -> ToolSpec {
    let phases: Vec<ToolPhase> = ordered_phases(spec)
        .into_iter()
        .map(convert_phase)
        .collect();

    let mut exclude = global_exclude.to_vec();
    exclude.extend(spec.files.exclude.clone());
    let workflows = convert_workflows(spec, &phases);

    ToolSpec {
        id: spec.id.clone(),
        display_name: spec.display_name.clone(),
        executable: spec.executable.clone(),
        install_hint: spec.install_hint.clone(),
        file_selection: FileSelection {
            include: spec.files.include.clone(),
            exclude,
        },
        workspace_indicator: spec.workspace_indicator.clone(),
        workflows,
        phases,
        messages: convert_messages(&spec.messages),
        diagnostics_directory: spec.diagnostics.directory.clone(),
    }
}

fn convert_workflows(spec: &pkl::ToolSpec, phases: &[ToolPhase]) -> Vec<ToolWorkflow> {
    if !spec.workflows.is_empty() {
        return ordered_workflows(spec)
            .into_iter()
            .map(|(id, workflow)| ToolWorkflow {
                id: id.clone(),
                check: workflow.check.as_ref().map(|command| {
                    convert_workflow_command(format!("{id}.check"), command, PhaseMode::Verify)
                }),
                remedy: workflow.remedy.as_ref().map(|command| {
                    convert_workflow_command(format!("{id}.remedy"), command, PhaseMode::Fix)
                }),
                check_scope: workflow.check_scope,
                invocation: workflow.invocation,
                compatibility_translation: false,
                enabled: workflow.enabled,
            })
            .collect();
    }

    // Compatibility translation for the existing immediate-runner phase
    // shape. Every mutator becomes a separate deferred workflow paired with
    // the last enabled verifier. Mutating-only tools remain explicitly marked
    // and are rejected as operationally unverifiable after one compatibility
    // remedy pass; Item 8 migrates all builtins away from that fallback.
    let verifier = phases
        .iter()
        .rev()
        .find(|phase| phase.enabled && phase.is_verifier())
        .cloned();
    let mut workflows = phases
        .iter()
        .filter(|phase| phase.enabled && !phase.is_verifier())
        .map(|remedy| ToolWorkflow {
            id: remedy.id.clone(),
            check: verifier.clone(),
            remedy: Some(remedy.clone()),
            check_scope: if spec.workspace_indicator.is_some() && !remedy.uses_file_arguments() {
                CheckScope::Workspace
            } else {
                CheckScope::TargetFiles
            },
            invocation: InvocationGranularity::Batch,
            compatibility_translation: true,
            enabled: true,
        })
        .collect::<Vec<_>>();
    if workflows.is_empty() {
        workflows.extend(
            phases
                .iter()
                .filter(|phase| phase.enabled && phase.is_verifier())
                .cloned()
                .map(|check| ToolWorkflow {
                    id: check.id.clone(),
                    check: Some(check),
                    remedy: None,
                    check_scope: if spec.workspace_indicator.is_some() {
                        CheckScope::Workspace
                    } else {
                        CheckScope::TargetFiles
                    },
                    invocation: InvocationGranularity::Batch,
                    compatibility_translation: true,
                    enabled: true,
                }),
        );
    }
    workflows
}

/// Workflows named by `workflowOrder` first, then the remaining workflows in
/// identifier order (Pkl mapping order is not preserved through JSON decoding).
fn ordered_workflows(spec: &pkl::ToolSpec) -> Vec<(&String, &pkl::Workflow)> {
    let mut seen = BTreeSet::new();
    let mut workflows = Vec::new();
    for id in &spec.workflow_order {
        if let Some(workflow) = spec.workflows.get(id) {
            if seen.insert(id.clone()) {
                workflows.push((id, workflow));
            }
        }
    }
    workflows.extend(
        spec.workflows
            .iter()
            .filter(|(id, _)| !seen.contains(id.as_str())),
    );
    workflows
}

fn convert_workflow_command(
    id: String,
    command: &pkl::WorkflowCommand,
    mode: PhaseMode,
) -> ToolPhase {
    ToolPhase {
        id,
        mode,
        program: command.program.clone(),
        args: command.argv.iter().map(convert_argv_element).collect(),
        exit_codes: convert_exit_codes(&command.exit_codes),
        issues_on_stdout: command.issues_on_stdout,
        writes: command.writes,
        extra_args: command.extra_args.clone(),
        enabled: true,
    }
}

/// Phases named by `phaseOrder` first, then the remaining phases sorted by
/// canonical mode order (format, fix, verify, check-only) and identifier.
pub(crate) fn ordered_phases(spec: &pkl::ToolSpec) -> Vec<(String, &pkl::Phase)> {
    let mut seen = BTreeSet::<String>::new();
    let mut out = Vec::<(String, &pkl::Phase)>::new();

    // Honor explicit phase order first.
    for id in &spec.phase_order {
        if let Some(phase) = spec.phases.get(id) {
            if seen.insert(id.clone()) {
                out.push((id.clone(), phase));
            }
        }
    }

    // Append any remaining phases sorted by canonical mode order, then by id.
    let mut remaining: Vec<(&String, &pkl::Phase)> = spec
        .phases
        .iter()
        .filter(|(id, _)| !seen.contains(id.as_str()))
        .collect();
    remaining.sort_by(|a, b| {
        canonical_mode_order(a.1.mode)
            .cmp(&canonical_mode_order(b.1.mode))
            .then_with(|| a.0.cmp(b.0))
    });
    for (id, phase) in remaining {
        out.push((id.clone(), phase));
    }
    out
}

fn canonical_mode_order(mode: PhaseMode) -> u8 {
    match mode {
        PhaseMode::Format => 0,
        PhaseMode::Fix => 1,
        PhaseMode::Verify => 2,
        PhaseMode::CheckOnly => 3,
    }
}

fn convert_phase((id, phase): (String, &pkl::Phase)) -> ToolPhase {
    ToolPhase {
        id,
        mode: phase.mode,
        program: phase.program.clone(),
        args: phase.argv.iter().map(convert_argv_element).collect(),
        exit_codes: convert_exit_codes(&phase.exit_codes),
        issues_on_stdout: false,
        writes: phase.writes,
        extra_args: phase.extra_args.clone(),
        enabled: phase.enabled,
    }
}

fn convert_argv_element(element: &pkl::ArgvElement) -> CommandArgTemplate {
    match element {
        pkl::ArgvElement::Literal(s) => CommandArgTemplate::Literal(s.clone()),
        pkl::ArgvElement::Token(t) => match t {
            pkl::ArgToken::Files => CommandArgTemplate::Files,
            pkl::ArgToken::WorkspaceFiles => CommandArgTemplate::WorkspaceFiles,
            pkl::ArgToken::Workspace => CommandArgTemplate::Workspace,
            pkl::ArgToken::WorkspaceIndicator => CommandArgTemplate::WorkspaceIndicator,
            pkl::ArgToken::ProjectRoot => CommandArgTemplate::ProjectRoot,
            pkl::ArgToken::ToolExecutable => CommandArgTemplate::ToolExecutable,
            pkl::ArgToken::ExtraArgs => CommandArgTemplate::ExtraArgs,
        },
    }
}

fn convert_exit_codes(codes: &pkl::ExitCodes) -> ExitCodePolicy {
    ExitCodePolicy {
        clean: codes.clean.clone(),
        issues: codes.issues.clone(),
        failure: codes.failure.clone(),
        unexpected: codes.unexpected,
    }
}

fn convert_messages(messages: &pkl::Messages) -> ToolMessages {
    ToolMessages {
        clean_changed_agent: messages.clean_changed_agent.clone(),
        issues_agent: messages.issues_agent.clone(),
        issues_changed_agent: messages.issues_changed_agent.clone(),
        unavailable_user: messages.unavailable_user.clone(),
        failed_user: messages.failed_user.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn unordered_phases_sort_by_mode_then_identifier_not_insertion_order() {
        let phase = |mode| pkl::Phase {
            mode,
            ..pkl::Phase::default()
        };
        let spec = pkl::ToolSpec {
            phases: BTreeMap::from([
                ("organize-imports".to_owned(), phase(PhaseMode::Fix)),
                ("verify".to_owned(), phase(PhaseMode::Verify)),
                ("autofix".to_owned(), phase(PhaseMode::Fix)),
                ("format".to_owned(), phase(PhaseMode::Format)),
            ]),
            ..pkl::ToolSpec::default()
        };
        let order = ordered_phases(&spec)
            .into_iter()
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        assert_eq!(order, ["format", "autofix", "organize-imports", "verify"]);
    }
}
