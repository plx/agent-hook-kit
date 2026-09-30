use super::{
    CheckOutcome, DeferredRunResult, FileStatus, OperationalProblem, ToolReport, UnavailableTool,
};
use crate::CommandPhase;
use crate::exec::{
    CommandError, ExecutionSettings, PhaseLog, PhaseStatus, ToolContext, ToolJob, render_command,
    resolve_worker_count, run_phase_command,
};
use crate::snapshot::{Snapshot, collect_matching_files, collect_workspace_files};
use crate::spec::{CheckScope, InvocationGranularity, ToolPhase, ToolSpec, WriteBehavior};
use hookkit_pkl_config::schema::MissingToolPolicy;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, Clone)]
pub(crate) struct ScheduledWorkflow {
    pub tool_index: usize,
    pub workflow_index: usize,
    pub job_index: usize,
    pub spec: Arc<ToolSpec>,
    pub workflow_id: String,
    pub check: Option<ToolPhase>,
    pub remedy: Option<ToolPhase>,
    pub check_scope: CheckScope,
    pub invocation: InvocationGranularity,
    pub compatibility_translation: bool,
    pub job: ToolJob,
    pub project_root: PathBuf,
    pub settings: Arc<ExecutionSettings>,
}

impl ScheduledWorkflow {
    pub(crate) fn report_id(&self) -> String {
        format!(
            "{:03}-{}-{:03}-{:03}",
            self.tool_index, self.spec.id, self.workflow_index, self.job_index
        )
    }

    fn context(&self) -> ToolContext<'_> {
        ToolContext {
            spec: &self.spec,
            project_root: &self.project_root,
            global_diagnostics_dir: None,
            settings: &self.settings,
        }
    }
}

/// Runner settings that shape one deferred execution.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WorkflowExecutionPolicy {
    /// Bounded parallelism for read-only checks (`settings.jobs`).
    pub jobs: u32,
    /// Skip a tool's remaining remedies after one of its commands failed.
    pub fail_fast: bool,
    /// Handling for executables that cannot be found.
    pub missing_tool_policy: MissingToolPolicy,
}

#[derive(Debug)]
pub(crate) struct DeferredLog {
    pub tool_index: usize,
    pub workflow_index: usize,
    pub job_index: usize,
    pub phase: CommandPhase,
    pub log: PhaseLog,
}

#[derive(Debug, Default)]
pub(crate) struct DeferredExecution {
    pub result: DeferredRunResult,
    pub logs: Vec<DeferredLog>,
}

#[derive(Debug, Default)]
struct WorkflowState {
    initial_check: Option<CheckOutcome>,
    fix_attempted: bool,
    final_check: Option<CheckOutcome>,
    changed_files: BTreeSet<PathBuf>,
    operational: bool,
    unavailable: bool,
}

#[derive(Debug)]
struct WriteImpact {
    workspace: PathBuf,
    changed_files: BTreeSet<PathBuf>,
}

/// Classified result of one check or remedy command.
enum CommandResult {
    Outcome(CheckOutcome),
    /// The executable is missing and `missingToolPolicy = "user-notice"`.
    Unavailable,
    Failed(String),
}

/// Execute one global Stop-time plan: all checks first, at most one remedy per
/// dirty workflow, then authoritative reruns for every invalidated check.
///
/// `failFast` is scoped to the failing tool: an operational failure skips that
/// tool's remaining remedies, while unrelated tools still repair their files.
/// Under `missingToolPolicy = "user-notice"` a missing executable is recorded
/// as an unavailable tool rather than an operational failure.
pub(crate) fn execute_deferred_workflows(
    plan: &[ScheduledWorkflow],
    policy: WorkflowExecutionPolicy,
) -> DeferredExecution {
    let mut execution = DeferredExecution::default();
    let mut states = (0..plan.len())
        .map(|_| WorkflowState::default())
        .collect::<Vec<_>>();

    let initial_indices = plan
        .iter()
        .enumerate()
        .filter_map(|(index, scheduled)| scheduled.check.as_ref().map(|_| index))
        .collect::<Vec<_>>();
    let mut remedies_stopped = BTreeSet::<usize>::new();
    for (index, log) in run_checks(plan, &initial_indices, policy.jobs) {
        let scheduled = &plan[index];
        let result = classify(&log, policy.missing_tool_policy);
        execution
            .logs
            .push(deferred_log(scheduled, CommandPhase::InitialCheck, log));
        match result {
            CommandResult::Outcome(outcome) => states[index].initial_check = Some(outcome),
            CommandResult::Unavailable => {
                states[index].unavailable = true;
                record_unavailable(&mut execution.result, scheduled, &execution.logs);
            }
            CommandResult::Failed(message) => {
                states[index].operational = true;
                record_problem(&mut execution.result, scheduled, "initial-check", message);
                if policy.fail_fast {
                    remedies_stopped.insert(scheduled.tool_index);
                }
            }
        }
    }

    let mut impacts = Vec::new();
    for (index, scheduled) in plan.iter().enumerate() {
        let needs_remedy = states[index].initial_check == Some(CheckOutcome::Issues)
            || (scheduled.check.is_none()
                && scheduled.compatibility_translation
                && scheduled.remedy.is_some());
        if !needs_remedy || states[index].operational || states[index].unavailable {
            continue;
        }
        let Some(remedy) = scheduled.remedy.as_ref() else {
            continue;
        };
        if remedies_stopped.contains(&scheduled.tool_index) {
            states[index].operational = true;
            record_problem(
                &mut execution.result,
                scheduled,
                "remedy",
                "remedy skipped after an earlier operational failure of this tool under failFast",
            );
            continue;
        }

        states[index].fix_attempted = true;
        let context = scheduled.context();
        let before = Snapshot::capture(&command_write_scope(
            remedy.writes,
            &scheduled.job,
            &context,
        ));
        let command = render_command(remedy, &scheduled.job, &context);
        let log = run_phase_command(
            remedy,
            &command,
            &scheduled.job.workspace_dir,
            &scheduled.settings,
        );
        let after = before.recapture(&command_write_scope(
            remedy.writes,
            &scheduled.job,
            &context,
        ));
        let changed_files = before
            .changed_files(&after)
            .into_iter()
            .collect::<BTreeSet<_>>();
        states[index]
            .changed_files
            .extend(changed_files.iter().cloned());
        if !changed_files.is_empty() {
            impacts.push(WriteImpact {
                workspace: scheduled.job.workspace_dir.clone(),
                changed_files,
            });
        }
        let result = classify(&log, policy.missing_tool_policy);
        execution
            .logs
            .push(deferred_log(scheduled, CommandPhase::Remedy, log));
        match result {
            CommandResult::Outcome(_) => {}
            CommandResult::Unavailable => {
                // A remedy-only program override is missing: the files still
                // get their authoritative final check below.
                record_unavailable(&mut execution.result, scheduled, &execution.logs);
            }
            CommandResult::Failed(message) => {
                states[index].operational = true;
                record_problem(&mut execution.result, scheduled, "remedy", message);
                if policy.fail_fast {
                    remedies_stopped.insert(scheduled.tool_index);
                }
            }
        }
    }

    let final_indices = plan
        .iter()
        .enumerate()
        .filter_map(|(index, scheduled)| {
            scheduled.check.as_ref()?;
            if states[index].unavailable {
                return None;
            }
            let invalidated = impacts
                .iter()
                .any(|impact| check_invalidated(scheduled, impact));
            (states[index].fix_attempted || invalidated).then_some(index)
        })
        .collect::<Vec<_>>();
    let rerun = final_indices.iter().copied().collect::<BTreeSet<_>>();
    for (index, log) in run_checks(plan, &final_indices, policy.jobs) {
        let scheduled = &plan[index];
        let result = classify(&log, policy.missing_tool_policy);
        execution
            .logs
            .push(deferred_log(scheduled, CommandPhase::FinalCheck, log));
        match result {
            CommandResult::Outcome(outcome) => states[index].final_check = Some(outcome),
            CommandResult::Unavailable => {
                states[index].unavailable = true;
                record_unavailable(&mut execution.result, scheduled, &execution.logs);
            }
            CommandResult::Failed(message) => {
                states[index].operational = true;
                record_problem(&mut execution.result, scheduled, "final-check", message);
            }
        }
    }

    for (index, scheduled) in plan.iter().enumerate() {
        if !rerun.contains(&index) {
            states[index].final_check = states[index].initial_check;
        }
        if scheduled.check.is_none() {
            states[index].operational = true;
            record_problem(
                &mut execution.result,
                scheduled,
                "final-check",
                "legacy mutating-only workflow has no authoritative non-mutating check",
            );
        }

        let mut report = ToolReport {
            id: scheduled.report_id(),
            tool_id: scheduled.spec.id.clone(),
            tool_name: scheduled.spec.display_name.clone(),
            workflow_id: scheduled.workflow_id.clone(),
            job_id: format!("{:03}", scheduled.job_index),
            candidate_files: scheduled.job.files.clone(),
            changed_files: states[index].changed_files.iter().cloned().collect(),
            initial_check: states[index].initial_check,
            fix_attempted: states[index].fix_attempted,
            final_check: states[index].final_check,
            conservative_attribution: scheduled.invocation != InvocationGranularity::PerFile
                && scheduled.job.files.len() > 1,
            artifact_ids: Vec::new(),
        };
        report.normalize();

        if states[index].operational || states[index].unavailable {
            execution.result.reports.insert(report.id.clone(), report);
            continue;
        }
        let Some(final_check) = states[index].final_check else {
            record_problem(
                &mut execution.result,
                scheduled,
                "final-check",
                "workflow completed without an authoritative final check",
            );
            execution.result.reports.insert(report.id.clone(), report);
            continue;
        };
        let status = match final_check {
            CheckOutcome::Issues => FileStatus::ManualFixesNeeded,
            CheckOutcome::Clean if states[index].fix_attempted => FileStatus::AutoFixed,
            CheckOutcome::Clean => FileStatus::Clean,
        };
        execution.result.record_conservative_report(report, status);
    }

    execution
        .logs
        .sort_by_key(|log| (log.tool_index, log.workflow_index, log.job_index, log.phase));
    execution
}

fn run_checks(
    plan: &[ScheduledWorkflow],
    indices: &[usize],
    jobs_setting: u32,
) -> Vec<(usize, PhaseLog)> {
    let worker_count = resolve_worker_count(jobs_setting, indices.len());
    if worker_count <= 1 {
        return indices
            .iter()
            .map(|index| (*index, run_check(&plan[*index])))
            .collect();
    }
    let cursor = AtomicUsize::new(0);
    let results = Mutex::new(Vec::with_capacity(indices.len()));
    std::thread::scope(|scope| {
        for _ in 0..worker_count {
            scope.spawn(|| {
                loop {
                    let offset = cursor.fetch_add(1, Ordering::Relaxed);
                    let Some(index) = indices.get(offset).copied() else {
                        break;
                    };
                    results
                        .lock()
                        .expect("deferred check mutex poisoned")
                        .push((index, run_check(&plan[index])));
                }
            });
        }
    });
    let mut results = results.into_inner().expect("deferred check mutex poisoned");
    results.sort_by_key(|(index, _)| *index);
    results
}

fn run_check(scheduled: &ScheduledWorkflow) -> PhaseLog {
    let check = scheduled.check.as_ref().expect("scheduled check");
    let context = scheduled.context();
    let command = render_command(check, &scheduled.job, &context);
    run_phase_command(
        check,
        &command,
        &scheduled.job.workspace_dir,
        &scheduled.settings,
    )
}

/// Classify one check or remedy log.
fn classify(log: &PhaseLog, missing_tool_policy: MissingToolPolicy) -> CommandResult {
    if log.error == Some(CommandError::NotFound)
        && missing_tool_policy == MissingToolPolicy::UserNotice
    {
        return CommandResult::Unavailable;
    }
    if let Some(error) = &log.error {
        return CommandResult::Failed(format!("{}: {error}", log.phase));
    }
    match log.classification {
        Some(PhaseStatus::Clean) => CommandResult::Outcome(CheckOutcome::Clean),
        Some(PhaseStatus::Issues) => CommandResult::Outcome(CheckOutcome::Issues),
        Some(PhaseStatus::Failure) | None => CommandResult::Failed(format!(
            "{} failed with exit code {:?}",
            log.phase, log.status
        )),
    }
}

fn command_write_scope(
    writes: WriteBehavior,
    job: &ToolJob,
    context: &ToolContext<'_>,
) -> BTreeSet<PathBuf> {
    match writes {
        WriteBehavior::None => BTreeSet::new(),
        WriteBehavior::TargetFiles => job.files.iter().cloned().collect(),
        WriteBehavior::MatchingGlobs => collect_matching_files(job, context),
        WriteBehavior::Workspace => collect_workspace_files(job, context),
    }
}

fn check_invalidated(scheduled: &ScheduledWorkflow, impact: &WriteImpact) -> bool {
    match scheduled.check_scope {
        CheckScope::TargetFiles => impact
            .changed_files
            .iter()
            .any(|path| scheduled.job.files.contains(path)),
        CheckScope::Workspace => {
            impact.workspace == scheduled.job.workspace_dir
                || impact
                    .changed_files
                    .iter()
                    .any(|path| path.starts_with(&scheduled.job.workspace_dir))
        }
    }
}

fn record_problem(
    result: &mut DeferredRunResult,
    scheduled: &ScheduledWorkflow,
    phase: &str,
    message: impl Into<String>,
) {
    result.record_operational_problem(OperationalProblem {
        id: format!("{}-{phase}", scheduled.report_id()),
        tool_id: Some(scheduled.spec.id.clone()),
        phase: Some(phase.into()),
        affected_files: scheduled.job.files.clone(),
        message: message.into(),
        artifact_ids: Vec::new(),
    });
}

fn record_unavailable(
    result: &mut DeferredRunResult,
    scheduled: &ScheduledWorkflow,
    logs: &[DeferredLog],
) {
    let executable = logs
        .last()
        .map(|log| log.log.program.clone())
        .unwrap_or_else(|| scheduled.spec.executable.clone());
    let mut message = format!(
        "{}: `{executable}` is unavailable; its files were not checked",
        scheduled.spec.display_name
    );
    if let Some(hint) = &scheduled.spec.install_hint {
        message.push_str(&format!(" ({hint})"));
    }
    result.record_unavailable_tool(UnavailableTool {
        tool_id: scheduled.spec.id.clone(),
        tool_name: scheduled.spec.display_name.clone(),
        executable,
        install_hint: scheduled.spec.install_hint.clone(),
        affected_files: scheduled.job.files.clone(),
        message,
    });
}

fn deferred_log(scheduled: &ScheduledWorkflow, phase: CommandPhase, log: PhaseLog) -> DeferredLog {
    DeferredLog {
        tool_index: scheduled.tool_index,
        workflow_index: scheduled.workflow_index,
        job_index: scheduled.job_index,
        phase,
        log,
    }
}
