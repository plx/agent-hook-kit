//! File selection, job partitioning, argv rendering, and bounded subprocess
//! execution shared by the immediate and deferred runners.

use crate::spec::{
    CommandArgTemplate, ExitCodePolicy, FileSelection, ToolPhase, ToolSpec, UnexpectedExitPolicy,
};
use crate::util::{invalid_data, path_arg, slash_path};
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

/// Conservative byte budget for file-list arguments in one invocation.
///
/// `ARG_MAX` covers argv strings, their pointer array, and the environment
/// (1 MiB on macOS, typically 2 MiB on Linux; Windows limits the whole command
/// line to 32,767 UTF-16 units). Batches larger than this budget are split into
/// several invocations instead of failing with `E2BIG`.
pub(crate) const FILE_ARGUMENT_BUDGET_BYTES: usize =
    if cfg!(windows) { 24 * 1024 } else { 128 * 1024 };

/// How long to keep draining output after the direct child exited. A daemon
/// grandchild that inherited the pipes must not hold the hook open forever.
const OUTPUT_DRAIN_GRACE: Duration = Duration::from_secs(2);

// ----------------------------------------------------------------------------
// File matching
// ----------------------------------------------------------------------------

pub(crate) struct FileMatcher {
    include: GlobSet,
    exclude: GlobSet,
    include_all: bool,
}

impl FileMatcher {
    pub(crate) fn new(config: &FileSelection) -> hookkit_core::Result<Self> {
        Ok(Self {
            include: build_globset(&config.include)?,
            exclude: build_globset(&config.exclude)?,
            include_all: config.include.is_empty(),
        })
    }

    /// Match include and exclude globs against the project-relative path.
    ///
    /// Only files outside `project_root` fall back to their absolute path, so
    /// a directory name above the project (for example `/home/me/build/app`
    /// with `**/build/**` excluded) can never exclude every project file.
    pub(crate) fn matches(&self, absolute_path: &Path, project_root: &Path) -> bool {
        let candidate = match absolute_path.strip_prefix(project_root) {
            Ok(relative) => slash_path(relative),
            Err(_) => slash_path(absolute_path),
        };
        (self.include_all || self.include.is_match(&candidate))
            && !self.exclude.is_match(&candidate)
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
// Jobs
// ----------------------------------------------------------------------------

/// Runner-wide execution settings shared by every job of one invocation.
#[derive(Debug, Clone, Default)]
pub(crate) struct ExecutionSettings {
    /// Deadline for one external command; `None` waits indefinitely.
    pub command_timeout: Option<Duration>,
    /// Budget shared by every command of this invocation, if any.
    pub run_budget: Option<RunBudget>,
    /// Directory basenames pruned from snapshot traversal.
    pub ignored_directory_names: BTreeSet<String>,
}

/// The `settings.runTimeoutSeconds` budget of one hook invocation.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RunBudget {
    /// Configured budget, for diagnostics.
    pub budget: Duration,
    /// Instant at which the budget is spent.
    pub deadline: Instant,
}

impl RunBudget {
    /// A budget of `budget` counted from `started`, the start of the hook.
    pub(crate) fn new(started: Instant, budget: Duration) -> Self {
        Self {
            budget,
            deadline: started + budget,
        }
    }
}

/// Deadline for the next command: the smaller of the per-command timeout and
/// the budget left, and whether the budget is the binding limit.
enum CommandDeadline {
    Unbounded,
    Command(Duration),
    Budget(Duration),
    BudgetSpent,
}

impl ExecutionSettings {
    fn next_deadline(&self) -> CommandDeadline {
        let remaining = self
            .run_budget
            .map(|budget| budget.deadline.saturating_duration_since(Instant::now()));
        match (self.command_timeout, remaining) {
            (_, Some(remaining)) if remaining.is_zero() => CommandDeadline::BudgetSpent,
            (Some(command), Some(remaining)) if command <= remaining => {
                CommandDeadline::Command(command)
            }
            (_, Some(remaining)) => CommandDeadline::Budget(remaining),
            (Some(command), None) => CommandDeadline::Command(command),
            (None, None) => CommandDeadline::Unbounded,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ToolContext<'a> {
    pub spec: &'a ToolSpec,
    pub project_root: &'a Path,
    pub global_diagnostics_dir: Option<&'a str>,
    pub settings: &'a ExecutionSettings,
}

#[derive(Debug, Clone)]
pub(crate) struct ToolJob {
    pub workspace_dir: PathBuf,
    pub workspace_indicator: Option<PathBuf>,
    pub files: Vec<PathBuf>,
}

pub(crate) fn build_jobs(paths: &[PathBuf], project_root: &Path, spec: &ToolSpec) -> Vec<ToolJob> {
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

/// Split jobs whose file-list arguments would exceed `budget` bytes into
/// consecutive chunks of the same workspace. Every chunk keeps at least one
/// file, so a single oversized path still gets one attempt.
pub(crate) fn split_jobs_for_argument_budget(jobs: Vec<ToolJob>, budget: usize) -> Vec<ToolJob> {
    let mut split = Vec::with_capacity(jobs.len());
    for job in jobs {
        let mut chunk = Vec::new();
        let mut used = 0usize;
        for file in &job.files {
            // Each argv entry costs its bytes, a NUL terminator, and a pointer.
            let cost = path_arg(file).len() + 1 + std::mem::size_of::<usize>();
            if !chunk.is_empty() && used + cost > budget {
                split.push(ToolJob {
                    workspace_dir: job.workspace_dir.clone(),
                    workspace_indicator: job.workspace_indicator.clone(),
                    files: std::mem::take(&mut chunk),
                });
                used = 0;
            }
            used += cost;
            chunk.push(file.clone());
        }
        if !chunk.is_empty() || job.files.is_empty() {
            split.push(ToolJob {
                files: chunk,
                ..job
            });
        }
    }
    split
}

// ----------------------------------------------------------------------------
// Command rendering and execution
// ----------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) struct RenderedCommand {
    pub program: String,
    pub args: Vec<String>,
}

pub(crate) fn render_command(
    phase: &ToolPhase,
    job: &ToolJob,
    context: &ToolContext<'_>,
) -> RenderedCommand {
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
                        .map(workspace_relative_arg)
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
            CommandArgTemplate::ToolExecutable => args.push(context.spec.executable.clone()),
            CommandArgTemplate::ExtraArgs => args.extend(phase.extra_args.iter().cloned()),
        }
    }
    RenderedCommand { program, args }
}

/// Render a workspace-relative path so a file named like an option (for
/// example `--plugin=x.js`) can never be parsed as one by the tool.
fn workspace_relative_arg(relative: &Path) -> String {
    let rendered = path_arg(relative);
    if rendered.starts_with('-') {
        format!(".{}{rendered}", std::path::MAIN_SEPARATOR)
    } else {
        rendered
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PhaseStatus {
    Clean,
    Issues,
    Failure,
}

/// Why an external command produced no usable exit status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CommandError {
    /// The executable could not be found.
    NotFound,
    /// The command exceeded its deadline and its process group was killed.
    TimedOut(Duration),
    /// The invocation's `settings.runTimeoutSeconds` budget ran out: the
    /// command was not started, or it was killed with its process group.
    RunBudgetExhausted(Duration),
    /// Spawning or waiting failed for another operating-system reason.
    Io(String),
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("not found"),
            Self::TimedOut(timeout) => {
                write!(f, "timed out after {} seconds", timeout.as_secs_f64())
            }
            Self::RunBudgetExhausted(budget) => write!(
                f,
                "stopped: the hook's {}-second run budget (settings.runTimeoutSeconds) ran out",
                budget.as_secs_f64()
            ),
            Self::Io(message) => f.write_str(message),
        }
    }
}

#[derive(Debug)]
pub(crate) struct PhaseLog {
    pub phase: String,
    pub command: String,
    pub program: String,
    pub arguments: Vec<String>,
    pub status: Option<i32>,
    pub classification: Option<PhaseStatus>,
    pub stdout: String,
    pub stderr: String,
    pub error: Option<CommandError>,
}

pub(crate) fn run_phase_command(
    phase: &ToolPhase,
    command: &RenderedCommand,
    cwd: &Path,
    settings: &ExecutionSettings,
) -> PhaseLog {
    let mut log = PhaseLog {
        phase: phase.id.clone(),
        command: display_command(&command.program, &command.args),
        program: command.program.clone(),
        arguments: command.args.clone(),
        status: None,
        classification: None,
        stdout: String::new(),
        stderr: String::new(),
        error: None,
    };
    let budget = settings
        .run_budget
        .map(|budget| budget.budget)
        .unwrap_or_default();
    let output = match settings.next_deadline() {
        CommandDeadline::BudgetSpent => {
            BoundedOutput::failed(CommandError::RunBudgetExhausted(budget))
        }
        CommandDeadline::Budget(remaining) => {
            let mut output = run_bounded(&command.program, &command.args, cwd, Some(remaining));
            if matches!(output.result, Err(CommandError::TimedOut(_))) {
                output.result = Err(CommandError::RunBudgetExhausted(budget));
            }
            output
        }
        CommandDeadline::Command(timeout) => {
            run_bounded(&command.program, &command.args, cwd, Some(timeout))
        }
        CommandDeadline::Unbounded => run_bounded(&command.program, &command.args, cwd, None),
    };
    log.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    log.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    match output.result {
        Ok(status) => {
            log.status = status;
            let mut classification = status.map(|code| classify_exit_code(&phase.exit_codes, code));
            if phase.issues_on_stdout
                && classification == Some(PhaseStatus::Clean)
                && !log.stdout.trim().is_empty()
            {
                classification = Some(PhaseStatus::Issues);
            }
            log.classification = classification;
        }
        Err(error) => log.error = Some(error),
    }
    log
}

struct BoundedOutput {
    result: Result<Option<i32>, CommandError>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl BoundedOutput {
    fn failed(error: CommandError) -> Self {
        Self {
            result: Err(error),
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }
}

/// Serializes every spawn below so a pipe created for one command (its output
/// pipes or its group's lifeline) is marked close-on-exec before a concurrent
/// job can fork. On platforms whose standard library creates a pipe and sets
/// `FD_CLOEXEC` in two steps, an unserialized fork in that window would leak
/// the pipe into an unrelated tool and keep it open after this hook exits.
static SPAWN_LOCK: Mutex<()> = Mutex::new(());

/// Run one command with null stdin, captured output, and an optional deadline.
///
/// On Unix the command runs in a process group of its own whose lifetime is
/// bound to this hook process (see [`ProcessGroup`]): a runner timeout kills
/// the whole group, so descendants such as the rustc processes below
/// `cargo clippy --fix` cannot keep writing after the hook returns, and if the
/// hook itself dies first (for example when the harness kills the hook's own
/// process group on its timeout or a turn interrupt) the group's watchdog
/// kills it too.
fn run_bounded(
    program: &str,
    args: &[String],
    cwd: &Path,
    timeout: Option<Duration>,
) -> BoundedOutput {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let spawned = {
        let _spawning = SPAWN_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        #[cfg(unix)]
        let group = ProcessGroup::start().ok();
        #[cfg(unix)]
        if let Some(group) = &group {
            use std::os::unix::process::CommandExt as _;
            command.process_group(group.id());
        }
        #[cfg(not(unix))]
        let group = None::<ProcessGroup>;
        command.spawn().map(|child| (child, group))
    };
    let (mut child, mut group) = match spawned {
        Ok(spawned) => spawned,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return BoundedOutput::failed(CommandError::NotFound);
        }
        Err(error) => return BoundedOutput::failed(CommandError::Io(error.to_string())),
    };
    let stdout = Drain::spawn(child.stdout.take());
    let stderr = Drain::spawn(child.stderr.take());
    let result = wait_with_deadline(&mut child, group.as_mut(), timeout);
    let grace_deadline = Instant::now() + OUTPUT_DRAIN_GRACE;
    let output = BoundedOutput {
        result,
        stdout: Drain::collect(stdout, grace_deadline),
        stderr: Drain::collect(stderr, grace_deadline),
    };
    if let Some(group) = group {
        group.release();
    }
    output
}

/// A process group whose members cannot outlive this process.
///
/// The group is led by a tiny `/bin/sh` watchdog that blocks reading a pipe
/// whose write end only this process holds. [`ProcessGroup::release`] tells
/// the watchdog to exit quietly once the command has finished; if this process
/// dies before that (for example because the harness killed the hook's own
/// process group, which the command's separate group does not share), the
/// pipe reaches end-of-file and the watchdog kills its whole group, and with
/// it the command and every descendant that stayed in the group. Dropping an
/// unreleased group has the same effect.
#[cfg(unix)]
struct ProcessGroup {
    leader: Child,
    lifeline: Option<std::process::ChildStdin>,
}

#[cfg(unix)]
impl ProcessGroup {
    /// Exits quietly on `release`; kills the group (itself included) on any
    /// other input, including end-of-file when the runner is gone.
    const WATCHDOG: &'static str = r#"IFS= read -r line; [ "$line" = release ] || kill -9 0"#;

    fn start() -> std::io::Result<Self> {
        use std::os::unix::process::CommandExt as _;
        let mut leader = Command::new("/bin/sh")
            .args(["-c", Self::WATCHDOG])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
        let lifeline = leader.stdin.take();
        Ok(Self { leader, lifeline })
    }

    /// Process-group id: the watchdog leader's pid.
    fn id(&self) -> i32 {
        i32::try_from(self.leader.id()).unwrap_or(i32::MAX)
    }

    /// Kill every member of the group, the watchdog included.
    fn kill(&mut self) {
        // SAFETY: `killpg` has no memory-safety preconditions. The group id
        // is the watchdog's pid, which stays reserved until it is reaped
        // below, so the signal cannot reach an unrelated group.
        unsafe {
            libc::killpg(self.id(), libc::SIGKILL);
        }
        // The watchdog is gone: a later `release` must not write to its pipe,
        // which would raise SIGPIPE in a process that has not ignored it.
        self.lifeline = None;
    }

    /// The command finished: let the watchdog exit without killing anything,
    /// so a daemon the command deliberately left behind keeps running.
    fn release(mut self) {
        use std::io::Write as _;
        if let Some(mut lifeline) = self.lifeline.take() {
            let _ = lifeline.write_all(b"release\n");
        }
        let _ = self.leader.wait();
    }
}

#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        // Closing an unreleased lifeline makes the watchdog kill the group;
        // either way the watchdog exits and is reaped here (a second `wait`
        // after `release` returns the recorded status).
        drop(self.lifeline.take());
        let _ = self.leader.wait();
    }
}

/// Non-Unix platforms have no process groups here; the command is killed
/// directly on timeout.
#[cfg(not(unix))]
struct ProcessGroup;

#[cfg(not(unix))]
impl ProcessGroup {
    fn kill(&mut self) {}

    fn release(self) {}
}

/// Background reader for one output pipe. Bytes are accumulated as they
/// arrive so output already written is kept even if a daemon grandchild keeps
/// the pipe open past the grace deadline.
struct Drain {
    buffer: Arc<Mutex<Vec<u8>>>,
    finished: mpsc::Receiver<()>,
}

impl Drain {
    fn spawn<R: Read + Send + 'static>(pipe: Option<R>) -> Option<Self> {
        let mut pipe = pipe?;
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let (sender, finished) = mpsc::channel();
        let sink = Arc::clone(&buffer);
        std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(read) => match sink.lock() {
                        Ok(mut bytes) => bytes.extend_from_slice(&chunk[..read]),
                        Err(_) => break,
                    },
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
            let _ = sender.send(());
        });
        Some(Self { buffer, finished })
    }

    fn collect(drain: Option<Self>, deadline: Instant) -> Vec<u8> {
        let Some(drain) = drain else {
            return Vec::new();
        };
        let _ = drain
            .finished
            .recv_timeout(deadline.saturating_duration_since(Instant::now()));
        drain
            .buffer
            .lock()
            .map(|mut bytes| std::mem::take(&mut *bytes))
            .unwrap_or_default()
    }
}

fn wait_with_deadline(
    child: &mut Child,
    group: Option<&mut ProcessGroup>,
    timeout: Option<Duration>,
) -> Result<Option<i32>, CommandError> {
    let Some(timeout) = timeout else {
        return child
            .wait()
            .map(|status| status.code())
            .map_err(|error| CommandError::Io(error.to_string()));
    };
    let deadline = Instant::now() + timeout;
    let mut pause = Duration::from_millis(1);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.code()),
            Ok(None) => {}
            Err(error) => return Err(CommandError::Io(error.to_string())),
        }
        let now = Instant::now();
        if now >= deadline {
            if let Some(group) = group {
                group.kill();
            }
            let _ = child.kill();
            let _ = child.wait();
            return Err(CommandError::TimedOut(timeout));
        }
        std::thread::sleep(pause.min(deadline - now));
        pause = (pause * 2).min(Duration::from_millis(50));
    }
}

pub(crate) fn classify_exit_code(policy: &ExitCodePolicy, code: i32) -> PhaseStatus {
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

/// Resolve `settings.jobs` to a worker-thread count for a batch of `job_count`
/// independent jobs.
///
/// `jobs = 0` selects "auto", which is reserved for future use and runs
/// serially for now. `jobs = n >= 1` runs up to `n` jobs concurrently, capped
/// at `job_count` since extra workers would have nothing to claim.
pub(crate) fn resolve_worker_count(jobs_setting: u32, job_count: usize) -> usize {
    if job_count == 0 {
        return 0;
    }
    let requested = match jobs_setting {
        0 => 1, // auto: reserved for future use; serial for now
        n => n as usize,
    };
    requested.clamp(1, job_count)
}

pub(crate) fn format_logs(logs: &[PhaseLog]) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::PhaseMode;
    use proptest::prelude::*;

    fn selection(include: &[&str], exclude: &[&str]) -> FileSelection {
        FileSelection {
            include: include.iter().map(|value| (*value).to_owned()).collect(),
            exclude: exclude.iter().map(|value| (*value).to_owned()).collect(),
        }
    }

    #[test]
    fn excludes_match_project_relative_paths_not_ancestors_of_the_root() {
        let matcher = FileMatcher::new(&selection(&["**/*.py"], &["**/build/**"])).unwrap();
        let root = Path::new("/home/me/build/app");
        assert!(matcher.matches(Path::new("/home/me/build/app/src/x.py"), root));
        assert!(!matcher.matches(Path::new("/home/me/build/app/build/gen.py"), root));
    }

    #[test]
    fn includes_do_not_match_through_directories_above_the_root() {
        let matcher = FileMatcher::new(&selection(&["home/**"], &[])).unwrap();
        let root = Path::new("/home/me/app");
        assert!(!matcher.matches(Path::new("/home/me/app/src/x.py"), root));
    }

    #[test]
    fn files_outside_the_root_fall_back_to_absolute_matching() {
        let matcher = FileMatcher::new(&selection(&["/outside/**"], &[])).unwrap();
        assert!(matcher.matches(Path::new("/outside/x.py"), Path::new("/project")));
    }

    #[test]
    fn default_global_excludes_match_nested_git_and_node_modules() {
        let settings = hookkit_pkl_config::Settings::default();
        let matcher = FileMatcher::new(&FileSelection {
            include: Vec::new(),
            exclude: settings.exclude,
        })
        .unwrap();
        let root = Path::new("/repo");
        for excluded in [
            "/repo/node_modules/a/index.js",
            "/repo/packages/a/node_modules/foo/index.js",
            "/repo/.git/config",
            "/repo/vendor/sub/.git/HEAD",
        ] {
            assert!(!matcher.matches(Path::new(excluded), root), "{excluded}");
        }
        assert!(matcher.matches(Path::new("/repo/packages/a/src/index.js"), root));
    }

    #[test]
    fn workspace_relative_files_starting_with_a_dash_cannot_become_options() {
        let spec = ToolSpec::new("t", "T", "tool");
        let settings = ExecutionSettings::default();
        let context = ToolContext {
            spec: &spec,
            project_root: Path::new("/ws"),
            global_diagnostics_dir: None,
            settings: &settings,
        };
        let job = ToolJob {
            workspace_dir: PathBuf::from("/ws"),
            workspace_indicator: None,
            files: vec![
                PathBuf::from("/ws/--plugin=evil.js"),
                PathBuf::from("/ws/src/ok.js"),
                PathBuf::from("/elsewhere/-x.js"),
            ],
        };
        let phase = ToolPhase::new("fmt", PhaseMode::Format)
            .with_args([CommandArgTemplate::WorkspaceFiles]);
        let rendered = render_command(&phase, &job, &context);
        assert_eq!(
            rendered.args,
            [
                format!(".{}--plugin=evil.js", std::path::MAIN_SEPARATOR),
                "src/ok.js".to_owned(),
                "/elsewhere/-x.js".to_owned(),
            ]
        );
    }

    #[test]
    fn oversized_batches_split_into_bounded_chunks_in_order() {
        let files = (0..5000)
            .map(|index| PathBuf::from(format!("/repo/generated/file-{index:05}.txt")))
            .collect::<Vec<_>>();
        let job = ToolJob {
            workspace_dir: PathBuf::from("/repo"),
            workspace_indicator: None,
            files: files.clone(),
        };
        let chunks = split_jobs_for_argument_budget(vec![job], 16 * 1024);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            let bytes = chunk
                .files
                .iter()
                .map(|file| path_arg(file).len() + 1 + std::mem::size_of::<usize>())
                .sum::<usize>();
            assert!(bytes <= 16 * 1024);
        }
        let rejoined = chunks
            .into_iter()
            .flat_map(|chunk| chunk.files)
            .collect::<Vec<_>>();
        assert_eq!(rejoined, files);
    }

    #[test]
    fn a_single_oversized_path_still_gets_one_invocation() {
        let job = ToolJob {
            workspace_dir: PathBuf::from("/repo"),
            workspace_indicator: None,
            files: vec![PathBuf::from(format!("/repo/{}", "x".repeat(64)))],
        };
        let chunks = split_jobs_for_argument_budget(vec![job], 8);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].files.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn timed_out_commands_kill_their_whole_process_group() {
        let root = std::env::temp_dir().join(format!(
            "hookkit-timeout-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let marker = root.join("grandchild-survived");
        let script = format!("(sleep 1; touch '{}') & sleep 30", marker.to_string_lossy());
        let started = Instant::now();
        let output = run_bounded(
            "sh",
            &["-c".to_owned(), script],
            &root,
            Some(Duration::from_millis(200)),
        );
        assert_eq!(
            output.result,
            Err(CommandError::TimedOut(Duration::from_millis(200)))
        );
        assert!(started.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(1500));
        assert!(!marker.exists(), "grandchild outlived the timeout");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Tools run in a process group of their own, so a harness that kills
    /// the hook's group does not reach them directly; the group's watchdog
    /// must kill them when the hook disappears without releasing the group.
    #[cfg(unix)]
    #[test]
    fn abandoned_process_groups_die_and_released_ones_keep_their_daemons() {
        let root = std::env::temp_dir().join(format!(
            "hookkit-lifeline-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let spawn_in = |group: &ProcessGroup, marker: &Path| {
            use std::os::unix::process::CommandExt as _;
            Command::new("sh")
                .args([
                    "-c".to_owned(),
                    format!("sleep 1; touch '{}'", marker.to_string_lossy()),
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .process_group(group.id())
                .spawn()
                .unwrap()
        };

        // Dropping the group without releasing it is what the hook's death
        // looks like to the watchdog: its lifeline reaches end-of-file.
        let abandoned = root.join("abandoned");
        let group = ProcessGroup::start().unwrap();
        let mut child = spawn_in(&group, &abandoned);
        drop(group);
        let status = child.wait().unwrap();
        assert!(
            !status.success(),
            "the watchdog killed the member: {status}"
        );

        // A released group lets a member the command left behind finish.
        let released = root.join("released");
        let group = ProcessGroup::start().unwrap();
        let mut child = spawn_in(&group, &released);
        group.release();
        assert!(child.wait().unwrap().success());

        std::thread::sleep(Duration::from_millis(300));
        assert!(!abandoned.exists(), "a member outlived an abandoned group");
        assert!(released.exists(), "a released group must not be killed");
        std::fs::remove_dir_all(root).unwrap();
    }

    fn marker_command(marker: &Path, delay: &str) -> RenderedCommand {
        RenderedCommand {
            program: "sh".into(),
            args: vec![
                "-c".into(),
                format!("sleep {delay}; touch '{}'", marker.to_string_lossy()),
            ],
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_run_budget_bounds_every_command_of_one_invocation() {
        let root = std::env::temp_dir().join(format!(
            "hookkit-run-budget-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let phase = ToolPhase::new("verify", PhaseMode::Verify);
        let budget = Duration::from_millis(300);

        // The budget, not the longer per-command timeout, stops the command.
        let settings = ExecutionSettings {
            command_timeout: Some(Duration::from_secs(60)),
            run_budget: Some(RunBudget::new(Instant::now(), budget)),
            ..ExecutionSettings::default()
        };
        let killed = root.join("killed");
        let started = Instant::now();
        let log = run_phase_command(&phase, &marker_command(&killed, "5"), &root, &settings);
        assert_eq!(log.error, Some(CommandError::RunBudgetExhausted(budget)));
        assert!(started.elapsed() < Duration::from_secs(4));

        // Once the budget is spent, later commands are not started at all.
        let skipped = root.join("skipped");
        let log = run_phase_command(&phase, &marker_command(&skipped, "0"), &root, &settings);
        assert_eq!(log.error, Some(CommandError::RunBudgetExhausted(budget)));
        let message = log
            .error
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        assert!(message.contains("settings.runTimeoutSeconds"), "{message}");

        // A command timeout shorter than the budget left stays a timeout.
        let settings = ExecutionSettings {
            command_timeout: Some(Duration::from_millis(200)),
            run_budget: Some(RunBudget::new(Instant::now(), Duration::from_secs(60))),
            ..ExecutionSettings::default()
        };
        let log = run_phase_command(&phase, &marker_command(&killed, "5"), &root, &settings);
        assert_eq!(
            log.error,
            Some(CommandError::TimedOut(Duration::from_millis(200)))
        );

        std::thread::sleep(Duration::from_millis(300));
        assert!(!killed.exists() && !skipped.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_executables_are_reported_as_not_found() {
        let output = run_bounded(
            "/definitely/missing/hookkit-tool",
            &[],
            Path::new("."),
            Some(Duration::from_secs(5)),
        );
        assert_eq!(output.result, Err(CommandError::NotFound));
    }

    #[cfg(unix)]
    #[test]
    fn a_daemon_holding_the_pipes_does_not_hang_collection() {
        let started = Instant::now();
        let output = run_bounded(
            "sh",
            &["-c".to_owned(), "echo ready; (sleep 8) & exit 0".to_owned()],
            Path::new("."),
            None,
        );
        assert_eq!(output.result, Ok(Some(0)));
        assert_eq!(String::from_utf8_lossy(&output.stdout), "ready\n");
        assert!(started.elapsed() < Duration::from_secs(6));
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

    proptest! {
        /// Property: worker selection is total and bounded. A non-empty batch
        /// always gets at least one worker, never more workers than jobs, and
        /// explicit settings are honored up to that cap (`0` means serial auto).
        #[test]
        fn worker_count_is_bounded(jobs_setting in any::<u32>(), job_count in any::<usize>()) {
            let actual = resolve_worker_count(jobs_setting, job_count);
            let expected = if job_count == 0 {
                0
            } else {
                usize::try_from(jobs_setting.max(1)).unwrap_or(usize::MAX).min(job_count)
            };

            prop_assert_eq!(actual, expected);
            prop_assert!(actual <= job_count);
            prop_assert_eq!(actual == 0, job_count == 0);
        }

        /// Property: overlapping exit-code policy lists have a documented
        /// precedence (clean, then issues, then failure), and unlisted values
        /// use exactly the configured fallback.
        #[test]
        fn exit_code_classification_has_stable_precedence(
            clean in prop::collection::vec(any::<i32>(), 0..30),
            issues in prop::collection::vec(any::<i32>(), 0..30),
            failure in prop::collection::vec(any::<i32>(), 0..30),
            code in any::<i32>(),
            unexpected_issues in any::<bool>(),
        ) {
            let unexpected = if unexpected_issues {
                UnexpectedExitPolicy::Issues
            } else {
                UnexpectedExitPolicy::Failure
            };
            let policy = ExitCodePolicy {
                clean: clean.clone(),
                issues: issues.clone(),
                failure: failure.clone(),
                unexpected,
            };
            let expected = if clean.contains(&code) {
                PhaseStatus::Clean
            } else if issues.contains(&code) {
                PhaseStatus::Issues
            } else if failure.contains(&code) {
                PhaseStatus::Failure
            } else if unexpected_issues {
                PhaseStatus::Issues
            } else {
                PhaseStatus::Failure
            };

            prop_assert_eq!(classify_exit_code(&policy, code), expected);
        }

        /// Property: argument chunking preserves every file exactly once and
        /// in order, never produces an empty chunk for a non-empty job, and
        /// respects the budget unless one path alone exceeds it.
        #[test]
        fn argument_chunking_is_a_bounded_order_preserving_partition(
            lengths in prop::collection::vec(1usize..200, 0..300),
            budget in 16usize..4096,
        ) {
            let files = lengths
                .iter()
                .enumerate()
                .map(|(index, length)| PathBuf::from(format!("/{index}-{}", "a".repeat(*length))))
                .collect::<Vec<_>>();
            let job = ToolJob {
                workspace_dir: PathBuf::from("/"),
                workspace_indicator: None,
                files: files.clone(),
            };
            let chunks = split_jobs_for_argument_budget(vec![job], budget);
            let rejoined = chunks.iter().flat_map(|chunk| chunk.files.clone()).collect::<Vec<_>>();
            prop_assert_eq!(&rejoined, &files);
            for chunk in &chunks {
                prop_assert!(!chunk.files.is_empty() || files.is_empty());
                let bytes = chunk
                    .files
                    .iter()
                    .map(|file| path_arg(file).len() + 1 + std::mem::size_of::<usize>())
                    .sum::<usize>();
                prop_assert!(bytes <= budget || chunk.files.len() == 1);
            }
        }
    }
}
