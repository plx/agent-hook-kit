//! Rust mirror of the post-tool-use Pkl schema.
//!
//! The Pkl module evaluates to JSON via `pkl eval --format json`; these types
//! deserialize from that JSON. The shape intentionally mirrors the runtime
//! `ToolSpec` family in `hookkit-tool-runner` so the bridge in
//! [`crate::convert`] is mechanical.

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
    }
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
            phases: BTreeMap::new(),
            phase_order: Vec::new(),
            messages: Messages::default(),
            diagnostics: Diagnostics::default(),
            enabled: true,
        }
    }
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
