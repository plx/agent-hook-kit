use super::FileStatus;
use super::execution::{DeferredExecution, ScheduledWorkflow, execute_deferred_workflows};
use crate::{
    CheckScope, CommandArgTemplate, ExitCodePolicy, InvocationGranularity, PhaseMode, ToolJob,
    ToolPhase, ToolSpec, UnexpectedExitPolicy, WriteBehavior,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    root: PathBuf,
    executable: PathBuf,
    trace: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;

        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "hookkit-deferred-{name}-{}-{sequence}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fixture root");
        let executable = root.join("fake-workflow");
        std::fs::write(
            &executable,
            r#"#!/bin/sh
case "$1" in
  -P)
    sed 's/DIRTY/CLEAN/g' "$2"
    exit $?
    ;;
  -iP)
    shift
    for file in "$@"; do
      sed 's/DIRTY/CLEAN/g' "$file" > "$file.tmp" && mv "$file.tmp" "$file"
    done
    exit $?
    ;;
esac
trace=$1
action=$2
shift 2
printf '%s\n' "$action" >> "$trace"
case "$action" in
  check)
    for file in "$@"; do
      if grep -Eq 'DIRTY|MANUAL' "$file"; then exit 1; fi
    done
    ;;
  stdout-check)
    for file in "$@"; do
      if grep -q 'DIRTY' "$file"; then printf '%s\n' "$file"; fi
    done
    ;;
  check-a)
    for file in "$@"; do
      if grep -q 'BAD_A' "$file"; then exit 1; fi
    done
    ;;
  check-b)
    for file in "$@"; do
      if grep -q 'BAD_B' "$file"; then exit 1; fi
    done
    ;;
  check-workspace)
    if grep -R -q 'BAD_A' "$1"; then exit 1; fi
    ;;
  fix)
    for file in "$@"; do
      sed 's/DIRTY/CLEAN/g' "$file" > "$file.tmp" && mv "$file.tmp" "$file"
    done
    ;;
  partial)
    for file in "$@"; do
      sed 's/DIRTY/MANUAL/g' "$file" > "$file.tmp" && mv "$file.tmp" "$file"
    done
    ;;
  nochange)
    ;;
  fix-b)
    for file in "$@"; do
      sed 's/BAD_B/BAD_A/g' "$file" > "$file.tmp" && mv "$file.tmp" "$file"
    done
    ;;
  failfix)
    for file in "$@"; do
      sed 's/DIRTY/CLEAN/g' "$file" > "$file.tmp" && mv "$file.tmp" "$file"
    done
    exit 2
    ;;
  crash)
    exit 2
    ;;
esac
exit 0
"#,
        )
        .expect("write fake workflow");
        let mut permissions = std::fs::metadata(&executable)
            .expect("fake metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions).expect("make fake executable");
        Self {
            trace: root.join("trace.log"),
            root,
            executable,
        }
    }

    fn file(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.root.join(name);
        std::fs::write(&path, contents).expect("write candidate");
        path
    }

    fn trace_lines(&self) -> Vec<String> {
        std::fs::read_to_string(&self.trace)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn command(
    fixture: &Fixture,
    id: &str,
    action: &str,
    writes: WriteBehavior,
    workspace_arg: bool,
) -> ToolPhase {
    let mut args = vec![
        CommandArgTemplate::literal(fixture.trace.to_string_lossy()),
        CommandArgTemplate::literal(action),
    ];
    args.push(if workspace_arg {
        CommandArgTemplate::Workspace
    } else {
        CommandArgTemplate::Files
    });
    ToolPhase {
        id: id.into(),
        mode: if writes == WriteBehavior::None {
            PhaseMode::Verify
        } else {
            PhaseMode::Fix
        },
        program: Some(fixture.executable.to_string_lossy().into_owned()),
        args,
        exit_codes: ExitCodePolicy {
            clean: vec![0],
            issues: vec![1],
            failure: vec![2],
            unexpected: UnexpectedExitPolicy::Failure,
        },
        issues_on_stdout: false,
        writes,
        extra_args: Vec::new(),
        enabled: true,
    }
}

fn scheduled(
    fixture: &Fixture,
    tool_index: usize,
    file: PathBuf,
    check_action: &str,
    remedy_action: Option<&str>,
) -> ScheduledWorkflow {
    scheduled_with_scope(
        fixture,
        tool_index,
        vec![file],
        check_action,
        remedy_action,
        CheckScope::TargetFiles,
        false,
    )
}

fn scheduled_with_scope(
    fixture: &Fixture,
    tool_index: usize,
    files: Vec<PathBuf>,
    check_action: &str,
    remedy_action: Option<&str>,
    check_scope: CheckScope,
    workspace_arg: bool,
) -> ScheduledWorkflow {
    let spec = Arc::new(ToolSpec::new(
        format!("tool-{tool_index}"),
        format!("Tool {tool_index}"),
        fixture.executable.to_string_lossy(),
    ));
    ScheduledWorkflow {
        tool_index,
        workflow_index: 0,
        job_index: 0,
        spec,
        workflow_id: "workflow".into(),
        check: Some(command(
            fixture,
            "check",
            check_action,
            WriteBehavior::None,
            workspace_arg,
        )),
        remedy: remedy_action
            .map(|action| command(fixture, "remedy", action, WriteBehavior::TargetFiles, false)),
        check_scope,
        invocation: InvocationGranularity::Batch,
        compatibility_translation: false,
        job: ToolJob {
            workspace_dir: fixture.root.clone(),
            workspace_indicator: None,
            files,
        },
        project_root: fixture.root.clone(),
    }
}

fn only_status(execution: &DeferredExecution, file: &PathBuf) -> Option<FileStatus> {
    execution.result.files.get(file).map(|result| result.status)
}

#[test]
fn initially_clean_skips_remedy() {
    let fixture = Fixture::new("clean");
    let file = fixture.file("clean.rs", "CLEAN\n");
    let plan = vec![scheduled(&fixture, 0, file.clone(), "check", Some("fix"))];
    let execution = execute_deferred_workflows(&plan, 1, true);
    assert_eq!(only_status(&execution, &file), Some(FileStatus::Clean));
    assert_eq!(fixture.trace_lines(), vec!["check"]);
}

#[test]
fn dirty_fixable_and_partly_fixable_use_one_remedy_then_final_check() {
    let fixture = Fixture::new("fixes");
    let fixed = fixture.file("fixed.rs", "DIRTY\n");
    let partial = fixture.file("partial.rs", "DIRTY\n");
    let plan = vec![
        scheduled(&fixture, 0, fixed.clone(), "check", Some("fix")),
        scheduled(&fixture, 1, partial.clone(), "check", Some("partial")),
    ];
    let execution = execute_deferred_workflows(&plan, 2, true);
    assert_eq!(only_status(&execution, &fixed), Some(FileStatus::AutoFixed));
    assert_eq!(
        only_status(&execution, &partial),
        Some(FileStatus::ManualFixesNeeded)
    );
    let trace = fixture.trace_lines();
    assert_eq!(trace.iter().filter(|line| *line == "fix").count(), 1);
    assert_eq!(trace.iter().filter(|line| *line == "partial").count(), 1);
}

#[test]
fn stdout_only_check_can_trigger_remedy_and_final_verification() {
    let fixture = Fixture::new("stdout-check");
    let file = fixture.file("dirty.go", "DIRTY\n");
    let mut workflow = scheduled(&fixture, 0, file.clone(), "stdout-check", Some("fix"));
    workflow.check.as_mut().expect("check").issues_on_stdout = true;

    let execution = execute_deferred_workflows(&[workflow], 1, true);

    assert_eq!(only_status(&execution, &file), Some(FileStatus::AutoFixed));
    assert_eq!(
        fixture.trace_lines(),
        vec!["stdout-check", "fix", "stdout-check"]
    );
}

#[test]
fn shell_comparator_uses_configured_tool_and_preserves_source_during_check() {
    let fixture = Fixture::new("shell-comparator");
    let file = fixture.file("dirty.yaml", "value: DIRTY\n");
    let spec = Arc::new(ToolSpec::new(
        "yq",
        "yq",
        fixture.executable.to_string_lossy(),
    ));
    let check = ToolPhase {
        id: "format.check".into(),
        mode: PhaseMode::Verify,
        program: Some("sh".into()),
        args: vec![
            CommandArgTemplate::literal("-c"),
            CommandArgTemplate::literal(
                "tool=$1; file=$2; shift 3; tmp=$(mktemp \"${TMPDIR:-/tmp}/hookkit-yq-test.XXXXXX\") || exit 2; trap 'rm -f \"$tmp\"' 0 HUP INT TERM; \"$tool\" -P \"$@\" \"$file\" >\"$tmp\" || exit 2; diff -u \"$file\" \"$tmp\"",
            ),
            CommandArgTemplate::literal("yq-check"),
            CommandArgTemplate::ToolExecutable,
            CommandArgTemplate::Files,
            CommandArgTemplate::literal("--"),
            CommandArgTemplate::ExtraArgs,
        ],
        exit_codes: ExitCodePolicy {
            clean: vec![0],
            issues: vec![1],
            failure: vec![2],
            unexpected: UnexpectedExitPolicy::Failure,
        },
        issues_on_stdout: false,
        writes: WriteBehavior::None,
        extra_args: Vec::new(),
        enabled: true,
    };
    let remedy = ToolPhase {
        id: "format.remedy".into(),
        mode: PhaseMode::Fix,
        program: None,
        args: vec![
            CommandArgTemplate::literal("-iP"),
            CommandArgTemplate::ExtraArgs,
            CommandArgTemplate::Files,
        ],
        exit_codes: ExitCodePolicy::default(),
        issues_on_stdout: false,
        writes: WriteBehavior::TargetFiles,
        extra_args: Vec::new(),
        enabled: true,
    };
    let plan = [ScheduledWorkflow {
        tool_index: 0,
        workflow_index: 0,
        job_index: 0,
        spec,
        workflow_id: "format".into(),
        check: Some(check),
        remedy: Some(remedy),
        check_scope: CheckScope::TargetFiles,
        invocation: InvocationGranularity::PerFile,
        compatibility_translation: false,
        job: ToolJob {
            workspace_dir: fixture.root.clone(),
            workspace_indicator: None,
            files: vec![file.clone()],
        },
        project_root: fixture.root.clone(),
    }];

    let execution = execute_deferred_workflows(&plan, 1, true);

    assert_eq!(only_status(&execution, &file), Some(FileStatus::AutoFixed));
    assert_eq!(
        std::fs::read_to_string(file).expect("fixed yaml"),
        "value: CLEAN\n"
    );
}

#[test]
fn dirty_without_remedy_and_noop_remedy_are_manual() {
    let fixture = Fixture::new("manual");
    let no_remedy = fixture.file("no-remedy.rs", "DIRTY\n");
    let no_change = fixture.file("no-change.rs", "DIRTY\n");
    let plan = vec![
        scheduled(&fixture, 0, no_remedy.clone(), "check", None),
        scheduled(&fixture, 1, no_change.clone(), "check", Some("nochange")),
    ];
    let execution = execute_deferred_workflows(&plan, 1, true);
    assert_eq!(
        only_status(&execution, &no_remedy),
        Some(FileStatus::ManualFixesNeeded)
    );
    assert_eq!(
        only_status(&execution, &no_change),
        Some(FileStatus::ManualFixesNeeded)
    );
    let noop_report = execution
        .result
        .reports
        .values()
        .find(|report| report.tool_id == "tool-1")
        .expect("noop report");
    assert!(noop_report.fix_attempted);
    assert!(noop_report.changed_files.is_empty());
}

#[test]
fn operational_initial_check_never_runs_remedy() {
    let fixture = Fixture::new("initial-failure");
    let file = fixture.file("file.rs", "DIRTY\n");
    let plan = vec![scheduled(&fixture, 0, file, "crash", Some("fix"))];
    let execution = execute_deferred_workflows(&plan, 1, true);
    assert!(execution.result.files.is_empty());
    assert!(execution.result.has_operational_problems());
    assert_eq!(fixture.trace_lines(), vec!["crash"]);
}

#[test]
fn operational_initial_check_stops_later_remedies_under_fail_fast() {
    let fixture = Fixture::new("initial-failure-stops-remedies");
    let earlier = fixture.file("earlier.rs", "DIRTY\n");
    let failed = fixture.file("failed.rs", "DIRTY\n");
    let later = fixture.file("later.rs", "DIRTY\n");
    let plan = vec![
        scheduled(&fixture, 0, earlier.clone(), "check", Some("fix")),
        scheduled(&fixture, 1, failed, "crash", Some("fix")),
        scheduled(&fixture, 2, later.clone(), "check", Some("fix")),
    ];

    let execution = execute_deferred_workflows(&plan, 1, true);

    assert!(execution.result.has_operational_problems());
    assert_eq!(
        std::fs::read_to_string(earlier).expect("read skipped earlier candidate"),
        "DIRTY\n"
    );
    assert_eq!(
        std::fs::read_to_string(later).expect("read skipped candidate"),
        "DIRTY\n"
    );
    assert_eq!(fixture.trace_lines(), vec!["check", "crash", "check"]);
}

#[test]
fn operational_initial_check_allows_later_remedies_without_fail_fast() {
    let fixture = Fixture::new("initial-failure-continues-remedies");
    let earlier = fixture.file("earlier.rs", "DIRTY\n");
    let failed = fixture.file("failed.rs", "DIRTY\n");
    let later = fixture.file("later.rs", "DIRTY\n");
    let plan = vec![
        scheduled(&fixture, 0, earlier.clone(), "check", Some("fix")),
        scheduled(&fixture, 1, failed, "crash", Some("fix")),
        scheduled(&fixture, 2, later.clone(), "check", Some("fix")),
    ];

    let execution = execute_deferred_workflows(&plan, 1, false);

    assert!(execution.result.has_operational_problems());
    assert_eq!(
        only_status(&execution, &earlier),
        Some(FileStatus::AutoFixed)
    );
    assert_eq!(only_status(&execution, &later), Some(FileStatus::AutoFixed));
    assert_eq!(
        std::fs::read_to_string(earlier).expect("read earlier fixed candidate"),
        "CLEAN\n"
    );
    assert_eq!(
        std::fs::read_to_string(later).expect("read fixed candidate"),
        "CLEAN\n"
    );
    assert_eq!(
        fixture.trace_lines(),
        vec!["check", "crash", "check", "fix", "fix", "check", "check"]
    );
}

#[test]
fn failed_remedy_keeps_changed_files_and_operational_problem() {
    let fixture = Fixture::new("remedy-failure");
    let file = fixture.file("file.rs", "DIRTY\n");
    let plan = vec![scheduled(
        &fixture,
        0,
        file.clone(),
        "check",
        Some("failfix"),
    )];
    let execution = execute_deferred_workflows(&plan, 1, true);
    assert!(execution.result.has_operational_problems());
    let report = execution.result.reports.values().next().expect("report");
    assert_eq!(report.changed_files, vec![file]);
    assert!(report.fix_attempted);
}

#[test]
fn legacy_mutating_only_workflow_is_operationally_unverifiable() {
    let fixture = Fixture::new("missing-final-check");
    let file = fixture.file("file.rs", "DIRTY\n");
    let mut scheduled = scheduled(&fixture, 0, file.clone(), "check", Some("fix"));
    scheduled.check = None;
    scheduled.compatibility_translation = true;
    let execution = execute_deferred_workflows(&[scheduled], 1, true);
    assert!(execution.result.files.is_empty());
    assert!(execution.result.has_operational_problems());
    let report = execution.result.reports.values().next().expect("report");
    assert!(report.fix_attempted);
    assert_eq!(report.changed_files, vec![file]);
    assert!(report.final_check.is_none());
}

#[test]
fn later_write_invalidates_prior_check_but_unrelated_write_does_not() {
    let fixture = Fixture::new("invalidation");
    let shared = fixture.file("shared.rs", "BAD_B\n");
    let unrelated = fixture.file("unrelated.rs", "BAD_B\n");
    let plan = vec![
        scheduled(&fixture, 0, shared.clone(), "check-a", None),
        scheduled(&fixture, 1, shared.clone(), "check-b", Some("fix-b")),
        scheduled(&fixture, 2, unrelated, "check-b", Some("fix-b")),
    ];
    let execution = execute_deferred_workflows(&plan, 3, true);
    assert_eq!(
        only_status(&execution, &shared),
        Some(FileStatus::ManualFixesNeeded)
    );
    assert_eq!(
        fixture
            .trace_lines()
            .iter()
            .filter(|line| *line == "check-a")
            .count(),
        2,
        "shared later write must rerun the earlier clean check"
    );

    let fixture = Fixture::new("unrelated-invalidation");
    let clean = fixture.file("clean.rs", "CLEAN\n");
    let dirty = fixture.file("dirty.rs", "BAD_B\n");
    let plan = vec![
        scheduled(&fixture, 0, clean, "check-a", None),
        scheduled(&fixture, 1, dirty, "check-b", Some("fix-b")),
    ];
    let _ = execute_deferred_workflows(&plan, 2, true);
    assert_eq!(
        fixture
            .trace_lines()
            .iter()
            .filter(|line| *line == "check-a")
            .count(),
        1
    );
}

#[test]
fn workspace_check_is_conservatively_invalidated() {
    let fixture = Fixture::new("workspace");
    let workspace_candidate = fixture.file("workspace.rs", "CLEAN\n");
    let dirty = fixture.file("dirty.rs", "BAD_B\n");
    let plan = vec![
        scheduled_with_scope(
            &fixture,
            0,
            vec![workspace_candidate.clone()],
            "check-workspace",
            None,
            CheckScope::Workspace,
            true,
        ),
        {
            let mut workflow = scheduled(&fixture, 1, dirty, "check-b", Some("fix-b"));
            workflow.remedy.as_mut().expect("remedy").writes = WriteBehavior::Workspace;
            workflow
        },
    ];
    let execution = execute_deferred_workflows(&plan, 2, true);
    assert_eq!(
        only_status(&execution, &workspace_candidate),
        Some(FileStatus::ManualFixesNeeded)
    );
    assert_eq!(
        fixture
            .trace_lines()
            .iter()
            .filter(|line| *line == "check-workspace")
            .count(),
        2
    );
}

#[test]
fn parallel_and_serial_jobs_produce_the_same_ordered_result() {
    let fixture = Fixture::new("parallel");
    let files = (0..6)
        .map(|index| fixture.file(&format!("file-{index}.rs"), "DIRTY\n"))
        .collect::<Vec<_>>();
    let plan = files
        .iter()
        .enumerate()
        .map(|(index, file)| scheduled(&fixture, index, file.clone(), "check", Some("fix")))
        .collect::<Vec<_>>();
    let serial = execute_deferred_workflows(&plan, 1, true).result;
    for file in &files {
        std::fs::write(file, "DIRTY\n").expect("reset candidate");
    }
    let parallel = execute_deferred_workflows(&plan, 4, true).result;
    assert_eq!(serial, parallel);
}
