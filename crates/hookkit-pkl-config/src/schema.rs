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
    /// Glob patterns excluded from tool file selection, matched against
    /// project-relative paths.
    pub exclude: Vec<String>,
    /// Handling for common output that the native harness cannot represent.
    pub lowering_policy: LoweringPolicy,
    /// Directory for the immediate runner's full tool diagnostics, relative to
    /// the project root. `None` (only reachable by constructing settings in
    /// Rust) falls back to the per-user artifact directory under the system
    /// temporary directory (`hookkit-artifacts-<uid>` on Unix, see
    /// `hookkit_runtime::artifacts::ArtifactManager::default_temp_dir`); it
    /// does not disable diagnostics files.
    pub diagnostics_directory: Option<String>,
    /// Behavior when a configured executable cannot be found.
    pub missing_tool_policy: MissingToolPolicy,
    /// Deadline in seconds for one external tool command; `0` disables it.
    /// A command that exceeds it is killed with its process group and
    /// reported as an operational failure.
    pub command_timeout_seconds: u64,
    /// Optional stop-time file-activity reconciliation settings.
    pub file_activity: Option<FileActivitySettings>,
    /// Templates and file groups used to render deferred results.
    pub deferred_reporting: DeferredReporting,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            jobs: 0,
            fail_fast: true,
            continue_after_issues: true,
            exclude: vec!["**/.git/**".into(), "**/node_modules/**".into()],
            lowering_policy: LoweringPolicy::default(),
            diagnostics_directory: Some(".agent-hook-kit/post-tool-use".into()),
            missing_tool_policy: MissingToolPolicy::default(),
            command_timeout_seconds: DEFAULT_COMMAND_TIMEOUT_SECONDS,
            file_activity: None,
            deferred_reporting: DeferredReporting::default(),
        }
    }
}

/// Default per-command deadline: below the 600-second default command-hook
/// timeout of Claude Code and Codex, so a hung tool is killed by the runner
/// before the harness abandons the hook and orphans the tool.
pub const DEFAULT_COMMAND_TIMEOUT_SECONDS: u64 = 300;

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
    /// Optional per-command timeout override in seconds.
    pub command_timeout_seconds: Option<u64>,
    /// Optional field-preserving file-activity settings overlay.
    pub file_activity: Option<FileActivitySettingsPatch>,
    /// Optional deferred-reporting settings overlay.
    pub deferred_reporting: Option<DeferredReportingPatch>,
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
        if let Some(command_timeout_seconds) = self.command_timeout_seconds {
            settings.command_timeout_seconds = command_timeout_seconds;
        }
        if let Some(file_activity) = self.file_activity {
            file_activity.apply_to(settings.file_activity.get_or_insert_with(Default::default));
        }
        if let Some(deferred_reporting) = self.deferred_reporting {
            deferred_reporting.apply_to(&mut settings.deferred_reporting);
        }
    }
}

/// User- and agent-facing templates for one deferred result category.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TemplatePair {
    /// Template rendered for the user.
    pub user: String,
    /// Template rendered for the coding agent.
    pub agent: String,
}

/// Named file classification group used in deferred reports.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileGroup {
    /// Stable group identifier.
    pub id: String,
    /// Human-readable group name.
    pub display_name: String,
    /// Glob patterns selecting files in the group.
    pub include: Vec<String>,
}

/// Templates and grouping rules used to render deferred workflow results.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeferredReporting {
    /// Ordered groups used to classify result files.
    pub groups: Vec<FileGroup>,
    /// Templates for files whose checks are clean.
    pub clean: TemplatePair,
    /// Templates for files changed successfully by a remedy.
    pub auto_fixed: TemplatePair,
    /// Templates for files that still need manual changes.
    pub manual_fixes_needed: TemplatePair,
    /// Templates for workflow execution failures.
    pub operational_error: TemplatePair,
    /// Aggregate template rendered for the user.
    pub master_user: String,
    /// Aggregate template rendered for the coding agent.
    pub master_agent: String,
    /// Whether categories with no files are included in rendered output.
    pub render_empty_buckets: bool,
}

impl Default for DeferredReporting {
    fn default() -> Self {
        Self {
            groups: default_file_groups(),
            clean: TemplatePair {
                user: "Checked {{ counts.clean }} clean file{% if counts.clean != 1 %}s{% endif %}: {% for file in clean_files %}{{ file.displayPath }}{% if not loop.last %}, {% endif %}{% endfor %}".into(),
                agent: String::new(),
            },
            auto_fixed: TemplatePair {
                user: "Auto-fixed {{ counts.auto_fixed }} file{% if counts.auto_fixed != 1 %}s{% endif %}: {% for file in auto_fixed_files %}{{ file.displayPath }}{% if not loop.last %}, {% endif %}{% endfor %}".into(),
                agent: "Auto-fixed {{ counts.auto_fixed }} file{% if counts.auto_fixed != 1 %}s{% endif %}; re-read changed files before editing further.".into(),
            },
            manual_fixes_needed: TemplatePair {
                user: "{{ counts.manual_fixes_needed }} file{% if counts.manual_fixes_needed != 1 %}s{% endif %} need{% if counts.manual_fixes_needed == 1 %}s{% endif %} manual fixes across {{ counts.manual_groups }} group{% if counts.manual_groups != 1 %}s{% endif %}: {% for file in manual_fix_files %}{{ file.displayPath }}{% if not loop.last %}, {% endif %}{% endfor %}".into(),
                agent: "{% for group in groups %}{% if group.manual_fix_files | length %}{{ group.display_name }}: {% for file in group.manual_fix_files %}{{ file.displayPath }}{% if not loop.last %}, {% endif %}{% endfor %}. Reports: {% for path in group.artifact_paths %}{{ path }}{% if not loop.last %}, {% endif %}{% endfor %}{% if not loop.last %}\n{% endif %}{% endif %}{% endfor %}".into(),
            },
            operational_error: TemplatePair {
                user: "{{ counts.operational_errors }} operational formatter/linter error{% if counts.operational_errors != 1 %}s{% endif %}. Details: {{ artifact_paths | join(\", \") }}".into(),
                agent: "Operational formatter/linter failures remain. Inspect {{ artifact_paths | join(\", \") }} before retrying Stop.".into(),
            },
            master_user: "{{ rendered_bucket_lists.user | join(\"\n\") }}{% if counts.unavailable_tools %}{% if rendered_bucket_lists.user | length %}\n{% endif %}{% for tool in unavailable_tools %}{{ tool.message }}{% if not loop.last %}\n{% endif %}{% endfor %}{% endif %}{% if counts.coverage_gaps %}{% if rendered_bucket_lists.user | length or counts.unavailable_tools %}\n{% endif %}File-activity coverage is incomplete for {{ counts.coverage_gaps }} reported gap{% if counts.coverage_gaps != 1 %}s{% endif %}; see {{ run.summary_path }}.{% endif %}".into(),
            master_agent: "{{ rendered_bucket_lists.agent | join(\"\n\") }}{% if counts.coverage_gaps %}{% if rendered_bucket_lists.agent | length %}\n{% endif %}File-activity coverage is incomplete; inspect the reported gaps in {{ run.summary_path }} before treating the run as exhaustive.{% endif %}".into(),
            render_empty_buckets: false,
        }
    }
}

fn default_file_groups() -> Vec<FileGroup> {
    vec![
        FileGroup {
            id: "c-cpp".into(),
            display_name: "C/C++".into(),
            include: [
                "*.c", "**/*.c", "*.h", "**/*.h", "*.cc", "**/*.cc", "*.cpp", "**/*.cpp", "*.cxx",
                "**/*.cxx", "*.hh", "**/*.hh", "*.hpp", "**/*.hpp", "*.hxx", "**/*.hxx",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        },
        FileGroup {
            id: "rust".into(),
            display_name: "Rust".into(),
            include: vec!["*.rs".into(), "**/*.rs".into()],
        },
        FileGroup {
            id: "python".into(),
            display_name: "Python".into(),
            include: ["*.py", "**/*.py", "*.pyi", "**/*.pyi"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        },
        FileGroup {
            id: "javascript-typescript".into(),
            display_name: "JavaScript/TypeScript".into(),
            include: [
                "*.js", "**/*.js", "*.jsx", "**/*.jsx", "*.ts", "**/*.ts", "*.tsx", "**/*.tsx",
                "*.mjs", "**/*.mjs", "*.cjs", "**/*.cjs",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        },
        FileGroup {
            id: "documentation".into(),
            display_name: "Documentation".into(),
            include: ["*.md", "**/*.md", "*.mdx", "**/*.mdx"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        },
        FileGroup {
            id: "other".into(),
            display_name: "Other".into(),
            include: vec!["**".into()],
        },
    ]
}

/// Field-preserving patch for a [`TemplatePair`].
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TemplatePairPatch {
    /// Optional replacement for the user-facing template.
    pub user: Option<String>,
    /// Optional replacement for the agent-facing template.
    pub agent: Option<String>,
}

impl TemplatePairPatch {
    fn apply_to(self, pair: &mut TemplatePair) {
        if let Some(user) = self.user {
            pair.user = user;
        }
        if let Some(agent) = self.agent {
            pair.agent = agent;
        }
    }
}

/// Field-preserving overlay for deferred reporting configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeferredReportingPatch {
    /// Optional replacement for the ordered file groups.
    pub groups: Option<Vec<FileGroup>>,
    /// Optional clean-result template patch.
    pub clean: Option<TemplatePairPatch>,
    /// Optional auto-fixed-result template patch.
    pub auto_fixed: Option<TemplatePairPatch>,
    /// Optional manual-fix-result template patch.
    pub manual_fixes_needed: Option<TemplatePairPatch>,
    /// Optional operational-error template patch.
    pub operational_error: Option<TemplatePairPatch>,
    /// Optional aggregate user-facing template replacement.
    pub master_user: Option<String>,
    /// Optional aggregate agent-facing template replacement.
    pub master_agent: Option<String>,
    /// Optional empty-category rendering override.
    pub render_empty_buckets: Option<bool>,
}

impl DeferredReportingPatch {
    /// Applies fields present in this patch to an existing reporting configuration.
    pub fn apply_to(self, reporting: &mut DeferredReporting) {
        if let Some(groups) = self.groups {
            reporting.groups = groups;
        }
        if let Some(pair) = self.clean {
            pair.apply_to(&mut reporting.clean);
        }
        if let Some(pair) = self.auto_fixed {
            pair.apply_to(&mut reporting.auto_fixed);
        }
        if let Some(pair) = self.manual_fixes_needed {
            pair.apply_to(&mut reporting.manual_fixes_needed);
        }
        if let Some(pair) = self.operational_error {
            pair.apply_to(&mut reporting.operational_error);
        }
        if let Some(master_user) = self.master_user {
            reporting.master_user = master_user;
        }
        if let Some(master_agent) = self.master_agent {
            reporting.master_agent = master_agent;
        }
        if let Some(render_empty_buckets) = self.render_empty_buckets {
            reporting.render_empty_buckets = render_empty_buckets;
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
    /// Behavior when reconciliation cannot establish complete activity coverage.
    pub coverage_gap_policy: CoverageGapPolicy,
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
            coverage_gap_policy: CoverageGapPolicy::BestEffort,
            ignored_directory_names: DEFAULT_IGNORED_DIRECTORY_NAMES
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
        }
    }
}

/// Directory basenames pruned by default from reconciliation, target
/// resolution, and snapshot walks: version-control metadata, dependency,
/// virtual-environment, cache, and build-tool output directories that are
/// never agent-authored source, and `.agent-hook-kit`, which holds the hook's
/// own configuration and diagnostics artifacts.
///
/// This is the same list as `hookkit_file_activity::DEFAULT_IGNORED_DIRECTORY_NAMES`,
/// so direct library callers and the bundled runners prune the same
/// directories; the runner's tests keep the two in sync.
pub const DEFAULT_IGNORED_DIRECTORY_NAMES: &[&str] = &[
    ".agent-hook-kit",
    ".context",
    ".direnv",
    ".git",
    ".gradle",
    ".hg",
    ".mypy_cache",
    ".next",
    ".nox",
    ".nuxt",
    ".parcel-cache",
    ".pytest_cache",
    ".ruff_cache",
    ".svelte-kit",
    ".svn",
    ".terraform",
    ".tox",
    ".turbo",
    ".venv",
    "__pycache__",
    "node_modules",
    "target",
    "venv",
];

/// Field-preserving overlay for [`FileActivitySettings`].
///
/// A later configuration layer that sets one `fileActivity` field leaves every
/// other field inherited from earlier layers. `ignoredDirectoryNames` replaces
/// the whole list when present.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FileActivitySettingsPatch {
    /// Optional mtime-scan override.
    pub filesystem_mtime: Option<bool>,
    /// Optional VCS fallback override.
    pub vcs: Option<FileActivityVcsFallback>,
    /// Optional clock tolerance override.
    pub timestamp_tolerance_millis: Option<u64>,
    /// Optional traversal budget override.
    pub max_entries: Option<usize>,
    /// Optional coverage-gap policy override.
    pub coverage_gap_policy: Option<CoverageGapPolicy>,
    /// Optional replacement for the ignored directory names.
    pub ignored_directory_names: Option<Vec<String>>,
}

impl FileActivitySettingsPatch {
    /// Overwrites each setting represented by `Some`, leaving others unchanged.
    pub fn apply_to(self, settings: &mut FileActivitySettings) {
        if let Some(filesystem_mtime) = self.filesystem_mtime {
            settings.filesystem_mtime = filesystem_mtime;
        }
        if let Some(vcs) = self.vcs {
            settings.vcs = vcs;
        }
        if let Some(timestamp_tolerance_millis) = self.timestamp_tolerance_millis {
            settings.timestamp_tolerance_millis = timestamp_tolerance_millis;
        }
        if let Some(max_entries) = self.max_entries {
            settings.max_entries = max_entries;
        }
        if let Some(coverage_gap_policy) = self.coverage_gap_policy {
            settings.coverage_gap_policy = coverage_gap_policy;
        }
        if let Some(ignored_directory_names) = self.ignored_directory_names {
            settings.ignored_directory_names = ignored_directory_names;
        }
    }
}

/// Behavior when file-activity reconciliation reports incomplete coverage.
///
/// A gap is reported once, in the Stop that first observes it; retrying the
/// same analysis cannot resolve it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CoverageGapPolicy {
    /// Continue with the available evidence while reporting coverage gaps.
    #[default]
    BestEffort,
    /// Also block the Stop in which a coverage gap is first reported.
    Strict,
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
    /// Restore deferred reporting configuration to its defaults before merging.
    pub reset_deferred_reporting: bool,
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
    /// Named deferred workflows.
    pub workflows: BTreeMap<String, Workflow>,
    /// Deferred workflow identifiers in execution order.
    pub workflow_order: Vec<String>,
    /// Optional fallback diagnostic when a remedy cannot be verified.
    pub unverified_remedy_fallback: Option<String>,
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
            workflows: BTreeMap::new(),
            workflow_order: Vec::new(),
            unverified_remedy_fallback: None,
            phases: BTreeMap::new(),
            phase_order: Vec::new(),
            messages: Messages::default(),
            diagnostics: Diagnostics::default(),
            enabled: true,
        }
    }
}

/// One repeatable deferred check with an optional automatic remedy.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Workflow {
    /// Read-only command used to detect issues.
    pub check: Option<WorkflowCommand>,
    /// Optional command used to repair detected issues.
    pub remedy: Option<WorkflowCommand>,
    /// Inputs whose changes invalidate a prior check.
    pub check_scope: CheckScope,
    /// Granularity used to divide candidates into invocations.
    pub invocation: InvocationGranularity,
    /// Whether this workflow participates in deferred execution.
    pub enabled: bool,
}

impl Default for Workflow {
    fn default() -> Self {
        Self {
            check: None,
            remedy: None,
            check_scope: CheckScope::default(),
            invocation: InvocationGranularity::default(),
            enabled: true,
        }
    }
}

/// One command in a deferred workflow.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkflowCommand {
    /// Per-command executable override.
    pub program: Option<String>,
    /// Argument template expanded for each invocation.
    pub argv: Vec<ArgvElement>,
    /// Exit-code classification.
    pub exit_codes: ExitCodes,
    /// Whether non-empty standard output represents actionable issues.
    pub issues_on_stdout: bool,
    /// Paths the command may modify.
    pub writes: WriteBehavior,
    /// Literal values expanded by [`ArgToken::ExtraArgs`].
    pub extra_args: Vec<String>,
}

impl Default for WorkflowCommand {
    fn default() -> Self {
        Self {
            program: None,
            argv: Vec::new(),
            exit_codes: ExitCodes::default(),
            issues_on_stdout: false,
            writes: WriteBehavior::None,
            extra_args: Vec::new(),
        }
    }
}

/// Inputs whose changes invalidate a prior workflow check.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckScope {
    /// Only changes to the workflow's target files invalidate its check.
    #[default]
    TargetFiles,
    /// Any change in the workspace invalidates the workflow's check.
    Workspace,
}

/// How candidates are divided into workflow invocations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InvocationGranularity {
    /// Invoke once for each selected file.
    PerFile,
    /// Invoke once for the selected file batch.
    #[default]
    Batch,
    /// Invoke once for each workspace partition.
    Workspace,
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
    /// Executable selected for the current tool command.
    ToolExecutable,
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
    /// Directory override relative to the project root, or `None` to use the
    /// runner-wide setting.
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
