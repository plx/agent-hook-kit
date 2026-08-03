use super::{CheckOutcome, DeferredRunResult, FileStatus, OperationalProblem, ToolReport};
use crate::{
    CheckScope, CommandPhase, InvocationGranularity, PhaseLog, PhaseStatus, Snapshot, ToolContext,
    ToolJob, ToolPhase, ToolSpec, WriteBehavior, collect_matching_files, collect_workspace_files,
    render_command, resolve_worker_count, run_phase_command,
};
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
        }
    }
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
}

#[derive(Debug)]
struct WriteImpact {
    workspace: PathBuf,
    changed_files: BTreeSet<PathBuf>,
}

/// Execute one global Stop-time plan: all checks first, at most one remedy per
/// dirty workflow, then authoritative reruns for every invalidated check.
pub(crate) fn execute_deferred_workflows(
    plan: &[ScheduledWorkflow],
    jobs_setting: u32,
    fail_fast: bool,
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
    let mut remedies_stopped = false;
    for (index, log) in run_checks(plan, &initial_indices, jobs_setting) {
        let scheduled = &plan[index];
        execution
            .logs
            .push(deferred_log(scheduled, CommandPhase::InitialCheck, log));
        let log = &execution.logs.last().expect("just pushed").log;
        match check_outcome(log) {
            Ok(outcome) => states[index].initial_check = Some(outcome),
            Err(message) => {
                states[index].operational = true;
                record_problem(&mut execution.result, scheduled, "initial-check", message);
                if fail_fast {
                    remedies_stopped = true;
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
        if !needs_remedy || states[index].operational {
            continue;
        }
        let Some(remedy) = scheduled.remedy.as_ref() else {
            continue;
        };
        if remedies_stopped {
            states[index].operational = true;
            record_problem(
                &mut execution.result,
                scheduled,
                "remedy",
                "remedy skipped after an earlier operational failure under failFast",
            );
            continue;
        }

        states[index].fix_attempted = true;
        let context = scheduled.context();
        let scope = command_write_scope(remedy.writes, &scheduled.job, &context);
        let before = Snapshot::read(&scope);
        let command = render_command(remedy, &scheduled.job, &context);
        let log = run_phase_command(remedy, &command, &scheduled.job.workspace_dir);
        let after_scope = command_write_scope(remedy.writes, &scheduled.job, &context);
        let after = Snapshot::read(&after_scope);
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
        let failed = command_failed(&log);
        execution
            .logs
            .push(deferred_log(scheduled, CommandPhase::Remedy, log));
        if let Some(message) = failed {
            states[index].operational = true;
            record_problem(&mut execution.result, scheduled, "remedy", message);
            if fail_fast {
                remedies_stopped = true;
            }
        }
    }

    let final_indices = plan
        .iter()
        .enumerate()
        .filter_map(|(index, scheduled)| {
            scheduled.check.as_ref()?;
            let invalidated = impacts
                .iter()
                .any(|impact| check_invalidated(scheduled, impact));
            (states[index].fix_attempted || invalidated).then_some(index)
        })
        .collect::<Vec<_>>();
    let rerun = final_indices.iter().copied().collect::<BTreeSet<_>>();
    for (index, log) in run_checks(plan, &final_indices, jobs_setting) {
        let scheduled = &plan[index];
        execution
            .logs
            .push(deferred_log(scheduled, CommandPhase::FinalCheck, log));
        let log = &execution.logs.last().expect("just pushed").log;
        match check_outcome(log) {
            Ok(outcome) => states[index].final_check = Some(outcome),
            Err(message) => {
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

        if states[index].operational {
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
    run_phase_command(check, &command, &scheduled.job.workspace_dir)
}

fn check_outcome(log: &PhaseLog) -> Result<CheckOutcome, String> {
    if let Some(message) = command_failed(log) {
        return Err(message);
    }
    match log.classification {
        Some(PhaseStatus::Clean) => Ok(CheckOutcome::Clean),
        Some(PhaseStatus::Issues) => Ok(CheckOutcome::Issues),
        Some(PhaseStatus::Failure) | None => Err("check produced no usable result".into()),
    }
}

fn command_failed(log: &PhaseLog) -> Option<String> {
    if let Some(error) = &log.error {
        return Some(format!("{}: {error}", log.phase));
    }
    match log.classification {
        Some(PhaseStatus::Failure) | None => Some(format!(
            "{} failed with exit code {:?}",
            log.phase, log.status
        )),
        Some(PhaseStatus::Clean | PhaseStatus::Issues) => None,
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
        WriteBehavior::MatchingGlobs => {
            collect_matching_files(&job.workspace_dir, &context.spec.file_selection)
        }
        WriteBehavior::Workspace => collect_workspace_files(&job.workspace_dir),
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

fn deferred_log(scheduled: &ScheduledWorkflow, phase: CommandPhase, log: PhaseLog) -> DeferredLog {
    DeferredLog {
        tool_index: scheduled.tool_index,
        workflow_index: scheduled.workflow_index,
        job_index: scheduled.job_index,
        phase,
        log,
    }
}
