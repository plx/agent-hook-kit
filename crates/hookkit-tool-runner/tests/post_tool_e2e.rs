//! Hermetic end-to-end tests for the immediate `post-tool-use-agent-hook`
//! binary and the shared CLI and state-root plumbing of the bundled hooks.

#![cfg(unix)]

use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn pkl_available() -> bool {
    Command::new("pkl")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Whether Pkl-dependent tests must run instead of skipping. CI installs Pkl
/// and sets `HOOKKIT_REQUIRE_PKL=1`, so a missing binary fails the test.
fn pkl_required() -> bool {
    std::env::var_os("HOOKKIT_REQUIRE_PKL").is_some_and(|value| !value.is_empty() && value != "0")
}

macro_rules! require_pkl {
    () => {
        if !pkl_available() {
            assert!(
                !pkl_required(),
                "HOOKKIT_REQUIRE_PKL is set, but the pkl binary is not on PATH"
            );
            eprintln!("skipping test: pkl binary not on PATH");
            return;
        }
    };
}

struct TempProject(PathBuf);

impl TempProject {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "hookkit-post-e2e-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        Self(std::fs::canonicalize(root).unwrap())
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn claude_post_tool(project: &Path, tool: &str, tool_input: Value) -> Value {
    json!({
        "session_id": "post-e2e",
        "transcript_path": "/tmp/post-e2e.jsonl",
        "cwd": project.to_string_lossy(),
        "hook_event_name": "PostToolUse",
        "tool_name": tool,
        "tool_input": tool_input,
        "tool_use_id": "post-e2e-tool",
        "tool_response": {}
    })
}

fn claude_command(binary: &str, args: &[&str], project: &Path, cwd: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .args(args)
        .current_dir(cwd)
        .env_remove("CLAUDE_ENV_FILE")
        .env_remove("CLAUDE_PLUGIN_ROOT")
        .env_remove("CLAUDE_PLUGIN_DATA")
        .env_remove("CLAUDE_CODE_REMOTE")
        .env("CLAUDECODE", "1")
        .env("CLAUDE_CODE_CHILD_SESSION", "1")
        .env("CLAUDE_CODE_SESSION_ID", "post-e2e")
        .env("CLAUDE_PROJECT_DIR", project)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn run_with_input(mut command: Command, input: &Value) -> Output {
    let mut child = command.spawn().unwrap();
    {
        use std::io::Write as _;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(input).unwrap())
            .unwrap();
    }
    child.wait_with_output().unwrap()
}

fn run_claude(binary: &str, args: &[&str], project: &Path, cwd: &Path, input: &Value) -> Output {
    run_with_input(claude_command(binary, args, project, cwd), input)
}

fn write_executable(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
}

/// A project config running one verify-only tool over `*.txt` files.
fn configure_txt_tool(project: &Path, executable: &Path, settings: &str) {
    let config_dir = project.join(".agent-hook-kit");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("post-tool-use.pkl"),
        format!(
            r#"amends "Config.pkl"

settings {{
{settings}
}}

tools {{
  ["ghost"] = new ToolSpec {{
    id = "ghost"
    displayName = "Ghost"
    executable = "{}"
    installHint = "install ghost"
    files {{ include = new Listing {{ "**/*.txt" }} }}
    phases {{
      ["verify"] = new Phase {{ mode = "verify"; argv = new Listing {{ new Files {{}} }} }}
    }}
  }}
}}
run = new Listing {{ "ghost" }}
"#,
            executable.display()
        ),
    )
    .unwrap();
}

#[test]
fn read_only_tool_calls_skip_config_evaluation() {
    let project = TempProject::new("read-only");
    let config_dir = project.path().join(".agent-hook-kit");
    std::fs::create_dir_all(&config_dir).unwrap();
    // A broken config would fail the hook if it were evaluated.
    std::fs::write(config_dir.join("post-tool-use.pkl"), "this is not pkl {{{").unwrap();

    let output = run_claude(
        env!("CARGO_BIN_EXE_post-tool-use-agent-hook"),
        &["--claude"],
        project.path(),
        project.path(),
        &claude_post_tool(project.path(), "Read", json!({"file_path": "a.txt"})),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({})
    );
}

#[test]
fn missing_tool_notices_reach_the_user_through_system_message() {
    require_pkl!();
    let project = TempProject::new("notice");
    let config_dir = project.path().join(".agent-hook-kit");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("post-tool-use.pkl"),
        format!(
            r#"amends "Config.pkl"

tools {{
  ["ghost"] = new ToolSpec {{
    id = "ghost"
    displayName = "Ghost"
    executable = "{}"
    installHint = "install ghost"
    files {{ include = new Listing {{ "**/*.txt" }} }}
    phases {{
      ["verify"] = new Phase {{ mode = "verify"; argv = new Listing {{ new Files {{}} }} }}
    }}
  }}
}}
run = new Listing {{ "ghost" }}
"#,
            project.path().join("bin/ghost").display()
        ),
    )
    .unwrap();
    std::fs::write(project.path().join("a.txt"), "x\n").unwrap();

    let output = run_claude(
        env!("CARGO_BIN_EXE_post-tool-use-agent-hook"),
        &["--claude"],
        project.path(),
        project.path(),
        &claude_post_tool(
            project.path(),
            "Write",
            json!({"file_path": "a.txt", "content": "x"}),
        ),
    );
    assert_eq!(output.status.code(), Some(0));
    assert!(
        output.stderr.is_empty(),
        "exit-0 stderr is debug-log only: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout: Value = serde_json::from_slice(&output.stdout).unwrap();
    let message = stdout["systemMessage"].as_str().unwrap();
    assert!(message.contains("Ghost"), "{message}");
    assert!(message.contains("install ghost"), "{message}");
}

/// A run failure reaches the user through `systemMessage` on exit 0: a
/// failed PostToolUse hook's stderr never reaches the agent, Claude shows the
/// user only its first line, and Codex drops it entirely.
#[test]
fn configuration_failures_are_reported_through_the_native_response() {
    require_pkl!();
    let project = TempProject::new("config-failure");
    let config_dir = project.path().join(".agent-hook-kit");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(config_dir.join("post-tool-use.pkl"), "this is not pkl {{{").unwrap();
    std::fs::write(project.path().join("a.txt"), "x\n").unwrap();

    let output = run_claude(
        env!("CARGO_BIN_EXE_post-tool-use-agent-hook"),
        &["--claude"],
        project.path(),
        project.path(),
        &claude_post_tool(
            project.path(),
            "Write",
            json!({"file_path": "a.txt", "content": "x"}),
        ),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout: Value = serde_json::from_slice(&output.stdout).unwrap();
    let message = stdout["systemMessage"].as_str().unwrap_or_default();
    assert!(
        message.starts_with("error: post-tool-use configuration could not be loaded")
            && message.contains("post-tool-use.pkl"),
        "{stdout}"
    );
    assert!(stdout.get("decision").is_none(), "{stdout}");
}

/// Codex runs each command hook in a new session and, when the hook times
/// out or the turn is interrupted, SIGKILLs the hook's whole process group.
/// Tools run in process groups of their own, so they must not survive that:
/// a fixer that outlived the hook would keep rewriting files under the agent.
#[test]
fn tools_die_with_a_hook_whose_process_group_is_killed() {
    require_pkl!();
    let project = TempProject::new("group-kill");
    let started = project.path().join("tool-started");
    let late = project.path().join("late-write.txt");
    let tool = project.path().join("bin/slow-fixer");
    write_executable(
        &tool,
        &format!(
            "#!/bin/sh\ntouch '{}'\nsleep 2\necho late > '{}'\n",
            started.display(),
            late.display()
        ),
    );
    configure_txt_tool(project.path(), &tool, "");
    std::fs::write(project.path().join("a.txt"), "x\n").unwrap();

    let mut command = claude_command(
        env!("CARGO_BIN_EXE_post-tool-use-agent-hook"),
        &["--claude"],
        project.path(),
        project.path(),
    );
    {
        use std::os::unix::process::CommandExt as _;
        // Like Codex, give the hook a process group of its own to kill.
        command.process_group(0);
    }
    let mut hook = command.spawn().unwrap();
    {
        use std::io::Write as _;
        hook.stdin
            .take()
            .unwrap()
            .write_all(
                &serde_json::to_vec(&claude_post_tool(
                    project.path(),
                    "Write",
                    json!({"file_path": "a.txt", "content": "x"}),
                ))
                .unwrap(),
            )
            .unwrap();
    }
    let waiting = Instant::now();
    while !started.exists() {
        assert!(
            waiting.elapsed() < Duration::from_secs(60),
            "the tool never started"
        );
        if let Some(status) = hook.try_wait().unwrap() {
            panic!("the hook exited before its tool started: {status}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let hook_group = libc::pid_t::try_from(hook.id()).unwrap();
    // SAFETY: `killpg` has no memory-safety preconditions; the group is the
    // hook's own, which is still running (it is not reaped until `wait`).
    assert_eq!(unsafe { libc::killpg(hook_group, libc::SIGKILL) }, 0);
    hook.wait().unwrap();

    std::thread::sleep(Duration::from_millis(3500));
    assert!(
        !late.exists(),
        "a tool outlived the hook whose process group was killed"
    );
}

/// Claude Code's `cwd` follows `cd`, even out of the project, while the
/// Stop runner discovers configuration from `CLAUDE_PROJECT_DIR`; both
/// runners must apply the same project configuration.
#[test]
fn claude_configuration_is_discovered_from_the_project_not_the_cwd() {
    require_pkl!();
    let project = TempProject::new("claude-discovery");
    let elsewhere = TempProject::new("claude-discovery-cwd");
    configure_txt_tool(
        project.path(),
        &project.path().join("bin/ghost"),
        "  runTimeoutSeconds = 30",
    );
    std::fs::write(project.path().join("a.txt"), "x\n").unwrap();

    let output = run_claude(
        env!("CARGO_BIN_EXE_post-tool-use-agent-hook"),
        &["--claude"],
        project.path(),
        elsewhere.path(),
        &claude_post_tool(
            elsewhere.path(),
            "Write",
            json!({"file_path": project.path().join("a.txt"), "content": "x"}),
        ),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout: Value = serde_json::from_slice(&output.stdout).unwrap();
    let message = stdout["systemMessage"].as_str().unwrap_or_default();
    assert!(
        message.contains("Ghost") && message.contains("install ghost"),
        "the project's configuration ran: {stdout}"
    );
}

/// `--config` relative to the project, including a bare file name, keeps
/// naming the same file after the agent changes directory.
#[test]
fn relative_config_paths_follow_the_project_root_not_the_hook_cwd() {
    require_pkl!();
    let project = TempProject::new("relative-config");
    let subdir = project.path().join("src");
    std::fs::create_dir_all(&subdir).unwrap();
    configure_txt_tool(project.path(), &project.path().join("bin/ghost"), "");
    std::fs::rename(
        project.path().join(".agent-hook-kit/post-tool-use.pkl"),
        project.path().join("hooks.pkl"),
    )
    .unwrap();
    std::fs::write(project.path().join("a.txt"), "x\n").unwrap();

    for cwd in [project.path(), subdir.as_path()] {
        let output = run_claude(
            env!("CARGO_BIN_EXE_post-tool-use-agent-hook"),
            &["--claude", "--config", "hooks.pkl"],
            project.path(),
            cwd,
            &claude_post_tool(
                cwd,
                "Write",
                json!({"file_path": project.path().join("a.txt"), "content": "x"}),
            ),
        );
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}: {}",
            cwd.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            stdout["systemMessage"]
                .as_str()
                .unwrap_or_default()
                .contains("Ghost"),
            "{}: {stdout}",
            cwd.display()
        );
    }
}

/// Antigravity accepts conversations without a workspace; a detected write
/// must still find the file's project configuration instead of failing.
#[test]
fn antigravity_writes_without_a_workspace_find_the_file_s_project() {
    require_pkl!();
    let project = TempProject::new("antigravity-no-workspace");
    configure_txt_tool(
        project.path(),
        &project.path().join("bin/ghost"),
        r#"  loweringPolicy = "best-effort""#,
    );
    let file = project.path().join("notes/a.txt");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "x\n").unwrap();
    let input = json!({
        "conversationId": "post-e2e-antigravity",
        "workspacePaths": [],
        "transcriptPath": project.path().join("transcript.jsonl").to_string_lossy(),
        "artifactDirectoryPath": project.path().join("artifacts").to_string_lossy(),
        "toolCall": {
            "name": "write_to_file",
            "args": {"TargetFile": file.to_string_lossy(), "CodeContent": "x"}
        },
        "stepIdx": 1
    });
    let mut command = Command::new(env!("CARGO_BIN_EXE_post-tool-use-agent-hook"));
    command
        .arg("--antigravity")
        .current_dir(project.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = run_with_input(command, &input);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"{}");
}

#[test]
fn relative_state_dirs_follow_the_project_root_not_the_hook_cwd() {
    let project = TempProject::new("state-root");
    let subdir = project.path().join("packages/foo");
    std::fs::create_dir_all(&subdir).unwrap();

    let output = run_claude(
        env!("CARGO_BIN_EXE_file-activity-agent-hook"),
        &["--claude", "--state-dir", ".context/hookkit-state"],
        project.path(),
        &subdir,
        &claude_post_tool(
            project.path(),
            "Write",
            json!({"file_path": "a.txt", "content": "x"}),
        ),
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(project.path().join(".context/hookkit-state").is_dir());
    assert!(
        !subdir.join(".context").exists(),
        "state must not split into the agent's current directory"
    );
}

#[test]
fn invalid_arguments_fail_with_a_named_diagnostic_and_exit_one() {
    for (binary, args, name) in [
        (
            env!("CARGO_BIN_EXE_turn-completion-agent-hook"),
            vec!["--harness=claud"],
            "turn-completion-agent-hook",
        ),
        (
            env!("CARGO_BIN_EXE_session-start-state-agent-hook"),
            vec!["--harness=antigravity"],
            "session-start-state-agent-hook",
        ),
        (
            env!("CARGO_BIN_EXE_turn-completion-agent-hook"),
            vec!["--claude", "--codex"],
            "turn-completion-agent-hook",
        ),
    ] {
        let output = Command::new(binary)
            .args(&args)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.starts_with(name), "{args:?}: {stderr}");
    }

    let help = Command::new(env!("CARGO_BIN_EXE_post-tool-use-agent-hook"))
        .arg("--help")
        .output()
        .unwrap();
    assert_eq!(help.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&help.stdout).contains("post-tool-use-agent-hook"));
}

#[test]
fn file_activity_journal_keys_do_not_embed_the_raw_hook_input() {
    let project = TempProject::new("event-key");
    let state = project.path().join("state");
    let large = "SECRET-CONTENT ".repeat(4096);
    let output = run_claude(
        env!("CARGO_BIN_EXE_file-activity-agent-hook"),
        &["--claude", "--state-dir", state.to_str().unwrap()],
        project.path(),
        project.path(),
        &claude_post_tool(
            project.path(),
            "Write",
            json!({"file_path": "a.txt", "content": large}),
        ),
    );
    assert_eq!(output.status.code(), Some(0));
    let mut journal = String::new();
    let mut stack = vec![state];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap().filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "ndjson")
            {
                journal.push_str(&std::fs::read_to_string(path).unwrap());
            }
        }
    }
    assert!(!journal.is_empty(), "the observation was journaled");
    assert!(
        !journal.contains("SECRET-CONTENT"),
        "tool input must not be copied into journal keys"
    );
}
