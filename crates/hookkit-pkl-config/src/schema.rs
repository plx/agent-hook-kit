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
    pub settings: Settings,
    pub merge: Merge,
    pub tools: BTreeMap<String, ToolSpec>,
    pub run: Vec<String>,
}

/// Root configuration patch loaded from one Pkl file before multi-file merge.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RunnerConfigPatch {
    pub settings: SettingsPatch,
    pub merge: Merge,
    pub tools: BTreeMap<String, ToolSpec>,
    pub run: Vec<String>,
}

impl RunnerConfigPatch {
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
    pub jobs: u32,
    pub fail_fast: bool,
    pub continue_after_issues: bool,
    pub exclude: Vec<String>,
    pub lowering_policy: LoweringPolicy,
    pub diagnostics_directory: Option<String>,
    pub missing_tool_policy: MissingToolPolicy,
    pub file_activity: Option<FileActivitySettings>,
    pub deferred_reporting: DeferredReporting,
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
            deferred_reporting: DeferredReporting::default(),
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
    pub jobs: Option<u32>,
    pub fail_fast: Option<bool>,
    pub continue_after_issues: Option<bool>,
    pub exclude: Option<Vec<String>>,
    pub lowering_policy: Option<LoweringPolicy>,
    pub diagnostics_directory: Option<String>,
    pub missing_tool_policy: Option<MissingToolPolicy>,
    pub file_activity: Option<FileActivitySettings>,
    pub deferred_reporting: Option<DeferredReportingPatch>,
}

impl SettingsPatch {
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
        if let Some(deferred_reporting) = self.deferred_reporting {
            deferred_reporting.apply_to(&mut settings.deferred_reporting);
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TemplatePair {
    pub user: String,
    pub agent: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileGroup {
    pub id: String,
    pub display_name: String,
    pub include: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeferredReporting {
    pub groups: Vec<FileGroup>,
    pub clean: TemplatePair,
    pub auto_fixed: TemplatePair,
    pub manual_fixes_needed: TemplatePair,
    pub operational_error: TemplatePair,
    pub master_user: String,
    pub master_agent: String,
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
            master_user: "{{ rendered_bucket_lists.user | join(\"\n\") }}{% if counts.coverage_gaps %}{% if rendered_bucket_lists.user | length %}\n{% endif %}File-activity coverage is incomplete for {{ counts.coverage_gaps }} retained gap{% if counts.coverage_gaps != 1 %}s{% endif %}; see {{ run.summary_path }}.{% endif %}".into(),
            master_agent: "{{ rendered_bucket_lists.agent | join(\"\n\") }}{% if counts.coverage_gaps %}{% if rendered_bucket_lists.agent | length %}\n{% endif %}File-activity coverage is incomplete; inspect retained gaps in {{ run.summary_path }} before treating the run as exhaustive.{% endif %}".into(),
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

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TemplatePairPatch {
    pub user: Option<String>,
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

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DeferredReportingPatch {
    pub groups: Option<Vec<FileGroup>>,
    pub clean: Option<TemplatePairPatch>,
    pub auto_fixed: Option<TemplatePairPatch>,
    pub manual_fixes_needed: Option<TemplatePairPatch>,
    pub operational_error: Option<TemplatePairPatch>,
    pub master_user: Option<String>,
    pub master_agent: Option<String>,
    pub render_empty_buckets: Option<bool>,
}

impl DeferredReportingPatch {
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
    pub filesystem_mtime: bool,
    pub vcs: FileActivityVcsFallback,
    pub timestamp_tolerance_millis: u64,
    pub max_entries: usize,
    pub coverage_gap_policy: CoverageGapPolicy,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CoverageGapPolicy {
    #[default]
    BestEffort,
    Strict,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileActivityVcsFallback {
    #[default]
    Disabled,
    GitDirty,
}

/// How to handle optional common-output intents the harness cannot represent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LoweringPolicy {
    Strict,
    BestEffort,
    #[default]
    BestEffortWithWarnings,
}

/// What to do when a configured tool executable is missing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MissingToolPolicy {
    #[default]
    UserNotice,
    HardFailure,
    HarnessBlock,
}

/// Merge controls for multi-file config discovery.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Merge {
    pub reset_all: bool,
    pub reset: Vec<MergeResetKey>,
    pub reset_tools: Vec<String>,
    pub reset_deferred_reporting: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MergeResetKey {
    Settings,
    Tools,
    Run,
}

/// One external tool available to the runner.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ToolSpec {
    pub id: String,
    pub display_name: String,
    pub executable: String,
    pub install_hint: Option<String>,
    pub files: FileSelection,
    pub workspace_indicator: Option<String>,
    pub workflows: BTreeMap<String, Workflow>,
    pub workflow_order: Vec<String>,
    pub phases: BTreeMap<String, Phase>,
    pub phase_order: Vec<String>,
    pub messages: Messages,
    pub diagnostics: Diagnostics,
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
    pub check: Option<WorkflowCommand>,
    pub remedy: Option<WorkflowCommand>,
    pub check_scope: CheckScope,
    pub invocation: InvocationGranularity,
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
    pub program: Option<String>,
    pub argv: Vec<ArgvElement>,
    pub exit_codes: ExitCodes,
    pub writes: WriteBehavior,
    pub extra_args: Vec<String>,
}

impl Default for WorkflowCommand {
    fn default() -> Self {
        Self {
            program: None,
            argv: Vec::new(),
            exit_codes: ExitCodes::default(),
            writes: WriteBehavior::None,
            extra_args: Vec::new(),
        }
    }
}

/// Inputs whose changes invalidate a prior workflow check.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckScope {
    #[default]
    TargetFiles,
    Workspace,
}

/// How candidates are divided into workflow invocations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InvocationGranularity {
    PerFile,
    #[default]
    Batch,
    Workspace,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FileSelection {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

/// One external command phase inside a tool spec.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Phase {
    pub mode: PhaseMode,
    pub program: Option<String>,
    pub argv: Vec<ArgvElement>,
    pub exit_codes: ExitCodes,
    pub writes: WriteBehavior,
    pub enabled: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PhaseMode {
    Format,
    Fix,
    Verify,
    CheckOnly,
}

/// A single argv element: either a literal string or a placeholder token.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ArgvElement {
    Literal(String),
    Token(ArgToken),
}

/// Placeholder tokens for argv expansion. The Pkl side emits these as
/// `{"type": "Files"}` etc.; we tag on `type` to keep the wire shape readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(tag = "type")]
pub enum ArgToken {
    Files,
    WorkspaceFiles,
    Workspace,
    WorkspaceIndicator,
    ProjectRoot,
    ExtraArgs,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExitCodes {
    pub clean: Vec<i32>,
    pub issues: Vec<i32>,
    pub failure: Vec<i32>,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UnexpectedExitPolicy {
    #[default]
    Failure,
    Issues,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WriteBehavior {
    #[default]
    None,
    TargetFiles,
    MatchingGlobs,
    Workspace,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Diagnostics {
    pub directory: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Messages {
    pub clean_changed_agent: String,
    pub issues_agent: String,
    pub issues_changed_agent: String,
    pub unavailable_user: Option<String>,
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

pub fn default_clean_changed_agent() -> String {
    "{{ tool }} changed {{ changed_files | join(\", \") }}; re-read changed files before editing further.".into()
}

pub fn default_issues_agent() -> String {
    "{{ tool }} reports issues; inspect diagnostics at {{ diagnostics_path }}.".into()
}

pub fn default_issues_changed_agent() -> String {
    "{{ tool }} changed {{ changed_files | join(\", \") }} and issues remain; re-read changed files, then inspect diagnostics at {{ diagnostics_path }}.".into()
}
