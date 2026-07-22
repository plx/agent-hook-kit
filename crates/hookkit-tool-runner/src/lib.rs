//! Reusable runners for immediate post-tool and session-batched completion hooks.
//!
//! The runner reads a harness-native post-tool-use event from stdin, loads the
//! Pkl-driven tool catalog through [`hookkit_pkl_config`], runs each tool in
//! the configured order, and lowers a unified result to the selected harness.
//! The completion runner consumes exact snapshots from
//! [`hookkit_session_state`] and commits detailed run bundles before deciding
//! whether a turn may stop.

mod deferred;

pub use deferred::{
    ArtifactClassification, CheckOutcome, CommandPhase, CoverageGap, DeferredRunResult,
    FileAssessment, FileResult, FileStatus, OperationalProblem, RunArtifact, ToolReport,
    ToolReportRef,
};
use deferred::{
    DeferredLog, DeferredReporter, RenderedBuckets, RenderedMessages, ScheduledWorkflow,
    StopLoweringMetadata, TemplateRun, execute_deferred_workflows, plan_stop_lowering,
};

use globset::{Glob, GlobSet, GlobSetBuilder};
use hookkit_common::message::{DiagnosticArtifact, DiagnosticReport};
use hookkit_common::{
    NoticeLevel, PostToolUseCommandEnvironment, PostToolUseInput, PostToolUseOutput,
    TurnCompletionCommandEnvironment, TurnCompletionInput, TurnCompletionOutput, UserNotice,
};
use hookkit_core::{HarnessId, HookkitError, RuntimeContext, Utf8PathBuf};
use hookkit_file_activity::{
    FileActivityEvent, FileActivityStore, FileActivityTarget, PendingFileActivity,
    ReconciliationOptions, ResolveOptions, VcsFallback, observe_post_tool as observe_file_activity,
    reconcile, resolve_files,
};
use hookkit_pkl_config::schema as pkl;
use hookkit_runtime::artifacts::{ArtifactKey, ArtifactManager};
use hookkit_session_state::{
    EntityOperationError, EntityOutcome, EntityView, FamilyId, RunBundle, SessionState,
    StateFamily, StateRoot, UtcTimestamp,
};
use minijinja::Environment;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const DEFAULT_CLEAN_CHANGED_AGENT: &str = "{{ tool }} changed {{ changed_files | join(\", \") }}; re-read changed files before editing further.";
const DEFAULT_ISSUES_AGENT: &str =
    "{{ tool }} reports issues; inspect diagnostics at {{ diagnostics_path }}.";
const DEFAULT_ISSUES_CHANGED_AGENT: &str = "{{ tool }} changed {{ changed_files | join(\", \") }} and issues remain; re-read changed files, then inspect diagnostics at {{ diagnostics_path }}.";

const BINARY_NAME: &str = "post-tool-use-agent-hook";
const TURN_COMPLETION_BINARY_NAME: &str = "turn-completion-agent-hook";
const SESSION_START_BINARY_NAME: &str = "session-start-state-agent-hook";
const FILE_ACTIVITY_BINARY_NAME: &str = "file-activity-agent-hook";
const BATCHED_TOOLS_FAMILY: &str = "agent-hook-kit.batched-tools";

// ----------------------------------------------------------------------------
// Public runtime types
// ----------------------------------------------------------------------------

/// A complete reusable hook CLI specification for one external tool.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub id: String,
    pub display_name: String,
    pub executable: String,
    pub install_hint: Option<String>,
    pub file_selection: FileSelection,
    pub workspace_indicator: Option<String>,
    pub workflows: Vec<ToolWorkflow>,
    pub phases: Vec<ToolPhase>,
    pub messages: ToolMessages,
    pub diagnostics_directory: Option<String>,
    pub enabled: bool,
}

impl ToolSpec {
    pub fn new(
        id: impl Into<String>,
        display_name: impl Into<String>,
        executable: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            display_name: display_name.into(),
            executable: executable.into(),
            install_hint: None,
            file_selection: FileSelection::default(),
            workspace_indicator: None,
            workflows: Vec::new(),
            phases: Vec::new(),
            messages: ToolMessages::default(),
            diagnostics_directory: None,
            enabled: true,
        }
    }

    pub fn with_install_hint(mut self, hint: impl Into<String>) -> Self {
        self.install_hint = Some(hint.into());
        self
    }

    pub fn with_file_selection(mut self, file_selection: FileSelection) -> Self {
        self.file_selection = file_selection;
        self
    }

    pub fn with_workspace_indicator(mut self, indicator: impl Into<String>) -> Self {
        self.workspace_indicator = Some(indicator.into());
        self
    }

    pub fn with_phase(mut self, phase: ToolPhase) -> Self {
        self.phases.push(phase);
        self
    }

    pub fn with_workflow(mut self, workflow: ToolWorkflow) -> Self {
        self.workflows.push(workflow);
        self
    }

    pub fn with_messages(mut self, messages: ToolMessages) -> Self {
        self.messages = messages;
        self
    }
}

/// One Stop-time non-mutating check and optional automatic remedy.
#[derive(Debug, Clone)]
pub struct ToolWorkflow {
    pub id: String,
    pub check: Option<ToolPhase>,
    pub remedy: Option<ToolPhase>,
    pub check_scope: CheckScope,
    pub invocation: InvocationGranularity,
    pub compatibility_translation: bool,
    pub enabled: bool,
}

/// Inputs whose writes invalidate a workflow's prior check.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CheckScope {
    #[default]
    TargetFiles,
    Workspace,
}

/// How a workflow divides the selected files into invocations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InvocationGranularity {
    PerFile,
    #[default]
    Batch,
    Workspace,
}

/// Include/exclude globs used to select modified files.
#[derive(Debug, Clone, Default)]
pub struct FileSelection {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

impl FileSelection {
    pub fn include(patterns: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            include: patterns.into_iter().map(Into::into).collect(),
            exclude: Vec::new(),
        }
    }

    pub fn with_exclude(mut self, patterns: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.exclude = patterns.into_iter().map(Into::into).collect();
        self
    }
}

/// One external command phase.
#[derive(Debug, Clone)]
pub struct ToolPhase {
    pub id: String,
    pub mode: PhaseMode,
    pub program: Option<String>,
    pub args: Vec<CommandArgTemplate>,
    pub exit_codes: ExitCodePolicy,
    pub issues_on_stdout: bool,
    pub writes: WriteBehavior,
    pub extra_args: Vec<String>,
    pub enabled: bool,
}

impl ToolPhase {
    pub fn new(id: impl Into<String>, mode: PhaseMode) -> Self {
        Self {
            id: id.into(),
            mode,
            program: None,
            args: Vec::new(),
            exit_codes: ExitCodePolicy::default(),
            issues_on_stdout: false,
            writes: WriteBehavior::None,
            extra_args: Vec::new(),
            enabled: true,
        }
    }

    pub fn with_program(mut self, program: impl Into<String>) -> Self {
        self.program = Some(program.into());
        self
    }

    pub fn with_args(mut self, args: impl IntoIterator<Item = CommandArgTemplate>) -> Self {
        self.args = args.into_iter().collect();
        self
    }

    pub fn with_exit_codes(mut self, exit_codes: ExitCodePolicy) -> Self {
        self.exit_codes = exit_codes;
        self
    }

    pub fn with_writes(mut self, writes: WriteBehavior) -> Self {
        self.writes = writes;
        self
    }

    pub fn with_extra_args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.extra_args = args.into_iter().map(Into::into).collect();
        self
    }

    fn is_verifier(&self) -> bool {
        matches!(self.mode, PhaseMode::Verify | PhaseMode::CheckOnly)
    }
}

/// High-level phase purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseMode {
    Format,
    Fix,
    Verify,
    CheckOnly,
}

/// What a phase may write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteBehavior {
    None,
    TargetFiles,
    MatchingGlobs,
    Workspace,
}

/// Command argument template.
#[derive(Debug, Clone)]
pub enum CommandArgTemplate {
    Literal(String),
    Files,
    WorkspaceFiles,
    Workspace,
    WorkspaceIndicator,
    ProjectRoot,
    ToolExecutable,
    ExtraArgs,
}

impl CommandArgTemplate {
    pub fn literal(value: impl Into<String>) -> Self {
        Self::Literal(value.into())
    }
}

/// Exit-code classification for a phase.
#[derive(Debug, Clone)]
pub struct ExitCodePolicy {
    pub clean: Vec<i32>,
    pub issues: Vec<i32>,
    pub failure: Vec<i32>,
    pub unexpected: UnexpectedExitPolicy,
}

impl Default for ExitCodePolicy {
    fn default() -> Self {
        Self {
            clean: vec![0],
            issues: Vec::new(),
            failure: Vec::new(),
            unexpected: UnexpectedExitPolicy::Failure,
        }
    }
}

impl ExitCodePolicy {
    pub fn clean() -> Self {
        Self::default()
    }

    pub fn issues(mut self, codes: impl IntoIterator<Item = i32>) -> Self {
        self.issues = codes.into_iter().collect();
        self
    }

    pub fn failure(mut self, codes: impl IntoIterator<Item = i32>) -> Self {
        self.failure = codes.into_iter().collect();
        self
    }

    pub fn unexpected(mut self, policy: UnexpectedExitPolicy) -> Self {
        self.unexpected = policy;
        self
    }
}

/// How to classify an exit code not listed in the policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnexpectedExitPolicy {
    Failure,
    Issues,
}

/// Tool-specific output templates.
#[derive(Debug, Clone)]
pub struct ToolMessages {
    pub clean_changed_agent: String,
    pub issues_agent: String,
    pub issues_changed_agent: String,
    pub unavailable_user: Option<String>,
    pub failed_user: Option<String>,
}

impl Default for ToolMessages {
    fn default() -> Self {
        Self {
            clean_changed_agent: DEFAULT_CLEAN_CHANGED_AGENT.to_string(),
            issues_agent: DEFAULT_ISSUES_AGENT.to_string(),
            issues_changed_agent: DEFAULT_ISSUES_CHANGED_AGENT.to_string(),
            unavailable_user: None,
            failed_user: None,
        }
    }
}

// ----------------------------------------------------------------------------
// Runner entry points
// ----------------------------------------------------------------------------

/// CLI options parsed from process args.
#[derive(Debug, Clone)]
pub struct Cli {
    pub harness: HarnessId,
    pub config_path: Option<PathBuf>,
}

/// CLI options for the stop-time batch runner.
#[derive(Debug, Clone)]
pub struct TurnCompletionCli {
    pub harness: HarnessId,
    pub config_path: Option<PathBuf>,
    pub state_dir: Option<PathBuf>,
}

/// CLI options for the library-owned precise session-start observer.
#[derive(Debug, Clone)]
pub struct SessionStartCli {
    pub harness: HarnessId,
    pub state_dir: Option<PathBuf>,
}

/// CLI options for the quiet post-tool file-activity observer.
#[derive(Debug, Clone)]
pub struct FileActivityCli {
    pub harness: HarnessId,
    pub state_dir: Option<PathBuf>,
}

/// Parse a supported PostToolUse harness and optional shared state root.
/// `--harness=claude|codex|gemini` remains a compatibility alias for the
/// former example binary.
#[allow(clippy::result_unit_err)]
pub fn parse_file_activity_args() -> Result<FileActivityCli, ()> {
    let mut harness = None;
    let mut state_dir = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--claude" => harness = Some(set_harness(harness, HarnessId::CLAUDE_CODE)?),
            "--codex" => harness = Some(set_harness(harness, HarnessId::CODEX)?),
            "--gemini" => harness = Some(set_harness(harness, HarnessId::GEMINI_CLI)?),
            "--harness" => {
                let Some(value) = args.next() else {
                    eprintln!("{}", file_activity_usage());
                    return Err(());
                };
                harness = Some(set_harness(harness, post_tool_harness(&value)?)?);
            }
            "--state-dir" => {
                let Some(path) = args.next() else {
                    eprintln!("{}", file_activity_usage());
                    return Err(());
                };
                state_dir = Some(PathBuf::from(path));
            }
            "--help" | "-h" => {
                eprintln!("{}", file_activity_usage());
                return Err(());
            }
            _ if arg.starts_with("--harness=") => {
                harness = Some(set_harness(
                    harness,
                    post_tool_harness(arg.trim_start_matches("--harness="))?,
                )?);
            }
            _ if arg.starts_with("--state-dir=") => {
                state_dir = Some(PathBuf::from(arg.trim_start_matches("--state-dir=")));
            }
            _ => {
                eprintln!("{}", file_activity_usage());
                return Err(());
            }
        }
    }
    let Some(harness) = harness else {
        eprintln!("{}", file_activity_usage());
        return Err(());
    };
    Ok(FileActivityCli { harness, state_dir })
}

/// Parse `--claude|--codex|--gemini [--config PATH]` from `std::env::args`.
#[allow(clippy::result_unit_err)]
pub fn parse_args() -> Result<Cli, ()> {
    let mut harness = None;
    let mut config_path = None;
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--claude" => harness = Some(set_harness(harness, HarnessId::CLAUDE_CODE)?),
            "--codex" => harness = Some(set_harness(harness, HarnessId::CODEX)?),
            "--gemini" => harness = Some(set_harness(harness, HarnessId::GEMINI_CLI)?),
            "--harness" => {
                let Some(value) = args.next() else {
                    eprintln!("{}", usage());
                    return Err(());
                };
                harness = Some(set_harness(harness, post_tool_harness(&value)?)?);
            }
            "--config" => {
                let Some(path) = args.next() else {
                    eprintln!("{}", usage());
                    return Err(());
                };
                config_path = Some(PathBuf::from(path));
            }
            "--help" | "-h" => {
                eprintln!("{}", usage());
                return Err(());
            }
            _ if arg.starts_with("--harness=") => {
                harness = Some(set_harness(
                    harness,
                    post_tool_harness(arg.trim_start_matches("--harness="))?,
                )?);
            }
            _ if arg.starts_with("--config=") => {
                config_path = Some(PathBuf::from(arg.trim_start_matches("--config=")));
            }
            _ => {
                eprintln!("{}", usage());
                return Err(());
            }
        }
    }

    let Some(harness) = harness else {
        eprintln!("{}", usage());
        return Err(());
    };

    Ok(Cli {
        harness,
        config_path,
    })
}

/// Parse the aligned turn-completion runner's harness, config, and state root.
#[allow(clippy::result_unit_err)]
pub fn parse_turn_completion_args() -> Result<TurnCompletionCli, ()> {
    let mut harness = None;
    let mut config_path = None;
    let mut state_dir = None;
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--claude" => harness = Some(set_harness(harness, HarnessId::CLAUDE_CODE)?),
            "--codex" => harness = Some(set_harness(harness, HarnessId::CODEX)?),
            "--gemini" => harness = Some(set_harness(harness, HarnessId::GEMINI_CLI)?),
            "--antigravity" => harness = Some(set_harness(harness, HarnessId::ANTIGRAVITY)?),
            "--harness" => {
                let Some(value) = args.next() else {
                    eprintln!("{}", turn_completion_usage());
                    return Err(());
                };
                harness = Some(set_harness(harness, turn_completion_harness(&value)?)?);
            }
            "--config" => {
                let Some(path) = args.next() else {
                    eprintln!("{}", turn_completion_usage());
                    return Err(());
                };
                config_path = Some(PathBuf::from(path));
            }
            "--state-dir" => {
                let Some(path) = args.next() else {
                    eprintln!("{}", turn_completion_usage());
                    return Err(());
                };
                state_dir = Some(PathBuf::from(path));
            }
            "--help" | "-h" => {
                eprintln!("{}", turn_completion_usage());
                return Err(());
            }
            _ if arg.starts_with("--harness=") => {
                harness = Some(set_harness(
                    harness,
                    turn_completion_harness(arg.trim_start_matches("--harness="))?,
                )?);
            }
            _ if arg.starts_with("--config=") => {
                config_path = Some(PathBuf::from(arg.trim_start_matches("--config=")));
            }
            _ if arg.starts_with("--state-dir=") => {
                state_dir = Some(PathBuf::from(arg.trim_start_matches("--state-dir=")));
            }
            _ => {
                eprintln!("{}", turn_completion_usage());
                return Err(());
            }
        }
    }

    let Some(harness) = harness else {
        eprintln!("{}", turn_completion_usage());
        return Err(());
    };
    Ok(TurnCompletionCli {
        harness,
        config_path,
        state_dir,
    })
}

/// Parse a supported native SessionStart harness and optional state root.
#[allow(clippy::result_unit_err)]
pub fn parse_session_start_args() -> Result<SessionStartCli, ()> {
    let mut harness = None;
    let mut state_dir = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--claude" => harness = Some(set_harness(harness, HarnessId::CLAUDE_CODE)?),
            "--codex" => harness = Some(set_harness(harness, HarnessId::CODEX)?),
            "--gemini" => harness = Some(set_harness(harness, HarnessId::GEMINI_CLI)?),
            "--harness" => {
                let Some(value) = args.next() else {
                    eprintln!("{}", session_start_usage());
                    return Err(());
                };
                harness = Some(set_harness(harness, post_tool_harness(&value)?)?);
            }
            "--state-dir" => {
                let Some(path) = args.next() else {
                    eprintln!("{}", session_start_usage());
                    return Err(());
                };
                state_dir = Some(PathBuf::from(path));
            }
            "--help" | "-h" => {
                eprintln!("{}", session_start_usage());
                return Err(());
            }
            _ if arg.starts_with("--harness=") => {
                harness = Some(set_harness(
                    harness,
                    post_tool_harness(arg.trim_start_matches("--harness="))?,
                )?);
            }
            _ if arg.starts_with("--state-dir=") => {
                state_dir = Some(PathBuf::from(arg.trim_start_matches("--state-dir=")));
            }
            _ => {
                eprintln!("{}", session_start_usage());
                return Err(());
            }
        }
    }
    let Some(harness) = harness else {
        eprintln!("{}", session_start_usage());
        return Err(());
    };
    Ok(SessionStartCli { harness, state_dir })
}

fn set_harness(current: Option<HarnessId>, next: HarnessId) -> Result<HarnessId, ()> {
    if current.is_some() {
        eprintln!("{}", usage());
        return Err(());
    }
    Ok(next)
}

fn post_tool_harness(value: &str) -> Result<HarnessId, ()> {
    match value {
        "claude" | "claude-code" => Ok(HarnessId::CLAUDE_CODE),
        "codex" => Ok(HarnessId::CODEX),
        "gemini" | "gemini-cli" => Ok(HarnessId::GEMINI_CLI),
        _ => Err(()),
    }
}

fn turn_completion_harness(value: &str) -> Result<HarnessId, ()> {
    match value {
        "antigravity" => Ok(HarnessId::ANTIGRAVITY),
        _ => post_tool_harness(value),
    }
}

fn usage() -> String {
    format!(
        "Usage: {BINARY_NAME} --claude|--codex|--gemini [--config PATH]\n       {BINARY_NAME} --harness=claude|codex|gemini [--config PATH]"
    )
}

fn file_activity_usage() -> String {
    format!(
        "Usage: {FILE_ACTIVITY_BINARY_NAME} --claude|--codex|--gemini [--state-dir PATH]\n       {FILE_ACTIVITY_BINARY_NAME} --harness=claude|codex|gemini [--state-dir PATH]"
    )
}

fn session_start_usage() -> String {
    format!(
        "Usage: {SESSION_START_BINARY_NAME} --claude|--codex|--gemini [--state-dir PATH]\n       {SESSION_START_BINARY_NAME} --harness=claude|codex|gemini [--state-dir PATH]"
    )
}

fn turn_completion_usage() -> String {
    format!(
        "Usage: {TURN_COMPLETION_BINARY_NAME} --claude|--codex|--gemini|--antigravity [--config PATH] [--state-dir PATH]\n       {TURN_COMPLETION_BINARY_NAME} --harness=claude|codex|gemini|antigravity [--config PATH] [--state-dir PATH]"
    )
}

// ----------------------------------------------------------------------------
// Shared immediate post-tool path discovery
// ----------------------------------------------------------------------------

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
/// Run the full post-tool-use hook from parsed CLI args.
pub fn run_runner(cli: Cli) -> std::process::ExitCode {
    hookkit_runtime::aligned::run_aligned_event::<hookkit_runtime::aligned::PostToolUse, _>(
        cli.harness,
        move |input, environment, ctx| {
            run_post_tool_input(input, environment, ctx, cli.config_path.as_deref())
        },
    )
}

/// Run the bundled quiet PostToolUse file-activity observer.
pub fn run_file_activity_observer(cli: FileActivityCli) -> std::process::ExitCode {
    hookkit_runtime::aligned::run_aligned_event::<hookkit_runtime::aligned::PostToolUse, _>(
        cli.harness,
        move |input, _environment, ctx| {
            let report = observe_file_activity(&input, ctx);
            let state_root = cli
                .state_dir
                .as_deref()
                .map(StateRoot::new)
                .unwrap_or_default();
            let store = FileActivityStore::ensure(ctx, state_root).map_err(activity_error)?;
            let invocation = String::from_utf8_lossy(ctx.raw().bytes());
            store
                .append_report(&invocation, &report)
                .map_err(activity_error)?;
            post_tool_no_op(ctx.harness())
        },
    )
}

fn post_tool_no_op(harness: &HarnessId) -> hookkit_core::Result<PostToolUseOutput> {
    match harness.as_str() {
        "claude-code" => Ok(PostToolUseOutput::Claude(
            hookkit_claude::protocol::PostToolUseOutput::no_op(),
        )),
        "codex" => Ok(PostToolUseOutput::Codex(
            hookkit_codex::protocol::PostToolUseOutput::no_op(),
        )),
        "gemini-cli" => Ok(PostToolUseOutput::Gemini(
            hookkit_gemini::protocol::AfterToolOutput::no_op(),
        )),
        _ => Err(invalid_data(format!(
            "file-activity observer does not support {harness}"
        ))),
    }
}

/// Run the stop-time batch hook from parsed CLI args.
pub fn run_turn_completion_runner(cli: TurnCompletionCli) -> std::process::ExitCode {
    hookkit_runtime::aligned::run_aligned_event::<hookkit_runtime::aligned::TurnCompletion, _>(
        cli.harness,
        move |input, environment, ctx| {
            run_turn_completion_input(
                input,
                environment,
                ctx,
                cli.config_path.as_deref(),
                cli.state_dir.as_deref(),
            )
        },
    )
}

/// Run the small, library-owned lifecycle observer used when precise native
/// session-start timing is desired even before another stateful hook runs.
pub fn run_session_start_observer(cli: SessionStartCli) -> std::process::ExitCode {
    let state_dir = cli.state_dir;
    match cli.harness.as_str() {
        "claude-code" => hookkit_runtime::typed::run_typed::<
            hookkit_claude::protocol::SessionStart,
            _,
        >(move |_, _, ctx| {
            ensure_session_metadata(ctx, state_dir.as_deref())?;
            Ok(hookkit_claude::protocol::SessionStartOutput::no_op())
        }),
        "codex" => hookkit_runtime::typed::run_typed::<hookkit_codex::catalog::SessionStart, _>(
            move |_, _, ctx| {
                ensure_session_metadata(ctx, state_dir.as_deref())?;
                Ok(hookkit_codex::catalog::SessionStartOutput::no_op())
            },
        ),
        "gemini-cli" => {
            hookkit_runtime::typed::run_typed::<hookkit_gemini::catalog::SessionStart, _>(
                move |_, _, ctx| {
                    ensure_session_metadata(ctx, state_dir.as_deref())?;
                    Ok(hookkit_gemini::catalog::SessionStartOutput::no_op())
                },
            )
        }
        _ => std::process::ExitCode::from(1),
    }
}

fn ensure_session_metadata(
    ctx: &RuntimeContext<'_>,
    state_dir: Option<&Path>,
) -> hookkit_core::Result<()> {
    let root = state_dir.map(StateRoot::new).unwrap_or_default();
    SessionState::ensure(ctx, root).map_err(state_error)?;
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchToolSummary {
    tool_id: String,
    file_count: usize,
    issues: bool,
    operational_failure: bool,
    artifacts: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchRunSummary {
    schema_version: u32,
    run: BatchRunIdentity,
    status: &'static str,
    source_entry_count: usize,
    source_entry_ids: Vec<String>,
    candidate_files: Vec<PathBuf>,
    counts: BatchCounts,
    clean_files: Vec<PathBuf>,
    auto_fixed_files: Vec<PathBuf>,
    manual_fix_files: Vec<PathBuf>,
    groups: Vec<BatchGroupSummary>,
    artifact_paths: Vec<PathBuf>,
    artifact_contents: BTreeMap<PathBuf, String>,
    state_disposition: PlannedStateDisposition,
    rendered_messages: RenderedMessageMetadata,
    tools: Vec<BatchToolSummary>,
    result: DeferredRunResult,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchRunIdentity {
    id: String,
    project_root: PathBuf,
    summary_path: PathBuf,
    state_directory: PathBuf,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchCounts {
    clean: usize,
    auto_fixed: usize,
    manual_fixes_needed: usize,
    operational_errors: usize,
    uncovered: usize,
    not_applicable: usize,
    coverage_gaps: usize,
    groups: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchGroupSummary {
    id: String,
    display_name: String,
    files: Vec<PathBuf>,
    count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlannedStateDisposition {
    source: &'static str,
    retry_files: Vec<Utf8PathBuf>,
    retry_targets: Vec<FileActivityTarget>,
    retry_gaps: Vec<String>,
    handled_baseline_files: Vec<Utf8PathBuf>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RenderedMessageMetadata {
    harness: String,
    lowering: StopLoweringMetadata,
    buckets: RenderedBuckets,
    user: Option<String>,
    agent: Option<String>,
    references_summary: bool,
}

struct BatchSummaryParts<'a> {
    run: &'a RunBundle,
    project_root: &'a Path,
    state_directory: &'a Path,
    harness: &'a HarnessId,
    status: &'static str,
    rendered_messages: RenderedMessages,
    lowering: StopLoweringMetadata,
    source: (usize, Vec<String>),
    candidates: &'a [PathBuf],
    tools: Vec<BatchToolSummary>,
    disposition: &'a DeferredStateDisposition,
    result: DeferredRunResult,
}

struct DeferredFailureContext<'a> {
    project_root: &'a Path,
    candidates: &'a [PathBuf],
    resolution: &'a ActivityResolution,
}

#[derive(Debug)]
struct ActivityResolution {
    not_applicable_files: BTreeSet<PathBuf>,
    unresolved_targets: Vec<FileActivityTarget>,
    gap_messages: BTreeSet<String>,
    truncated: bool,
}

#[derive(Debug, Clone)]
struct DeferredStateDisposition {
    retry_files: BTreeSet<Utf8PathBuf>,
    retry_targets: Vec<FileActivityTarget>,
    retry_gaps: BTreeSet<String>,
    handled_files: BTreeSet<Utf8PathBuf>,
}

fn run_turn_completion_input(
    _turn_completion: TurnCompletionInput,
    _environment: &TurnCompletionCommandEnvironment,
    ctx: &RuntimeContext<'_>,
    config_path: Option<&Path>,
    state_dir: Option<&Path>,
) -> hookkit_core::Result<TurnCompletionOutput> {
    let cwd = ctx
        .workspace_roots()
        .first()
        .map(|root| PathBuf::from(root.as_str()))
        .ok_or_else(|| invalid_data("turn-completion input has no workspace root".into()))?;
    let state_root = state_dir.map(StateRoot::new).unwrap_or_default();
    let state = SessionState::ensure(ctx, state_root).map_err(state_error)?;
    let activity_store = FileActivityStore::from_state(state.clone()).map_err(activity_error)?;
    let runner_family = state
        .family(FamilyId::new(BATCHED_TOOLS_FAMILY, 1).map_err(state_error)?)
        .map_err(state_error)?;

    // Two concurrent stop hooks must not run formatters against the same
    // sealed window at once. Post-tool producers do not take this lock; exact
    // generation acknowledgement preserves observations appended while we run.
    let _runner_lock = runner_family
        .exclusive_lock("turn-completion")
        .map_err(state_error)?;
    let loaded = hookkit_pkl_config::discover_and_load(&cwd, config_path);
    let activity_settings = loaded
        .as_ref()
        .ok()
        .and_then(|loaded| loaded.config.settings.file_activity.clone())
        .unwrap_or_default();
    let mut reconciliation =
        ReconciliationOptions::new(ctx.workspace_roots().to_vec(), UtcTimestamp::now());
    reconciliation.filesystem_mtime = activity_settings.filesystem_mtime;
    reconciliation.vcs = match activity_settings.vcs {
        pkl::FileActivityVcsFallback::Disabled => VcsFallback::Disabled,
        pkl::FileActivityVcsFallback::GitDirty => VcsFallback::GitDirty,
    };
    reconciliation.timestamp_tolerance =
        Duration::from_millis(activity_settings.timestamp_tolerance_millis);
    reconciliation.max_entries = activity_settings.max_entries;
    reconciliation.ignored_directory_names = activity_settings
        .ignored_directory_names
        .iter()
        .cloned()
        .collect();
    reconcile(&activity_store, reconciliation).map_err(activity_error)?;
    activity_store
        .pending()
        .try_with_entity(|view| {
            run_turn_completion_view(
                ctx,
                loaded,
                &activity_settings,
                &activity_store,
                &runner_family,
                view,
            )
        })
        .map_err(|error| match error {
            EntityOperationError::State(error) => state_error(error),
            EntityOperationError::Operation(error) => error,
        })
}

fn run_turn_completion_view(
    ctx: &RuntimeContext<'_>,
    loaded: Result<hookkit_pkl_config::Loaded, hookkit_pkl_config::PklConfigError>,
    activity_settings: &pkl::FileActivitySettings,
    activity_store: &FileActivityStore,
    runner_family: &StateFamily,
    view: &EntityView<'_, PendingFileActivity>,
) -> hookkit_core::Result<EntityOutcome<TurnCompletionOutput>> {
    if view.events().is_empty() {
        let lowering = plan_stop_lowering(
            ctx.harness(),
            false,
            None,
            None,
            pkl::LoweringPolicy::BestEffortWithWarnings,
        )?;
        return Ok(EntityOutcome::retain(lowering.finish()?));
    }
    let fallback_project_root = ctx
        .workspace_roots()
        .first()
        .map(|root| normalize_path(root.as_std_path()))
        .ok_or_else(|| invalid_data("turn-completion input has no workspace root".into()))?;

    let mut resolve_options = ResolveOptions::new(ctx.workspace_roots().to_vec());
    resolve_options.max_entries = activity_settings.max_entries;
    resolve_options.ignored_directory_names = activity_settings
        .ignored_directory_names
        .iter()
        .cloned()
        .collect();
    if let Ok(state_directory) =
        Utf8PathBuf::from_path_buf(activity_store.state().directory().into())
    {
        resolve_options.excluded_roots.insert(state_directory);
    }
    let resolved = resolve_files(view.state(), &resolve_options).map_err(activity_error)?;
    let resolution = ActivityResolution {
        not_applicable_files: resolved
            .not_applicable_files
            .into_iter()
            .map(|path| normalize_path(path.as_std_path()))
            .collect(),
        unresolved_targets: resolved.unresolved_targets,
        gap_messages: source_gap_messages(view),
        truncated: resolved.truncated,
    };
    let mut candidates = resolved
        .files
        .into_iter()
        .map(|path| normalize_path(path.as_std_path()))
        .collect::<Vec<_>>();
    candidates.sort();
    let source_entry_count = view.events().len();
    let source_entry_ids = view
        .events()
        .iter()
        .map(|entry| entry.id().to_string())
        .collect::<Vec<_>>();
    let mut run = Some(
        runner_family
            .start_run("turn-completion")
            .map_err(state_error)?,
    );

    let loaded = match loaded {
        Ok(loaded) => loaded,
        Err(error) => {
            let run = run.take().expect("run bundle is available");
            let contents = error.to_string();
            let artifact_path = run
                .write_text("config-error.log", &contents)
                .map_err(state_error)?;
            let mut result = DeferredRunResult::default();
            result.record_artifact(RunArtifact {
                id: "configuration".into(),
                absolute_path: artifact_path,
                run_relative_path: "config-error.log".into(),
                media_type: "text/plain; charset=utf-8".into(),
                tool_id: None,
                workflow_id: None,
                job_id: None,
                report_id: None,
                phase: CommandPhase::Configuration,
                classification: ArtifactClassification::ConfigurationError,
                exit_code: None,
                program: None,
                arguments: Vec::new(),
                working_directory: None,
                files: candidates.clone(),
                candidate_files: candidates.clone(),
                changed_files: Vec::new(),
                contents: contents.clone(),
            });
            result.record_operational_problem(OperationalProblem {
                id: "configuration".into(),
                tool_id: None,
                phase: Some("configuration".into()),
                affected_files: candidates.clone(),
                message: contents.clone(),
                artifact_ids: vec!["configuration".into()],
            });
            record_activity_resolution(&mut result, &resolution);
            let disposition = plan_deferred_state_disposition(&result, &resolution)?;
            let rendered_messages =
                failure_rendered_messages(&run.directory().join("summary.json"), &contents);
            let lowering = plan_stop_lowering(
                ctx.harness(),
                true,
                rendered_messages.user.as_deref(),
                rendered_messages.agent.as_deref(),
                pkl::LoweringPolicy::BestEffortWithWarnings,
            )?;
            let summary = build_batch_summary(BatchSummaryParts {
                run: &run,
                project_root: &fallback_project_root,
                state_directory: activity_store.state().directory(),
                harness: ctx.harness(),
                status: "operational-failure",
                rendered_messages,
                lowering: lowering.metadata.clone(),
                source: (source_entry_count, source_entry_ids.clone()),
                candidates: &candidates,
                tools: Vec::new(),
                disposition: &disposition,
                result,
            })?;
            let run_id = summary.run.id.clone();
            run.commit(&summary).map_err(state_error)?;
            let output = lowering.finish()?;
            apply_deferred_state_disposition(activity_store, disposition, run_id)?;
            return Ok(EntityOutcome::acknowledge(output));
        }
    };

    let project_root = normalize_path(&loaded.project_root);
    let lowering_policy = loaded.config.settings.lowering_policy;
    let reporter = match DeferredReporter::new(&loaded.config.settings.deferred_reporting) {
        Ok(reporter) => reporter,
        Err(error) => {
            let run = run.take().expect("run bundle is available");
            return commit_deferred_config_failure(
                ctx,
                activity_store,
                run,
                DeferredFailureContext {
                    project_root: &project_root,
                    candidates: &candidates,
                    resolution: &resolution,
                },
                (source_entry_count, source_entry_ids),
                lowering_policy,
                error.to_string(),
            );
        }
    };
    let tools = match resolve_run_order(&loaded.config) {
        Ok(tools) => tools,
        Err(error) => {
            let run = run.take().expect("run bundle is available");
            let contents = error.to_string();
            let artifact_path = run
                .write_text("config-error.log", &contents)
                .map_err(state_error)?;
            let mut result = DeferredRunResult::default();
            result.record_artifact(RunArtifact {
                id: "configuration".into(),
                absolute_path: artifact_path,
                run_relative_path: "config-error.log".into(),
                media_type: "text/plain; charset=utf-8".into(),
                tool_id: None,
                workflow_id: None,
                job_id: None,
                report_id: None,
                phase: CommandPhase::Configuration,
                classification: ArtifactClassification::ConfigurationError,
                exit_code: None,
                program: None,
                arguments: Vec::new(),
                working_directory: None,
                files: candidates.clone(),
                candidate_files: candidates.clone(),
                changed_files: Vec::new(),
                contents: contents.clone(),
            });
            result.record_operational_problem(OperationalProblem {
                id: "configuration".into(),
                tool_id: None,
                phase: Some("configuration".into()),
                affected_files: candidates.clone(),
                message: contents.clone(),
                artifact_ids: vec!["configuration".into()],
            });
            record_activity_resolution(&mut result, &resolution);
            let disposition = plan_deferred_state_disposition(&result, &resolution)?;
            let rendered_messages =
                failure_rendered_messages(&run.directory().join("summary.json"), &contents);
            let lowering = plan_stop_lowering(
                ctx.harness(),
                true,
                rendered_messages.user.as_deref(),
                rendered_messages.agent.as_deref(),
                lowering_policy,
            )?;
            let summary = build_batch_summary(BatchSummaryParts {
                run: &run,
                project_root: &project_root,
                state_directory: activity_store.state().directory(),
                harness: ctx.harness(),
                status: "operational-failure",
                rendered_messages,
                lowering: lowering.metadata.clone(),
                source: (source_entry_count, source_entry_ids.clone()),
                candidates: &candidates,
                tools: Vec::new(),
                disposition: &disposition,
                result,
            })?;
            let run_id = summary.run.id.clone();
            run.commit(&summary).map_err(state_error)?;
            let output = lowering.finish()?;
            apply_deferred_state_disposition(activity_store, disposition, run_id)?;
            return Ok(EntityOutcome::acknowledge(output));
        }
    };

    let (plan, planned_tools) = match build_deferred_plan(
        &tools,
        &candidates,
        &project_root,
        &loaded.config.settings.exclude,
    ) {
        Ok(plan) => plan,
        Err(error) => {
            let run = run.take().expect("run bundle is available");
            return commit_deferred_config_failure(
                ctx,
                activity_store,
                run,
                DeferredFailureContext {
                    project_root: &project_root,
                    candidates: &candidates,
                    resolution: &resolution,
                },
                (source_entry_count, source_entry_ids),
                lowering_policy,
                error.to_string(),
            );
        }
    };
    let mut execution = execute_deferred_workflows(
        &plan,
        loaded.config.settings.jobs,
        loaded.config.settings.fail_fast,
    );
    let summaries = write_deferred_artifacts(
        run.as_ref().expect("run bundle is available"),
        &plan,
        &planned_tools,
        &execution.logs,
        &mut execution.result,
    )?;
    let mut result = execution.result;
    record_activity_resolution(&mut result, &resolution);

    let operational_files = result
        .operational_problems
        .values()
        .flat_map(|problem| problem.affected_files.iter().cloned())
        .collect::<BTreeSet<_>>();
    for candidate in &candidates {
        if !result.files.contains_key(candidate) && !operational_files.contains(candidate) {
            result.record_uncovered(candidate.clone());
        }
    }

    reporter.apply_groups(&mut result, &project_root);
    let rendered_messages = {
        let active_run = run.as_ref().expect("run bundle is available");
        let template_run_id = run_id(active_run.directory())?;
        let summary_path = active_run.directory().join("summary.json");
        match reporter.render(
            &result,
            TemplateRun {
                id: &template_run_id,
                project_root: &project_root,
                summary_path: &summary_path,
                state_directory: activity_store.state().directory(),
            },
        ) {
            Ok(messages) => messages,
            Err(error) => record_reporting_failure(
                active_run,
                &mut result,
                &candidates,
                &summary_path,
                error.to_string(),
            )?,
        }
    };

    let should_block = deferred_should_block(&result, activity_settings.coverage_gap_policy);
    let lowering = plan_stop_lowering(
        ctx.harness(),
        should_block,
        rendered_messages.user.as_deref(),
        rendered_messages.agent.as_deref(),
        lowering_policy,
    )?;
    let disposition = plan_deferred_state_disposition(&result, &resolution)?;
    let status = if result.has_operational_problems() {
        "operational-failure"
    } else if result.has_manual_fixes() {
        "issues"
    } else if !result.uncovered_files.is_empty() && result.files.is_empty() {
        "not-applicable"
    } else {
        "clean"
    };
    let run = run.take().expect("run bundle is available");
    let summary = build_batch_summary(BatchSummaryParts {
        run: &run,
        project_root: &project_root,
        state_directory: activity_store.state().directory(),
        harness: ctx.harness(),
        status,
        rendered_messages,
        lowering: lowering.metadata.clone(),
        source: (source_entry_count, source_entry_ids),
        candidates: &candidates,
        tools: summaries,
        disposition: &disposition,
        result,
    })?;
    let run_id = summary.run.id.clone();
    run.commit(&summary).map_err(state_error)?;
    let output = lowering.finish()?;
    apply_deferred_state_disposition(activity_store, disposition, run_id)?;
    Ok(EntityOutcome::acknowledge(output))
}

#[derive(Debug)]
struct PlannedDeferredTool {
    index: usize,
    spec: Arc<ToolSpec>,
    files: Vec<PathBuf>,
}

fn build_deferred_plan(
    schemas: &[&pkl::ToolSpec],
    candidates: &[PathBuf],
    project_root: &Path,
    global_exclude: &[String],
) -> hookkit_core::Result<(Vec<ScheduledWorkflow>, Vec<PlannedDeferredTool>)> {
    let mut plan = Vec::new();
    let mut planned_tools = Vec::new();
    for (tool_index, schema) in schemas.iter().enumerate() {
        if !schema.enabled {
            continue;
        }
        for id in &schema.workflow_order {
            if !schema.workflows.contains_key(id) {
                return Err(invalid_data(format!(
                    "tool `{}` workflowOrder references unknown workflow `{id}`",
                    schema.id
                )));
            }
        }
        let spec = Arc::new(convert_tool_spec(schema, global_exclude));
        let matcher = FileMatcher::new(&spec.file_selection)?;
        let files = candidates
            .iter()
            .filter(|path| matcher.matches(path, project_root))
            .cloned()
            .collect::<Vec<_>>();
        if files.is_empty() {
            continue;
        }
        let base_jobs = build_jobs(&files, project_root, &spec);
        if base_jobs.is_empty() {
            continue;
        }
        for (workflow_index, workflow) in spec.workflows.iter().enumerate() {
            if !workflow.enabled {
                continue;
            }
            if workflow.check.is_none() && !workflow.compatibility_translation {
                return Err(invalid_data(format!(
                    "tool `{}` workflow `{}` requires a non-mutating check",
                    spec.id, workflow.id
                )));
            }
            if workflow
                .check
                .as_ref()
                .is_some_and(|check| check.writes != WriteBehavior::None)
            {
                return Err(invalid_data(format!(
                    "tool `{}` workflow `{}` check must declare writes = none",
                    spec.id, workflow.id
                )));
            }
            if workflow
                .remedy
                .as_ref()
                .is_some_and(|remedy| remedy.writes == WriteBehavior::None)
            {
                return Err(invalid_data(format!(
                    "tool `{}` workflow `{}` remedy must declare a write scope",
                    spec.id, workflow.id
                )));
            }
            let jobs = workflow_jobs(&base_jobs, workflow.invocation);
            for (job_index, job) in jobs.into_iter().enumerate() {
                plan.push(ScheduledWorkflow {
                    tool_index,
                    workflow_index,
                    job_index,
                    spec: Arc::clone(&spec),
                    workflow_id: workflow.id.clone(),
                    check: workflow.check.clone(),
                    remedy: workflow.remedy.clone(),
                    check_scope: workflow.check_scope,
                    invocation: workflow.invocation,
                    compatibility_translation: workflow.compatibility_translation,
                    job,
                    project_root: project_root.to_path_buf(),
                });
            }
        }
        planned_tools.push(PlannedDeferredTool {
            index: tool_index,
            spec,
            files,
        });
    }
    Ok((plan, planned_tools))
}

fn workflow_jobs(base_jobs: &[ToolJob], invocation: InvocationGranularity) -> Vec<ToolJob> {
    if invocation != InvocationGranularity::PerFile {
        return base_jobs.to_vec();
    }
    base_jobs
        .iter()
        .flat_map(|job| {
            job.files.iter().cloned().map(|file| ToolJob {
                workspace_dir: job.workspace_dir.clone(),
                workspace_indicator: job.workspace_indicator.clone(),
                files: vec![file],
            })
        })
        .collect()
}

fn write_deferred_artifacts(
    run: &RunBundle,
    plan: &[ScheduledWorkflow],
    tools: &[PlannedDeferredTool],
    logs: &[DeferredLog],
    result: &mut DeferredRunResult,
) -> hookkit_core::Result<Vec<BatchToolSummary>> {
    let mut tool_artifacts = BTreeMap::<usize, Vec<String>>::new();
    for log in logs {
        let scheduled = plan
            .iter()
            .find(|scheduled| {
                scheduled.tool_index == log.tool_index
                    && scheduled.workflow_index == log.workflow_index
                    && scheduled.job_index == log.job_index
            })
            .ok_or_else(|| invalid_data("deferred log has no scheduled workflow".into()))?;
        let report_id = scheduled.report_id();
        let changed_files = result
            .reports
            .get(&report_id)
            .map(|report| report.changed_files.clone())
            .unwrap_or_default();
        let candidate_files = scheduled.job.files.clone();
        let files = candidate_files
            .iter()
            .chain(changed_files.iter())
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let phase = command_phase_name(log.phase);
        let tool_component = safe_artifact_component(&scheduled.spec.id);
        let workflow_component = safe_artifact_component(&scheduled.workflow_id);
        let relative = format!(
            "tools/{:03}-{tool_component}/workflows/{:03}-{workflow_component}/jobs/{:03}/{phase}.log",
            scheduled.tool_index, scheduled.workflow_index, scheduled.job_index,
        );
        let contents = format_deferred_artifact(log)?;
        let absolute = run.write_text(&relative, &contents).map_err(state_error)?;
        let artifact_id = format!("{report_id}-{phase}");
        attach_report_artifact(result, &report_id, &artifact_id);
        result.record_artifact(RunArtifact {
            id: artifact_id.clone(),
            absolute_path: absolute,
            run_relative_path: relative.clone().into(),
            media_type: "text/plain; charset=utf-8".into(),
            tool_id: Some(scheduled.spec.id.clone()),
            workflow_id: Some(scheduled.workflow_id.clone()),
            job_id: Some(format!("{:03}", scheduled.job_index)),
            report_id: Some(report_id),
            phase: log.phase,
            classification: artifact_classification(&log.log),
            exit_code: log.log.status,
            program: Some(log.log.program.clone()),
            arguments: log.log.arguments.clone(),
            working_directory: Some(scheduled.job.workspace_dir.clone()),
            files,
            candidate_files,
            changed_files,
            contents,
        });
        tool_artifacts
            .entry(scheduled.tool_index)
            .or_default()
            .push(relative);
    }

    let mut summaries = Vec::new();
    for tool in tools {
        let issues = result.reports.values().any(|report| {
            report.tool_id == tool.spec.id && report.final_check == Some(CheckOutcome::Issues)
        });
        let operational_failure = result
            .operational_problems
            .values()
            .any(|problem| problem.tool_id.as_deref() == Some(tool.spec.id.as_str()));
        summaries.push(BatchToolSummary {
            tool_id: tool.spec.id.clone(),
            file_count: tool.files.len(),
            issues,
            operational_failure,
            artifacts: tool_artifacts.remove(&tool.index).unwrap_or_default(),
        });
    }
    Ok(summaries)
}

fn attach_report_artifact(result: &mut DeferredRunResult, report_id: &str, artifact_id: &str) {
    if let Some(report) = result.reports.get_mut(report_id) {
        report.artifact_ids.push(artifact_id.into());
        report.artifact_ids.sort();
        report.artifact_ids.dedup();
    }
    for file in result.files.values_mut() {
        for report in &mut file.reports {
            if report.report_id == report_id {
                report.artifact_ids.push(artifact_id.into());
                report.artifact_ids.sort();
                report.artifact_ids.dedup();
            }
        }
    }
    for problem in result.operational_problems.values_mut() {
        if problem.id.starts_with(&format!("{report_id}-")) {
            problem.artifact_ids.push(artifact_id.into());
            problem.artifact_ids.sort();
            problem.artifact_ids.dedup();
        }
    }
}

fn format_deferred_artifact(log: &DeferredLog) -> hookkit_core::Result<String> {
    let argv = std::iter::once(log.log.program.as_str())
        .chain(log.log.arguments.iter().map(String::as_str))
        .collect::<Vec<_>>();
    let argv = serde_json::to_string(&argv)
        .map_err(|error| invalid_data(format!("could not serialize command argv: {error}")))?;
    Ok(format!(
        "workflow_index: {}\njob_index: {}\ncommand_phase: {}\nargv: {argv}\n{}",
        log.workflow_index,
        log.job_index,
        command_phase_name(log.phase),
        format_logs(std::slice::from_ref(&log.log)),
    ))
}

fn artifact_classification(log: &PhaseLog) -> ArtifactClassification {
    if log.error.is_some() {
        return ArtifactClassification::SpawnError;
    }
    match log.classification {
        Some(PhaseStatus::Clean) => ArtifactClassification::Clean,
        Some(PhaseStatus::Issues) => ArtifactClassification::Issues,
        Some(PhaseStatus::Failure) => ArtifactClassification::Failure,
        None if log.status.is_none() => ArtifactClassification::Failure,
        None => ArtifactClassification::Unclassified,
    }
}

fn command_phase_name(phase: CommandPhase) -> &'static str {
    match phase {
        CommandPhase::InitialCheck => "initial-check",
        CommandPhase::Remedy => "remedy",
        CommandPhase::FinalCheck => "final-check",
        CommandPhase::Combined => "combined",
        CommandPhase::Configuration => "configuration",
    }
}

fn safe_artifact_component(value: &str) -> String {
    let component = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if component.is_empty() {
        "unnamed".into()
    } else {
        component
    }
}

fn build_batch_summary(parts: BatchSummaryParts<'_>) -> hookkit_core::Result<BatchRunSummary> {
    let run_id = run_id(parts.run.directory())?;
    let summary_path = parts.run.directory().join("summary.json");
    let RenderedMessages {
        buckets,
        user,
        agent,
    } = parts.rendered_messages;
    let summary_text = summary_path.to_string_lossy();
    let references_summary = user
        .iter()
        .chain(agent.iter())
        .any(|message| message.contains(summary_text.as_ref()));
    let clean_files = files_with_status(&parts.result, FileStatus::Clean);
    let auto_fixed_files = files_with_status(&parts.result, FileStatus::AutoFixed);
    let manual_fix_files = files_with_status(&parts.result, FileStatus::ManualFixesNeeded);
    let mut grouped = BTreeMap::<String, Vec<PathBuf>>::new();
    for file in parts.result.files.values() {
        grouped
            .entry(file.group_id.clone())
            .or_default()
            .push(file.path.clone());
    }
    let groups = grouped
        .into_iter()
        .map(|(id, mut files)| {
            files.sort();
            files.dedup();
            BatchGroupSummary {
                display_name: if id == "other" {
                    "Other".into()
                } else {
                    id.clone()
                },
                id,
                count: files.len(),
                files,
            }
        })
        .collect::<Vec<_>>();
    let artifact_paths = parts
        .result
        .artifacts
        .values()
        .map(|artifact| artifact.absolute_path.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let artifact_contents = parts
        .result
        .artifacts
        .values()
        .map(|artifact| (artifact.absolute_path.clone(), artifact.contents.clone()))
        .collect();
    let counts = BatchCounts {
        clean: clean_files.len(),
        auto_fixed: auto_fixed_files.len(),
        manual_fixes_needed: manual_fix_files.len(),
        operational_errors: parts.result.operational_problems.len(),
        uncovered: parts.result.uncovered_files.len(),
        not_applicable: parts.result.not_applicable_files.len(),
        coverage_gaps: parts.result.coverage_gaps.len(),
        groups: groups.len(),
    };
    let state_disposition = PlannedStateDisposition {
        source: "acknowledge-sealed-window",
        retry_files: parts.disposition.retry_files.iter().cloned().collect(),
        retry_targets: parts.disposition.retry_targets.clone(),
        retry_gaps: parts.disposition.retry_gaps.iter().cloned().collect(),
        handled_baseline_files: parts.disposition.handled_files.iter().cloned().collect(),
    };
    let (source_entry_count, source_entry_ids) = parts.source;
    Ok(BatchRunSummary {
        schema_version: 1,
        run: BatchRunIdentity {
            id: run_id,
            project_root: parts.project_root.to_path_buf(),
            summary_path,
            state_directory: parts.state_directory.to_path_buf(),
        },
        status: parts.status,
        source_entry_count,
        source_entry_ids,
        candidate_files: parts.candidates.to_vec(),
        counts,
        clean_files,
        auto_fixed_files,
        manual_fix_files,
        groups,
        artifact_paths,
        artifact_contents,
        state_disposition,
        rendered_messages: RenderedMessageMetadata {
            harness: parts.harness.to_string(),
            lowering: parts.lowering,
            buckets,
            user,
            agent,
            references_summary,
        },
        tools: parts.tools,
        result: parts.result,
    })
}

fn files_with_status(result: &DeferredRunResult, status: FileStatus) -> Vec<PathBuf> {
    result
        .files
        .values()
        .filter(|file| file.status == status)
        .map(|file| file.path.clone())
        .collect()
}

fn failure_rendered_messages(summary: &Path, detail: &str) -> RenderedMessages {
    RenderedMessages {
        buckets: RenderedBuckets::default(),
        user: Some(format!(
            "Deferred formatter/linter reporting failed. Details: {}",
            summary.display()
        )),
        agent: Some(format!(
            "Deferred reporting configuration failed: {detail}. Inspect {} before retrying completion.",
            summary.display()
        )),
    }
}

fn source_gap_messages(view: &EntityView<'_, PendingFileActivity>) -> BTreeSet<String> {
    view.events()
        .iter()
        .filter_map(|record| match record.event() {
            FileActivityEvent::Gap(gap) => Some(gap.detail.clone()),
            FileActivityEvent::Retry(retry) if retry.target.is_none() => Some(retry.reason.clone()),
            FileActivityEvent::Evidence(_) | FileActivityEvent::Retry(_) => None,
        })
        .collect()
}

fn deferred_should_block(
    result: &DeferredRunResult,
    coverage_policy: pkl::CoverageGapPolicy,
) -> bool {
    result.has_manual_fixes()
        || result.has_operational_problems()
        || (coverage_policy == pkl::CoverageGapPolicy::Strict && !result.coverage_gaps.is_empty())
}

fn record_activity_resolution(result: &mut DeferredRunResult, resolution: &ActivityResolution) {
    for path in &resolution.not_applicable_files {
        result.record_not_applicable(path.clone());
    }
    for (index, target) in resolution.unresolved_targets.iter().enumerate() {
        let target = serde_json::to_string(target)
            .unwrap_or_else(|_| "unserializable file activity target".into());
        result.record_coverage_gap(CoverageGap {
            id: format!("unresolved-target-{index:03}"),
            target: Some(target.clone()),
            message: format!("file activity target could not be fully materialized: {target}"),
            retained: true,
        });
    }
    for (index, message) in resolution.gap_messages.iter().enumerate() {
        result.record_coverage_gap(CoverageGap {
            id: format!("source-gap-{index:03}"),
            target: None,
            message: message.clone(),
            retained: true,
        });
    }
    if resolution.truncated {
        result.record_coverage_gap(CoverageGap {
            id: "resolution-budget-exhausted".into(),
            target: None,
            message: "file activity target resolution exhausted its traversal budget".into(),
            retained: true,
        });
    }
}

fn plan_deferred_state_disposition(
    result: &DeferredRunResult,
    resolution: &ActivityResolution,
) -> hookkit_core::Result<DeferredStateDisposition> {
    let mut retry_files = BTreeSet::new();
    for file in result.files.values() {
        if file.status == FileStatus::ManualFixesNeeded {
            retry_files.insert(utf8_activity_path(&file.path)?);
        }
    }
    for problem in result.operational_problems.values() {
        for path in &problem.affected_files {
            retry_files.insert(utf8_activity_path(path)?);
        }
    }

    let mut handled_files = BTreeSet::new();
    for file in result.files.values() {
        if matches!(file.status, FileStatus::Clean | FileStatus::AutoFixed) {
            handled_files.insert(utf8_activity_path(&file.path)?);
        }
    }
    // A missing exact path is itself a stable handled state. Recording it
    // prevents an opt-in Git-dirty fallback from resurrecting the same
    // deletion immediately after the source observation is discharged.
    for path in &resolution.not_applicable_files {
        handled_files.insert(utf8_activity_path(path)?);
    }
    handled_files.retain(|path| !retry_files.contains(path));

    let mut retry_targets = resolution.unresolved_targets.clone();
    retry_targets.sort();
    retry_targets.dedup();
    let mut retry_gaps = resolution.gap_messages.clone();
    if resolution.truncated {
        retry_gaps.insert("file activity target resolution exhausted its traversal budget".into());
    }
    Ok(DeferredStateDisposition {
        retry_files,
        retry_targets,
        retry_gaps,
        handled_files,
    })
}

fn apply_deferred_state_disposition(
    activity_store: &FileActivityStore,
    disposition: DeferredStateDisposition,
    run_id: String,
) -> hookkit_core::Result<()> {
    activity_store
        .requeue_exact("deferred-unresolved-file", disposition.retry_files)
        .map_err(activity_error)?;
    activity_store
        .requeue_targets("deferred-unresolved-target", disposition.retry_targets)
        .map_err(activity_error)?;
    activity_store
        .requeue_gaps("deferred-coverage-gap", disposition.retry_gaps)
        .map_err(activity_error)?;
    if disposition.handled_files.is_empty() {
        return Ok(());
    }
    let baseline_report = activity_store
        .record_handled_baselines(disposition.handled_files, run_id)
        .map_err(activity_error)?;
    if baseline_report.failures.is_empty() {
        return Ok(());
    }
    let failures = baseline_report
        .failures
        .iter()
        .map(|failure| format!("{}: {}", failure.path, failure.message))
        .collect::<Vec<_>>()
        .join("; ");
    Err(invalid_data(format!(
        "could not record all handled file baselines; source window retained: {failures}"
    )))
}

fn utf8_activity_path(path: &Path) -> hookkit_core::Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(normalize_path(path)).map_err(|path| {
        invalid_data(format!(
            "deferred file activity path is not valid UTF-8: {}",
            path.display()
        ))
    })
}

fn run_id(directory: &Path) -> hookkit_core::Result<String> {
    directory
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or_else(|| {
            invalid_data(format!(
                "run directory has no UTF-8 id: {}",
                directory.display()
            ))
        })
}

fn record_reporting_failure(
    run: &RunBundle,
    result: &mut DeferredRunResult,
    candidates: &[PathBuf],
    summary_path: &Path,
    contents: String,
) -> hookkit_core::Result<RenderedMessages> {
    let artifact_path = run
        .write_text("reporting-error.log", &contents)
        .map_err(state_error)?;
    result.record_artifact(RunArtifact {
        id: "reporting-configuration".into(),
        absolute_path: artifact_path,
        run_relative_path: "reporting-error.log".into(),
        media_type: "text/plain; charset=utf-8".into(),
        tool_id: None,
        workflow_id: None,
        job_id: None,
        report_id: None,
        phase: CommandPhase::Configuration,
        classification: ArtifactClassification::ConfigurationError,
        exit_code: None,
        program: None,
        arguments: Vec::new(),
        working_directory: None,
        files: candidates.to_vec(),
        candidate_files: candidates.to_vec(),
        changed_files: Vec::new(),
        contents: contents.clone(),
    });
    result.record_operational_problem(OperationalProblem {
        id: "reporting-configuration".into(),
        tool_id: None,
        phase: Some("configuration".into()),
        affected_files: candidates.to_vec(),
        message: contents.clone(),
        artifact_ids: vec!["reporting-configuration".into()],
    });
    Ok(failure_rendered_messages(summary_path, &contents))
}

fn commit_deferred_config_failure(
    ctx: &RuntimeContext<'_>,
    activity_store: &FileActivityStore,
    run: RunBundle,
    failure: DeferredFailureContext<'_>,
    source: (usize, Vec<String>),
    lowering_policy: pkl::LoweringPolicy,
    contents: String,
) -> hookkit_core::Result<EntityOutcome<TurnCompletionOutput>> {
    let (source_entry_count, source_entry_ids) = source;
    let artifact_path = run
        .write_text("config-error.log", &contents)
        .map_err(state_error)?;
    let mut result = DeferredRunResult::default();
    result.record_artifact(RunArtifact {
        id: "configuration".into(),
        absolute_path: artifact_path,
        run_relative_path: "config-error.log".into(),
        media_type: "text/plain; charset=utf-8".into(),
        tool_id: None,
        workflow_id: None,
        job_id: None,
        report_id: None,
        phase: CommandPhase::Configuration,
        classification: ArtifactClassification::ConfigurationError,
        exit_code: None,
        program: None,
        arguments: Vec::new(),
        working_directory: None,
        files: failure.candidates.to_vec(),
        candidate_files: failure.candidates.to_vec(),
        changed_files: Vec::new(),
        contents: contents.clone(),
    });
    result.record_operational_problem(OperationalProblem {
        id: "configuration".into(),
        tool_id: None,
        phase: Some("configuration".into()),
        affected_files: failure.candidates.to_vec(),
        message: contents.clone(),
        artifact_ids: vec!["configuration".into()],
    });
    record_activity_resolution(&mut result, failure.resolution);
    let disposition = plan_deferred_state_disposition(&result, failure.resolution)?;
    let rendered_messages =
        failure_rendered_messages(&run.directory().join("summary.json"), &contents);
    let lowering = plan_stop_lowering(
        ctx.harness(),
        true,
        rendered_messages.user.as_deref(),
        rendered_messages.agent.as_deref(),
        lowering_policy,
    )?;
    let summary = build_batch_summary(BatchSummaryParts {
        run: &run,
        project_root: failure.project_root,
        state_directory: activity_store.state().directory(),
        harness: ctx.harness(),
        status: "operational-failure",
        rendered_messages,
        lowering: lowering.metadata.clone(),
        source: (source_entry_count, source_entry_ids),
        candidates: failure.candidates,
        tools: Vec::new(),
        disposition: &disposition,
        result,
    })?;
    let run_id = summary.run.id.clone();
    run.commit(&summary).map_err(state_error)?;
    let output = lowering.finish()?;
    apply_deferred_state_disposition(activity_store, disposition, run_id)?;
    Ok(EntityOutcome::acknowledge(output))
}

fn state_error(error: hookkit_session_state::StateError) -> HookkitError {
    std::io::Error::other(error).into()
}

fn activity_error(error: hookkit_file_activity::FileActivityError) -> HookkitError {
    std::io::Error::other(error).into()
}

/// Run an exact aligned input through the Pkl-driven runner.
fn run_post_tool_input(
    post_tool: PostToolUseInput,
    _environment: &PostToolUseCommandEnvironment,
    ctx: &RuntimeContext<'_>,
    config_path: Option<&Path>,
) -> hookkit_core::Result<PostToolUseOutput> {
    let harness = ctx.harness();
    if matches!(post_tool, PostToolUseInput::Antigravity(_)) {
        return lower_domain_outcome(
            harness,
            RunnerDomainOutcome::UnsupportedHarness {
                harness: harness.to_string(),
                reason: "the native event has no tool call or changed-file payload".into(),
            },
        );
    }
    let cwd = ctx
        .workspace_roots()
        .first()
        .map(|root| PathBuf::from(root.as_str()))
        .ok_or_else(|| invalid_data("post-tool-use input has no workspace root".into()))?;
    let loaded = hookkit_pkl_config::discover_and_load(&cwd, config_path)
        .map_err(|e| invalid_data(e.to_string()))?;

    let project_root = normalize_path(&loaded.project_root);
    let lowering = loaded.config.settings.lowering_policy;
    let missing_tool_policy = loaded.config.settings.missing_tool_policy;
    let fail_fast = loaded.config.settings.fail_fast;
    let continue_after_issues = loaded.config.settings.continue_after_issues;

    let mut output = RunnerPostToolUseOutput::new(lowering);
    let mut had_hard_failure = false;
    let mut had_harness_block_message: Option<String> = None;

    let tools = resolve_run_order(&loaded.config)?;
    if tools.is_empty() {
        return lower_domain_outcome(harness, RunnerDomainOutcome::Clean);
    }

    let global_exclude = &loaded.config.settings.exclude;
    let global_diagnostics_dir = loaded.config.settings.diagnostics_directory.clone();

    for schema_spec in tools {
        if !schema_spec.enabled {
            continue;
        }
        let spec = convert_tool_spec(schema_spec, global_exclude);
        let context = ToolContext {
            spec: &spec,
            project_root: &project_root,
            global_diagnostics_dir: global_diagnostics_dir.as_deref(),
        };

        let candidates = discover_modified_files(&post_tool, ctx);
        let matcher = FileMatcher::new(&spec.file_selection)?;
        let runnable_paths = candidates
            .into_iter()
            .filter(|p| p.is_file())
            .filter(|p| matcher.matches(p, &project_root))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();

        if runnable_paths.is_empty() {
            continue;
        }

        let jobs = build_jobs(&runnable_paths, &project_root, &spec);
        if jobs.is_empty() {
            continue;
        }

        let outcomes = run_jobs(&jobs, &context, loaded.config.settings.jobs);

        let batch_status = accumulate_outcomes(
            outcomes,
            &context,
            ctx,
            missing_tool_policy,
            &mut output,
            &mut had_hard_failure,
            &mut had_harness_block_message,
        )?;

        if had_harness_block_message.is_some()
            || had_hard_failure
            || (fail_fast && batch_status.operational_failure)
            || (!continue_after_issues && batch_status.issues)
        {
            break;
        }
    }

    let outcome = if let Some(message) = had_harness_block_message {
        RunnerDomainOutcome::HarnessBlock { message, output }
    } else if had_hard_failure {
        RunnerDomainOutcome::OperationalFailure {
            message: "tool unavailable with missingToolPolicy=hard-failure".into(),
        }
    } else if is_empty_output(&output) {
        RunnerDomainOutcome::Clean
    } else {
        RunnerDomainOutcome::Report(output)
    };
    lower_domain_outcome(harness, outcome)
}

#[derive(Debug, Default)]
pub struct RunnerPostToolUseOutput {
    notices: Vec<UserNotice>,
    agent_feedback: Vec<String>,
    diagnostics: Vec<DiagnosticReport>,
    harness_block: Option<String>,
    lowering: pkl::LoweringPolicy,
}

impl RunnerPostToolUseOutput {
    fn new(lowering: pkl::LoweringPolicy) -> Self {
        Self {
            lowering,
            ..Self::default()
        }
    }

    fn with_user_notice(mut self, notice: UserNotice) -> Self {
        self.notices.push(notice);
        self
    }

    fn with_agent_feedback(mut self, feedback: impl Into<String>) -> Self {
        self.agent_feedback.push(feedback.into());
        self
    }

    fn with_diagnostic_report(mut self, report: DiagnosticReport) -> Self {
        self.diagnostics.push(report);
        self
    }

    fn with_harness_block(mut self, message: impl Into<String>) -> Self {
        self.harness_block = Some(message.into());
        self
    }
}

/// Runner-owned semantic result. Tool policy and classification deliberately do
/// not leak into core/common crates.
#[derive(Debug)]
pub enum RunnerDomainOutcome {
    Clean,
    Report(RunnerPostToolUseOutput),
    HarnessBlock {
        message: String,
        output: RunnerPostToolUseOutput,
    },
    OperationalFailure {
        message: String,
    },
    UnsupportedHarness {
        harness: String,
        reason: String,
    },
}

fn lower_domain_outcome(
    harness: &HarnessId,
    outcome: RunnerDomainOutcome,
) -> hookkit_core::Result<PostToolUseOutput> {
    match outcome {
        RunnerDomainOutcome::Clean => lower_report(harness, RunnerPostToolUseOutput::default()),
        RunnerDomainOutcome::Report(output) => lower_report(harness, output),
        RunnerDomainOutcome::HarnessBlock { message, output } => {
            lower_report(harness, output.with_harness_block(message))
        }
        RunnerDomainOutcome::OperationalFailure { message } => Err(invalid_data(message)),
        RunnerDomainOutcome::UnsupportedHarness { harness, reason } => Err(invalid_data(format!(
            "post-tool-use runner does not support {harness}: {reason}"
        ))),
    }
}

fn lower_report(
    harness: &HarnessId,
    output: RunnerPostToolUseOutput,
) -> hookkit_core::Result<PostToolUseOutput> {
    if let Some(message) = output.harness_block {
        return match harness.as_str() {
            "claude-code" => Ok(PostToolUseOutput::Claude(
                hookkit_claude::protocol::PostToolUseOutput::blocking_error(message),
            )),
            "codex" => Ok(PostToolUseOutput::Codex(
                hookkit_codex::protocol::PostToolUseOutput::blocking_error(message),
            )),
            "gemini-cli" => Ok(PostToolUseOutput::Gemini(
                hookkit_gemini::protocol::AfterToolOutput::blocking_error(message),
            )),
            _ => Err(invalid_data(format!(
                "post-tool-use runner does not support {harness}"
            ))),
        };
    }

    let mut stderr = output
        .notices
        .iter()
        .map(format_notice)
        .chain(output.diagnostics.iter().map(format_diagnostic))
        .collect::<Vec<_>>();
    if !stderr.is_empty() {
        match output.lowering {
            pkl::LoweringPolicy::Strict => {
                return Err(invalid_data(format!(
                    "{harness} PostToolUse has no structured user-only message channel"
                )));
            }
            pkl::LoweringPolicy::BestEffort => {}
            pkl::LoweringPolicy::BestEffortWithWarnings => stderr
                .push("hookkit: redirected user notices and diagnostics to protocol stderr".into()),
        }
    }
    let stderr = (!stderr.is_empty()).then(|| stderr.join("\n"));
    let context = output.agent_feedback.join("\n");

    match harness.as_str() {
        "claude-code" => {
            let native = if context.is_empty() {
                hookkit_claude::protocol::PostToolUseOutput::no_op()
            } else {
                hookkit_claude::protocol::PostToolUseOutput::with_context(context)
            };
            Ok(PostToolUseOutput::Claude(match stderr {
                Some(stderr) => native.with_protocol_stderr(stderr)?,
                None => native,
            }))
        }
        "codex" => {
            let native = if context.is_empty() {
                hookkit_codex::protocol::PostToolUseOutput::no_op()
            } else {
                hookkit_codex::protocol::PostToolUseOutput::with_context(context)
            };
            Ok(PostToolUseOutput::Codex(match stderr {
                Some(stderr) => native.with_protocol_stderr(stderr)?,
                None => native,
            }))
        }
        "gemini-cli" => {
            let native = if context.is_empty() {
                hookkit_gemini::protocol::AfterToolOutput::no_op()
            } else {
                hookkit_gemini::protocol::AfterToolOutput::with_context(context)
            };
            Ok(PostToolUseOutput::Gemini(match stderr {
                Some(stderr) => native.with_protocol_stderr(stderr)?,
                None => native,
            }))
        }
        _ => Err(invalid_data(format!(
            "post-tool-use runner does not support {harness}"
        ))),
    }
}

fn format_notice(notice: &UserNotice) -> String {
    match notice.level {
        NoticeLevel::Info => notice.text.clone(),
        NoticeLevel::Warning => format!("warning: {}", notice.text),
        NoticeLevel::Error => format!("error: {}", notice.text),
    }
}

fn format_diagnostic(diagnostic: &DiagnosticReport) -> String {
    let mut rendered = format!("{}:\n{}", diagnostic.title, diagnostic.text.trim());
    if let Some(artifact) = &diagnostic.artifact {
        rendered.push_str(&format!("\nartifact: {}", artifact.absolute_path.display()));
    }
    rendered
}

fn is_empty_output(output: &RunnerPostToolUseOutput) -> bool {
    output.notices.is_empty()
        && output.agent_feedback.is_empty()
        && output.diagnostics.is_empty()
        && output.harness_block.is_none()
}

/// Resolve the `run` list to ordered tool specs.
fn resolve_run_order(config: &pkl::RunnerConfig) -> hookkit_core::Result<Vec<&pkl::ToolSpec>> {
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

#[derive(Debug, Clone, Copy, Default)]
struct ToolBatchStatus {
    operational_failure: bool,
    issues: bool,
}

/// Convert a Pkl-shaped tool spec to the runtime execution type.
fn convert_tool_spec(spec: &pkl::ToolSpec, global_exclude: &[String]) -> ToolSpec {
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
        enabled: spec.enabled,
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
                check_scope: match workflow.check_scope {
                    pkl::CheckScope::TargetFiles => CheckScope::TargetFiles,
                    pkl::CheckScope::Workspace => CheckScope::Workspace,
                },
                invocation: match workflow.invocation {
                    pkl::InvocationGranularity::PerFile => InvocationGranularity::PerFile,
                    pkl::InvocationGranularity::Batch => InvocationGranularity::Batch,
                    pkl::InvocationGranularity::Workspace => InvocationGranularity::Workspace,
                },
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
            check_scope: if spec.workspace_indicator.is_some()
                && !remedy.args.iter().any(|arg| {
                    matches!(
                        arg,
                        CommandArgTemplate::Files | CommandArgTemplate::WorkspaceFiles
                    )
                }) {
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
        writes: convert_writes(command.writes),
        extra_args: command.extra_args.clone(),
        enabled: true,
    }
}

fn ordered_phases(spec: &pkl::ToolSpec) -> Vec<(String, &pkl::Phase)> {
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

fn canonical_mode_order(mode: pkl::PhaseMode) -> u8 {
    match mode {
        pkl::PhaseMode::Format => 0,
        pkl::PhaseMode::Fix => 1,
        pkl::PhaseMode::Verify => 2,
        pkl::PhaseMode::CheckOnly => 3,
    }
}

fn convert_phase((id, phase): (String, &pkl::Phase)) -> ToolPhase {
    ToolPhase {
        id,
        mode: convert_phase_mode(phase.mode),
        program: phase.program.clone(),
        args: phase.argv.iter().map(convert_argv_element).collect(),
        exit_codes: convert_exit_codes(&phase.exit_codes),
        issues_on_stdout: false,
        writes: convert_writes(phase.writes),
        extra_args: phase.extra_args.clone(),
        enabled: phase.enabled,
    }
}

fn convert_phase_mode(mode: pkl::PhaseMode) -> PhaseMode {
    match mode {
        pkl::PhaseMode::Format => PhaseMode::Format,
        pkl::PhaseMode::Fix => PhaseMode::Fix,
        pkl::PhaseMode::Verify => PhaseMode::Verify,
        pkl::PhaseMode::CheckOnly => PhaseMode::CheckOnly,
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
        unexpected: match codes.unexpected {
            pkl::UnexpectedExitPolicy::Failure => UnexpectedExitPolicy::Failure,
            pkl::UnexpectedExitPolicy::Issues => UnexpectedExitPolicy::Issues,
        },
    }
}

fn convert_writes(writes: pkl::WriteBehavior) -> WriteBehavior {
    match writes {
        pkl::WriteBehavior::None => WriteBehavior::None,
        pkl::WriteBehavior::TargetFiles => WriteBehavior::TargetFiles,
        pkl::WriteBehavior::MatchingGlobs => WriteBehavior::MatchingGlobs,
        pkl::WriteBehavior::Workspace => WriteBehavior::Workspace,
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

// ----------------------------------------------------------------------------
// File matching
// ----------------------------------------------------------------------------

struct FileMatcher {
    include: GlobSet,
    exclude: GlobSet,
    include_all: bool,
}

impl FileMatcher {
    fn new(config: &FileSelection) -> hookkit_core::Result<Self> {
        Ok(Self {
            include: build_globset(&config.include)?,
            exclude: build_globset(&config.exclude)?,
            include_all: config.include.is_empty(),
        })
    }

    fn matches(&self, absolute_path: &Path, project_root: &Path) -> bool {
        let rel = absolute_path
            .strip_prefix(project_root)
            .unwrap_or(absolute_path);
        let rel = slash_path(rel);
        let abs = slash_path(absolute_path);

        (self.include_all || self.include.is_match(&rel) || self.include.is_match(&abs))
            && !(self.exclude.is_match(&rel) || self.exclude.is_match(&abs))
    }
}

fn build_globset(patterns: &[String]) -> hookkit_core::Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = Glob::new(pattern)
            .map_err(|e| invalid_data(format!("invalid file glob `{pattern}`: {e}")))?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|e| invalid_data(format!("invalid file glob set: {e}")))
}

// ----------------------------------------------------------------------------
// Per-tool execution
// ----------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ToolContext<'a> {
    spec: &'a ToolSpec,
    project_root: &'a Path,
    global_diagnostics_dir: Option<&'a str>,
}

#[derive(Debug, Clone)]
struct ToolJob {
    workspace_dir: PathBuf,
    workspace_indicator: Option<PathBuf>,
    files: Vec<PathBuf>,
}

fn build_jobs(paths: &[PathBuf], project_root: &Path, spec: &ToolSpec) -> Vec<ToolJob> {
    if let Some(indicator) = &spec.workspace_indicator {
        let mut grouped = BTreeMap::<PathBuf, ToolJob>::new();
        for path in paths {
            if let Some(indicator_path) = nearest_workspace_indicator(path, project_root, indicator)
            {
                let workspace_dir = indicator_path
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| project_root.to_path_buf());
                grouped
                    .entry(workspace_dir.clone())
                    .or_insert_with(|| ToolJob {
                        workspace_dir,
                        workspace_indicator: Some(indicator_path),
                        files: Vec::new(),
                    })
                    .files
                    .push(path.clone());
            }
        }
        grouped.into_values().collect()
    } else {
        vec![ToolJob {
            workspace_dir: project_root.to_path_buf(),
            workspace_indicator: None,
            files: paths.to_vec(),
        }]
    }
}

fn nearest_workspace_indicator(
    path: &Path,
    project_root: &Path,
    indicator: &str,
) -> Option<PathBuf> {
    let mut current = path.parent();
    while let Some(dir) = current {
        let candidate = dir.join(indicator);
        if candidate.is_file() {
            return Some(candidate);
        }
        if dir == project_root {
            break;
        }
        current = dir.parent();
    }
    None
}

#[derive(Debug)]
enum ToolRunOutcome {
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
struct CompletedToolOutcome {
    issues: IssueState,
    changes: ChangeState,
    diagnostics: String,
    files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IssueState {
    Clean,
    Issues,
}

#[derive(Debug)]
enum ChangeState {
    Unchanged,
    Changed { files: Vec<PathBuf> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhaseStatus {
    Clean,
    Issues,
    Failure,
}

#[derive(Debug)]
struct PhaseLog {
    phase: String,
    command: String,
    program: String,
    arguments: Vec<String>,
    status: Option<i32>,
    classification: Option<PhaseStatus>,
    stdout: String,
    stderr: String,
    error: Option<String>,
}

/// Run a tool's independent per-workspace jobs, honoring `settings.jobs` for
/// bounded parallelism. Outcomes are returned in job order regardless of which
/// job finishes first, so downstream aggregation stays deterministic.
fn run_jobs(jobs: &[ToolJob], context: &ToolContext<'_>, jobs_setting: u32) -> Vec<ToolRunOutcome> {
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

/// Resolve `settings.jobs` to a worker-thread count for a batch of `job_count`
/// independent jobs.
///
/// `jobs = 0` selects "auto", which is reserved for future use and runs
/// serially for now. `jobs = n >= 1` runs up to `n` jobs concurrently, capped
/// at `job_count` since extra workers would have nothing to claim.
fn resolve_worker_count(jobs_setting: u32, job_count: usize) -> usize {
    if job_count == 0 {
        return 0;
    }
    let requested = match jobs_setting {
        0 => 1, // auto: reserved for future use; serial for now
        n => n as usize,
    };
    requested.clamp(1, job_count)
}

fn run_job(job: &ToolJob, context: &ToolContext<'_>) -> ToolRunOutcome {
    let before_scope = snapshot_scope(job, context);
    let before = Snapshot::read(&before_scope);
    let mut logs = Vec::new();
    let mut saw_issues = false;
    let mut verify_state = None;

    for phase in &context.spec.phases {
        if !phase.enabled {
            continue;
        }

        let command = render_command(phase, job, context);
        let log = run_phase_command(phase, &command, &job.workspace_dir);

        if let Some(error) = &log.error {
            if error == "not found" {
                let executable = command.program.clone();
                return ToolRunOutcome::ToolUnavailable {
                    phase: phase.id.clone(),
                    executable,
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

    let after_scope = snapshot_scope(job, context);
    let after = Snapshot::read(&after_scope);
    let changed_files = before.changed_files(&after);
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
    let after_scope = snapshot_scope(job, context);
    let after = Snapshot::read(&after_scope);
    before.changed_files(&after)
}

#[derive(Debug)]
struct RenderedCommand {
    program: String,
    args: Vec<String>,
}

fn render_command(phase: &ToolPhase, job: &ToolJob, context: &ToolContext<'_>) -> RenderedCommand {
    let program = phase
        .program
        .clone()
        .unwrap_or_else(|| context.spec.executable.clone());
    let mut args = Vec::new();
    for arg in &phase.args {
        match arg {
            CommandArgTemplate::Literal(value) => args.push(value.clone()),
            CommandArgTemplate::Files => args.extend(job.files.iter().map(|path| path_arg(path))),
            CommandArgTemplate::WorkspaceFiles => {
                args.extend(job.files.iter().map(|path| {
                    path.strip_prefix(&job.workspace_dir)
                        .map(path_arg)
                        .unwrap_or_else(|_| path_arg(path))
                }));
            }
            CommandArgTemplate::Workspace => args.push(path_arg(&job.workspace_dir)),
            CommandArgTemplate::WorkspaceIndicator => {
                if let Some(path) = &job.workspace_indicator {
                    args.push(path_arg(path));
                }
            }
            CommandArgTemplate::ProjectRoot => args.push(path_arg(context.project_root)),
            CommandArgTemplate::ToolExecutable => args.push(context.spec.executable.clone()),
            CommandArgTemplate::ExtraArgs => args.extend(phase.extra_args.iter().cloned()),
        }
    }
    RenderedCommand { program, args }
}

fn run_phase_command(phase: &ToolPhase, command: &RenderedCommand, cwd: &Path) -> PhaseLog {
    match Command::new(&command.program)
        .args(&command.args)
        .current_dir(cwd)
        .output()
    {
        Ok(output) => {
            let status = output.status.code();
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let mut classification = status.map(|code| classify_exit_code(&phase.exit_codes, code));
            if phase.issues_on_stdout
                && classification == Some(PhaseStatus::Clean)
                && !stdout.trim().is_empty()
            {
                classification = Some(PhaseStatus::Issues);
            }
            PhaseLog {
                phase: phase.id.clone(),
                command: display_command(&command.program, &command.args),
                program: command.program.clone(),
                arguments: command.args.clone(),
                status,
                classification,
                stdout,
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                error: None,
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => PhaseLog {
            phase: phase.id.clone(),
            command: display_command(&command.program, &command.args),
            program: command.program.clone(),
            arguments: command.args.clone(),
            status: None,
            classification: None,
            stdout: String::new(),
            stderr: String::new(),
            error: Some("not found".to_string()),
        },
        Err(e) => PhaseLog {
            phase: phase.id.clone(),
            command: display_command(&command.program, &command.args),
            program: command.program.clone(),
            arguments: command.args.clone(),
            status: None,
            classification: None,
            stdout: String::new(),
            stderr: String::new(),
            error: Some(e.to_string()),
        },
    }
}

fn classify_exit_code(policy: &ExitCodePolicy, code: i32) -> PhaseStatus {
    if policy.clean.contains(&code) {
        PhaseStatus::Clean
    } else if policy.issues.contains(&code) {
        PhaseStatus::Issues
    } else if policy.failure.contains(&code) {
        PhaseStatus::Failure
    } else {
        match policy.unexpected {
            UnexpectedExitPolicy::Failure => PhaseStatus::Failure,
            UnexpectedExitPolicy::Issues => PhaseStatus::Issues,
        }
    }
}

// ----------------------------------------------------------------------------
// Snapshots
// ----------------------------------------------------------------------------

#[derive(Debug)]
struct Snapshot {
    files: BTreeMap<PathBuf, Option<Vec<u8>>>,
}

impl Snapshot {
    fn read(paths: &BTreeSet<PathBuf>) -> Self {
        let files = paths
            .iter()
            .map(|path| {
                let bytes = if path.is_file() {
                    std::fs::read(path).ok()
                } else {
                    None
                };
                (path.clone(), bytes)
            })
            .collect();
        Self { files }
    }

    fn changed_files(&self, after: &Self) -> Vec<PathBuf> {
        self.files
            .keys()
            .chain(after.files.keys())
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|path| self.files.get(path) != after.files.get(path))
            .collect()
    }
}

fn snapshot_scope(job: &ToolJob, context: &ToolContext<'_>) -> BTreeSet<PathBuf> {
    let mut scope = BTreeSet::new();
    let mut include_target_files = false;
    let mut include_matching_globs = false;
    let mut include_workspace = false;

    for phase in &context.spec.phases {
        if !phase.enabled {
            continue;
        }
        match phase.writes {
            WriteBehavior::None => {}
            WriteBehavior::TargetFiles => include_target_files = true,
            WriteBehavior::MatchingGlobs => include_matching_globs = true,
            WriteBehavior::Workspace => include_workspace = true,
        }
    }

    if include_target_files {
        scope.extend(job.files.iter().cloned());
    }
    if include_matching_globs {
        scope.extend(collect_matching_files(
            &job.workspace_dir,
            &context.spec.file_selection,
        ));
    }
    if include_workspace {
        scope.extend(collect_workspace_files(&job.workspace_dir));
    }
    scope
}

fn collect_matching_files(base: &Path, selection: &FileSelection) -> BTreeSet<PathBuf> {
    let matcher = match FileMatcher::new(selection) {
        Ok(matcher) => matcher,
        Err(_) => return BTreeSet::new(),
    };
    walk_files(base)
        .into_iter()
        .filter(|path| matcher.matches(path, base))
        .collect()
}

fn collect_workspace_files(base: &Path) -> BTreeSet<PathBuf> {
    walk_files(base).into_iter().collect()
}

fn walk_files(base: &Path) -> Vec<PathBuf> {
    walkdir::WalkDir::new(base)
        .into_iter()
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            !matches!(name.as_ref(), ".git" | "target" | "node_modules")
        })
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.path().to_path_buf())
        .collect()
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
    had_hard_failure: &mut bool,
    had_harness_block_message: &mut Option<String>,
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

    let mut status = ToolBatchStatus {
        operational_failure: !unavailable.is_empty() || !failure_diagnostics.is_empty(),
        issues: !issue_diagnostics.is_empty(),
    };

    if !unavailable.is_empty() {
        match missing_tool_policy {
            pkl::MissingToolPolicy::UserNotice => {
                for (phase, executable, install_hint) in &unavailable {
                    let message = render_unavailable_message(
                        context,
                        phase,
                        executable,
                        install_hint.as_deref(),
                    )?;
                    *output = std::mem::take(output).with_user_notice(UserNotice::warning(message));
                }
            }
            pkl::MissingToolPolicy::HardFailure => {
                *had_hard_failure = true;
                return Ok(status);
            }
            pkl::MissingToolPolicy::HarnessBlock => {
                if let Some((phase, executable, install_hint)) = unavailable.first() {
                    let message = render_unavailable_message(
                        context,
                        phase,
                        executable,
                        install_hint.as_deref(),
                    )?;
                    *had_harness_block_message = Some(message);
                }
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

    let changed_paths = changed_files
        .iter()
        .map(|path| rel_display(path, context.project_root))
        .collect::<Vec<_>>();
    let issue_paths = issue_files
        .iter()
        .map(|path| rel_display(path, context.project_root))
        .collect::<Vec<_>>();

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
    } else if !changed_paths.is_empty() {
        *output = std::mem::take(output).with_user_notice(UserNotice::info(format!(
            "{}: changed {}",
            context.spec.display_name,
            changed_paths.join(", ")
        )));
        let template = context.spec.messages.clean_changed_agent.clone();
        let rendered =
            render_template(&template, context, &changed_paths, &issue_paths, None, None)?;
        *output = std::mem::take(output).with_agent_feedback(rendered);
    }

    status.operational_failure = status.operational_failure || *had_hard_failure;
    Ok(status)
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

fn write_diagnostics(
    label: &str,
    diagnostics: &str,
    context: &ToolContext<'_>,
    ctx: &RuntimeContext<'_>,
) -> hookkit_core::Result<PathBuf> {
    let base_dir = match (
        context.spec.diagnostics_directory.as_deref(),
        context.global_diagnostics_dir,
    ) {
        (Some(dir), _) => absolute_from(Path::new(dir), context.project_root),
        (None, Some(dir)) => absolute_from(Path::new(dir), context.project_root),
        (None, None) => std::env::temp_dir().join("hookkit-artifacts"),
    };
    let manager = ArtifactManager::new(base_dir)?;
    manager
        .write_text(
            &runner_artifact_key(ctx, format!("{}-{label}", context.spec.id)),
            diagnostics,
        )
        .map_err(Into::into)
}

fn runner_artifact_key(ctx: &RuntimeContext<'_>, label: String) -> ArtifactKey {
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

fn format_logs(logs: &[PhaseLog]) -> String {
    let mut out = String::new();
    for log in logs {
        out.push_str(&format!(
            "[{phase}] command: {command}\nstatus: {status:?}\nclassification: {classification:?}\n",
            phase = log.phase,
            command = log.command,
            status = log.status,
            classification = log.classification
        ));
        if let Some(err) = &log.error {
            out.push_str(&format!("error: {err}\n"));
        }
        if !log.stdout.trim().is_empty() {
            out.push_str("stdout:\n");
            out.push_str(&log.stdout);
            if !log.stdout.ends_with('\n') {
                out.push('\n');
            }
        }
        if !log.stderr.trim().is_empty() {
            out.push_str("stderr:\n");
            out.push_str(&log.stderr);
            if !log.stderr.ends_with('\n') {
                out.push('\n');
            }
        }
        out.push('\n');
    }
    out
}

fn display_command(program: &str, args: &[String]) -> String {
    std::iter::once(program.to_string())
        .chain(args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn absolute_from(path: &Path, base: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

fn slash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn rel_display(path: &Path, project_root: &Path) -> String {
    path.strip_prefix(project_root)
        .map(slash_path)
        .unwrap_or_else(|_| slash_path(path))
}

fn invalid_data(message: String) -> HookkitError {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn domain_outcomes_keep_clean_failure_and_unsupported_distinct() {
        assert!(matches!(
            lower_domain_outcome(&HarnessId::CLAUDE_CODE, RunnerDomainOutcome::Clean).unwrap(),
            PostToolUseOutput::Claude(_)
        ));
        assert!(
            lower_domain_outcome(
                &HarnessId::CLAUDE_CODE,
                RunnerDomainOutcome::OperationalFailure {
                    message: "checker crashed".into(),
                }
            )
            .is_err()
        );
        assert!(
            lower_domain_outcome(
                &HarnessId::ANTIGRAVITY,
                RunnerDomainOutcome::UnsupportedHarness {
                    harness: "antigravity".into(),
                    reason: "no changed-file data".into(),
                }
            )
            .is_err()
        );
    }

    #[test]
    fn resolve_worker_count_honors_jobs_setting() {
        // auto (0) is reserved for future use and runs serially for now.
        assert_eq!(resolve_worker_count(0, 5), 1);
        // explicit serial.
        assert_eq!(resolve_worker_count(1, 5), 1);
        // bounded parallelism up to the requested count.
        assert_eq!(resolve_worker_count(4, 5), 4);
        // never spin up more workers than there are jobs.
        assert_eq!(resolve_worker_count(8, 5), 5);
        assert_eq!(resolve_worker_count(2, 1), 1);
        // no jobs means no workers.
        assert_eq!(resolve_worker_count(4, 0), 0);
        assert_eq!(resolve_worker_count(0, 0), 0);
    }

    #[test]
    fn state_disposition_discharges_successes_and_retries_only_unfinished_files() {
        let root = PathBuf::from("/tmp/hookkit-selective-disposition");
        let clean = root.join("clean.rs");
        let auto_fixed = root.join("auto.rs");
        let manual = root.join("manual.rs");
        let operational = root.join("operational.rs");
        let deleted = root.join("deleted.rs");
        let mut result = DeferredRunResult::default();
        result.record_file(FileAssessment::new(&clean, FileStatus::Clean));
        result.record_file(FileAssessment::new(&auto_fixed, FileStatus::AutoFixed));
        result.record_file(FileAssessment::new(&manual, FileStatus::ManualFixesNeeded));
        result.record_operational_problem(OperationalProblem {
            id: "tool-failure".into(),
            tool_id: Some("tool".into()),
            phase: Some("initial-check".into()),
            affected_files: vec![operational.clone()],
            message: "tool crashed".into(),
            artifact_ids: Vec::new(),
        });
        let unresolved = FileActivityTarget::Workspace {
            root: Some(Utf8PathBuf::from("/tmp/hookkit-selective-disposition")),
        };
        let resolution = ActivityResolution {
            not_applicable_files: BTreeSet::from([deleted.clone()]),
            unresolved_targets: vec![unresolved.clone()],
            gap_messages: BTreeSet::from(["dynamic shell target".into()]),
            truncated: false,
        };

        let disposition = plan_deferred_state_disposition(&result, &resolution).unwrap();
        assert_eq!(
            disposition.retry_files,
            BTreeSet::from([
                Utf8PathBuf::from_path_buf(manual).unwrap(),
                Utf8PathBuf::from_path_buf(operational).unwrap(),
            ])
        );
        assert_eq!(disposition.retry_targets, vec![unresolved]);
        assert_eq!(
            disposition.retry_gaps,
            BTreeSet::from(["dynamic shell target".into()])
        );
        assert_eq!(
            disposition.handled_files,
            BTreeSet::from([
                Utf8PathBuf::from_path_buf(auto_fixed).unwrap(),
                Utf8PathBuf::from_path_buf(clean).unwrap(),
                Utf8PathBuf::from_path_buf(deleted).unwrap(),
            ])
        );
    }

    #[test]
    fn coverage_gap_policy_is_best_effort_by_default_and_strict_on_request() {
        let mut result = DeferredRunResult::default();
        result.record_file(FileAssessment::new("clean.rs", FileStatus::Clean));
        result.record_coverage_gap(CoverageGap {
            id: "gap".into(),
            target: None,
            message: "dynamic target".into(),
            retained: true,
        });

        assert!(!deferred_should_block(
            &result,
            pkl::CoverageGapPolicy::BestEffort
        ));
        assert!(deferred_should_block(
            &result,
            pkl::CoverageGapPolicy::Strict
        ));
    }

    #[test]
    fn deferred_artifact_argv_is_unambiguous_and_path_components_are_safe() {
        let log = DeferredLog {
            tool_index: 0,
            workflow_index: 1,
            job_index: 2,
            phase: CommandPhase::InitialCheck,
            log: PhaseLog {
                phase: "check".into(),
                command: "checker argument with spaces line break".into(),
                program: "checker tool".into(),
                arguments: vec!["argument with spaces".into(), "line\nbreak".into()],
                status: Some(0),
                classification: Some(PhaseStatus::Clean),
                stdout: "ok".into(),
                stderr: String::new(),
                error: None,
            },
        };
        let contents = format_deferred_artifact(&log).unwrap();
        let argv = contents
            .lines()
            .find_map(|line| line.strip_prefix("argv: "))
            .expect("argv line");
        assert_eq!(
            serde_json::from_str::<Vec<String>>(argv).unwrap(),
            vec!["checker tool", "argument with spaces", "line\nbreak"]
        );
        assert_eq!(
            safe_artifact_component("../../tool/name"),
            "______tool_name"
        );
    }

    proptest! {
        /// Property: worker selection is total and bounded. A non-empty batch
        /// always gets at least one worker, never more workers than jobs, and
        /// explicit settings are honored up to that cap (`0` means serial auto).
        #[test]
        fn worker_count_is_bounded(jobs_setting in any::<u32>(), job_count in any::<usize>()) {
            let actual = resolve_worker_count(jobs_setting, job_count);
            let expected = if job_count == 0 {
                0
            } else {
                usize::try_from(jobs_setting.max(1)).unwrap_or(usize::MAX).min(job_count)
            };

            prop_assert_eq!(actual, expected);
            prop_assert!(actual <= job_count);
            prop_assert_eq!(actual == 0, job_count == 0);
        }

        /// Property: overlapping exit-code policy lists have a documented
        /// precedence (clean, then issues, then failure), and unlisted values
        /// use exactly the configured fallback.
        #[test]
        fn exit_code_classification_has_stable_precedence(
            clean in prop::collection::vec(any::<i32>(), 0..30),
            issues in prop::collection::vec(any::<i32>(), 0..30),
            failure in prop::collection::vec(any::<i32>(), 0..30),
            code in any::<i32>(),
            unexpected_issues in any::<bool>(),
        ) {
            let unexpected = if unexpected_issues {
                UnexpectedExitPolicy::Issues
            } else {
                UnexpectedExitPolicy::Failure
            };
            let policy = ExitCodePolicy {
                clean: clean.clone(),
                issues: issues.clone(),
                failure: failure.clone(),
                unexpected,
            };
            let expected = if clean.contains(&code) {
                PhaseStatus::Clean
            } else if issues.contains(&code) {
                PhaseStatus::Issues
            } else if failure.contains(&code) {
                PhaseStatus::Failure
            } else if unexpected_issues {
                PhaseStatus::Issues
            } else {
                PhaseStatus::Failure
            };

            prop_assert_eq!(classify_exit_code(&policy, code), expected);
        }

        /// Property: lexical normalization for not-yet-created output paths is
        /// idempotent, absolute, and cannot retain traversal above root.
        #[test]
        fn non_existing_output_path_normalization_is_stable(
            segments in prop::collection::vec(prop_oneof![Just(".".to_owned()), Just("..".to_owned()), "[a-z]{1,8}"], 0..30),
        ) {
            let path = PathBuf::from(format!(
                "/hookkit-property-path-that-does-not-exist/{}/{}",
                std::process::id(),
                segments.join("/")
            ));
            let once = normalize_path(&path);
            let twice = normalize_path(&once);

            prop_assert_eq!(&once, &twice);
            prop_assert!(once.is_absolute());
            let contains_traversal = once.components().any(|component| {
                matches!(component, Component::CurDir | Component::ParentDir)
            });
            prop_assert!(!contains_traversal);
        }
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
        let context = ToolContext {
            spec: &spec,
            project_root: &root,
            global_diagnostics_dir: None,
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
    #[test]
    fn hermetic_fake_executable_smoke() {
        let root =
            std::env::temp_dir().join(format!("hookkit-hermetic-smoke-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create smoke directory");

        let fake = root.join("fake-checker");
        std::fs::write(&fake, "#!/bin/sh\nprintf 'fake checker clean\\n'\n")
            .expect("write fake executable");
        let mut permissions = std::fs::metadata(&fake)
            .expect("read fake executable metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&fake, permissions).expect("make fake executable runnable");

        let target = root.join("input.rs");
        std::fs::write(&target, "fn main() {}\n").expect("write smoke input");
        let spec = ToolSpec::new("fake", "Fake checker", fake.to_string_lossy().into_owned())
            .with_phase(
                ToolPhase::new("verify", PhaseMode::Verify).with_args([CommandArgTemplate::Files]),
            );
        let context = ToolContext {
            spec: &spec,
            project_root: &root,
            global_diagnostics_dir: None,
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
}
