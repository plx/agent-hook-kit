//! Rust mirror of the post-tool-use Pkl schema.
//!
//! The Pkl module evaluates to JSON via `pkl eval --format json`; these types
//! deserialize from that JSON. The shape intentionally mirrors the runtime
//! `ToolSpec` family in `hookkit-tool-runner` so the downstream conversion is
//! mechanical.

use serde::Deserialize;
use std::collections::BTreeMap;

/// Root configuration loaded from one or more Pkl files.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RunnerConfig {
    /// Runner-wide behavior after all overlays are applied.
    pub settings: Settings,
    /// Merge directives retained from this evaluated configuration.
    pub merge: Merge,
    /// Tool specifications keyed by configured name.
    pub tools: BTreeMap<String, ToolSpec>,
    /// Tool identifiers to execute, in configured order.
    pub run: Vec<String>,
}

/// Root configuration patch loaded from one Pkl file before multi-file merge.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RunnerConfigPatch {
    /// Field-preserving settings overlay.
    pub settings: SettingsPatch,
    /// Reset directives applied before merging this layer.
    pub merge: Merge,
    /// Tool definitions contributed by this layer.
    pub tools: BTreeMap<String, ToolSpec>,
    /// Run-list contribution from this layer.
    pub run: Vec<String>,
}

impl RunnerConfigPatch {
    /// Applies the patch to default settings and returns a standalone config.
    pub fn into_config(self) -> RunnerConfig {
        let mut settings = Settings::default();
        self.settings.apply_to(&mut settings);
        RunnerConfig {
            settings,
            merge: self.merge,
            tools: self.tools,
            run: self.run,
        }
    }
}

/// Top-level runner settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Maximum concurrent jobs; zero selects the runner's automatic limit.
    pub jobs: u32,
    /// Whether to stop scheduling after the first failed job.
    pub fail_fast: bool,
    /// Whether later tools may run after an earlier tool reports issues.
    pub continue_after_issues: bool,
    /// Glob patterns excluded from tool file selection.
    pub exclude: Vec<String>,
    /// Handling for common output that the native harness cannot represent.
    pub lowering_policy: LoweringPolicy,
    /// Directory used for full tool diagnostics, or `None` to disable files.
    pub diagnostics_directory: Option<String>,
    /// Behavior when a configured executable cannot be found.
    pub missing_tool_policy: MissingToolPolicy,
    /// Optional stop-time file-activity reconciliation settings.
    pub file_activity: Option<FileActivitySettings>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            jobs: 0,
            fail_fast: true,
            continue_after_issues: true,
            exclude: vec![".git/**".into(), "node_modules/**".into()],
            lowering_policy: LoweringPolicy::default(),
            diagnostics_directory: Some(".agent-hook-kit/post-tool-use".into()),
            missing_tool_policy: MissingToolPolicy::default(),
            file_activity: None,
        }
    }
}

/// Field-preserving settings overlay for one Pkl file.
///
/// NOTE: Pkl's JSON output always emits every class field with its evaluated
/// value, so we cannot distinguish "user omitted the field" from "user set the
/// field to its Pkl default" purely at the deserialization layer. Each
/// `Option<T>` here represents "field was non-null in JSON"; a `null` value
/// from Pkl deserializes to `None`. Per-field explicit clearing (e.g., setting
/// `diagnosticsDirectory = null` to unset an inherited value) would require a
/// Pkl-side sentinel convention or a separate `merge.resetSettings` mechanism;
/// see PR #6–#16 discussion.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SettingsPatch {
    /// Optional `jobs` override.
    pub jobs: Option<u32>,
    /// Optional `fail_fast` override.
    pub fail_fast: Option<bool>,
    /// Optional `continue_after_issues` override.
    pub continue_after_issues: Option<bool>,
    /// Optional replacement for the global exclusion list.
    pub exclude: Option<Vec<String>>,
    /// Optional lowering-policy override.
    pub lowering_policy: Option<LoweringPolicy>,
    /// Optional diagnostics directory override.
    ///
    /// `None` means absent at this Rust layer and therefore cannot explicitly
    /// clear an inherited directory; see the type-level note.
    pub diagnostics_directory: Option<String>,
    /// Optional missing-tool-policy override.
    pub missing_tool_policy: Option<MissingToolPolicy>,
    /// Optional file-activity settings override.
    pub file_activity: Option<FileActivitySettings>,
}

impl SettingsPatch {
    /// Overwrites each setting represented by `Some`, leaving others unchanged.
    pub fn apply_to(self, settings: &mut Settings) {
        if let Some(jobs) = self.jobs {
            settings.jobs = jobs;
        }
        if let Some(fail_fast) = self.fail_fast {
            settings.fail_fast = fail_fast;
        }
        if let Some(continue_after_issues) = self.continue_after_issues {
            settings.continue_after_issues = continue_after_issues;
        }
        if let Some(exclude) = self.exclude {
            settings.exclude = exclude;
        }
        if let Some(lowering_policy) = self.lowering_policy {
            settings.lowering_policy = lowering_policy;
        }
        if let Some(diagnostics_directory) = self.diagnostics_directory {
            settings.diagnostics_directory = Some(diagnostics_directory);
        }
        if let Some(missing_tool_policy) = self.missing_tool_policy {
            settings.missing_tool_policy = missing_tool_policy;
        }
        if let Some(file_activity) = self.file_activity {
            settings.file_activity = Some(file_activity);
        }
    }
}

/// Stop-time fallback behavior for the pending file-activity window.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FileActivitySettings {
    /// Whether stop-time reconciliation scans modification times.
    pub filesystem_mtime: bool,
    /// Optional VCS dirty-state fallback.
    pub vcs: FileActivityVcsFallback,
    /// Clock-resolution allowance around reconciliation interval bounds.
    pub timestamp_tolerance_millis: u64,
    /// Maximum directory entries visited by reconciliation.
    pub max_entries: usize,
    /// Directory basenames pruned from recursive traversal.
    pub ignored_directory_names: Vec<String>,
}

impl Default for FileActivitySettings {
    fn default() -> Self {
        Self {
            filesystem_mtime: true,
            vcs: FileActivityVcsFallback::Disabled,
            timestamp_tolerance_millis: 2_000,
            max_entries: 100_000,
            ignored_directory_names: vec![
                ".context".into(),
                ".git".into(),
                ".hg".into(),
                ".svn".into(),
                "node_modules".into(),
                "target".into(),
            ],
        }
    }
}

/// Optional VCS evidence used by stop-time file-activity reconciliation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileActivityVcsFallback {
    /// Do not inspect version-control state.
    #[default]
    Disabled,
    /// Treat paths from `git status` as heuristic dirty-file evidence.
    GitDirty,
}

/// How to handle optional common-output intents the harness cannot represent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LoweringPolicy {
    /// Fail when a common output intent cannot be represented exactly.
    Strict,
    /// Drop unsupported intent without adding a warning.
    BestEffort,
    /// Drop unsupported intent and retain a lowering warning.
    #[default]
    BestEffortWithWarnings,
}

/// What to do when a configured tool executable is missing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MissingToolPolicy {
    /// Emit a user-visible notice and continue.
    #[default]
    UserNotice,
    /// Treat the missing executable as a runner failure.
    HardFailure,
    /// Ask the harness to block through its native decision mechanism.
    HarnessBlock,
}

/// Merge controls for multi-file config discovery.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Merge {
    /// Reset settings, tools, and run list before applying this layer.
    pub reset_all: bool,
    /// Selected top-level sections to reset before applying this layer.
    pub reset: Vec<MergeResetKey>,
    /// Tool identifiers to remove before merging definitions from this layer.
    pub reset_tools: Vec<String>,
}

/// Top-level configuration section that a merge layer can reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MergeResetKey {
    /// Reset runner settings to defaults.
    Settings,
    /// Remove all previously defined tools.
    Tools,
    /// Clear the accumulated run list.
    Run,
}

/// One external tool available to the runner.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ToolSpec {
    /// Stable identifier referenced by the run list.
    pub id: String,
    /// Human-readable tool name used in messages.
    pub display_name: String,
    /// Default executable name or path.
    pub executable: String,
    /// Optional installation guidance shown when the executable is missing.
    pub install_hint: Option<String>,
    /// Include and exclusion globs used to select files.
    pub files: FileSelection,
    /// Optional marker used to partition files into nearest workspaces.
    pub workspace_indicator: Option<String>,
    /// Named execution phases.
    pub phases: BTreeMap<String, Phase>,
    /// Phase identifiers in execution order.
    pub phase_order: Vec<String>,
    /// User- and agent-facing message templates.
    pub messages: Messages,
    /// Per-tool diagnostic output configuration.
    pub diagnostics: Diagnostics,
    /// Whether the tool participates when referenced by the run list.
    pub enabled: bool,
}

impl Default for ToolSpec {
    fn default() -> Self {
        Self {
            id: String::new(),
            display_name: String::new(),
            executable: String::new(),
            install_hint: None,
            files: FileSelection::default(),
            workspace_indicator: None,
            phases: BTreeMap::new(),
            phase_order: Vec::new(),
            messages: Messages::default(),
            diagnostics: Diagnostics::default(),
            enabled: true,
        }
    }
}

/// File globs used to select inputs for a tool.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FileSelection {
    /// Inclusion globs evaluated relative to the project root.
    pub include: Vec<String>,
    /// Tool-specific exclusion globs.
    pub exclude: Vec<String>,
}

/// One external command phase inside a tool spec.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Phase {
    /// Semantic role of this phase.
    pub mode: PhaseMode,
    /// Per-phase executable override.
    pub program: Option<String>,
    /// Argument template expanded for each job.
    pub argv: Vec<ArgvElement>,
    /// Exit-code classification.
    pub exit_codes: ExitCodes,
    /// Paths the phase may modify.
    pub writes: WriteBehavior,
    /// Whether this phase participates in execution.
    pub enabled: bool,
    /// Literal values expanded by [`ArgToken::ExtraArgs`].
    pub extra_args: Vec<String>,
}

impl Default for Phase {
    fn default() -> Self {
        Self {
            mode: PhaseMode::Verify,
            program: None,
            argv: Vec::new(),
            exit_codes: ExitCodes::default(),
            writes: WriteBehavior::None,
            enabled: true,
            extra_args: Vec::new(),
        }
    }
}

/// Semantic execution mode of a tool phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
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

/// A single argv element: either a literal string or a placeholder token.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ArgvElement {
    /// A literal argument.
    Literal(String),
    /// A placeholder expanded from the current job context.
    Token(ArgToken),
}

/// Placeholder tokens for argv expansion. The Pkl side emits these as
/// `{"type": "Files"}` etc.; we tag on `type` to keep the wire shape readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(tag = "type")]
pub enum ArgToken {
    /// Files selected for the current job.
    Files,
    /// All selected files in the current workspace partition.
    WorkspaceFiles,
    /// Root of the current workspace partition.
    Workspace,
    /// Full path to the marker that established the workspace partition.
    WorkspaceIndicator,
    /// Root associated with the discovered project configuration.
    ProjectRoot,
    /// Literal extra arguments configured on the phase.
    ExtraArgs,
}

/// Classification table for tool process exit codes.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExitCodes {
    /// Exit codes indicating a clean result.
    pub clean: Vec<i32>,
    /// Exit codes indicating actionable issues rather than execution failure.
    pub issues: Vec<i32>,
    /// Exit codes indicating tool failure.
    pub failure: Vec<i32>,
    /// Classification for codes absent from the explicit lists.
    pub unexpected: UnexpectedExitPolicy,
}

impl Default for ExitCodes {
    fn default() -> Self {
        Self {
            clean: vec![0],
            issues: Vec::new(),
            failure: Vec::new(),
            unexpected: UnexpectedExitPolicy::default(),
        }
    }
}

/// Classification applied to an exit code absent from all configured lists.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UnexpectedExitPolicy {
    /// Treat the result as an execution failure.
    #[default]
    Failure,
    /// Treat the result as actionable issues.
    Issues,
}

/// Declared file-mutation scope of a phase.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WriteBehavior {
    /// The phase is not expected to modify files.
    #[default]
    None,
    /// The phase may modify only the target files passed to it.
    TargetFiles,
    /// The phase may modify any file selected by the tool globs.
    MatchingGlobs,
    /// The phase may modify any file in the workspace partition.
    Workspace,
}

/// Per-tool diagnostic artifact configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Diagnostics {
    /// Directory override, or `None` to use the runner-wide setting.
    pub directory: Option<String>,
}

/// Templates used to summarize tool outcomes for users and agents.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Messages {
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

impl Default for Messages {
    fn default() -> Self {
        Self {
            clean_changed_agent: default_clean_changed_agent(),
            issues_agent: default_issues_agent(),
            issues_changed_agent: default_issues_changed_agent(),
            unavailable_user: None,
            failed_user: None,
        }
    }
}

/// Returns the default agent template for a clean result that changed files.
pub fn default_clean_changed_agent() -> String {
    "{{ tool }} changed {{ changed_files | join(\", \") }}; re-read changed files before editing further.".into()
}

/// Returns the default agent template for a result with remaining issues.
pub fn default_issues_agent() -> String {
    "{{ tool }} reports issues; inspect diagnostics at {{ diagnostics_path }}.".into()
}

/// Returns the default agent template for changed files with remaining issues.
pub fn default_issues_changed_agent() -> String {
    "{{ tool }} changed {{ changed_files | join(\", \") }} and issues remain; re-read changed files, then inspect diagnostics at {{ diagnostics_path }}.".into()
}
