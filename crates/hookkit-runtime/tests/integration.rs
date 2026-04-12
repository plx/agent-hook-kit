//! Integration tests for example hook executables.
//!
//! These tests build the example binaries, pipe fixture JSON into stdin,
//! and verify stdout/stderr/exit code behavior.

use std::collections::HashSet;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

fn workspace_root() -> String {
    // CARGO_MANIFEST_DIR points to crates/hookkit-runtime, go up two levels
    let manifest = env!("CARGO_MANIFEST_DIR");
    manifest.replace("/crates/hookkit-runtime", "").to_string()
}

fn fixture_bytes(harness: &str, name: &str) -> Vec<u8> {
    let path = format!("{}/fixtures/{harness}/{name}", workspace_root());
    std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read fixture {path}: {e}"))
}

fn run_example(binary: &str, fixture: &[u8], extra_args: &[&str]) -> std::process::Output {
    ensure_built(binary);
    let binary_path = format!("{}/target/debug/{binary}", workspace_root());
    Command::new(&binary_path)
        .args(extra_args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child.stdin.take().unwrap().write_all(fixture).unwrap();
            child.wait_with_output()
        })
        .unwrap_or_else(|e| panic!("failed to run {binary_path}: {e}"))
}

fn ensure_built(binary: &str) {
    static BUILT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let built = BUILT.get_or_init(|| Mutex::new(HashSet::new()));
    let mut guard = built.lock().expect("build lock poisoned");
    if guard.contains(binary) {
        return;
    }

    let status = Command::new("cargo")
        .args(["build", "-p", binary, "--bin", binary])
        .current_dir(workspace_root())
        .status()
        .expect("failed to start cargo build for example binary");
    assert!(status.success(), "failed to build example binary {binary}");

    guard.insert(binary.to_string());
}

// --- codex-bash-guard ---

#[test]
fn codex_bash_guard_allows_safe_command() {
    let fixture = serde_json::json!({
        "sessionId": "test",
        "cwd": "/tmp",
        "hookEventName": "PreToolUse",
        "toolName": "Bash",
        "toolInput": {"command": "cargo test"}
    });
    let output = run_example(
        "codex-bash-guard",
        &serde_json::to_vec(&fixture).unwrap(),
        &[],
    );
    assert!(output.status.success(), "should succeed for safe command");
}

#[test]
fn codex_bash_guard_denies_force_push() {
    let fixture = fixture_bytes("codex", "pre_tool_use.json");
    let output = run_example("codex-bash-guard", &fixture, &[]);
    assert!(output.status.success(), "deny should use JSON output on stdout");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("should be JSON");
    assert_eq!(json["decision"], "deny");
    assert!(json["reason"].as_str().unwrap().contains("Denied"));
}

#[test]
fn codex_bash_guard_ignores_non_pretool() {
    let fixture = fixture_bytes("codex", "session_start.json");
    let output = run_example("codex-bash-guard", &fixture, &[]);
    assert!(output.status.success(), "should pass for non-PreToolUse");
}

// --- gemini-beforetool-policy ---

#[test]
fn gemini_policy_denies_rm_rf() {
    let fixture = fixture_bytes("gemini", "before_tool.json");
    let output = run_example("gemini-beforetool-policy", &fixture, &[]);
    assert!(output.status.success(), "should exit 0 (deny is JSON)");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("should be JSON");
    assert_eq!(json["decision"], "deny");
}

#[test]
fn gemini_policy_allows_safe_command() {
    let fixture = serde_json::json!({
        "sessionId": "test",
        "cwd": "/tmp",
        "hookEventName": "BeforeTool",
        "toolName": "shell",
        "toolInput": {"command": ["cargo", "test"]}
    });
    let output = run_example(
        "gemini-beforetool-policy",
        &serde_json::to_vec(&fixture).unwrap(),
        &[],
    );
    assert!(output.status.success());
    // Should have empty stdout (allow)
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.trim().is_empty(), "stdout should be empty for allow");
}

// --- claude-sessionstart-context ---

#[test]
fn claude_context_injects_on_session_start() {
    let fixture = fixture_bytes("claude", "session_start.json");
    let output = run_example("claude-sessionstart-context", &fixture, &[]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("should be JSON");
    assert!(json["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .is_some());
    assert!(json["systemMessage"].as_str().is_some());
}

#[test]
fn claude_context_ignores_non_session_start() {
    let fixture = fixture_bytes("claude", "stop.json");
    let output = run_example("claude-sessionstart-context", &fixture, &[]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim().is_empty(),
        "should be empty for non-SessionStart"
    );
}

// --- shared-posttool-autofix ---

#[test]
fn shared_autofix_claude_clean_success_stays_quiet() {
    let fixture = fixture_bytes("claude", "post_tool_use.json");
    let output = run_example("shared-posttool-autofix", &fixture, &["--claude"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.trim().is_empty(), "clean path should stay quiet");
}

#[test]
fn shared_autofix_codex_stays_quiet() {
    let fixture = fixture_bytes("codex", "post_tool_use.json");
    let output = run_example("shared-posttool-autofix", &fixture, &["--codex"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Codex doesn't support context — should be quiet
    assert!(stdout.trim().is_empty());
}

#[test]
fn shared_autofix_claude_manual_mode_emits_user_and_agent_signals() {
    let fixture = serde_json::json!({
        "sessionId": "test-manual",
        "cwd": "/tmp",
        "hookEventName": "PostToolUse",
        "toolName": "Write",
        "toolInput": {
            "file_path": "/tmp/demo.rs",
            "content": "fn main() {}",
            "__hookkit_test_outcome": "manual"
        }
    });
    let output = run_example(
        "shared-posttool-autofix",
        &serde_json::to_vec(&fixture).unwrap(),
        &["--claude"],
    );
    assert!(output.status.success());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Diagnostics:"),
        "manual mode should print concise user status with artifact path"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("should be JSON");
    assert!(
        json["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("Manual fixes remain"),
        "manual mode should provide agent guidance"
    );
}

#[test]
fn shared_autofix_codex_manual_mode_stays_model_quiet() {
    let fixture = serde_json::json!({
        "sessionId": "test-manual-codex",
        "cwd": "/tmp",
        "hookEventName": "PostToolUse",
        "toolName": "Bash",
        "toolInput": {
            "command": "cargo clippy",
            "__hookkit_test_outcome": "manual"
        }
    });
    let output = run_example(
        "shared-posttool-autofix",
        &serde_json::to_vec(&fixture).unwrap(),
        &["--codex"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.trim().is_empty(), "codex manual mode should stay stdout-quiet");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Diagnostics:"));
}
