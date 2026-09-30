//! Immediate PostToolUse runner: execution, aggregation, and exact native
//! lowering for Claude Code, Codex, and Antigravity.
//!
//! Neither Claude Code nor Codex shows stderr from a hook that exits 0 (Claude
//! writes it to the debug log; Codex ignores it), while both show the
//! top-level `systemMessage` of a PostToolUse response to the user. User
//! notices are therefore lowered to `systemMessage`, agent feedback to
//! `hookSpecificOutput.additionalContext`, and a `harness-block` decision to
//! the structured `decision: "block"` response so all three audiences survive
//! together. Full tool output stays in diagnostics artifacts.

use crate::convert::{convert_tool_spec, resolve_run_order};
use crate::exec::{
    CommandError, ExecutionSettings, FILE_ARGUMENT_BUDGET_BYTES, FileMatcher, PhaseStatus,
    ToolContext, ToolJob, build_jobs, format_logs, render_command, resolve_worker_count,
    run_phase_command, split_jobs_for_argument_budget,
};
use crate::snapshot::{Snapshot, snapshot_scope};
use crate::util::{
    absolute_from, invalid_data, normalize_path, rel_display, slash_path, truncate_chars,
    unsupported_harness,
};
use hookkit_common::message::{DiagnosticArtifact, DiagnosticReport};
use hookkit_common::{
    NoticeLevel, PostToolUseCommandEnvironment, PostToolUseInput, PostToolUseOutput, UserNotice,
};
use hookkit_core::{BuiltinHarness, HarnessId, RuntimeContext};
use hookkit_file_activity::{FileActivityTarget, observe_post_tool as observe_file_activity};
use hookkit_pkl_config::schema as pkl;
use hookkit_runtime::artifacts::{ArtifactKey, ArtifactManager};
use minijinja::Environment;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// Claude Code truncates a hook's `systemMessage` at 10,000 characters. The
/// runner truncates first (for Codex too) so the pointer to the full
/// diagnostics artifact is never the part that gets cut.
pub(crate) const SYSTEM_MESSAGE_MAX_CHARS: usize = 10_000;

/// Paths the originating tool call may have written, from the shared
/// file-activity analysis (structured, patch, and shell inference).
fn discover_modified_files(input: &PostToolUseInput, context: &RuntimeContext<'_>) -> Vec<PathBuf> {
    observe_file_activity(input, context)
        .evidence()
        .filter_map(|evidence| match &evidence.target {
            FileActivityTarget::Path { path, .. } => Some(normalize_path(path.as_std_path())),
            FileActivityTarget::Workspace { .. } => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Settings shared by every job, derived from the resolved Pkl settings.
pub(crate) fn execution_settings(settings: &pkl::Settings) -> ExecutionSettings {
    ExecutionSettings {
        command_timeout: (settings.command_timeout_seconds > 0)
            .then(|| Duration::from_secs(settings.command_timeout_seconds)),
        ignored_directory_names: settings
            .file_activity
            .clone()
            .unwrap_or_default()
            .ignored_directory_names
            .into_iter()
            .collect(),
    }
}

/// Run an exact aligned input through the Pkl-driven runner.
pub(crate) fn run_post_tool_input(
    post_tool: PostToolUseInput,
    _environment: &PostToolUseCommandEnvironment,
    ctx: &RuntimeContext<'_>,
    config_path: Option<&Path>,
) -> hookkit_core::Result<PostToolUseOutput> {
    let harness = ctx.harness();
    let lowering_warning_artifact = lowering_warning_artifact(&post_tool, ctx);

    // Analyze the tool call once, before any Pkl evaluation: a read-only or
    // failed call must not pay for staging and `pkl eval`.
    let modified = discover_modified_files(&post_tool, ctx)
        .into_iter()
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    if modified.is_empty() {
        return lower_domain_outcome(
            harness,
            RunnerDomainOutcome::Clean,
            lowering_warning_artifact.as_ref(),
        );
    }

    let cwd = ctx
        .workspace_roots()
        .first()
        .map(|root| PathBuf::from(root.as_str()))
        .ok_or_else(|| invalid_data("post-tool-use input has no workspace root".into()))?;
    let loaded = hookkit_pkl_config::discover_and_load(&cwd, config_path)?;

    let project_root = normalize_path(&loaded.project_root);
    let settings = &loaded.config.settings;
    let execution = execution_settings(settings);

    let mut output = RunnerPostToolUseOutput::new(settings.lowering_policy);
    let mut hard_failure: Option<String> = None;
    let mut harness_block: Option<String> = None;

    let tools = resolve_run_order(&loaded.config)?;
    for schema_spec in tools {
        if !schema_spec.enabled {
            continue;
        }
        let spec = convert_tool_spec(schema_spec, &settings.exclude);
        let context = ToolContext {
            spec: &spec,
            project_root: &project_root,
            global_diagnostics_dir: settings.diagnostics_directory.as_deref(),
            settings: &execution,
        };

        let matcher = FileMatcher::new(&spec.file_selection)?;
        let runnable_paths = modified
            .iter()
            .filter(|path| matcher.matches(path, &project_root))
            .cloned()
            .collect::<Vec<_>>();
        if runnable_paths.is_empty() {
            continue;
        }

        let mut jobs = build_jobs(&runnable_paths, &project_root, &spec);
        if spec.phases_use_file_arguments() {
            jobs = split_jobs_for_argument_budget(jobs, FILE_ARGUMENT_BUDGET_BYTES);
        }
        if jobs.is_empty() {
            continue;
        }

        let outcomes = run_jobs(&jobs, &context, settings.jobs);
        let batch_status = accumulate_outcomes(
            outcomes,
            &context,
            ctx,
            settings.missing_tool_policy,
            &mut output,
            &mut hard_failure,
            &mut harness_block,
        )?;

        if harness_block.is_some()
            || hard_failure.is_some()
            || (settings.fail_fast && batch_status.operational_failure)
            || (!settings.continue_after_issues && batch_status.issues)
        {
            break;
        }
    }

    let outcome = if let Some(message) = harness_block {
        RunnerDomainOutcome::HarnessBlock { message, output }
    } else if let Some(message) = hard_failure {
        RunnerDomainOutcome::OperationalFailure { message, output }
    } else if output.is_empty() {
        RunnerDomainOutcome::Clean
    } else {
        RunnerDomainOutcome::Report(output)
    };
    lower_domain_outcome(harness, outcome, lowering_warning_artifact.as_ref())
}

/// Accumulated common output produced by the post-tool runner.
#[derive(Debug, Default)]
pub(crate) struct RunnerPostToolUseOutput {
    notices: Vec<UserNotice>,
    agent_feedback: Vec<String>,
    diagnostics: Vec<DiagnosticReport>,
    harness_block: Option<String>,
    lowering: pkl::LoweringPolicy,
}

impl RunnerPostToolUseOutput {
    pub(crate) fn new(lowering: pkl::LoweringPolicy) -> Self {
        Self {
            lowering,
            ..Self::default()
        }
    }

    pub(crate) fn with_user_notice(mut self, notice: UserNotice) -> Self {
        self.notices.push(notice);
        self
    }

    pub(crate) fn with_agent_feedback(mut self, feedback: impl Into<String>) -> Self {
        self.agent_feedback.push(feedback.into());
        self
    }

    pub(crate) fn with_diagnostic_report(mut self, report: DiagnosticReport) -> Self {
        self.diagnostics.push(report);
        self
    }

    fn with_harness_block(mut self, message: impl Into<String>) -> Self {
        self.harness_block = Some(message.into());
        self
    }

    fn is_empty(&self) -> bool {
        self.notices.is_empty()
            && self.agent_feedback.is_empty()
            && self.diagnostics.is_empty()
            && self.harness_block.is_none()
    }

    /// User-facing text: every notice, plus a pointer for each diagnostic
    /// whose artifact is not already named by a notice. Full diagnostic text
    /// stays in the artifact; only artifact-less diagnostics are inlined.
    fn rendered_user(&self) -> Option<String> {
        let mut lines = self.notices.iter().map(format_notice).collect::<Vec<_>>();
        for diagnostic in &self.diagnostics {
            match &diagnostic.artifact {
                Some(artifact) => {
                    let path = artifact.absolute_path.display().to_string();
                    if !lines.iter().any(|line| line.contains(&path)) {
                        lines.push(format!("{}: {path}", diagnostic.title));
                    }
                }
                None => lines.push(format!("{}:\n{}", diagnostic.title, diagnostic.text.trim())),
            }
        }
        let text = lines.join("\n");
        if text.trim().is_empty() {
            return None;
        }
        let pointer = self
            .diagnostics
            .iter()
            .find_map(|diagnostic| diagnostic.artifact.as_ref())
            .map(|artifact| {
                format!(
                    "\n… truncated; full diagnostics: {}",
                    artifact.absolute_path.display()
                )
            })
            .unwrap_or_else(|| "\n… truncated".to_owned());
        Some(truncate_chars(&text, SYSTEM_MESSAGE_MAX_CHARS, &pointer))
    }

    fn rendered_agent(&self) -> Option<String> {
        let text = self.agent_feedback.join("\n");
        (!text.trim().is_empty()).then_some(text)
    }

    /// Exhaustive text of everything accumulated, for error reports.
    fn describe(&self) -> String {
        self.notices
            .iter()
            .map(format_notice)
            .chain(self.agent_feedback.iter().cloned())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Runner-owned semantic result. Tool policy and classification deliberately do
/// not leak into core/common crates.
#[derive(Debug)]
pub(crate) enum RunnerDomainOutcome {
    /// No messages, diagnostics, or block decision were produced.
    Clean,
    /// Common output should be lowered to the selected harness.
    Report(RunnerPostToolUseOutput),
    /// The configured policy requests a harness-native block decision.
    HarnessBlock {
        /// Reason presented through the harness decision mechanism.
        message: String,
        /// Additional notices, feedback, and diagnostics to lower.
        output: RunnerPostToolUseOutput,
    },
    /// Runner execution failed independently of tool-reported issues.
    OperationalFailure {
        /// Human-readable failure diagnostic.
        message: String,
        /// Output accumulated from earlier tools, reported with the failure.
        output: RunnerPostToolUseOutput,
    },
}

#[derive(Debug)]
pub(crate) struct LoweringWarningArtifact {
    /// The event's artifact directory, or why it cannot be used.
    directory: Result<PathBuf, String>,
    key: ArtifactKey,
}

fn lowering_warning_artifact(
    input: &PostToolUseInput,
    ctx: &RuntimeContext<'_>,
) -> Option<LoweringWarningArtifact> {
    let PostToolUseInput::Antigravity(input) = input else {
        return None;
    };
    let directory = antigravity_artifact_directory(
        ctx.artifact_directory()?.as_str(),
        dirs::home_dir().as_deref(),
    );
    Some(LoweringWarningArtifact {
        directory,
        key: runner_artifact_key(
            ctx,
            format!("post-tool-use-step-{}-lowering-warning", input.step_idx),
        ),
    })
}

/// Resolve Antigravity's `artifactDirectoryPath` to an absolute directory.
///
/// The official payload examples show a literal `~/.gemini/...` path, so a
/// leading `~` (alone or followed by `/`) is expanded against `home`. Any other
/// relative path, including `~user/...`, is refused rather than created
/// relative to the hook's current directory.
fn antigravity_artifact_directory(raw: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    let path = Path::new(raw);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let rest = if raw == "~" {
        Some("")
    } else {
        raw.strip_prefix("~/")
    };
    match (rest, home) {
        (Some(rest), Some(home)) if home.is_absolute() => {
            Ok(hookkit_core::normalize_path(home.join(rest)))
        }
        (Some(_), _) => Err(format!(
            "artifactDirectoryPath `{raw}` starts with `~`, but the home directory is unknown"
        )),
        (None, _) => Err(format!("artifactDirectoryPath `{raw}` is not absolute")),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoweringWarningRecord<'a> {
    format_version: u8,
    kind: &'static str,
    harness: &'static str,
    event: &'static str,
    lowering_policy: &'static str,
    unavailable: LoweringWarningMessages<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoweringWarningMessages<'a> {
    user_notices: &'a [UserNotice],
    diagnostics: Vec<LoweringWarningDiagnostic>,
    agent_feedback: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    block_reason: Option<&'a str>,
    rendered_user: Vec<String>,
    rendered_agent: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoweringWarningDiagnostic {
    title: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    artifact: Option<LoweringWarningDiagnosticArtifact>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoweringWarningDiagnosticArtifact {
    absolute_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_relative_path: Option<String>,
    media_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
}

fn record_antigravity_lowering_warning(
    target: Option<&LoweringWarningArtifact>,
    output: &RunnerPostToolUseOutput,
    rendered_agent: &str,
) -> hookkit_core::Result<PathBuf> {
    let target = target.ok_or_else(|| {
        invalid_data(
            "cannot record Antigravity PostToolUse lowering loss: exact input has no artifact directory"
                .into(),
        )
    })?;
    let directory = target.directory.as_ref().map_err(|reason| {
        invalid_data(format!(
            "cannot record Antigravity PostToolUse lowering loss: {reason}"
        ))
    })?;
    let diagnostics = output
        .diagnostics
        .iter()
        .map(|diagnostic| LoweringWarningDiagnostic {
            title: diagnostic.title.clone(),
            text: diagnostic.text.clone(),
            artifact: diagnostic.artifact.as_ref().map(|artifact| {
                LoweringWarningDiagnosticArtifact {
                    absolute_path: artifact.absolute_path.to_string_lossy().into_owned(),
                    project_relative_path: artifact
                        .project_relative_path
                        .as_ref()
                        .map(|path| path.to_string_lossy().into_owned()),
                    media_type: artifact.media_type.clone(),
                    summary: artifact.summary.clone(),
                }
            }),
        })
        .collect();
    let rendered_user = output
        .notices
        .iter()
        .map(format_notice)
        .chain(output.diagnostics.iter().map(format_diagnostic))
        .collect();
    let record = LoweringWarningRecord {
        format_version: 1,
        kind: "post-tool-use-lowering-loss",
        harness: "antigravity",
        event: "PostToolUse",
        lowering_policy: "best-effort-with-warnings",
        unavailable: LoweringWarningMessages {
            user_notices: &output.notices,
            diagnostics,
            agent_feedback: &output.agent_feedback,
            block_reason: output.harness_block.as_deref(),
            rendered_user,
            rendered_agent,
        },
    };
    let value = serde_json::to_value(record)?;
    let manager = ArtifactManager::new(directory).map_err(|error| {
        invalid_data(format!(
            "cannot create Antigravity lowering-warning artifact directory {}: {error}",
            directory.display()
        ))
    })?;
    manager
        .write_json_unique(&target.key, &value)
        .map_err(|error| {
            invalid_data(format!(
                "cannot write Antigravity lowering-warning artifact in {}: {error}",
                directory.display()
            ))
        })
}

pub(crate) fn lower_domain_outcome(
    harness: &HarnessId,
    outcome: RunnerDomainOutcome,
    lowering_warning_artifact: Option<&LoweringWarningArtifact>,
) -> hookkit_core::Result<PostToolUseOutput> {
    match outcome {
        RunnerDomainOutcome::Clean => lower_report(
            harness,
            RunnerPostToolUseOutput::default(),
            lowering_warning_artifact,
        ),
        RunnerDomainOutcome::Report(output) => {
            lower_report(harness, output, lowering_warning_artifact)
        }
        RunnerDomainOutcome::HarnessBlock { message, output } => lower_report(
            harness,
            output.with_harness_block(message),
            lowering_warning_artifact,
        ),
        RunnerDomainOutcome::OperationalFailure { message, output } => {
            let accumulated = output.describe();
            Err(invalid_data(if accumulated.is_empty() {
                message
            } else {
                format!("{message}\nearlier tool results:\n{accumulated}")
            }))
        }
    }
}

pub(crate) fn lower_report(
    harness: &HarnessId,
    output: RunnerPostToolUseOutput,
    lowering_warning_artifact: Option<&LoweringWarningArtifact>,
) -> hookkit_core::Result<PostToolUseOutput> {
    let user = output.rendered_user();
    let agent = output.rendered_agent();
    let block = output
        .harness_block
        .as_deref()
        .map(|reason| nonblank_block_reason(reason, agent.as_deref()));

    match BuiltinHarness::from_id(harness) {
        Some(BuiltinHarness::ClaudeCode) => {
            let mut native = match &agent {
                Some(agent) => hookkit_claude::protocol::PostToolUseOutput::with_context(agent),
                None => hookkit_claude::protocol::PostToolUseOutput::no_op(),
            };
            if let Some(reason) = block {
                native = native.with_block(reason)?;
            }
            if let Some(user) = user {
                native = native.with_system_message(user)?;
            }
            Ok(PostToolUseOutput::Claude(native))
        }
        Some(BuiltinHarness::Codex) => {
            let mut native = match &agent {
                Some(agent) => hookkit_codex::protocol::PostToolUseOutput::with_context(agent),
                None => hookkit_codex::protocol::PostToolUseOutput::no_op(),
            };
            if let Some(reason) = block {
                native = native.with_block(reason)?;
            }
            if let Some(user) = user {
                native = native.with_system_message(user)?;
            }
            Ok(PostToolUseOutput::Codex(native))
        }
        Some(BuiltinHarness::Antigravity) => {
            // Antigravity's PostToolUse output is an empty object: no user,
            // agent, or block channel exists, so every message is unavailable.
            let unrepresentable = user.is_some() || agent.is_some() || block.is_some();
            let mut native = hookkit_antigravity::PostToolUseOutput::default();
            if !unrepresentable {
                return Ok(PostToolUseOutput::Antigravity(native));
            }
            match output.lowering {
                pkl::LoweringPolicy::Strict => {
                    return Err(invalid_data(
                        "antigravity PostToolUse has no user, agent, or block channel for the runner's messages"
                            .into(),
                    ));
                }
                pkl::LoweringPolicy::BestEffort => {}
                pkl::LoweringPolicy::BestEffortWithWarnings => {
                    let path = record_antigravity_lowering_warning(
                        lowering_warning_artifact,
                        &output,
                        agent.as_deref().unwrap_or_default(),
                    )?;
                    native = native.with_protocol_stderr(format!(
                        "hookkit: Antigravity PostToolUse could not represent user/agent messages; full lowering record: {}",
                        path.display()
                    ))?;
                }
            }
            Ok(PostToolUseOutput::Antigravity(native))
        }
        _ => Err(unsupported_harness(
            harness,
            "the post-tool-use runner has no PostToolUse lowering for this harness",
        )),
    }
}

/// Codex rejects `decision: "block"` with a blank reason (the hook fails open)
/// and Claude requires one, so a block always carries text.
fn nonblank_block_reason(reason: &str, agent: Option<&str>) -> String {
    if !reason.trim().is_empty() {
        return reason.to_owned();
    }
    agent
        .filter(|agent| !agent.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| "A configured post-tool check requested a block.".to_owned())
}

fn format_notice(notice: &UserNotice) -> String {
    match notice.level {
        NoticeLevel::Warning => format!("warning: {}", notice.text),
        NoticeLevel::Error => format!("error: {}", notice.text),
        // `Info` and any level added later render without a prefix.
        _ => notice.text.clone(),
    }
}

fn format_diagnostic(diagnostic: &DiagnosticReport) -> String {
    let mut rendered = format!("{}:\n{}", diagnostic.title, diagnostic.text.trim());
    if let Some(artifact) = &diagnostic.artifact {
        rendered.push_str(&format!("\nartifact: {}", artifact.absolute_path.display()));
    }
    rendered
}

#[derive(Debug, Clone, Copy, Default)]
struct ToolBatchStatus {
    operational_failure: bool,
    issues: bool,
}

// ----------------------------------------------------------------------------
// Per-job execution
// ----------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) enum ToolRunOutcome {
    Completed(CompletedToolOutcome),
    ToolUnavailable {
        phase: String,
        executable: String,
        install_hint: Option<String>,
        changed_files: Vec<PathBuf>,
    },
    ToolFailed {
        phase: String,
        exit_code: Option<i32>,
        diagnostics: String,
        changed_files: Vec<PathBuf>,
    },
}

#[derive(Debug)]
pub(crate) struct CompletedToolOutcome {
    pub issues: IssueState,
    pub changes: ChangeState,
    pub diagnostics: String,
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IssueState {
    Clean,
    Issues,
}

#[derive(Debug)]
pub(crate) enum ChangeState {
    Unchanged,
    Changed { files: Vec<PathBuf> },
}

/// Run a tool's independent jobs, honoring `settings.jobs` for bounded
/// parallelism. Outcomes are returned in job order regardless of which job
/// finishes first, so downstream aggregation stays deterministic.
///
/// Jobs are forced to run serially when two of them share a workspace and the
/// tool may write beyond its target files (argument-budget chunks of one
/// workspace): overlapping snapshot scopes would otherwise misattribute one
/// job's writes to another.
pub(crate) fn run_jobs(
    jobs: &[ToolJob],
    context: &ToolContext<'_>,
    jobs_setting: u32,
) -> Vec<ToolRunOutcome> {
    let shared_workspace = jobs
        .iter()
        .map(|job| &job.workspace_dir)
        .collect::<BTreeSet<_>>()
        .len()
        < jobs.len();
    let jobs_setting = if shared_workspace && context.spec.phases_write_beyond_targets() {
        1
    } else {
        jobs_setting
    };
    let worker_count = resolve_worker_count(jobs_setting, jobs.len());
    if worker_count <= 1 {
        return jobs.iter().map(|job| run_job(job, context)).collect();
    }

    // Work-stealing over a shared cursor: each worker claims the next index via
    // an atomic fetch-add, so uneven job costs balance across threads. The
    // mutex is held only to stash a finished outcome, never across `run_job`.
    let cursor = AtomicUsize::new(0);
    let outcomes = Mutex::new(Vec::<(usize, ToolRunOutcome)>::with_capacity(jobs.len()));

    std::thread::scope(|scope| {
        for _ in 0..worker_count {
            scope.spawn(|| {
                loop {
                    let idx = cursor.fetch_add(1, Ordering::Relaxed);
                    let Some(job) = jobs.get(idx) else { break };
                    let outcome = run_job(job, context);
                    outcomes
                        .lock()
                        .expect("run_jobs outcome mutex poisoned")
                        .push((idx, outcome));
                }
            });
        }
    });

    let mut outcomes = outcomes
        .into_inner()
        .expect("run_jobs outcome mutex poisoned");
    outcomes.sort_by_key(|(idx, _)| *idx);
    outcomes.into_iter().map(|(_, outcome)| outcome).collect()
}

pub(crate) fn run_job(job: &ToolJob, context: &ToolContext<'_>) -> ToolRunOutcome {
    let before = Snapshot::capture(&snapshot_scope(job, context));
    let mut logs = Vec::new();
    let mut saw_issues = false;
    let mut verify_state = None;

    for phase in &context.spec.phases {
        if !phase.enabled {
            continue;
        }

        let command = render_command(phase, job, context);
        let log = run_phase_command(
            phase,
            &command,
            &job.workspace_dir,
            context.settings.command_timeout,
        );

        if let Some(error) = &log.error {
            if *error == CommandError::NotFound {
                return ToolRunOutcome::ToolUnavailable {
                    phase: phase.id.clone(),
                    executable: command.program,
                    install_hint: context.spec.install_hint.clone(),
                    changed_files: changed_files_since(&before, job, context),
                };
            }
            logs.push(log);
            return ToolRunOutcome::ToolFailed {
                phase: phase.id.clone(),
                exit_code: None,
                diagnostics: format_logs(&logs),
                changed_files: changed_files_since(&before, job, context),
            };
        }

        match log.classification {
            Some(PhaseStatus::Clean) => {
                if phase.is_verifier() && verify_state != Some(IssueState::Issues) {
                    // Don't downgrade a prior verifier's Issues verdict.
                    verify_state = Some(IssueState::Clean);
                }
            }
            Some(PhaseStatus::Issues) => {
                saw_issues = true;
                if phase.is_verifier() {
                    verify_state = Some(IssueState::Issues);
                }
            }
            Some(PhaseStatus::Failure) | None => {
                logs.push(log);
                return ToolRunOutcome::ToolFailed {
                    phase: phase.id.clone(),
                    exit_code: logs.last().and_then(|log| log.status),
                    diagnostics: format_logs(&logs),
                    changed_files: changed_files_since(&before, job, context),
                };
            }
        }

        logs.push(log);
    }

    let changed_files = changed_files_since(&before, job, context);
    let issues = verify_state.unwrap_or(if saw_issues {
        IssueState::Issues
    } else {
        IssueState::Clean
    });
    let changes = if changed_files.is_empty() {
        ChangeState::Unchanged
    } else {
        ChangeState::Changed {
            files: changed_files,
        }
    };

    ToolRunOutcome::Completed(CompletedToolOutcome {
        issues,
        changes,
        diagnostics: format_logs(&logs),
        files: job.files.clone(),
    })
}

fn changed_files_since(
    before: &Snapshot,
    job: &ToolJob,
    context: &ToolContext<'_>,
) -> Vec<PathBuf> {
    let after = before.recapture(&snapshot_scope(job, context));
    before.changed_files(&after)
}

// ----------------------------------------------------------------------------
// Outcome aggregation and reporting
// ----------------------------------------------------------------------------

fn accumulate_outcomes(
    outcomes: Vec<ToolRunOutcome>,
    context: &ToolContext<'_>,
    ctx: &RuntimeContext<'_>,
    missing_tool_policy: pkl::MissingToolPolicy,
    output: &mut RunnerPostToolUseOutput,
    hard_failure: &mut Option<String>,
    harness_block: &mut Option<String>,
) -> hookkit_core::Result<ToolBatchStatus> {
    let mut changed_files = BTreeSet::new();
    let mut issue_files = BTreeSet::new();
    let mut issue_diagnostics = Vec::new();
    let mut failure_diagnostics = Vec::new();
    let mut unavailable = Vec::new();

    for outcome in outcomes {
        match outcome {
            ToolRunOutcome::Completed(completed) => {
                if let ChangeState::Changed { files } = completed.changes {
                    changed_files.extend(files);
                }
                if completed.issues == IssueState::Issues {
                    issue_files.extend(completed.files);
                    issue_diagnostics.push(completed.diagnostics);
                }
            }
            ToolRunOutcome::ToolUnavailable {
                phase,
                executable,
                install_hint,
                changed_files: files,
            } => {
                changed_files.extend(files);
                unavailable.push((phase, executable, install_hint));
            }
            ToolRunOutcome::ToolFailed {
                phase,
                exit_code,
                diagnostics,
                changed_files: files,
            } => {
                changed_files.extend(files);
                failure_diagnostics.push((phase, exit_code, diagnostics));
            }
        }
    }

    let status = ToolBatchStatus {
        operational_failure: !unavailable.is_empty() || !failure_diagnostics.is_empty(),
        issues: !issue_diagnostics.is_empty(),
    };

    let changed_paths = changed_files
        .iter()
        .map(|path| rel_display(path, context.project_root))
        .collect::<Vec<_>>();
    let issue_paths = issue_files
        .iter()
        .map(|path| rel_display(path, context.project_root))
        .collect::<Vec<_>>();

    if !unavailable.is_empty() {
        let messages = unavailable
            .iter()
            .map(|(phase, executable, install_hint)| {
                render_unavailable_message(context, phase, executable, install_hint.as_deref())
            })
            .collect::<hookkit_core::Result<Vec<_>>>()?;
        match missing_tool_policy {
            pkl::MissingToolPolicy::UserNotice => {
                for message in messages {
                    *output = std::mem::take(output).with_user_notice(UserNotice::warning(message));
                }
            }
            pkl::MissingToolPolicy::HardFailure => {
                record_changed_files_feedback(output, context, &changed_paths, &issue_paths)?;
                *hard_failure = Some(format!(
                    "tool unavailable with missingToolPolicy=hard-failure: {}",
                    messages.join("; ")
                ));
                return Ok(status);
            }
            pkl::MissingToolPolicy::HarnessBlock => {
                // Earlier writes by this tool's other jobs are still reported
                // so the agent re-reads them alongside the block reason.
                record_changed_files_feedback(output, context, &changed_paths, &issue_paths)?;
                *harness_block = messages.into_iter().next();
                return Ok(status);
            }
        }
    }

    if !failure_diagnostics.is_empty() {
        let diagnostics = failure_diagnostics
            .iter()
            .map(|(phase, exit_code, diagnostics)| {
                format!(
                    "== phase {phase} failed (exit {exit_code:?}) ==\n{}",
                    diagnostics.trim()
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let artifact = write_diagnostics("tool-failure", &diagnostics, context, ctx)?;
        let message = render_failed_message(context, &artifact, failure_diagnostics[0].0.as_str())?;
        *output = std::mem::take(output)
            .with_user_notice(UserNotice::error(message))
            .with_diagnostic_report(report_with_artifact(
                format!("{} failure diagnostics", context.spec.display_name),
                diagnostics,
                artifact,
                context.project_root,
            ));
    }

    if !issue_diagnostics.is_empty() {
        let diagnostics = issue_diagnostics
            .iter()
            .map(|diagnostics| diagnostics.trim())
            .filter(|diagnostics| !diagnostics.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        let artifact = write_diagnostics("tool-issues", &diagnostics, context, ctx)?;
        *output = std::mem::take(output)
            .with_user_notice(UserNotice::warning(format!(
                "{}: issues remain; diagnostics: {}",
                context.spec.display_name,
                artifact.display()
            )))
            .with_diagnostic_report(report_with_artifact(
                format!("{} diagnostics", context.spec.display_name),
                diagnostics,
                artifact.clone(),
                context.project_root,
            ));

        let template = if changed_paths.is_empty() {
            context.spec.messages.issues_agent.clone()
        } else {
            context.spec.messages.issues_changed_agent.clone()
        };
        let rendered = render_template(
            &template,
            context,
            &changed_paths,
            &issue_paths,
            Some(&artifact),
            None,
        )?;
        *output = std::mem::take(output).with_agent_feedback(rendered);
    } else {
        record_changed_files_feedback(output, context, &changed_paths, &issue_paths)?;
    }

    Ok(status)
}

/// Report files a tool changed without leaving issues: a short user notice
/// and the tool's clean-changed agent template.
fn record_changed_files_feedback(
    output: &mut RunnerPostToolUseOutput,
    context: &ToolContext<'_>,
    changed_paths: &[String],
    issue_paths: &[String],
) -> hookkit_core::Result<()> {
    if changed_paths.is_empty() {
        return Ok(());
    }
    *output = std::mem::take(output).with_user_notice(UserNotice::info(format!(
        "{}: changed {}",
        context.spec.display_name,
        changed_paths.join(", ")
    )));
    let rendered = render_template(
        &context.spec.messages.clean_changed_agent,
        context,
        changed_paths,
        issue_paths,
        None,
        None,
    )?;
    *output = std::mem::take(output).with_agent_feedback(rendered);
    Ok(())
}

fn render_unavailable_message(
    context: &ToolContext<'_>,
    phase: &str,
    executable: &str,
    install_hint: Option<&str>,
) -> hookkit_core::Result<String> {
    if let Some(template) = context.spec.messages.unavailable_user.as_ref() {
        return render_template(
            template,
            context,
            &[],
            &[],
            None,
            Some((phase, executable, install_hint)),
        );
    }

    let mut message = format!(
        "{}: `{}` is unavailable while running phase `{phase}`",
        context.spec.display_name, executable
    );
    if let Some(hint) = install_hint {
        message.push_str(&format!("; {hint}"));
    }
    Ok(message)
}

fn render_failed_message(
    context: &ToolContext<'_>,
    diagnostics_path: &Path,
    phase: &str,
) -> hookkit_core::Result<String> {
    if let Some(template) = context.spec.messages.failed_user.as_ref() {
        return render_template(
            template,
            context,
            &[],
            &[],
            Some(diagnostics_path),
            Some((phase, "", None)),
        );
    }

    Ok(format!(
        "{}: phase `{phase}` failed; diagnostics: {}",
        context.spec.display_name,
        diagnostics_path.display()
    ))
}

fn render_template(
    template: &str,
    context: &ToolContext<'_>,
    changed_files: &[String],
    issue_files: &[String],
    diagnostics_path: Option<&Path>,
    phase_error: Option<(&str, &str, Option<&str>)>,
) -> hookkit_core::Result<String> {
    let diagnostics_path_text = diagnostics_path
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_default();
    let diagnostics_rel_path = diagnostics_path
        .and_then(|path| path.strip_prefix(context.project_root).ok())
        .map(slash_path)
        .unwrap_or_default();
    let (phase, executable, install_hint) = phase_error.unwrap_or(("", "", None));
    let json_context = serde_json::json!({
        "tool": context.spec.display_name,
        "tool_id": context.spec.id,
        "changed_files": changed_files,
        "issue_files": issue_files,
        "diagnostics_path": diagnostics_path_text,
        "diagnostics_absolute_path": diagnostics_path_text,
        "diagnostics_rel_path": diagnostics_rel_path,
        "diagnostics_project_path": diagnostics_rel_path,
        "project_root": context.project_root.to_string_lossy(),
        "phase": phase,
        "executable": executable,
        "install_hint": install_hint.unwrap_or(""),
    });

    Environment::new()
        .render_str(template, &json_context)
        .map_err(|e| invalid_data(format!("failed to render message template: {e}")))
}

/// Resolve the directory that receives immediate-runner diagnostics:
/// the tool's own `diagnostics.directory`, else `settings.diagnosticsDirectory`
/// (both relative to the project root), else the per-user
/// [`ArtifactManager::default_temp_dir`].
pub(crate) fn diagnostics_directory(
    tool_directory: Option<&str>,
    global_directory: Option<&str>,
    project_root: &Path,
) -> PathBuf {
    match tool_directory.or(global_directory) {
        Some(dir) => absolute_from(Path::new(dir), project_root),
        None => ArtifactManager::default_temp_dir(),
    }
}

/// The artifact manager for [`diagnostics_directory`]. Without a configured
/// directory it is [`ArtifactManager::in_temp_dir`], which refuses a shared
/// temporary directory another user created first.
fn diagnostics_manager(
    tool_directory: Option<&str>,
    global_directory: Option<&str>,
    project_root: &Path,
) -> std::io::Result<ArtifactManager> {
    match tool_directory.or(global_directory) {
        Some(_) => ArtifactManager::new(diagnostics_directory(
            tool_directory,
            global_directory,
            project_root,
        )),
        None => ArtifactManager::in_temp_dir(),
    }
}

fn write_diagnostics(
    label: &str,
    diagnostics: &str,
    context: &ToolContext<'_>,
    ctx: &RuntimeContext<'_>,
) -> hookkit_core::Result<PathBuf> {
    let manager = diagnostics_manager(
        context.spec.diagnostics_directory.as_deref(),
        context.global_diagnostics_dir,
        context.project_root,
    )?;
    manager
        .write_text(
            &runner_artifact_key(ctx, format!("{}-{label}", context.spec.id)),
            diagnostics,
        )
        .map_err(Into::into)
}

pub(crate) fn runner_artifact_key(ctx: &RuntimeContext<'_>, label: String) -> ArtifactKey {
    let session = ctx
        .session_id()
        .map(ToString::to_string)
        .or_else(|| ctx.conversation_id().map(ToString::to_string))
        .unwrap_or_else(|| "unknown-session".to_string());
    let mut key = ArtifactKey::new(session, label);
    if let Some(turn) = ctx.turn_id() {
        key = key.with_turn(turn.to_string());
    }
    if let Some(tool_call) = ctx.tool_call_id() {
        key = key.with_tool_use(tool_call.to_string());
    }
    key
}

fn report_with_artifact(
    title: String,
    diagnostics: String,
    artifact_path: PathBuf,
    project_root: &Path,
) -> DiagnosticReport {
    let mut artifact = DiagnosticArtifact::new(artifact_path.clone(), "text/plain");
    if let Ok(rel_path) = artifact_path.strip_prefix(project_root) {
        artifact = artifact.with_project_relative_path(rel_path);
    }
    artifact = artifact.with_summary(&title);
    DiagnosticReport::new(title, diagnostics).with_artifact(artifact)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::ExecutionSettings;
    use crate::spec::{CommandArgTemplate, PhaseMode, ToolPhase, ToolSpec};
    use hookkit_core::EventSpec as _;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn claude_stdout(output: PostToolUseOutput) -> (serde_json::Value, Vec<u8>, u8) {
        let PostToolUseOutput::Claude(native) = output else {
            panic!("expected Claude output");
        };
        let emission = hookkit_claude::protocol::PostToolUse::emit(native).unwrap();
        let stdout = if emission.stdout().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_slice(emission.stdout()).unwrap()
        };
        (stdout, emission.stderr().to_vec(), emission.exit_code())
    }

    fn codex_stdout(output: PostToolUseOutput) -> (serde_json::Value, Vec<u8>, u8) {
        let PostToolUseOutput::Codex(native) = output else {
            panic!("expected Codex output");
        };
        let emission = hookkit_codex::protocol::PostToolUse::emit(native).unwrap();
        let stdout = if emission.stdout().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_slice(emission.stdout()).unwrap()
        };
        (stdout, emission.stderr().to_vec(), emission.exit_code())
    }

    fn report_with_notice(policy: pkl::LoweringPolicy) -> RunnerPostToolUseOutput {
        RunnerPostToolUseOutput::new(policy)
            .with_user_notice(UserNotice::warning(
                "ruff: issues remain; diagnostics: /repo/.agent-hook-kit/ruff.txt",
            ))
            .with_diagnostic_report(
                DiagnosticReport::new("ruff diagnostics", "a.py:1:1: F401 unused import")
                    .with_artifact(DiagnosticArtifact::new(
                        "/repo/.agent-hook-kit/ruff.txt",
                        "text/plain",
                    )),
            )
            .with_agent_feedback("ruff reports issues; inspect /repo/.agent-hook-kit/ruff.txt.")
    }

    #[test]
    fn domain_outcomes_keep_clean_and_failure_distinct() {
        assert!(matches!(
            lower_domain_outcome(&HarnessId::CLAUDE_CODE, RunnerDomainOutcome::Clean, None)
                .unwrap(),
            PostToolUseOutput::Claude(_)
        ));
        let failure = lower_domain_outcome(
            &HarnessId::CLAUDE_CODE,
            RunnerDomainOutcome::OperationalFailure {
                message: "checker crashed".into(),
                output: RunnerPostToolUseOutput::default()
                    .with_agent_feedback("prettier changed a.ts; re-read changed files"),
            },
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(failure.contains("checker crashed"), "{failure}");
        assert!(
            failure.contains("prettier changed a.ts"),
            "earlier tools' feedback is kept on the failure path: {failure}"
        );
        assert!(matches!(
            lower_domain_outcome(&HarnessId::ANTIGRAVITY, RunnerDomainOutcome::Clean, None)
                .unwrap(),
            PostToolUseOutput::Antigravity(_)
        ));
    }

    #[test]
    fn claude_and_codex_show_user_notices_through_system_message_not_stderr() {
        for strict in [false, true] {
            let policy = if strict {
                pkl::LoweringPolicy::Strict
            } else {
                pkl::LoweringPolicy::BestEffortWithWarnings
            };
            for (stdout, stderr, exit) in [
                claude_stdout(
                    lower_report(&HarnessId::CLAUDE_CODE, report_with_notice(policy), None)
                        .unwrap(),
                ),
                codex_stdout(
                    lower_report(&HarnessId::CODEX, report_with_notice(policy), None).unwrap(),
                ),
            ] {
                assert_eq!(exit, 0);
                assert!(stderr.is_empty(), "exit-0 stderr is never shown");
                let system = stdout["systemMessage"].as_str().expect("systemMessage");
                assert!(system.contains("warning: ruff: issues remain"), "{system}");
                assert!(
                    !system.contains("F401"),
                    "full diagnostics stay in the artifact: {system}"
                );
                assert_eq!(
                    stdout["hookSpecificOutput"]["additionalContext"],
                    "ruff reports issues; inspect /repo/.agent-hook-kit/ruff.txt."
                );
            }
        }
    }

    #[test]
    fn diagnostics_without_a_notice_are_pointed_to_by_artifact_path() {
        let output = RunnerPostToolUseOutput::new(pkl::LoweringPolicy::BestEffort)
            .with_diagnostic_report(
                DiagnosticReport::new("tsc diagnostics", "full output").with_artifact(
                    DiagnosticArtifact::new("/repo/.agent-hook-kit/tsc.txt", "text/plain"),
                ),
            );
        let (stdout, _, _) =
            claude_stdout(lower_report(&HarnessId::CLAUDE_CODE, output, None).unwrap());
        assert_eq!(
            stdout["systemMessage"],
            "tsc diagnostics: /repo/.agent-hook-kit/tsc.txt"
        );
    }

    #[test]
    fn long_user_messages_are_truncated_with_an_artifact_pointer() {
        let output = RunnerPostToolUseOutput::new(pkl::LoweringPolicy::BestEffort)
            .with_user_notice(UserNotice::warning("x".repeat(20_000)))
            .with_diagnostic_report(DiagnosticReport::new("big", "y").with_artifact(
                DiagnosticArtifact::new("/repo/.agent-hook-kit/big.txt", "text/plain"),
            ));
        let (stdout, _, _) =
            claude_stdout(lower_report(&HarnessId::CLAUDE_CODE, output, None).unwrap());
        let system = stdout["systemMessage"].as_str().unwrap();
        assert!(system.chars().count() <= SYSTEM_MESSAGE_MAX_CHARS);
        assert!(
            system.ends_with("truncated; full diagnostics: /repo/.agent-hook-kit/big.txt"),
            "{}",
            &system[system.len() - 120..]
        );
    }

    #[test]
    fn harness_block_keeps_earlier_feedback_and_notices() {
        let outcome = || RunnerDomainOutcome::HarnessBlock {
            message: "eslint: `eslint` is unavailable while running phase `check`".into(),
            output: RunnerPostToolUseOutput::new(pkl::LoweringPolicy::BestEffortWithWarnings)
                .with_user_notice(UserNotice::info("prettier: changed a.ts"))
                .with_agent_feedback("prettier changed a.ts; re-read changed files"),
        };
        for (stdout, stderr, exit) in [
            claude_stdout(lower_domain_outcome(&HarnessId::CLAUDE_CODE, outcome(), None).unwrap()),
            codex_stdout(lower_domain_outcome(&HarnessId::CODEX, outcome(), None).unwrap()),
        ] {
            assert_eq!(exit, 0);
            assert!(stderr.is_empty());
            assert_eq!(stdout["decision"], "block");
            assert_eq!(
                stdout["reason"],
                "eslint: `eslint` is unavailable while running phase `check`"
            );
            assert_eq!(
                stdout["hookSpecificOutput"]["additionalContext"],
                "prettier changed a.ts; re-read changed files"
            );
            assert_eq!(stdout["systemMessage"], "prettier: changed a.ts");
        }
    }

    #[test]
    fn harness_block_on_antigravity_follows_the_lowering_policy() {
        let strict = RunnerDomainOutcome::HarnessBlock {
            message: "missing tool".into(),
            output: RunnerPostToolUseOutput::new(pkl::LoweringPolicy::Strict),
        };
        let error = lower_domain_outcome(&HarnessId::ANTIGRAVITY, strict, None)
            .unwrap_err()
            .to_string();
        assert!(!error.contains("does not support"), "{error}");

        let best_effort = RunnerDomainOutcome::HarnessBlock {
            message: "missing tool".into(),
            output: RunnerPostToolUseOutput::new(pkl::LoweringPolicy::BestEffort),
        };
        let native = match lower_domain_outcome(&HarnessId::ANTIGRAVITY, best_effort, None).unwrap()
        {
            PostToolUseOutput::Antigravity(native) => native,
            _ => panic!("expected Antigravity output"),
        };
        let emission = hookkit_antigravity::PostToolUse::emit(native).unwrap();
        assert_eq!(emission.stdout(), b"{}");
    }

    #[test]
    fn blank_block_reasons_are_replaced() {
        assert_eq!(nonblank_block_reason("  ", Some("fix it")), "fix it");
        assert!(!nonblank_block_reason("", None).trim().is_empty());
        assert_eq!(nonblank_block_reason("reason", Some("fix it")), "reason");
    }

    #[test]
    fn antigravity_warning_lowering_records_full_loss_and_preserves_exact_stdout() {
        let directory = unique_test_directory("antigravity-lowering-warning");
        let target = LoweringWarningArtifact {
            directory: Ok(directory.clone()),
            key: ArtifactKey::new("conversation-7", "post-tool-use-step-3-lowering-warning"),
        };
        let diagnostic_artifact =
            DiagnosticArtifact::new(directory.join("complete-diagnostic.txt"), "text/plain")
                .with_project_relative_path(".agent-hook-kit/complete-diagnostic.txt")
                .with_summary("complete tool output");
        let warning = RunnerPostToolUseOutput::new(pkl::LoweringPolicy::BestEffortWithWarnings)
            .with_user_notice(UserNotice::warning("review every diagnostic line"))
            .with_diagnostic_report(
                DiagnosticReport::new("lint report", "first line\nsecond line")
                    .with_artifact(diagnostic_artifact),
            )
            .with_agent_feedback("re-read generated.rs\nthen repair it");

        let native = match lower_report(&HarnessId::ANTIGRAVITY, warning, Some(&target)).unwrap() {
            PostToolUseOutput::Antigravity(native) => native,
            _ => panic!("expected Antigravity output"),
        };
        let emission = hookkit_antigravity::PostToolUse::emit(native).unwrap();
        let artifact_path = directory.join(format!("{}.json", target.key.filename()));

        assert_eq!(emission.stdout(), b"{}");
        assert_eq!(emission.exit_code(), 0);
        assert!(
            String::from_utf8_lossy(emission.stderr())
                .contains(&artifact_path.to_string_lossy().into_owned())
        );
        let record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&artifact_path).unwrap()).unwrap();
        assert_eq!(record["formatVersion"], 1);
        assert_eq!(record["kind"], "post-tool-use-lowering-loss");
        assert_eq!(record["harness"], "antigravity");
        assert_eq!(record["event"], "PostToolUse");
        assert_eq!(record["loweringPolicy"], "best-effort-with-warnings");
        assert_eq!(
            record["unavailable"]["userNotices"][0]["text"],
            "review every diagnostic line"
        );
        assert_eq!(
            record["unavailable"]["diagnostics"][0]["text"],
            "first line\nsecond line"
        );
        assert_eq!(
            record["unavailable"]["diagnostics"][0]["artifact"]["summary"],
            "complete tool output"
        );
        assert_eq!(
            record["unavailable"]["agentFeedback"][0],
            "re-read generated.rs\nthen repair it"
        );
        assert_eq!(
            record["unavailable"]["renderedAgent"],
            "re-read generated.rs\nthen repair it"
        );

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn antigravity_artifact_directories_expand_home_and_refuse_relative_paths() {
        let home = Path::new("/home/agent");
        assert_eq!(
            antigravity_artifact_directory("/tmp/artifacts", Some(home)),
            Ok(PathBuf::from("/tmp/artifacts"))
        );
        // The official examples use a literal `~/` prefix.
        assert_eq!(
            antigravity_artifact_directory("~/.gemini/antigravity/brain/c1", Some(home)),
            Ok(PathBuf::from("/home/agent/.gemini/antigravity/brain/c1"))
        );
        assert_eq!(
            antigravity_artifact_directory("~", Some(home)),
            Ok(PathBuf::from("/home/agent"))
        );
        for (raw, home) in [
            ("~/.gemini/brain", None),
            ("~/.gemini/brain", Some(Path::new("relative-home"))),
            ("~other/.gemini/brain", Some(home)),
            ("brain/c1", Some(home)),
            ("", Some(home)),
        ] {
            assert!(
                antigravity_artifact_directory(raw, home).is_err(),
                "{raw:?} with {home:?}"
            );
        }
    }

    #[test]
    fn antigravity_warning_lowering_refuses_an_unusable_artifact_directory() {
        let target = LoweringWarningArtifact {
            directory: antigravity_artifact_directory("brain/c1", None),
            key: ArtifactKey::new("conversation-10", "post-tool-use-step-6-lowering-warning"),
        };
        let warning = RunnerPostToolUseOutput::new(pkl::LoweringPolicy::BestEffortWithWarnings)
            .with_agent_feedback("this must be retained");
        let error = lower_report(&HarnessId::ANTIGRAVITY, warning, Some(&target)).unwrap_err();
        assert!(error.to_string().contains("not absolute"), "{error}");
        assert!(!Path::new("brain").exists());
    }

    #[test]
    fn diagnostics_fall_back_to_the_per_user_artifact_directory() {
        let project = Path::new("/work/project");
        assert_eq!(
            diagnostics_directory(None, None, project),
            ArtifactManager::default_temp_dir()
        );
        #[cfg(unix)]
        assert_ne!(
            diagnostics_directory(None, None, project),
            std::env::temp_dir().join("hookkit-artifacts"),
            "the shared, predictable directory must not be the default on Unix"
        );
        assert_eq!(
            diagnostics_directory(Some("tool"), Some("global"), project),
            project.join("tool")
        );
        assert_eq!(
            diagnostics_directory(None, Some("global"), project),
            project.join("global")
        );
        assert_eq!(
            diagnostics_manager(None, None, project).unwrap().base_dir(),
            ArtifactManager::default_temp_dir()
        );
    }

    #[test]
    fn antigravity_best_effort_omits_unavailable_messages_without_a_record() {
        let directory = unique_test_directory("antigravity-lowering-best-effort");
        let target = LoweringWarningArtifact {
            directory: Ok(directory.clone()),
            key: ArtifactKey::new("conversation-8", "post-tool-use-step-4-lowering-warning"),
        };
        let best_effort = RunnerPostToolUseOutput::new(pkl::LoweringPolicy::BestEffort)
            .with_user_notice(UserNotice::warning("review diagnostics"))
            .with_agent_feedback("re-read generated.rs");
        let native =
            match lower_report(&HarnessId::ANTIGRAVITY, best_effort, Some(&target)).unwrap() {
                PostToolUseOutput::Antigravity(native) => native,
                _ => panic!("expected Antigravity output"),
            };
        let emission = hookkit_antigravity::PostToolUse::emit(native).unwrap();

        assert_eq!(emission.stdout(), b"{}");
        assert!(emission.stderr().is_empty());
        assert!(
            !directory
                .join(format!("{}.json", target.key.filename()))
                .exists()
        );
    }

    #[test]
    fn antigravity_warning_lowering_never_overwrites_a_reused_step_key() {
        let directory = unique_test_directory("antigravity-lowering-collision");
        let target = LoweringWarningArtifact {
            directory: Ok(directory.clone()),
            key: ArtifactKey::new("conversation-8", "post-tool-use-step-0-lowering-warning"),
        };
        let lower = |feedback: &str| {
            let output = RunnerPostToolUseOutput::new(pkl::LoweringPolicy::BestEffortWithWarnings)
                .with_agent_feedback(feedback);
            let native = match lower_report(&HarnessId::ANTIGRAVITY, output, Some(&target)).unwrap()
            {
                PostToolUseOutput::Antigravity(native) => native,
                _ => panic!("expected Antigravity output"),
            };
            let emission = hookkit_antigravity::PostToolUse::emit(native).unwrap();
            let stderr = String::from_utf8(emission.stderr().to_vec()).unwrap();
            PathBuf::from(stderr.rsplit_once(": ").unwrap().1)
        };

        let first = lower("first invocation");
        let second = lower("second invocation");

        assert_ne!(first, second);
        let first_record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(first).unwrap()).unwrap();
        let second_record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(second).unwrap()).unwrap();
        assert_eq!(
            first_record["unavailable"]["agentFeedback"][0],
            "first invocation"
        );
        assert_eq!(
            second_record["unavailable"]["agentFeedback"][0],
            "second invocation"
        );

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn antigravity_strict_lowering_errors_instead_of_recording_loss() {
        let strict = RunnerPostToolUseOutput::new(pkl::LoweringPolicy::Strict)
            .with_agent_feedback("re-read generated.rs");
        assert!(lower_report(&HarnessId::ANTIGRAVITY, strict, None).is_err());
    }

    #[test]
    fn antigravity_warning_lowering_errors_when_the_record_cannot_be_written() {
        let directory = unique_test_directory("antigravity-lowering-write-failure");
        std::fs::create_dir_all(&directory).unwrap();
        let not_a_directory = directory.join("regular-file");
        std::fs::write(&not_a_directory, "occupied").unwrap();
        let target = LoweringWarningArtifact {
            directory: Ok(not_a_directory),
            key: ArtifactKey::new("conversation-9", "post-tool-use-step-5-lowering-warning"),
        };
        let warning = RunnerPostToolUseOutput::new(pkl::LoweringPolicy::BestEffortWithWarnings)
            .with_agent_feedback("this must be retained");

        assert!(lower_report(&HarnessId::ANTIGRAVITY, warning, Some(&target)).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn unique_test_directory(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "hookkit-tool-runner-{label}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn job_with_file(root: &Path, name: &str) -> ToolJob {
        ToolJob {
            workspace_dir: root.to_path_buf(),
            workspace_indicator: None,
            files: vec![root.join(name)],
        }
    }

    fn completed_files(outcome: &ToolRunOutcome) -> &[PathBuf] {
        match outcome {
            ToolRunOutcome::Completed(completed) => completed.files.as_slice(),
            other => panic!("expected Completed outcome, got {other:?}"),
        }
    }

    #[test]
    fn run_jobs_preserves_order_across_concurrency_levels() {
        // A spec with no phases makes `run_job` complete without spawning any
        // process or touching the filesystem, so this stays fast and
        // deterministic while still exercising the parallel execution path.
        let root = PathBuf::from("/tmp/hookkit-run-jobs-test");
        let spec = ToolSpec::new("test-tool", "Test Tool", "test-exec");
        let settings = ExecutionSettings::default();
        let context = ToolContext {
            spec: &spec,
            project_root: &root,
            global_diagnostics_dir: None,
            settings: &settings,
        };
        let jobs: Vec<ToolJob> = (0..16)
            .map(|i| job_with_file(&root, &format!("file-{i:02}.rs")))
            .collect();

        // Serial (1), auto (0), and bounded-parallel (>1, including more than
        // CPUs) must all return one outcome per job, in job order.
        for jobs_setting in [0u32, 1, 4, 32] {
            let outcomes = run_jobs(&jobs, &context, jobs_setting);
            assert_eq!(outcomes.len(), jobs.len(), "jobs_setting={jobs_setting}");
            for (job, outcome) in jobs.iter().zip(&outcomes) {
                assert_eq!(
                    completed_files(outcome),
                    job.files.as_slice(),
                    "jobs_setting={jobs_setting}: outcome order must match job order",
                );
            }
        }
    }

    #[cfg(unix)]
    fn fake_executable(root: &Path, name: &str, body: &str) -> PathBuf {
        let fake = root.join(name);
        std::fs::write(&fake, body).expect("write fake executable");
        let mut permissions = std::fs::metadata(&fake)
            .expect("read fake executable metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&fake, permissions).expect("make fake executable runnable");
        fake
    }

    #[cfg(unix)]
    #[test]
    fn hermetic_fake_executable_smoke() {
        let root =
            std::env::temp_dir().join(format!("hookkit-hermetic-smoke-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create smoke directory");

        let fake = fake_executable(
            &root,
            "fake-checker",
            "#!/bin/sh\nprintf 'fake checker clean\\n'\n",
        );
        let target = root.join("input.rs");
        std::fs::write(&target, "fn main() {}\n").expect("write smoke input");
        let spec = ToolSpec::new("fake", "Fake checker", fake.to_string_lossy().into_owned())
            .with_phase(
                ToolPhase::new("verify", PhaseMode::Verify).with_args([CommandArgTemplate::Files]),
            );
        let settings = ExecutionSettings::default();
        let context = ToolContext {
            spec: &spec,
            project_root: &root,
            global_diagnostics_dir: None,
            settings: &settings,
        };
        let job = job_with_file(&root, "input.rs");

        let outcome = run_job(&job, &context);
        let ToolRunOutcome::Completed(completed) = outcome else {
            panic!("expected completed fake-tool run");
        };
        assert_eq!(completed.issues, IssueState::Clean);
        assert!(matches!(completed.changes, ChangeState::Unchanged));
        assert!(completed.diagnostics.contains("fake checker clean"));

        std::fs::remove_dir_all(&root).expect("remove smoke directory");
    }

    #[cfg(unix)]
    #[test]
    fn hung_tools_time_out_as_operational_failures() {
        let root =
            std::env::temp_dir().join(format!("hookkit-timeout-smoke-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create timeout directory");
        let fake = fake_executable(&root, "hung-checker", "#!/bin/sh\nsleep 30\n");
        std::fs::write(root.join("input.rs"), "fn main() {}\n").expect("write input");
        let spec = ToolSpec::new("hung", "Hung checker", fake.to_string_lossy().into_owned())
            .with_phase(
                ToolPhase::new("verify", PhaseMode::Verify).with_args([CommandArgTemplate::Files]),
            );
        let settings = ExecutionSettings {
            command_timeout: Some(std::time::Duration::from_millis(200)),
            ..ExecutionSettings::default()
        };
        let context = ToolContext {
            spec: &spec,
            project_root: &root,
            global_diagnostics_dir: None,
            settings: &settings,
        };
        let started = std::time::Instant::now();
        let outcome = run_job(&job_with_file(&root, "input.rs"), &context);
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        let ToolRunOutcome::ToolFailed { diagnostics, .. } = outcome else {
            panic!("expected a timed-out tool to fail, got {outcome:?}");
        };
        assert!(diagnostics.contains("timed out"), "{diagnostics}");

        std::fs::remove_dir_all(&root).expect("remove timeout directory");
    }
}
