//! Session-batched Stop runner: reconciliation, deferred execution, durable
//! run bundles, loop protection, and exact native Stop lowering.

use crate::convert::{convert_tool_spec, resolve_run_order};
use crate::deferred::{
    DeferredLog, DeferredReporter, RenderedBuckets, RenderedMessages, ScheduledWorkflow,
    StopLoweringMetadata, StopLoweringPlan, TemplateRun, WorkflowExecutionPolicy,
    execute_deferred_workflows, plan_stop_lowering,
};
use crate::exec::{
    CommandError, ExecutionSettings, FILE_ARGUMENT_BUDGET_BYTES, FileMatcher, PhaseLog,
    PhaseStatus, ToolJob, build_jobs, format_logs, split_jobs_for_argument_budget,
};
use crate::post_tool::{diagnostics_directory, execution_settings};
use crate::spec::{InvocationGranularity, ToolSpec, ToolWorkflow, WriteBehavior};
use crate::stop_guard::{
    ContinuationSignal, GuardRecord, StopDecision, StopGuardStore, blocking_fingerprint,
};
use crate::util::{activity_error, invalid_data, normalize_path, state_error};
use crate::{
    ArtifactClassification, CheckOutcome, CommandPhase, CoverageGap, DeferredRunResult, FileStatus,
    OperationalProblem, RunArtifact,
};
use hookkit_common::{TurnCompletionCommandEnvironment, TurnCompletionInput, TurnCompletionOutput};
use hookkit_core::{HarnessId, RuntimeContext, Utf8PathBuf};
use hookkit_file_activity::{
    FileActivityEvent, FileActivityStore, FileActivityTarget, PendingFileActivity,
    ReconciliationOptions, ResolveOptions, VcsFallback, reconcile, resolve_files,
};
use hookkit_pkl_config::schema as pkl;
use hookkit_pkl_config::{Loaded, PklConfigError};
use hookkit_session_state::{
    EntityOperationError, EntityOutcome, EntityView, FamilyId, RunBundle, SessionState,
    StateFamily, UtcTimestamp,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const BATCHED_TOOLS_FAMILY: &str = "agent-hook-kit.batched-tools";
const SUMMARY_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BatchToolSummary {
    tool_id: String,
    file_count: usize,
    issues: bool,
    operational_failure: bool,
    unavailable: bool,
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
    stop_decision: StopDecision,
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
    unavailable_tools: usize,
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
    reported_gaps: Vec<String>,
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
    decision: StopDecision,
    source: (usize, Vec<String>),
    candidates: &'a [PathBuf],
    tools: Vec<BatchToolSummary>,
    disposition: &'a DeferredStateDisposition,
    result: DeferredRunResult,
}

#[derive(Debug)]
struct ActivityResolution {
    not_applicable_files: BTreeSet<PathBuf>,
    /// Targets that could not be fully materialized. Reported once.
    unresolved_targets: Vec<FileActivityTarget>,
    /// Targets never attempted because an earlier target exhausted the
    /// traversal budget. Retained for the next Stop.
    skipped_targets: Vec<FileActivityTarget>,
    /// Message-only gaps from observation or earlier runs. Reported once.
    gap_messages: BTreeSet<String>,
    truncated: bool,
}

#[derive(Debug, Clone)]
struct DeferredStateDisposition {
    retry_files: BTreeSet<Utf8PathBuf>,
    retry_targets: Vec<FileActivityTarget>,
    reported_gaps: BTreeSet<String>,
    handled_files: BTreeSet<Utf8PathBuf>,
}

/// Everything one Stop attempt needs besides the consumed entity view.
struct TurnCompletionRun<'a, 'ctx> {
    ctx: &'a RuntimeContext<'ctx>,
    roots: &'a [Utf8PathBuf],
    activity_settings: &'a pkl::FileActivitySettings,
    activity_store: &'a FileActivityStore,
    runner_family: &'a StateFamily,
    excluded_roots: &'a BTreeSet<Utf8PathBuf>,
    continuation: ContinuationSignal,
}

/// Workspace roots for one Stop. Claude Code runs hooks in the agent's
/// current directory, which follows `cd` in its Bash tool, so the stable
/// `CLAUDE_PROJECT_DIR` comes first; other roots from the input that are not
/// inside it are kept after it.
fn turn_completion_roots(
    environment: &TurnCompletionCommandEnvironment,
    ctx: &RuntimeContext<'_>,
) -> hookkit_core::Result<Vec<Utf8PathBuf>> {
    let mut roots = Vec::new();
    if let TurnCompletionCommandEnvironment::Claude(environment) = environment {
        roots.push(environment.project_dir.clone());
    }
    for root in ctx.workspace_roots() {
        let canonical = normalize_path(root.as_std_path());
        let covered = roots
            .iter()
            .any(|existing| canonical.starts_with(normalize_path(existing.as_std_path())));
        if !covered {
            roots.push(root.clone());
        }
    }
    if roots.is_empty() {
        return Err(invalid_data(
            "turn-completion input has no workspace root".into(),
        ));
    }
    Ok(roots)
}

pub(crate) fn run_turn_completion_input(
    turn_completion: TurnCompletionInput,
    environment: &TurnCompletionCommandEnvironment,
    ctx: &RuntimeContext<'_>,
    config_path: Option<&Path>,
    state_dir: Option<&Path>,
) -> hookkit_core::Result<TurnCompletionOutput> {
    let roots = turn_completion_roots(environment, ctx)?;
    let cwd = PathBuf::from(roots[0].as_str());
    let state_root = crate::resolve_state_root(state_dir, Some(&cwd));
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
    let excluded_roots =
        excluded_activity_roots(loaded.as_ref().ok(), activity_store.state().directory());
    let mut reconciliation = ReconciliationOptions::new(roots.clone(), UtcTimestamp::now());
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
    reconciliation.excluded_roots = excluded_roots.clone();
    reconcile(&activity_store, reconciliation).map_err(activity_error)?;
    let run = TurnCompletionRun {
        ctx,
        roots: &roots,
        activity_settings: &activity_settings,
        activity_store: &activity_store,
        runner_family: &runner_family,
        excluded_roots: &excluded_roots,
        continuation: ContinuationSignal::from_input(&turn_completion),
    };
    activity_store
        .pending()
        .try_with_entity(|view| run_turn_completion_view(&run, loaded, view))
        .map_err(|error| match error {
            EntityOperationError::State(error) => state_error(error),
            EntityOperationError::Operation(error) => error,
        })
}

/// Roots excluded from reconciliation and target resolution: the session
/// state directory and every configured diagnostics directory, so the hook's
/// own artifacts never become Stop candidates.
fn excluded_activity_roots(
    loaded: Option<&Loaded>,
    state_directory: &Path,
) -> BTreeSet<Utf8PathBuf> {
    let mut roots = BTreeSet::new();
    let mut insert = |path: PathBuf| {
        for path in [normalize_path(&path), hookkit_core::normalize_path(&path)] {
            if let Ok(path) = Utf8PathBuf::from_path_buf(path) {
                roots.insert(path);
            }
        }
    };
    insert(state_directory.to_path_buf());
    if let Some(loaded) = loaded {
        let settings = &loaded.config.settings;
        let global = settings.diagnostics_directory.as_deref();
        insert(diagnostics_directory(None, global, &loaded.project_root));
        for tool in loaded.config.tools.values() {
            if let Some(directory) = tool.diagnostics.directory.as_deref() {
                insert(diagnostics_directory(
                    Some(directory),
                    global,
                    &loaded.project_root,
                ));
            }
        }
    }
    roots
}

fn run_turn_completion_view(
    run: &TurnCompletionRun<'_, '_>,
    loaded: Result<Loaded, PklConfigError>,
    view: &EntityView<'_, PendingFileActivity>,
) -> hookkit_core::Result<EntityOutcome<TurnCompletionOutput>> {
    let ctx = run.ctx;
    if view.events().is_empty() {
        let lowering = plan_stop_lowering(
            ctx.harness(),
            false,
            None,
            None,
            "",
            pkl::LoweringPolicy::BestEffortWithWarnings,
        )?;
        return Ok(EntityOutcome::retain(lowering.finish()?));
    }
    let fallback_project_root = normalize_path(run.roots[0].as_std_path());

    let mut resolve_options = ResolveOptions::new(run.roots.to_vec());
    resolve_options.max_entries = run.activity_settings.max_entries;
    resolve_options.ignored_directory_names = run
        .activity_settings
        .ignored_directory_names
        .iter()
        .cloned()
        .collect();
    resolve_options.excluded_roots = run.excluded_roots.clone();
    let resolved = resolve_files(view.state(), &resolve_options).map_err(activity_error)?;
    // Scopes the budget never reached are retried on the next Stop; every
    // other unresolved scope, including the one whose walk ran out of
    // budget (it would stop at the same point again), is reported once.
    let skipped_targets = resolved.unattempted_targets;
    let mut unresolved_targets = resolved.unresolved_targets;
    unresolved_targets.retain(|target| !skipped_targets.contains(target));
    let resolution = ActivityResolution {
        not_applicable_files: resolved
            .not_applicable_files
            .into_iter()
            .map(|path| normalize_path(path.as_std_path()))
            .collect(),
        unresolved_targets,
        skipped_targets,
        gap_messages: source_gap_messages(view),
        truncated: resolved.truncated,
    };
    let excluded = run
        .excluded_roots
        .iter()
        .map(|root| root.as_std_path())
        .collect::<Vec<_>>();
    let mut candidates = resolved
        .files
        .into_iter()
        .map(|path| normalize_path(path.as_std_path()))
        .filter(|path| !excluded.iter().any(|root| path.starts_with(root)))
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.dedup();
    let source = (
        view.events().len(),
        view.events()
            .iter()
            .map(|entry| entry.id().to_string())
            .collect::<Vec<_>>(),
    );
    let bundle = run
        .runner_family
        .start_run("turn-completion")
        .map_err(state_error)?;
    let guard = StopGuardStore::open(run.runner_family).map_err(state_error)?;

    let loaded = match loaded {
        Ok(loaded) => loaded,
        Err(error) => {
            // A missing `pkl` is an environment problem the agent cannot
            // fix: report it and keep the files queued without blocking.
            let blocks = !matches!(error, PklConfigError::PklNotFound);
            return commit_deferred_config_failure(
                run,
                bundle,
                &guard,
                ConfigFailure {
                    project_root: &fallback_project_root,
                    candidates: &candidates,
                    resolution: &resolution,
                    source,
                    lowering_policy: pkl::LoweringPolicy::BestEffortWithWarnings,
                    contents: error.to_string(),
                    blocks,
                },
            );
        }
    };

    let project_root = normalize_path(&loaded.project_root);
    let settings = &loaded.config.settings;
    let lowering_policy = settings.lowering_policy;
    let config_failure = |contents: String| ConfigFailure {
        project_root: &project_root,
        candidates: &candidates,
        resolution: &resolution,
        source: source.clone(),
        lowering_policy,
        contents,
        blocks: true,
    };
    let reporter = match DeferredReporter::new(&settings.deferred_reporting) {
        Ok(reporter) => reporter,
        Err(error) => {
            return commit_deferred_config_failure(
                run,
                bundle,
                &guard,
                config_failure(error.to_string()),
            );
        }
    };
    let tools = match resolve_run_order(&loaded.config) {
        Ok(tools) => tools,
        Err(error) => {
            return commit_deferred_config_failure(
                run,
                bundle,
                &guard,
                config_failure(error.to_string()),
            );
        }
    };
    let execution_settings = Arc::new(execution_settings(settings));
    let (plan, planned_tools) = match build_deferred_plan(
        &tools,
        &candidates,
        &project_root,
        &settings.exclude,
        &execution_settings,
    ) {
        Ok(plan) => plan,
        Err(error) => {
            return commit_deferred_config_failure(
                run,
                bundle,
                &guard,
                config_failure(error.to_string()),
            );
        }
    };
    let mut execution = execute_deferred_workflows(
        &plan,
        WorkflowExecutionPolicy {
            jobs: settings.jobs,
            fail_fast: settings.fail_fast,
            missing_tool_policy: settings.missing_tool_policy,
        },
    );
    let summaries = write_deferred_artifacts(
        &bundle,
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
    let template_run_id = run_id(bundle.directory())?;
    let summary_path = bundle.directory().join("summary.json");
    let rendered_messages = match reporter.render(
        &result,
        TemplateRun {
            id: &template_run_id,
            project_root: &project_root,
            summary_path: &summary_path,
            state_directory: run.activity_store.state().directory(),
        },
    ) {
        Ok(messages) => messages,
        Err(error) => record_reporting_failure(
            &bundle,
            &mut result,
            &candidates,
            &summary_path,
            error.to_string(),
        )?,
    };

    let decision = StopDecision::decide(
        blocking_fingerprint(&result, run.activity_settings.coverage_gap_policy),
        guard.previous_fingerprint().map_err(state_error)?,
        run.continuation,
    );
    let rendered_messages = with_repeat_note(rendered_messages, &decision);
    let lowering = plan_stop_lowering(
        ctx.harness(),
        decision.blocked,
        rendered_messages.user.as_deref(),
        rendered_messages.agent.as_deref(),
        &fallback_block_reason(&summary_path),
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
    let summary = build_batch_summary(BatchSummaryParts {
        run: &bundle,
        project_root: &project_root,
        state_directory: run.activity_store.state().directory(),
        harness: ctx.harness(),
        status,
        rendered_messages,
        lowering: lowering.metadata.clone(),
        decision: decision.clone(),
        source,
        candidates: &candidates,
        tools: summaries,
        disposition: &disposition,
        result,
    })?;
    finish_run(
        run,
        bundle,
        &guard,
        summary,
        lowering,
        disposition,
        decision,
    )
}

/// Commit the summary, produce the native output, then change pending state
/// and record the loop-guard fingerprint, in that order.
fn finish_run(
    run: &TurnCompletionRun<'_, '_>,
    bundle: RunBundle,
    guard: &StopGuardStore,
    summary: BatchRunSummary,
    lowering: StopLoweringPlan,
    disposition: DeferredStateDisposition,
    decision: StopDecision,
) -> hookkit_core::Result<EntityOutcome<TurnCompletionOutput>> {
    let run_id = summary.run.id.clone();
    bundle.commit(&summary).map_err(state_error)?;
    let output = lowering.finish()?;
    apply_deferred_state_disposition(run.activity_store, disposition, run_id.clone())?;
    guard
        .record(&GuardRecord {
            fingerprint: decision.fingerprint,
            run_id,
        })
        .map_err(state_error)?;
    Ok(EntityOutcome::acknowledge(output))
}

fn fallback_block_reason(summary_path: &Path) -> String {
    format!(
        "Deferred formatter/linter checks need attention before stopping; inspect {}.",
        summary_path.display()
    )
}

/// Tell the user why an unchanged repeat no longer blocks.
fn with_repeat_note(mut messages: RenderedMessages, decision: &StopDecision) -> RenderedMessages {
    if decision.suppressed_repeat {
        let note = "hookkit: not blocking Stop again because the same deferred results were already reported and nothing changed; they stay queued for the next Stop.";
        messages.user = Some(match messages.user.take() {
            Some(user) if !user.trim().is_empty() => format!("{user}\n{note}"),
            _ => note.to_owned(),
        });
    }
    messages
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
    settings: &Arc<ExecutionSettings>,
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
            let jobs = workflow_jobs(&base_jobs, workflow);
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
                    settings: Arc::clone(settings),
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

/// Jobs for one workflow: one per file for per-file invocation, otherwise the
/// workspace jobs, split so file-list arguments stay within the argv budget.
fn workflow_jobs(base_jobs: &[ToolJob], workflow: &ToolWorkflow) -> Vec<ToolJob> {
    if workflow.invocation == InvocationGranularity::PerFile {
        return base_jobs
            .iter()
            .flat_map(|job| {
                job.files.iter().cloned().map(|file| ToolJob {
                    workspace_dir: job.workspace_dir.clone(),
                    workspace_indicator: job.workspace_indicator.clone(),
                    files: vec![file],
                })
            })
            .collect();
    }
    if workflow.uses_file_arguments() {
        return split_jobs_for_argument_budget(base_jobs.to_vec(), FILE_ARGUMENT_BUDGET_BYTES);
    }
    base_jobs.to_vec()
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
            unavailable: result.unavailable_tools.contains_key(&tool.spec.id),
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

pub(crate) fn format_deferred_artifact(log: &DeferredLog) -> hookkit_core::Result<String> {
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
    match &log.error {
        Some(CommandError::TimedOut(_)) => return ArtifactClassification::TimedOut,
        Some(CommandError::NotFound | CommandError::Io(_)) => {
            return ArtifactClassification::SpawnError;
        }
        None => {}
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

pub(crate) fn safe_artifact_component(value: &str) -> String {
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
        unavailable_tools: parts.result.unavailable_tools.len(),
        uncovered: parts.result.uncovered_files.len(),
        not_applicable: parts.result.not_applicable_files.len(),
        coverage_gaps: parts.result.coverage_gaps.len(),
        groups: groups.len(),
    };
    let state_disposition = PlannedStateDisposition {
        source: "acknowledge-sealed-window",
        retry_files: parts.disposition.retry_files.iter().cloned().collect(),
        retry_targets: parts.disposition.retry_targets.clone(),
        reported_gaps: parts.disposition.reported_gaps.iter().cloned().collect(),
        handled_baseline_files: parts.disposition.handled_files.iter().cloned().collect(),
    };
    let (source_entry_count, source_entry_ids) = parts.source;
    Ok(BatchRunSummary {
        schema_version: SUMMARY_SCHEMA_VERSION,
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
        stop_decision: parts.decision,
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
            retained: false,
        });
    }
    for (index, message) in resolution.gap_messages.iter().enumerate() {
        result.record_coverage_gap(CoverageGap {
            id: format!("source-gap-{index:03}"),
            target: None,
            message: message.clone(),
            retained: false,
        });
    }
    if resolution.truncated {
        result.record_coverage_gap(CoverageGap {
            id: "resolution-budget-exhausted".into(),
            target: None,
            message: "file activity target resolution exhausted its traversal budget".into(),
            retained: false,
        });
    }
    if !resolution.skipped_targets.is_empty() {
        result.record_coverage_gap(CoverageGap {
            id: "resolution-targets-deferred".into(),
            target: None,
            message: format!(
                "{} file activity target(s) were not attempted after the traversal budget ran out and are retained for the next Stop",
                resolution.skipped_targets.len()
            ),
            retained: true,
        });
    }
}

/// Plan how the sealed window is discharged. Manual-fix and operational files
/// are retried; files of unavailable tools are not (an environment problem
/// cannot be fixed by the agent), gaps and unresolvable targets are reported
/// once, and only never-attempted targets are re-queued.
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

    let mut retry_targets = resolution.skipped_targets.clone();
    retry_targets.sort();
    retry_targets.dedup();
    let reported_gaps = result
        .coverage_gaps
        .values()
        .filter(|gap| !gap.retained)
        .map(|gap| gap.message.clone())
        .collect();
    Ok(DeferredStateDisposition {
        retry_files,
        retry_targets,
        reported_gaps,
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
        .requeue_targets("deferred-unattempted-target", disposition.retry_targets)
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

/// Record one configuration or reporting failure as a durable artifact plus
/// an operational problem affecting every candidate.
fn record_configuration_failure(
    run: &RunBundle,
    result: &mut DeferredRunResult,
    candidates: &[PathBuf],
    id: &str,
    file_name: &str,
    contents: &str,
) -> hookkit_core::Result<()> {
    let artifact_path = run.write_text(file_name, contents).map_err(state_error)?;
    result.record_artifact(RunArtifact {
        id: id.into(),
        absolute_path: artifact_path,
        run_relative_path: file_name.into(),
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
        contents: contents.to_owned(),
    });
    result.record_operational_problem(OperationalProblem {
        id: id.into(),
        tool_id: None,
        phase: Some("configuration".into()),
        affected_files: candidates.to_vec(),
        message: contents.to_owned(),
        artifact_ids: vec![id.into()],
    });
    Ok(())
}

fn record_reporting_failure(
    run: &RunBundle,
    result: &mut DeferredRunResult,
    candidates: &[PathBuf],
    summary_path: &Path,
    contents: String,
) -> hookkit_core::Result<RenderedMessages> {
    record_configuration_failure(
        run,
        result,
        candidates,
        "reporting-configuration",
        "reporting-error.log",
        &contents,
    )?;
    Ok(failure_rendered_messages(summary_path, &contents))
}

struct ConfigFailure<'a> {
    project_root: &'a Path,
    candidates: &'a [PathBuf],
    resolution: &'a ActivityResolution,
    source: (usize, Vec<String>),
    lowering_policy: pkl::LoweringPolicy,
    contents: String,
    /// Whether the agent could plausibly fix the failure (a configuration
    /// error it may have introduced) rather than an environment problem.
    blocks: bool,
}

fn commit_deferred_config_failure(
    run: &TurnCompletionRun<'_, '_>,
    bundle: RunBundle,
    guard: &StopGuardStore,
    failure: ConfigFailure<'_>,
) -> hookkit_core::Result<EntityOutcome<TurnCompletionOutput>> {
    let mut result = DeferredRunResult::default();
    record_configuration_failure(
        &bundle,
        &mut result,
        failure.candidates,
        "configuration",
        "config-error.log",
        &failure.contents,
    )?;
    record_activity_resolution(&mut result, failure.resolution);
    let disposition = plan_deferred_state_disposition(&result, failure.resolution)?;
    let summary_path = bundle.directory().join("summary.json");
    let rendered_messages = failure_rendered_messages(&summary_path, &failure.contents);
    let fingerprint = failure
        .blocks
        .then(|| blocking_fingerprint(&result, run.activity_settings.coverage_gap_policy))
        .flatten();
    let decision = StopDecision::decide(
        fingerprint,
        guard.previous_fingerprint().map_err(state_error)?,
        run.continuation,
    );
    let rendered_messages = with_repeat_note(rendered_messages, &decision);
    let lowering = plan_stop_lowering(
        run.ctx.harness(),
        decision.blocked,
        rendered_messages.user.as_deref(),
        rendered_messages.agent.as_deref(),
        &fallback_block_reason(&summary_path),
        failure.lowering_policy,
    )?;
    let summary = build_batch_summary(BatchSummaryParts {
        run: &bundle,
        project_root: failure.project_root,
        state_directory: run.activity_store.state().directory(),
        harness: run.ctx.harness(),
        status: "operational-failure",
        rendered_messages,
        lowering: lowering.metadata.clone(),
        decision: decision.clone(),
        source: failure.source,
        candidates: failure.candidates,
        tools: Vec::new(),
        disposition: &disposition,
        result,
    })?;
    finish_run(run, bundle, guard, summary, lowering, disposition, decision)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FileAssessment;
    use crate::exec::PhaseLog;

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
        let skipped = FileActivityTarget::exact(Utf8PathBuf::from(
            "/tmp/hookkit-selective-disposition/later.rs",
        ));
        let resolution = ActivityResolution {
            not_applicable_files: BTreeSet::from([deleted.clone()]),
            unresolved_targets: vec![unresolved],
            skipped_targets: vec![skipped.clone()],
            gap_messages: BTreeSet::from(["dynamic shell target".into()]),
            truncated: true,
        };
        record_activity_resolution(&mut result, &resolution);

        let disposition = plan_deferred_state_disposition(&result, &resolution).unwrap();
        let normalized = |path: &PathBuf| utf8_activity_path(path).unwrap();
        assert_eq!(
            disposition.retry_files,
            BTreeSet::from([normalized(&manual), normalized(&operational)])
        );
        assert_eq!(
            disposition.retry_targets,
            vec![skipped],
            "only never-attempted targets are retained"
        );
        assert!(
            disposition.reported_gaps.contains("dynamic shell target"),
            "message-only gaps are reported once, not re-queued"
        );
        assert_eq!(
            disposition.handled_files,
            BTreeSet::from([
                normalized(&auto_fixed),
                normalized(&clean),
                normalized(&deleted),
            ])
        );
    }

    #[test]
    fn unavailable_tools_are_neither_retried_nor_blocking() {
        let mut result = DeferredRunResult::default();
        result.record_unavailable_tool(crate::UnavailableTool {
            tool_id: "prettier".into(),
            tool_name: "Prettier".into(),
            executable: "prettier".into(),
            install_hint: None,
            affected_files: vec![PathBuf::from("/repo/a.ts")],
            message: "Prettier: `prettier` is unavailable".into(),
        });
        let resolution = ActivityResolution {
            not_applicable_files: BTreeSet::new(),
            unresolved_targets: Vec::new(),
            skipped_targets: Vec::new(),
            gap_messages: BTreeSet::new(),
            truncated: false,
        };
        let disposition = plan_deferred_state_disposition(&result, &resolution).unwrap();
        assert!(disposition.retry_files.is_empty());
        assert!(
            blocking_fingerprint(&result, pkl::CoverageGapPolicy::Strict).is_none(),
            "a missing tool must not block Stop"
        );
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

    #[test]
    fn timed_out_commands_get_their_own_artifact_classification() {
        let log = PhaseLog {
            phase: "check".into(),
            command: "slow".into(),
            program: "slow".into(),
            arguments: Vec::new(),
            status: None,
            classification: None,
            stdout: String::new(),
            stderr: String::new(),
            error: Some(CommandError::TimedOut(Duration::from_secs(3))),
        };
        assert_eq!(
            artifact_classification(&log),
            ArtifactClassification::TimedOut
        );
    }
}
