//! Reusable runner for the `post-tool-use-agent-hook` CLI.
//!
//! The runner reads a harness-native post-tool-use event from stdin, loads the
//! Pkl-driven tool catalog through [`hookkit_pkl_config`], runs each tool in
//! the configured order, and lowers a unified result to the selected harness.

use globset::{Glob, GlobSet, GlobSetBuilder};
use hookkit_common::UserNotice;
use hookkit_common::input::CommonHookInput;
use hookkit_common::message::{DiagnosticArtifact, DiagnosticReport};
use hookkit_common::output::{CommonHookOutput, CommonPostToolUseOutput, LoweringPolicy};
use hookkit_core::{Harness, HookkitError};
use hookkit_pkl_config::schema as pkl;
use hookkit_runtime::RuntimeContext;
use hookkit_runtime::artifacts::ArtifactManager;
use minijinja::Environment;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

const DEFAULT_CLEAN_CHANGED_AGENT: &str = "{{ tool }} changed {{ changed_files | join(\", \") }}; re-read changed files before editing further.";
const DEFAULT_ISSUES_AGENT: &str =
    "{{ tool }} reports issues; inspect diagnostics at {{ diagnostics_path }}.";
const DEFAULT_ISSUES_CHANGED_AGENT: &str = "{{ tool }} changed {{ changed_files | join(\", \") }} and issues remain; re-read changed files, then inspect diagnostics at {{ diagnostics_path }}.";

const BINARY_NAME: &str = "post-tool-use-agent-hook";

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

    pub fn with_messages(mut self, messages: ToolMessages) -> Self {
        self.messages = messages;
        self
    }
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
    pub harness: Harness,
    pub config_path: Option<PathBuf>,
}

/// Parse `--claude|--codex|--gemini [--config PATH]` from `std::env::args`.
#[allow(clippy::result_unit_err)]
pub fn parse_args() -> Result<Cli, ()> {
    let mut harness = None;
    let mut config_path = None;
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--claude" => harness = Some(set_harness(harness, Harness::Claude)?),
            "--codex" => harness = Some(set_harness(harness, Harness::Codex)?),
            "--gemini" => harness = Some(set_harness(harness, Harness::Gemini)?),
            "--config" => {
                let Some(path) = args.next() else {
                    eprintln!("{}", usage());
                    return Err(());
                };
                config_path = Some(PathBuf::from(path));
            }
            "--dump-parsed" => {}
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

fn set_harness(current: Option<Harness>, next: Harness) -> Result<Harness, ()> {
    if current.is_some() {
        eprintln!("{}", usage());
        return Err(());
    }
    Ok(next)
}

fn usage() -> String {
    format!("Usage: {BINARY_NAME} --claude|--codex|--gemini [--config PATH]")
}

/// Run the full post-tool-use hook from parsed CLI args.
pub fn run_runner(cli: Cli) -> std::process::ExitCode {
    hookkit_runtime::run_common(cli.harness, move |input, ctx| {
        run_common_input(input, ctx, cli.config_path.as_deref())
    })
}

/// Run a parsed common input through the Pkl-driven runner. Exposed for tests
/// that prefer to drive the runner without spinning up an entire process.
pub fn run_common_input(
    input: CommonHookInput,
    ctx: &RuntimeContext,
    config_path: Option<&Path>,
) -> hookkit_core::Result<CommonHookOutput> {
    let CommonHookInput::PostToolUse(post_tool) = input else {
        return Ok(CommonHookOutput::empty());
    };

    let cwd = PathBuf::from(ctx.cwd.as_str());
    let loaded = hookkit_pkl_config::discover_and_load(&cwd, config_path)
        .map_err(|e| invalid_data(e.to_string()))?;

    let project_root = normalize_path(&loaded.project_root);
    let lowering = lowering_from(&loaded.config.settings.lowering_policy);
    let missing_tool_policy = loaded.config.settings.missing_tool_policy;
    let fail_fast = loaded.config.settings.fail_fast;
    let continue_after_issues = loaded.config.settings.continue_after_issues;

    let mut output = CommonPostToolUseOutput::new().with_lowering_policy(lowering);
    let mut had_hard_failure = false;
    let mut had_harness_block_message: Option<String> = None;

    let tools = resolve_run_order(&loaded.config)?;
    if tools.is_empty() {
        return Ok(CommonHookOutput::empty());
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

        let candidates = post_tool.modified_files(&cwd, Some(&project_root));
        let matcher = FileMatcher::new(&spec.file_selection)?;
        let runnable_paths = candidates
            .into_iter()
            .map(|c| c.absolute_path)
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
    lower_domain_outcome(outcome)
}

/// Runner-owned semantic result. Tool policy and classification deliberately do
/// not leak into core/common crates.
#[derive(Debug)]
pub enum RunnerDomainOutcome {
    Clean,
    Report(CommonPostToolUseOutput),
    HarnessBlock {
        message: String,
        output: CommonPostToolUseOutput,
    },
    OperationalFailure {
        message: String,
    },
    UnsupportedHarness {
        harness: String,
        reason: String,
    },
}

fn lower_domain_outcome(outcome: RunnerDomainOutcome) -> hookkit_core::Result<CommonHookOutput> {
    match outcome {
        RunnerDomainOutcome::Clean => Ok(CommonHookOutput::empty()),
        RunnerDomainOutcome::Report(output) => Ok(CommonHookOutput::PostToolUse(output)),
        RunnerDomainOutcome::HarnessBlock { message, output } => Ok(CommonHookOutput::PostToolUse(
            output.with_harness_block(message),
        )),
        RunnerDomainOutcome::OperationalFailure { message } => Err(invalid_data(message)),
        RunnerDomainOutcome::UnsupportedHarness { harness, reason } => Err(invalid_data(format!(
            "post-tool-use runner does not support {harness}: {reason}"
        ))),
    }
}

fn is_empty_output(output: &CommonPostToolUseOutput) -> bool {
    output.notices.is_empty()
        && output.agent_feedback.is_empty()
        && output.diagnostics.is_empty()
        && output.replace_tool_result.is_none()
        && output.tail_tool_call.is_none()
        && output.session_control.is_none()
        && output.harness_block.is_none()
}

fn lowering_from(policy: &pkl::LoweringPolicy) -> LoweringPolicy {
    match policy {
        pkl::LoweringPolicy::Strict => LoweringPolicy::Strict,
        pkl::LoweringPolicy::BestEffort => LoweringPolicy::BestEffort,
        pkl::LoweringPolicy::BestEffortWithWarnings => LoweringPolicy::BestEffortWithWarnings,
    }
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
        if let Some(phase) = spec.phases.get(id)
            && seen.insert(id.clone())
        {
            out.push((id.clone(), phase));
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
    ctx: &RuntimeContext,
    missing_tool_policy: pkl::MissingToolPolicy,
    output: &mut CommonPostToolUseOutput,
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
                if let Some((phase, executable, install_hint)) = unavailable.first() {
                    let message = render_unavailable_message(
                        context,
                        phase,
                        executable,
                        install_hint.as_deref(),
                    )?;
                    eprintln!("{message}");
                }
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
    ctx: &RuntimeContext,
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
            &ctx.artifact_key(format!("{}-{label}", context.spec.id)),
            diagnostics,
        )
        .map_err(Into::into)
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

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn domain_outcomes_keep_clean_failure_and_unsupported_distinct() {
        assert!(matches!(
            lower_domain_outcome(RunnerDomainOutcome::Clean).unwrap(),
            CommonHookOutput::Empty
        ));
        assert!(
            lower_domain_outcome(RunnerDomainOutcome::OperationalFailure {
                message: "checker crashed".into(),
            })
            .is_err()
        );
        assert!(
            lower_domain_outcome(RunnerDomainOutcome::UnsupportedHarness {
                harness: "antigravity".into(),
                reason: "no changed-file data".into(),
            })
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
