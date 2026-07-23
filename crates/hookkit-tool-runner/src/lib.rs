//! Reusable runners for immediate post-tool and session-batched completion hooks.
#![deny(missing_docs)]
//!
//! The runner reads a harness-native post-tool-use event from stdin, loads the
//! Pkl-driven tool catalog through [`hookkit_pkl_config`], runs each tool in
//! the configured order, and lowers a unified result to the selected harness.
//! The completion runner consumes exact snapshots from
//! [`hookkit_session_state`] and commits detailed run bundles before deciding
//! whether a turn may stop.

use globset::{Glob, GlobSet, GlobSetBuilder};
use hookkit_common::message::{DiagnosticArtifact, DiagnosticReport};
use hookkit_common::{
    NoticeLevel, PostToolUseCommandEnvironment, PostToolUseInput, PostToolUseOutput,
    TurnCompletionCommandEnvironment, TurnCompletionInput, TurnCompletionOutput, UserNotice,
};
use hookkit_core::{HarnessId, HookkitError, RuntimeContext};
use hookkit_file_activity::{
    FileActivityStore, PendingFileActivity, ReconciliationOptions, ResolveOptions, VcsFallback,
    reconcile, resolve_files,
};
use hookkit_pkl_config::schema as pkl;
use hookkit_runtime::artifacts::{ArtifactKey, ArtifactManager};
use hookkit_session_state::{
    EntityOperationError, EntityOutcome, EntityView, FamilyId, SessionState, StateFamily,
    StateRoot, UtcTimestamp,
};
use minijinja::Environment;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const DEFAULT_CLEAN_CHANGED_AGENT: &str = "{{ tool }} changed {{ changed_files | join(\", \") }}; re-read changed files before editing further.";
const DEFAULT_ISSUES_AGENT: &str =
    "{{ tool }} reports issues; inspect diagnostics at {{ diagnostics_path }}.";
const DEFAULT_ISSUES_CHANGED_AGENT: &str = "{{ tool }} changed {{ changed_files | join(\", \") }} and issues remain; re-read changed files, then inspect diagnostics at {{ diagnostics_path }}.";

const BINARY_NAME: &str = "post-tool-use-agent-hook";
const TURN_COMPLETION_BINARY_NAME: &str = "turn-completion-agent-hook";
const SESSION_START_BINARY_NAME: &str = "session-start-state-agent-hook";
const BATCHED_TOOLS_FAMILY: &str = "agent-hook-kit.batched-tools";

// ----------------------------------------------------------------------------
// Public runtime types
// ----------------------------------------------------------------------------

/// A complete reusable hook CLI specification for one external tool.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    /// Stable identifier referenced by configuration and diagnostics.
    pub id: String,
    /// Human-readable name used in output templates.
    pub display_name: String,
    /// Default executable name or path.
    pub executable: String,
    /// Optional installation guidance shown when the executable is missing.
    pub install_hint: Option<String>,
    /// Include and exclusion globs used to select files.
    pub file_selection: FileSelection,
    /// Optional marker used to partition files into nearest workspaces.
    pub workspace_indicator: Option<String>,
    /// External commands executed in vector order.
    pub phases: Vec<ToolPhase>,
    /// User- and agent-facing output templates.
    pub messages: ToolMessages,
    /// Per-tool diagnostic directory override.
    pub diagnostics_directory: Option<String>,
    /// Whether this specification participates in execution.
    pub enabled: bool,
}

impl ToolSpec {
    /// Creates an enabled tool with no phases and default file/message settings.
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
            phases: Vec::new(),
            messages: ToolMessages::default(),
            diagnostics_directory: None,
            enabled: true,
        }
    }

    /// Sets installation guidance shown when the executable is unavailable.
    pub fn with_install_hint(mut self, hint: impl Into<String>) -> Self {
        self.install_hint = Some(hint.into());
        self
    }

    /// Replaces the tool's file selection.
    pub fn with_file_selection(mut self, file_selection: FileSelection) -> Self {
        self.file_selection = file_selection;
        self
    }

    /// Sets the marker used to partition files into nearest workspaces.
    pub fn with_workspace_indicator(mut self, indicator: impl Into<String>) -> Self {
        self.workspace_indicator = Some(indicator.into());
        self
    }

    /// Appends a phase to the execution order.
    pub fn with_phase(mut self, phase: ToolPhase) -> Self {
        self.phases.push(phase);
        self
    }

    /// Replaces the tool's output templates.
    pub fn with_messages(mut self, messages: ToolMessages) -> Self {
        self.messages = messages;
        self
    }
}

/// Include/exclude globs used to select modified files.
#[derive(Debug, Clone, Default)]
pub struct FileSelection {
    /// Inclusion globs evaluated relative to the project root.
    pub include: Vec<String>,
    /// Exclusion globs applied after inclusion.
    pub exclude: Vec<String>,
}

impl FileSelection {
    /// Creates a selection from inclusion patterns with no exclusions.
    pub fn include(patterns: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            include: patterns.into_iter().map(Into::into).collect(),
            exclude: Vec::new(),
        }
    }

    /// Replaces the exclusion patterns and returns the selection.
    pub fn with_exclude(mut self, patterns: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.exclude = patterns.into_iter().map(Into::into).collect();
        self
    }
}

/// One external command phase.
#[derive(Debug, Clone)]
pub struct ToolPhase {
    /// Stable phase identifier.
    pub id: String,
    /// Semantic role of the phase.
    pub mode: PhaseMode,
    /// Per-phase executable override, or `None` to use [`ToolSpec::executable`].
    pub program: Option<String>,
    /// Argument template expanded for each job.
    pub args: Vec<CommandArgTemplate>,
    /// Exit-code classification.
    pub exit_codes: ExitCodePolicy,
    /// Paths the command may modify.
    pub writes: WriteBehavior,
    /// Literal values expanded by [`CommandArgTemplate::ExtraArgs`].
    pub extra_args: Vec<String>,
    /// Whether the phase participates in execution.
    pub enabled: bool,
}

impl ToolPhase {
    /// Creates an enabled phase with no arguments and failure-on-unexpected exit codes.
    pub fn new(id: impl Into<String>, mode: PhaseMode) -> Self {
        Self {
            id: id.into(),
            mode,
            program: None,
            args: Vec::new(),
            exit_codes: ExitCodePolicy::default(),
            writes: WriteBehavior::None,
            extra_args: Vec::new(),
            enabled: true,
        }
    }

    /// Sets a phase-specific executable.
    pub fn with_program(mut self, program: impl Into<String>) -> Self {
        self.program = Some(program.into());
        self
    }

    /// Replaces the phase's argument template.
    pub fn with_args(mut self, args: impl IntoIterator<Item = CommandArgTemplate>) -> Self {
        self.args = args.into_iter().collect();
        self
    }

    /// Replaces the exit-code classification.
    pub fn with_exit_codes(mut self, exit_codes: ExitCodePolicy) -> Self {
        self.exit_codes = exit_codes;
        self
    }

    /// Declares the paths this phase may modify.
    pub fn with_writes(mut self, writes: WriteBehavior) -> Self {
        self.writes = writes;
        self
    }

    /// Replaces the literal values expanded by the extra-arguments token.
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
    /// Rewrite inputs into canonical formatting.
    Format,
    /// Apply automatic fixes.
    Fix,
    /// Verify inputs without expected modification.
    Verify,
    /// Run a read-only check whose issues are diagnostic.
    CheckOnly,
}

/// What a phase may write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteBehavior {
    /// The phase is not expected to modify files.
    None,
    /// The phase may modify only its target files.
    TargetFiles,
    /// The phase may modify any file selected by the tool globs.
    MatchingGlobs,
    /// The phase may modify any file in its workspace partition.
    Workspace,
}

/// Command argument template.
#[derive(Debug, Clone)]
pub enum CommandArgTemplate {
    /// A literal argument.
    Literal(String),
    /// Files selected for the current job.
    Files,
    /// The same files as [`CommandArgTemplate::Files`], but rewritten relative to
    /// the current workspace partition root (falling back to the absolute path
    /// for any file that lies outside that root).
    WorkspaceFiles,
    /// Root of the current workspace partition.
    Workspace,
    /// Full marker path that established the workspace partition.
    WorkspaceIndicator,
    /// Root associated with the discovered project configuration.
    ProjectRoot,
    /// Literal extra arguments configured on the phase.
    ExtraArgs,
}

impl CommandArgTemplate {
    /// Creates a literal argument template.
    pub fn literal(value: impl Into<String>) -> Self {
        Self::Literal(value.into())
    }
}

/// Exit-code classification for a phase.
#[derive(Debug, Clone)]
pub struct ExitCodePolicy {
    /// Exit codes indicating a clean result.
    pub clean: Vec<i32>,
    /// Exit codes indicating actionable issues rather than execution failure.
    pub issues: Vec<i32>,
    /// Exit codes indicating tool failure.
    pub failure: Vec<i32>,
    /// Classification for codes absent from all explicit lists.
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
    /// Creates the default policy, where only zero is clean.
    pub fn clean() -> Self {
        Self::default()
    }

    /// Replaces the exit codes classified as issues.
    pub fn issues(mut self, codes: impl IntoIterator<Item = i32>) -> Self {
        self.issues = codes.into_iter().collect();
        self
    }

    /// Replaces the exit codes classified as failures.
    pub fn failure(mut self, codes: impl IntoIterator<Item = i32>) -> Self {
        self.failure = codes.into_iter().collect();
        self
    }

    /// Sets the classification for unlisted exit codes.
    pub fn unexpected(mut self, policy: UnexpectedExitPolicy) -> Self {
        self.unexpected = policy;
        self
    }
}

/// How to classify an exit code not listed in the policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnexpectedExitPolicy {
    /// Treat the result as an execution failure.
    Failure,
    /// Treat the result as actionable issues.
    Issues,
}

/// Tool-specific output templates.
#[derive(Debug, Clone)]
pub struct ToolMessages {
    /// Agent message when the tool changed files and left no issues.
    pub clean_changed_agent: String,
    /// Agent message when issues remain but files did not change.
    pub issues_agent: String,
    /// Agent message when files changed and issues remain.
    pub issues_changed_agent: String,
    /// Optional user message when the executable is unavailable.
    pub unavailable_user: Option<String>,
    /// Optional user message when tool execution fails.
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
    /// Harness whose native post-tool event is read from standard input.
    pub harness: HarnessId,
    /// Explicit Pkl file, or `None` to use layered discovery.
    pub config_path: Option<PathBuf>,
}

/// CLI options for the stop-time batch runner.
#[derive(Debug, Clone)]
pub struct TurnCompletionCli {
    /// Harness whose native turn-completion event is read from standard input.
    pub harness: HarnessId,
    /// Explicit Pkl file, or `None` to use layered discovery.
    pub config_path: Option<PathBuf>,
    /// Session-state directory override.
    pub state_dir: Option<PathBuf>,
}

/// CLI options for the library-owned precise session-start observer.
#[derive(Debug, Clone)]
pub struct SessionStartCli {
    /// Harness whose native session-start event is read from standard input.
    pub harness: HarnessId,
    /// Session-state directory override.
    pub state_dir: Option<PathBuf>,
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

fn usage() -> String {
    format!("Usage: {BINARY_NAME} --claude|--codex|--gemini [--config PATH]")
}

fn session_start_usage() -> String {
    format!("Usage: {SESSION_START_BINARY_NAME} --claude|--codex|--gemini [--state-dir PATH]")
}

fn turn_completion_usage() -> String {
    format!(
        "Usage: {TURN_COMPLETION_BINARY_NAME} --claude|--codex|--gemini|--antigravity [--config PATH] [--state-dir PATH]"
    )
}

// ----------------------------------------------------------------------------
// Runner-owned post-tool observation and path discovery
// ----------------------------------------------------------------------------

/// The runner deliberately owns these best-effort interpretations of open tool
/// payloads. They are workflow policy, not cross-harness protocol facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObservedToolStatus {
    Success,
    Failure,
    Unknown,
}

#[derive(Debug, Clone, Copy)]
// Path selection currently needs only `value`; the rest of the observation is
// kept here because result interpretation is runner policy and is exercised by
// runner tests rather than exported from `hookkit-common`.
#[allow(dead_code)]
struct ToolResultObservation<'a> {
    value: Option<JsonPayload<'a>>,
    status: ObservedToolStatus,
    stdout: Option<&'a str>,
    stderr: Option<&'a str>,
    exit_code: Option<i64>,
}

#[derive(Debug, Clone, Copy)]
struct PostToolObservation<'a> {
    tool_name: Option<&'a str>,
    tool_input: Option<JsonPayload<'a>>,
    result: ToolResultObservation<'a>,
}

#[derive(Debug, Clone, Copy)]
enum JsonPayload<'a> {
    Value(&'a serde_json::Value),
    Object(&'a serde_json::Map<String, serde_json::Value>),
}

fn observe_post_tool(input: &PostToolUseInput) -> Option<PostToolObservation<'_>> {
    let (tool_name, tool_input, tool_result) = match input {
        PostToolUseInput::Claude(event) => (
            event.tool_name.as_str(),
            JsonPayload::Value(&event.tool_input),
            JsonPayload::Value(&event.tool_response),
        ),
        PostToolUseInput::Codex(event) => (
            event.tool_name.as_str(),
            JsonPayload::Value(&event.tool_input),
            JsonPayload::Value(&event.tool_response),
        ),
        PostToolUseInput::Gemini(event) => (
            event.tool_name.as_str(),
            JsonPayload::Object(&event.tool_input),
            JsonPayload::Object(&event.tool_response),
        ),
        PostToolUseInput::Antigravity(_) => return None,
        _ => return None,
    };

    Some(PostToolObservation {
        tool_name: Some(tool_name),
        tool_input: Some(tool_input),
        result: observe_tool_result(Some(tool_result)),
    })
}

fn observe_tool_result(value: Option<JsonPayload<'_>>) -> ToolResultObservation<'_> {
    ToolResultObservation {
        value,
        status: infer_tool_status(value),
        stdout: find_result_string(value, &["stdout", "standardOutput"]),
        stderr: find_result_string(value, &["stderr", "standardError"]),
        exit_code: find_result_i64(value, &["exitCode", "exit_code", "code"]),
    }
}

fn infer_tool_status(value: Option<JsonPayload<'_>>) -> ObservedToolStatus {
    let Some(value) = value else {
        return ObservedToolStatus::Unknown;
    };

    if let Some(success) = find_result_bool(Some(value), &["success", "ok"]) {
        return if success {
            ObservedToolStatus::Success
        } else {
            ObservedToolStatus::Failure
        };
    }

    if let Some(exit_code) = find_result_i64(Some(value), &["exitCode", "exit_code"]) {
        return if exit_code == 0 {
            ObservedToolStatus::Success
        } else {
            ObservedToolStatus::Failure
        };
    }

    ObservedToolStatus::Unknown
}

fn find_result_string<'a>(value: Option<JsonPayload<'a>>, keys: &[&str]) -> Option<&'a str> {
    match value? {
        JsonPayload::Value(serde_json::Value::Object(map)) | JsonPayload::Object(map) => {
            for key in keys {
                if let Some(found) = map.get(*key).and_then(serde_json::Value::as_str) {
                    return Some(found);
                }
            }
            map.values()
                .find_map(|nested| find_result_string(Some(JsonPayload::Value(nested)), keys))
        }
        JsonPayload::Value(serde_json::Value::Array(values)) => values
            .iter()
            .find_map(|nested| find_result_string(Some(JsonPayload::Value(nested)), keys)),
        _ => None,
    }
}

fn find_result_bool(value: Option<JsonPayload<'_>>, keys: &[&str]) -> Option<bool> {
    match value? {
        JsonPayload::Value(serde_json::Value::Object(map)) | JsonPayload::Object(map) => {
            for key in keys {
                if let Some(found) = map.get(*key).and_then(serde_json::Value::as_bool) {
                    return Some(found);
                }
            }
            map.values()
                .find_map(|nested| find_result_bool(Some(JsonPayload::Value(nested)), keys))
        }
        JsonPayload::Value(serde_json::Value::Array(values)) => values
            .iter()
            .find_map(|nested| find_result_bool(Some(JsonPayload::Value(nested)), keys)),
        _ => None,
    }
}

fn find_result_i64(value: Option<JsonPayload<'_>>, keys: &[&str]) -> Option<i64> {
    match value? {
        JsonPayload::Value(serde_json::Value::Object(map)) | JsonPayload::Object(map) => {
            for key in keys {
                if let Some(found) = map.get(*key).and_then(serde_json::Value::as_i64) {
                    return Some(found);
                }
            }
            map.values()
                .find_map(|nested| find_result_i64(Some(JsonPayload::Value(nested)), keys))
        }
        JsonPayload::Value(serde_json::Value::Array(values)) => values
            .iter()
            .find_map(|nested| find_result_i64(Some(JsonPayload::Value(nested)), keys)),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiscoveredPathRole {
    ModifiedFile,
    ReadFile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiscoveredPathSource {
    ToolInput(&'static str),
    ToolResult(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiscoveredPath {
    absolute_path: PathBuf,
    role: DiscoveredPathRole,
    source: DiscoveredPathSource,
}

#[derive(Debug, Clone, Copy)]
enum OpenPayloadSource {
    ToolInput,
    ToolResult,
}

#[derive(Debug, Clone, Copy)]
struct RawDiscoveredPath<'a> {
    path: &'a str,
    role: DiscoveredPathRole,
    source: DiscoveredPathSource,
}

fn discover_modified_files(input: &PostToolUseInput, cwd: &Path) -> Vec<PathBuf> {
    discover_path_candidates(input, cwd)
        .into_iter()
        .filter(|candidate| candidate.role == DiscoveredPathRole::ModifiedFile)
        .map(|candidate| candidate.absolute_path)
        .collect()
}

fn discover_path_candidates(input: &PostToolUseInput, cwd: &Path) -> Vec<DiscoveredPath> {
    let Some(observation) = observe_post_tool(input) else {
        return Vec::new();
    };
    let mut raw = Vec::new();
    if let Some(tool_input) = observation.tool_input {
        collect_open_payload_paths(
            tool_input,
            OpenPayloadSource::ToolInput,
            observation.tool_name,
            &mut raw,
        );
    }
    if let Some(tool_result) = observation.result.value {
        collect_open_payload_paths(
            tool_result,
            OpenPayloadSource::ToolResult,
            observation.tool_name,
            &mut raw,
        );
    }

    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    for raw_candidate in raw {
        let absolute_path = normalize_path(&absolute_from(Path::new(raw_candidate.path), cwd));
        if !seen.insert(slash_path(&absolute_path)) {
            continue;
        }
        candidates.push(DiscoveredPath {
            absolute_path,
            role: raw_candidate.role,
            source: raw_candidate.source,
        });
    }
    candidates
}

fn collect_open_payload_paths<'a>(
    value: JsonPayload<'a>,
    source: OpenPayloadSource,
    tool_name: Option<&str>,
    out: &mut Vec<RawDiscoveredPath<'a>>,
) {
    match value {
        JsonPayload::Value(serde_json::Value::Object(map)) | JsonPayload::Object(map) => {
            for (key, value) in map {
                if let Some(path) = value.as_str().filter(|path| !path.trim().is_empty()) {
                    if let Some(candidate) = path_candidate_from_field(key, path, source, tool_name)
                    {
                        out.push(candidate);
                    }
                }
                collect_open_payload_paths(JsonPayload::Value(value), source, tool_name, out);
            }
        }
        JsonPayload::Value(serde_json::Value::Array(values)) => {
            for value in values {
                collect_open_payload_paths(JsonPayload::Value(value), source, tool_name, out);
            }
        }
        _ => {}
    }
}

fn path_candidate_from_field<'a>(
    key: &str,
    path: &'a str,
    source: OpenPayloadSource,
    tool_name: Option<&str>,
) -> Option<RawDiscoveredPath<'a>> {
    let is_target_field = matches!(
        key,
        "file_path" | "filePath" | "target_file" | "targetFile" | "absolute_path" | "absolutePath"
    );
    let is_path_field = key == "path";
    if !is_target_field && !is_path_field {
        return None;
    }

    let tool_writes = tool_name.is_some_and(is_known_file_writing_tool);
    let role = match (source, is_target_field, is_path_field, tool_writes) {
        (OpenPayloadSource::ToolInput, true, _, true)
        | (OpenPayloadSource::ToolInput, false, true, true)
        | (OpenPayloadSource::ToolResult, _, _, true) => DiscoveredPathRole::ModifiedFile,
        (OpenPayloadSource::ToolInput, true, _, false)
        | (OpenPayloadSource::ToolResult, true, _, false) => DiscoveredPathRole::ReadFile,
        (OpenPayloadSource::ToolInput, false, true, false)
        | (OpenPayloadSource::ToolResult, false, true, false) => return None,
        _ => return None,
    };

    Some(RawDiscoveredPath {
        path,
        role,
        source: match source {
            OpenPayloadSource::ToolInput => DiscoveredPathSource::ToolInput(static_path_key(key)),
            OpenPayloadSource::ToolResult => DiscoveredPathSource::ToolResult(static_path_key(key)),
        },
    })
}

fn static_path_key(key: &str) -> &'static str {
    match key {
        "file_path" => "file_path",
        "filePath" => "filePath",
        "target_file" => "target_file",
        "targetFile" => "targetFile",
        "absolute_path" => "absolute_path",
        "absolutePath" => "absolutePath",
        "path" => "path",
        _ => "unknown",
    }
}

fn is_known_file_writing_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" | "write_file" | "replace" | "save_file"
    )
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
    log: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchRunSummary {
    status: &'static str,
    source_entry_count: usize,
    source_entry_ids: Vec<String>,
    files: Vec<String>,
    tools: Vec<BatchToolSummary>,
    acknowledged: bool,
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
            run_turn_completion_view(ctx, loaded, &activity_settings, &runner_family, view)
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
    runner_family: &StateFamily,
    view: &EntityView<'_, PendingFileActivity>,
) -> hookkit_core::Result<EntityOutcome<TurnCompletionOutput>> {
    if view.events().is_empty() {
        return Ok(EntityOutcome::retain(lower_turn_completion(
            ctx.harness(),
            None,
        )?));
    }

    let mut resolve_options = ResolveOptions::new(ctx.workspace_roots().to_vec());
    resolve_options.max_entries = activity_settings.max_entries;
    resolve_options.ignored_directory_names = activity_settings
        .ignored_directory_names
        .iter()
        .cloned()
        .collect();
    let resolved = resolve_files(view.state(), &resolve_options).map_err(activity_error)?;
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
            run.write_text("config-error.log", &error.to_string())
                .map_err(state_error)?;
            let summary_path = run
                .commit(&BatchRunSummary {
                    status: "operational-failure",
                    source_entry_count,
                    source_entry_ids: source_entry_ids.clone(),
                    files: candidates.iter().map(|path| slash_path(path)).collect(),
                    tools: Vec::new(),
                    acknowledged: false,
                })
                .map_err(state_error)?;
            return Ok(EntityOutcome::retain(lower_turn_completion(
                ctx.harness(),
                Some(&summary_path),
            )?));
        }
    };

    let project_root = normalize_path(&loaded.project_root);
    let tools = match resolve_run_order(&loaded.config) {
        Ok(tools) => tools,
        Err(error) => {
            let run = run.take().expect("run bundle is available");
            run.write_text("config-error.log", &error.to_string())
                .map_err(state_error)?;
            let summary_path = run
                .commit(&BatchRunSummary {
                    status: "operational-failure",
                    source_entry_count,
                    source_entry_ids: source_entry_ids.clone(),
                    files: candidates.iter().map(|path| slash_path(path)).collect(),
                    tools: Vec::new(),
                    acknowledged: false,
                })
                .map_err(state_error)?;
            return Ok(EntityOutcome::retain(lower_turn_completion(
                ctx.harness(),
                Some(&summary_path),
            )?));
        }
    };

    let global_exclude = &loaded.config.settings.exclude;
    let global_diagnostics_dir = loaded.config.settings.diagnostics_directory.as_deref();
    let mut summaries = Vec::new();
    let mut has_issues = false;
    let mut has_operational_failure = false;

    for (index, schema_spec) in tools.into_iter().enumerate() {
        if !schema_spec.enabled {
            continue;
        }
        let spec = convert_tool_spec(schema_spec, global_exclude);
        let context = ToolContext {
            spec: &spec,
            project_root: &project_root,
            global_diagnostics_dir,
        };
        let matcher = FileMatcher::new(&spec.file_selection)?;
        let runnable_paths = candidates
            .iter()
            .filter(|path| matcher.matches(path, &project_root))
            .cloned()
            .collect::<Vec<_>>();
        if runnable_paths.is_empty() {
            continue;
        }
        let jobs = build_jobs(&runnable_paths, &project_root, &spec);
        if jobs.is_empty() {
            continue;
        }

        let outcomes = run_jobs(&jobs, &context, loaded.config.settings.jobs);
        let status = batch_outcome_status(&outcomes);
        let log_path = format!("tools/{index:03}.log");
        run.as_ref()
            .expect("run bundle is available")
            .write_text(&log_path, &format_batch_outcomes(&outcomes))
            .map_err(state_error)?;
        summaries.push(BatchToolSummary {
            tool_id: spec.id.clone(),
            file_count: runnable_paths.len(),
            issues: status.issues,
            operational_failure: status.operational_failure,
            log: log_path,
        });
        has_issues |= status.issues;
        has_operational_failure |= status.operational_failure;

        if (loaded.config.settings.fail_fast && status.operational_failure)
            || (!loaded.config.settings.continue_after_issues && status.issues)
        {
            break;
        }
    }

    let should_block = has_issues || has_operational_failure;
    let status = if has_operational_failure {
        "operational-failure"
    } else if has_issues {
        "issues"
    } else {
        "clean"
    };
    let run = run.take().expect("run bundle is available");
    let summary_path = run
        .commit(&BatchRunSummary {
            status,
            source_entry_count,
            source_entry_ids,
            files: candidates.iter().map(|path| slash_path(path)).collect(),
            tools: summaries,
            acknowledged: !should_block,
        })
        .map_err(state_error)?;

    if should_block {
        Ok(EntityOutcome::retain(lower_turn_completion(
            ctx.harness(),
            Some(&summary_path),
        )?))
    } else {
        Ok(EntityOutcome::acknowledge(lower_turn_completion(
            ctx.harness(),
            None,
        )?))
    }
}

fn batch_outcome_status(outcomes: &[ToolRunOutcome]) -> ToolBatchStatus {
    let mut status = ToolBatchStatus {
        operational_failure: false,
        issues: false,
    };
    for outcome in outcomes {
        match outcome {
            ToolRunOutcome::Completed(completed) => {
                status.issues |= completed.issues == IssueState::Issues;
            }
            ToolRunOutcome::ToolUnavailable { .. } | ToolRunOutcome::ToolFailed { .. } => {
                status.operational_failure = true;
            }
        }
    }
    status
}

fn format_batch_outcomes(outcomes: &[ToolRunOutcome]) -> String {
    let mut output = String::new();
    for (index, outcome) in outcomes.iter().enumerate() {
        output.push_str(&format!("== job {} ==\n", index + 1));
        match outcome {
            ToolRunOutcome::Completed(completed) => {
                output.push_str(&format!(
                    "result: {:?}\nfiles: {}\n",
                    completed.issues,
                    completed
                        .files
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                output.push_str(&completed.diagnostics);
            }
            ToolRunOutcome::ToolUnavailable {
                phase,
                executable,
                install_hint,
                changed_files,
            } => {
                output.push_str(&format!(
                    "result: unavailable\nphase: {phase}\nexecutable: {executable}\nchanged files: {}\n",
                    changed_files
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                if let Some(hint) = install_hint {
                    output.push_str(&format!("install hint: {hint}\n"));
                }
            }
            ToolRunOutcome::ToolFailed {
                phase,
                exit_code,
                diagnostics,
                changed_files,
            } => {
                output.push_str(&format!(
                    "result: failure\nphase: {phase}\nexit code: {exit_code:?}\nchanged files: {}\n",
                    changed_files
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                output.push_str(diagnostics);
            }
        }
        if !output.ends_with('\n') {
            output.push('\n');
        }
        output.push('\n');
    }
    output
}

fn lower_turn_completion(
    harness: &HarnessId,
    problem_summary: Option<&Path>,
) -> hookkit_core::Result<TurnCompletionOutput> {
    let Some(summary) = problem_summary else {
        return match harness.as_str() {
            "claude-code" => Ok(TurnCompletionOutput::Claude(
                hookkit_claude::catalog::StopOutput::no_op(),
            )),
            "codex" => Ok(TurnCompletionOutput::Codex(
                hookkit_codex::catalog::StopOutput::no_op(),
            )),
            "gemini-cli" => Ok(TurnCompletionOutput::Gemini(
                hookkit_gemini::catalog::AfterAgentOutput::no_op(),
            )),
            "antigravity" => Ok(TurnCompletionOutput::Antigravity(
                hookkit_antigravity::StopOutput {
                    decision: "stop".into(),
                    reason: None,
                },
            )),
            _ => Err(invalid_data(format!(
                "turn-completion runner does not support {harness}"
            ))),
        };
    };

    let agent_message = format!(
        "Do not stop yet: session-batched checks need manual attention. Inspect {} and the referenced tool logs, fix the problems, then try to stop again.",
        summary.display()
    );
    let user_message = format!(
        "Some session-batched formatter/linter checks still need attention. Details: {}",
        summary.display()
    );
    match harness.as_str() {
        "claude-code" => Ok(TurnCompletionOutput::Claude(
            hookkit_claude::catalog::StopOutput::block_with_context(
                agent_message.clone(),
                agent_message,
            )
            .with_system_message(user_message)?,
        )),
        "codex" => Ok(TurnCompletionOutput::Codex(
            hookkit_codex::catalog::StopOutput::block(agent_message)
                .with_system_message(user_message)?,
        )),
        "gemini-cli" => Ok(TurnCompletionOutput::Gemini(
            hookkit_gemini::catalog::AfterAgentOutput::deny(agent_message, false)
                .with_system_message(user_message)?,
        )),
        "antigravity" => Ok(TurnCompletionOutput::Antigravity(
            hookkit_antigravity::StopOutput {
                decision: "continue".into(),
                reason: Some(agent_message),
            },
        )),
        _ => Err(invalid_data(format!(
            "turn-completion runner does not support {harness}"
        ))),
    }
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

        let candidates = discover_modified_files(&post_tool, &cwd);
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

/// Accumulated common output produced by the post-tool runner.
///
/// Fields are runner-owned so callers receive this value through
/// [`RunnerDomainOutcome`] and lower it with the selected harness workflow.
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
    },
    /// The selected harness cannot represent or execute this workflow.
    UnsupportedHarness {
        /// Selected harness identifier.
        harness: String,
        /// Explanation of the unsupported behavior.
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
    let phases = ordered_phases(spec)
        .into_iter()
        .map(convert_phase)
        .collect();

    let mut exclude = global_exclude.to_vec();
    exclude.extend(spec.files.exclude.clone());

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
        phases,
        messages: convert_messages(&spec.messages),
        diagnostics_directory: spec.diagnostics.directory.clone(),
        enabled: spec.enabled,
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
            PhaseLog {
                phase: phase.id.clone(),
                command: display_command(&command.program, &command.args),
                status,
                classification: status.map(|code| classify_exit_code(&phase.exit_codes, code)),
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                error: None,
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => PhaseLog {
            phase: phase.id.clone(),
            command: display_command(&command.program, &command.args),
            status: None,
            classification: None,
            stdout: String::new(),
            stderr: String::new(),
            error: Some("not found".to_string()),
        },
        Err(e) => PhaseLog {
            phase: phase.id.clone(),
            command: display_command(&command.program, &command.args),
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

    fn claude_post_tool(
        tool_name: &str,
        tool_input: serde_json::Value,
        tool_response: serde_json::Value,
    ) -> PostToolUseInput {
        PostToolUseInput::Claude(hookkit_claude::protocol::PostToolUseInput {
            session_id: "r5-test".into(),
            transcript_path: "/tmp/r5-transcript.jsonl".into(),
            cwd: "/hookkit-r5-root".into(),
            hook_event_name: "PostToolUse".into(),
            tool_name: tool_name.into(),
            tool_input,
            tool_use_id: "tool-r5".into(),
            tool_response,
            agent_id: None,
            agent_type: None,
            duration_ms: None,
            effort: None,
            permission_mode: None,
            prompt_id: None,
            extra: BTreeMap::new(),
        })
    }

    #[test]
    fn runner_discovers_nested_writer_paths_and_deduplicates_payload_spellings() {
        let root = Path::new("/hookkit-r5-root");
        let input = claude_post_tool(
            "Write",
            serde_json::json!({
                "wrapper": [{"file_path": "src/./app.py"}],
                "duplicate": {"filePath": "src/app.py"}
            }),
            serde_json::json!({
                "nested": {
                    "targetFile": "/hookkit-r5-root/src/app.py",
                    "absolute_path": "/hookkit-r5-root/src/generated.py"
                }
            }),
        );

        let candidates = discover_path_candidates(&input, root);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].absolute_path, root.join("src/app.py"));
        assert_eq!(candidates[0].role, DiscoveredPathRole::ModifiedFile);
        assert!(
            matches!(
                candidates[0].source,
                DiscoveredPathSource::ToolInput("file_path" | "filePath")
            ),
            "first occurrence should come from the tool input: {candidates:?}"
        );
        assert_eq!(
            candidates[1],
            DiscoveredPath {
                absolute_path: root.join("src/generated.py"),
                role: DiscoveredPathRole::ModifiedFile,
                source: DiscoveredPathSource::ToolResult("absolute_path"),
            }
        );
        assert_eq!(
            discover_modified_files(&input, root),
            vec![root.join("src/app.py"), root.join("src/generated.py")]
        );
    }

    #[test]
    fn runner_does_not_treat_read_tool_paths_as_modified() {
        let root = Path::new("/hookkit-r5-root");
        let input = claude_post_tool(
            "Read",
            serde_json::json!({"nested": {"file_path": "src/app.py"}}),
            serde_json::json!({"content": "def f(): pass\n"}),
        );

        assert_eq!(
            discover_path_candidates(&input, root),
            vec![DiscoveredPath {
                absolute_path: root.join("src/app.py"),
                role: DiscoveredPathRole::ReadFile,
                source: DiscoveredPathSource::ToolInput("file_path"),
            }]
        );
        assert!(discover_modified_files(&input, root).is_empty());
    }

    #[test]
    fn runner_owns_recursive_tool_result_observation() {
        let input = claude_post_tool(
            "Write",
            serde_json::json!({}),
            serde_json::json!({
                "wrapper": [{
                    "ok": false,
                    "standardOutput": "partial output",
                    "standardError": "write failed",
                    "exit_code": 7
                }]
            }),
        );

        let observation = observe_post_tool(&input).unwrap().result;
        assert_eq!(observation.status, ObservedToolStatus::Failure);
        assert_eq!(observation.stdout, Some("partial output"));
        assert_eq!(observation.stderr, Some("write failed"));
        assert_eq!(observation.exit_code, Some(7));
    }

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
