//! Structural validation and deterministic audit output for embedded tools.

use crate::schema::{
    ArgToken, ArgvElement, CheckScope, ExitCodes, InvocationGranularity, Phase, PhaseMode,
    ToolSpec, WorkflowCommand, WriteBehavior,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// All structural catalog violations found in one validation pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogValidationError {
    /// Human-readable structural violations in deterministic order.
    pub errors: Vec<String>,
}

impl fmt::Display for CatalogValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, error) in self.errors.iter().enumerate() {
            if index > 0 {
                formatter.write_str("\n")?;
            }
            write!(formatter, "- {error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for CatalogValidationError {}

/// Reject catalog entries that cannot support authoritative deferred checks.
///
/// Legacy `phases` remain valid when their compatibility translation pairs
/// each mutator with a read-only verifier. A mutating-only legacy entry must
/// carry a nonempty `unverifiedRemedyFallback` explanation; the shipped
/// catalog intentionally contains no such fallback.
pub fn validate_builtin_catalog(
    specs: &BTreeMap<String, ToolSpec>,
) -> Result<(), CatalogValidationError> {
    let mut errors = Vec::new();
    let mut tool_ids = BTreeSet::new();
    for (key, spec) in specs {
        if !spec.enabled {
            continue;
        }
        let prefix = format!("{key} ({})", spec.id);
        if spec.id.trim().is_empty() {
            errors.push(format!("{key}: tool id is empty"));
        } else if !tool_ids.insert(spec.id.as_str()) {
            errors.push(format!("{prefix}: duplicate tool id"));
        }
        if spec.executable.trim().is_empty() {
            errors.push(format!("{prefix}: executable is empty"));
        }
        validate_order(
            &prefix,
            "workflowOrder",
            &spec.workflow_order,
            spec.workflows.keys(),
            &mut errors,
        );
        validate_order(
            &prefix,
            "phaseOrder",
            &spec.phase_order,
            spec.phases.keys(),
            &mut errors,
        );

        if spec.workflows.is_empty() {
            validate_compatibility_tool(&prefix, spec, &mut errors);
        } else {
            validate_explicit_tool(&prefix, spec, &mut errors);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(CatalogValidationError { errors })
    }
}

fn validate_order<'a>(
    prefix: &str,
    field: &str,
    order: &[String],
    available: impl Iterator<Item = &'a String>,
    errors: &mut Vec<String>,
) {
    let available = available.map(String::as_str).collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for id in order {
        if !available.contains(id.as_str()) {
            errors.push(format!("{prefix}: {field} names unknown entry {id}"));
        }
        if !seen.insert(id) {
            errors.push(format!("{prefix}: {field} repeats {id}"));
        }
    }
}

fn validate_explicit_tool(prefix: &str, spec: &ToolSpec, errors: &mut Vec<String>) {
    if spec.unverified_remedy_fallback.is_some() {
        errors.push(format!(
            "{prefix}: unverifiedRemedyFallback is stale because explicit workflows exist"
        ));
    }
    let mut enabled = 0;
    for (id, workflow) in ordered_workflows(spec) {
        if !workflow.enabled {
            continue;
        }
        enabled += 1;
        let label = format!("{prefix}: workflow {id}");
        let Some(check) = workflow.check.as_ref() else {
            errors.push(format!("{label} has no authoritative check"));
            continue;
        };
        validate_command(&format!("{label} check"), check, true, errors);
        if let Some(remedy) = &workflow.remedy {
            validate_command(&format!("{label} remedy"), remedy, false, errors);
        }
    }
    if enabled == 0 {
        errors.push(format!("{prefix}: no deferred workflow is enabled"));
    }
}

fn validate_compatibility_tool(prefix: &str, spec: &ToolSpec, errors: &mut Vec<String>) {
    let phases = ordered_phases(spec);
    let enabled = phases
        .into_iter()
        .filter(|(_, phase)| phase.enabled)
        .collect::<Vec<_>>();
    let mutators = enabled
        .iter()
        .filter(|(_, phase)| !is_verifier(phase.mode))
        .collect::<Vec<_>>();
    let verifiers = enabled
        .iter()
        .filter(|(_, phase)| is_verifier(phase.mode))
        .collect::<Vec<_>>();

    for (id, phase) in &enabled {
        validate_exit_codes(&format!("{prefix}: phase {id}"), &phase.exit_codes, errors);
        if is_verifier(phase.mode) && phase.writes != WriteBehavior::None {
            errors.push(format!("{prefix}: verifier phase {id} declares writes"));
        }
        if !is_verifier(phase.mode) && phase.writes == WriteBehavior::None {
            errors.push(format!("{prefix}: mutating phase {id} has writes=none"));
        }
    }

    if enabled.is_empty() {
        errors.push(format!("{prefix}: no phase is enabled"));
    }
    if !mutators.is_empty() && verifiers.is_empty() {
        match spec.unverified_remedy_fallback.as_deref().map(str::trim) {
            Some(reason) if !reason.is_empty() => {}
            _ => errors.push(format!(
                "{prefix}: auto-fix compatibility translation has no authoritative final check"
            )),
        }
    } else if spec.unverified_remedy_fallback.is_some() {
        errors.push(format!(
            "{prefix}: unverifiedRemedyFallback is stale because a verifier is available"
        ));
    }
}

fn validate_command(
    label: &str,
    command: &WorkflowCommand,
    is_check: bool,
    errors: &mut Vec<String>,
) {
    validate_exit_codes(label, &command.exit_codes, errors);
    if command
        .program
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        errors.push(format!("{label} has an empty program override"));
    }
    if is_check && command.writes != WriteBehavior::None {
        errors.push(format!("{label} is not read-only"));
    }
    if !is_check && command.writes == WriteBehavior::None {
        errors.push(format!("{label} has no declared write scope"));
    }
}

fn validate_exit_codes(label: &str, codes: &ExitCodes, errors: &mut Vec<String>) {
    if codes.clean.is_empty() {
        errors.push(format!("{label} has no clean exit code"));
    }
    let mut classified = BTreeMap::<i32, &'static str>::new();
    for (kind, values) in [
        ("clean", &codes.clean),
        ("issues", &codes.issues),
        ("failure", &codes.failure),
    ] {
        for code in values {
            if let Some(previous) = classified.insert(*code, kind) {
                errors.push(format!(
                    "{label} classifies exit code {code} as both {previous} and {kind}"
                ));
            }
        }
    }
}

/// Render the checked-in catalog audit. The output is intentionally derived
/// from the same decoded specs the validator inspects so command, scope, and
/// granularity changes cannot silently drift from the inventory.
pub fn render_builtin_catalog_markdown(specs: &BTreeMap<String, ToolSpec>) -> String {
    let mut output = String::from(
        "# Built-in deferred workflow audit\n\n<!-- markdownlint-disable MD013 -->\n\n",
    );
    output.push_str(
        "Generated from the embedded Pkl catalog. `explicit` rows use declared deferred workflows; `compatibility` rows are structurally translated from legacy phases by pairing each mutator with the final enabled read-only verifier. This inventory does not claim cross-version real-tool verification: command semantics remain version-dependent unless a limitation says otherwise, and the opt-in real-tool lane must use controlled tool versions. Batch and workspace findings are conservatively attributed to every candidate in the invocation when no diagnostic adapter identifies exact files.\n\n",
    );
    output.push_str("| Built-in | Tool ID | Mode | Checks | Remedies | Check scope | Invocation | Precision / known limitation |\n");
    output.push_str("| --- | --- | --- | --- | --- | --- | --- | --- |\n");
    for (key, spec) in specs {
        let audit = audit_tool(spec);
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
            cell(key),
            cell(&spec.id),
            audit.mode,
            cell(&audit.checks.join("; ")),
            cell(&audit.remedies.join("; ")),
            cell(&audit.scopes.join("; ")),
            cell(&audit.invocations.join("; ")),
            cell(&audit.limitation),
        ));
    }
    output
}

struct ToolAudit {
    mode: &'static str,
    checks: Vec<String>,
    remedies: Vec<String>,
    scopes: Vec<String>,
    invocations: Vec<String>,
    limitation: String,
}

fn audit_tool(spec: &ToolSpec) -> ToolAudit {
    if !spec.enabled {
        return ToolAudit {
            mode: "disabled",
            checks: vec!["—".into()],
            remedies: vec!["—".into()],
            scopes: vec!["—".into()],
            invocations: vec!["—".into()],
            limitation: "Disabled draft; no deferred support is claimed.".into(),
        };
    }
    if !spec.workflows.is_empty() {
        let workflows = ordered_workflows(spec)
            .into_iter()
            .filter(|(_, workflow)| workflow.enabled)
            .collect::<Vec<_>>();
        return ToolAudit {
            mode: "explicit",
            checks: workflows
                .iter()
                .filter_map(|(id, workflow)| {
                    workflow
                        .check
                        .as_ref()
                        .map(|command| format!("{id}: {}", workflow_command(spec, command)))
                })
                .collect(),
            remedies: nonempty_or_dash(
                workflows
                    .iter()
                    .filter_map(|(id, workflow)| {
                        workflow
                            .remedy
                            .as_ref()
                            .map(|command| format!("{id}: {}", workflow_command(spec, command)))
                    })
                    .collect(),
            ),
            scopes: workflows
                .iter()
                .map(|(id, workflow)| format!("{id}: {}", check_scope(workflow.check_scope)))
                .collect(),
            invocations: workflows
                .iter()
                .map(|(id, workflow)| format!("{id}: {}", invocation(workflow.invocation)))
                .collect(),
            limitation: explicit_limitation(spec),
        };
    }

    let phases = ordered_phases(spec)
        .into_iter()
        .filter(|(_, phase)| phase.enabled)
        .collect::<Vec<_>>();
    let verifier = phases
        .iter()
        .rev()
        .find(|(_, phase)| is_verifier(phase.mode));
    let mutators = phases
        .iter()
        .filter(|(_, phase)| !is_verifier(phase.mode))
        .collect::<Vec<_>>();
    let checks = if mutators.is_empty() {
        phases
            .iter()
            .filter(|(_, phase)| is_verifier(phase.mode))
            .map(|(id, phase)| format!("{id}: {}", phase_command(spec, phase)))
            .collect()
    } else {
        verifier
            .map(|(id, phase)| vec![format!("{id}: {}", phase_command(spec, phase))])
            .unwrap_or_else(|| vec!["—".into()])
    };
    let remedies = nonempty_or_dash(
        mutators
            .iter()
            .map(|(id, phase)| format!("{id}: {}", phase_command(spec, phase)))
            .collect(),
    );
    let scopes = if mutators.is_empty() {
        vec![if spec.workspace_indicator.is_some() {
            "workspace".into()
        } else {
            "target-files".into()
        }]
    } else {
        mutators
            .iter()
            .map(|(id, phase)| format!("{id}: {}", compatibility_scope(spec, phase)))
            .collect()
    };
    ToolAudit {
        mode: "compatibility",
        checks,
        remedies,
        scopes,
        invocations: vec!["batch".into()],
        limitation: if mutators.is_empty() {
            "Read-only checks are compatibility-translated as batched invocations; real-tool behavior is version-dependent.".into()
        } else if let Some(reason) = &spec.unverified_remedy_fallback {
            format!("Unverified mutator-first fallback: {reason}")
        } else {
            "Each remedy is compatibility-paired with the final read-only phase; real-tool behavior is version-dependent.".into()
        },
    }
}

fn explicit_limitation(spec: &ToolSpec) -> String {
    match spec.id.as_str() {
        "go-fmt" | "gofumpt" | "goimports" => {
            "Read-only list mode reports dirty files through stdout with exit 0; parse failures depend on the installed tool version.".into()
        }
        "golines" => {
            "Read-only dry-run diffs are detected through stdout; the original upstream is archived and installed-version behavior may vary.".into()
        }
        "gomod-tidy" => {
            "Requires a Go release with `go mod tidy -diff`; exit 1 is treated as source issues and may be ambiguous with some command failures.".into()
        }
        "yq" => {
            "Per-file check requires POSIX `sh`, `mktemp`, and `diff`; formatting behavior depends on the installed yq version.".into()
        }
        "ruff" => {
            "Lint remedies precede format remedies when both are initially dirty; a lint fix that dirties an initially clean format check is reported for manual follow-up after the bounded pass.".into()
        }
        _ => "Explicit checks are structurally validated; real-tool behavior is version-dependent.".into(),
    }
}

fn ordered_workflows(spec: &ToolSpec) -> Vec<(&String, &crate::schema::Workflow)> {
    let mut seen = BTreeSet::new();
    let mut workflows = Vec::new();
    for id in &spec.workflow_order {
        if let Some(workflow) = spec.workflows.get(id) {
            if seen.insert(id.as_str()) {
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

fn ordered_phases(spec: &ToolSpec) -> Vec<(&String, &Phase)> {
    let mut seen = BTreeSet::new();
    let mut phases = Vec::new();
    for id in &spec.phase_order {
        if let Some(phase) = spec.phases.get(id) {
            if seen.insert(id.as_str()) {
                phases.push((id, phase));
            }
        }
    }
    let mut remaining = spec
        .phases
        .iter()
        .filter(|(id, _)| !seen.contains(id.as_str()))
        .collect::<Vec<_>>();
    remaining.sort_by(|left, right| {
        phase_rank(left.1.mode)
            .cmp(&phase_rank(right.1.mode))
            .then_with(|| left.0.cmp(right.0))
    });
    phases.extend(remaining);
    phases
}

fn phase_rank(mode: PhaseMode) -> u8 {
    match mode {
        PhaseMode::Format => 0,
        PhaseMode::Fix => 1,
        PhaseMode::Verify => 2,
        PhaseMode::CheckOnly => 3,
    }
}

fn is_verifier(mode: PhaseMode) -> bool {
    matches!(mode, PhaseMode::Verify | PhaseMode::CheckOnly)
}

fn workflow_command(spec: &ToolSpec, command: &WorkflowCommand) -> String {
    command_text(
        command.program.as_deref().unwrap_or(&spec.executable),
        &command.argv,
    )
}

fn phase_command(spec: &ToolSpec, phase: &Phase) -> String {
    command_text(
        phase.program.as_deref().unwrap_or(&spec.executable),
        &phase.argv,
    )
}

fn command_text(program: &str, argv: &[ArgvElement]) -> String {
    std::iter::once(program.to_owned())
        .chain(argv.iter().map(argv_text))
        .collect::<Vec<_>>()
        .join(" ")
}

fn argv_text(element: &ArgvElement) -> String {
    match element {
        ArgvElement::Literal(value) => value.replace('`', "'").replace('|', "\\|"),
        ArgvElement::Token(token) => format!(
            "{{{}}}",
            match token {
                ArgToken::Files => "files",
                ArgToken::WorkspaceFiles => "workspace-files",
                ArgToken::Workspace => "workspace",
                ArgToken::WorkspaceIndicator => "workspace-indicator",
                ArgToken::ProjectRoot => "project-root",
                ArgToken::ToolExecutable => "tool-executable",
                ArgToken::ExtraArgs => "extra-args",
            }
        ),
    }
}

fn check_scope(scope: CheckScope) -> &'static str {
    match scope {
        CheckScope::TargetFiles => "target-files",
        CheckScope::Workspace => "workspace",
    }
}

fn invocation(value: InvocationGranularity) -> &'static str {
    match value {
        InvocationGranularity::PerFile => "per-file",
        InvocationGranularity::Batch => "batch",
        InvocationGranularity::Workspace => "workspace",
    }
}

fn compatibility_scope(spec: &ToolSpec, phase: &Phase) -> &'static str {
    if spec.workspace_indicator.is_some()
        && !phase.argv.iter().any(|argument| {
            matches!(
                argument,
                ArgvElement::Token(ArgToken::Files | ArgToken::WorkspaceFiles)
            )
        })
    {
        "workspace"
    } else {
        "target-files"
    }
}

fn nonempty_or_dash(values: Vec<String>) -> Vec<String> {
    if values.is_empty() {
        vec!["—".into()]
    } else {
        values
    }
}

fn cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}
