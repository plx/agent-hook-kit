//! Hermetic end-to-end tests for the immediate `post-tool-use-agent-hook`
//! binary and the shared CLI and state-root plumbing of the bundled hooks.

#![cfg(unix)]

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

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

fn run_claude(binary: &str, args: &[&str], project: &Path, cwd: &Path, input: &Value) -> Output {
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
