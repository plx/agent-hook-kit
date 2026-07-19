use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "hookkit-guard-integration-{label}-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn input(harness: &str, cwd: &Path, path: &str) -> serde_json::Value {
    let cwd = cwd.to_str().unwrap();
    match harness {
        "claude" => serde_json::json!({
            "session_id": "session",
            "transcript_path": "/tmp/transcript.jsonl",
            "cwd": cwd,
            "permission_mode": "default",
            "hook_event_name": "PreToolUse",
            "tool_name": "read_file",
            "tool_input": {"path": path},
            "tool_use_id": "call"
        }),
        "codex" => serde_json::json!({
            "session_id": "session",
            "transcript_path": null,
            "cwd": cwd,
            "hook_event_name": "PreToolUse",
            "model": "gpt-test",
            "turn_id": "turn",
            "permission_mode": "default",
            "tool_name": "read_file",
            "tool_use_id": "call",
            "tool_input": {"path": path}
        }),
        "gemini" => serde_json::json!({
            "session_id": "session",
            "transcript_path": "/tmp/transcript.jsonl",
            "cwd": cwd,
            "hook_event_name": "BeforeTool",
            "timestamp": "2026-07-19T00:00:00Z",
            "tool_name": "read_file",
            "tool_input": {"path": path}
        }),
        "antigravity" => serde_json::json!({
            "conversationId": "session",
            "workspacePaths": [cwd],
            "transcriptPath": "/tmp/transcript.jsonl",
            "artifactDirectoryPath": "/tmp/artifacts",
            "toolCall": {"name": "read_file", "args": {"path": path}},
            "stepIdx": 1
        }),
        _ => unreachable!(),
    }
}

fn run(harness: &str, config: &Path, input: &serde_json::Value) -> std::process::Output {
    let binary = env!("CARGO_BIN_EXE_forbidden-file-guard");
    let mut command = Command::new(binary);
    command
        .args([
            format!("--harness={harness}"),
            format!("--config={}", config.display()),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    clear_hook_environment(&mut command);
    let cwd = input
        .get("cwd")
        .or_else(|| input.get("workspacePaths").and_then(|roots| roots.get(0)))
        .and_then(serde_json::Value::as_str)
        .unwrap();
    match harness {
        "claude" => {
            command
                .env("CLAUDECODE", "1")
                .env("CLAUDE_CODE_CHILD_SESSION", "1")
                .env("CLAUDE_CODE_SESSION_ID", "session")
                .env("CLAUDE_PROJECT_DIR", cwd);
        }
        "gemini" => {
            command
                .env("GEMINI_PROJECT_DIR", cwd)
                .env("GEMINI_PLANS_DIR", format!("{cwd}/.gemini/plans"))
                .env("GEMINI_CWD", cwd)
                .env("GEMINI_SESSION_ID", "session")
                .env("CLAUDE_PROJECT_DIR", cwd);
        }
        _ => {}
    }
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(input).unwrap())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn decision(output: &std::process::Output) -> String {
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    value["decision"]
        .as_str()
        .or_else(|| value["hookSpecificOutput"]["permissionDecision"].as_str())
        .unwrap()
        .to_owned()
}

#[test]
fn every_aligned_harness_emits_native_deny_and_allow() {
    let temporary = TempDirectory::new("harnesses");
    let config = temporary.0.join("policy.yaml");
    std::fs::write(
        &config,
        "patterns: ['.env', '**/.env']\naccess_policy: inspect_known\n",
    )
    .unwrap();

    for harness in ["claude", "codex", "gemini", "antigravity"] {
        let denied = run(harness, &config, &input(harness, &temporary.0, ".env"));
        assert!(denied.status.success(), "{harness}: {:?}", denied.stderr);
        assert_eq!(decision(&denied), "deny", "{harness}");

        let allowed = run(harness, &config, &input(harness, &temporary.0, "notes.txt"));
        assert!(allowed.status.success(), "{harness}: {:?}", allowed.stderr);
        assert_eq!(decision(&allowed), "allow", "{harness}");
    }
}

#[test]
fn codex_shell_heredoc_patch_is_denied_through_stdin() {
    let temporary = TempDirectory::new("heredoc");
    let config = temporary.0.join("policy.yaml");
    std::fs::write(&config, "patterns: ['secrets/**']\n").unwrap();
    let mut input = input("codex", &temporary.0, "unused");
    input["tool_name"] = serde_json::json!("Bash");
    input["tool_input"] = serde_json::json!({
        "command": "apply_patch <<'PATCH'\n*** Add File: secrets/token.txt\n+token\nPATCH\n"
    });

    let output = run("codex", &config, &input);
    assert!(output.status.success());
    assert_eq!(decision(&output), "deny");
}

fn clear_hook_environment(command: &mut Command) {
    for name in [
        "CLAUDECODE",
        "CLAUDE_CODE_CHILD_SESSION",
        "CLAUDE_CODE_SESSION_ID",
        "CLAUDE_PROJECT_DIR",
        "CLAUDE_ENV_FILE",
        "CLAUDE_EFFORT",
        "TRACEPARENT",
        "CLAUDE_CODE_REMOTE",
        "CLAUDE_CODE_REMOTE_SESSION_ID",
        "CLAUDE_CODE_BRIDGE_SESSION_ID",
        "CLAUDE_PLUGIN_ROOT",
        "CLAUDE_PLUGIN_DATA",
        "PLUGIN_ROOT",
        "PLUGIN_DATA",
        "GEMINI_PROJECT_DIR",
        "GEMINI_PLANS_DIR",
        "GEMINI_CWD",
        "GEMINI_SESSION_ID",
    ] {
        command.env_remove(name);
    }
}
