//! Crate-private execution specs converted from the Pkl schema.
//!
//! These types are the runner's resolved view of one tool after layered
//! configuration, ordering, and global exclusions have been applied. They are
//! intentionally not public: every public entry point takes CLI options and
//! Pkl configuration, so exposing builders that no public function accepts
//! would only advertise an API that cannot be executed. Enumerations that are
//! identical to the Pkl schema are reused from [`hookkit_pkl_config::schema`]
//! instead of being mirrored.

pub(crate) use hookkit_pkl_config::schema::{
    CheckScope, InvocationGranularity, PhaseMode, UnexpectedExitPolicy, WriteBehavior,
};

const DEFAULT_CLEAN_CHANGED_AGENT: &str = "{{ tool }} changed {{ changed_files | join(\", \") }}; re-read changed files before editing further.";
const DEFAULT_ISSUES_AGENT: &str =
    "{{ tool }} reports issues; inspect diagnostics at {{ diagnostics_path }}.";
const DEFAULT_ISSUES_CHANGED_AGENT: &str = "{{ tool }} changed {{ changed_files | join(\", \") }} and issues remain; re-read changed files, then inspect diagnostics at {{ diagnostics_path }}.";

/// A complete resolved specification for one external tool.
#[derive(Debug, Clone)]
pub(crate) struct ToolSpec {
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
    /// Deferred workflows executed at turn completion.
    pub workflows: Vec<ToolWorkflow>,
    /// External commands executed in vector order.
    pub phases: Vec<ToolPhase>,
    /// User- and agent-facing output templates.
    pub messages: ToolMessages,
    /// Per-tool diagnostic directory override.
    pub diagnostics_directory: Option<String>,
}

impl ToolSpec {
    /// Creates an enabled tool with no phases and default file/message settings.
    #[cfg(test)]
    pub(crate) fn new(
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
        }
    }

    /// Appends a phase to the execution order.
    #[cfg(test)]
    pub(crate) fn with_phase(mut self, phase: ToolPhase) -> Self {
        self.phases.push(phase);
        self
    }

    /// Whether any enabled immediate phase expands file-list argument tokens.
    pub(crate) fn phases_use_file_arguments(&self) -> bool {
        self.phases
            .iter()
            .any(|phase| phase.enabled && phase.uses_file_arguments())
    }

    /// Whether any enabled immediate phase may write beyond its own targets.
    pub(crate) fn phases_write_beyond_targets(&self) -> bool {
        self.phases.iter().any(|phase| {
            phase.enabled
                && matches!(
                    phase.writes,
                    WriteBehavior::MatchingGlobs | WriteBehavior::Workspace
                )
        })
    }
}

/// One Stop-time non-mutating check and optional automatic remedy.
#[derive(Debug, Clone)]
pub(crate) struct ToolWorkflow {
    /// Stable workflow identifier.
    pub id: String,
    /// Read-only command used to detect issues.
    pub check: Option<ToolPhase>,
    /// Optional command used to repair detected issues.
    pub remedy: Option<ToolPhase>,
    /// Inputs whose changes invalidate a prior check.
    pub check_scope: CheckScope,
    /// Granularity used to divide selected files into invocations.
    pub invocation: InvocationGranularity,
    /// Whether this workflow was translated from legacy immediate phases.
    pub compatibility_translation: bool,
    /// Whether this workflow participates in deferred execution.
    pub enabled: bool,
}

impl ToolWorkflow {
    /// Whether the check or remedy expands file-list argument tokens.
    pub(crate) fn uses_file_arguments(&self) -> bool {
        self.check
            .iter()
            .chain(self.remedy.iter())
            .any(ToolPhase::uses_file_arguments)
    }
}

/// Include/exclude globs used to select modified files.
#[derive(Debug, Clone, Default)]
pub(crate) struct FileSelection {
    /// Inclusion globs evaluated relative to the project root.
    pub include: Vec<String>,
    /// Exclusion globs applied after inclusion.
    pub exclude: Vec<String>,
}

/// One external command phase.
#[derive(Debug, Clone)]
pub(crate) struct ToolPhase {
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
    /// Whether non-empty standard output represents actionable issues.
    pub issues_on_stdout: bool,
    /// Paths the command may modify.
    pub writes: WriteBehavior,
    /// Literal values expanded by [`CommandArgTemplate::ExtraArgs`].
    pub extra_args: Vec<String>,
    /// Whether the phase participates in execution.
    pub enabled: bool,
}

impl ToolPhase {
    /// Creates an enabled phase with no arguments and failure-on-unexpected exit codes.
    #[cfg(test)]
    pub(crate) fn new(id: impl Into<String>, mode: PhaseMode) -> Self {
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

    /// Replaces the phase's argument template.
    #[cfg(test)]
    pub(crate) fn with_args(mut self, args: impl IntoIterator<Item = CommandArgTemplate>) -> Self {
        self.args = args.into_iter().collect();
        self
    }

    pub(crate) fn is_verifier(&self) -> bool {
        matches!(self.mode, PhaseMode::Verify | PhaseMode::CheckOnly)
    }

    /// Whether the argument template expands one argument per selected file.
    pub(crate) fn uses_file_arguments(&self) -> bool {
        self.args.iter().any(|arg| {
            matches!(
                arg,
                CommandArgTemplate::Files | CommandArgTemplate::WorkspaceFiles
            )
        })
    }
}

/// Command argument template.
#[derive(Debug, Clone)]
pub(crate) enum CommandArgTemplate {
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
    /// Executable selected for the current tool command.
    ToolExecutable,
    /// Literal extra arguments configured on the phase.
    ExtraArgs,
}

impl CommandArgTemplate {
    /// Creates a literal argument template.
    #[cfg(test)]
    pub(crate) fn literal(value: impl Into<String>) -> Self {
        Self::Literal(value.into())
    }
}

/// Exit-code classification for a phase.
#[derive(Debug, Clone)]
pub(crate) struct ExitCodePolicy {
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

/// Tool-specific output templates.
#[derive(Debug, Clone)]
pub(crate) struct ToolMessages {
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
