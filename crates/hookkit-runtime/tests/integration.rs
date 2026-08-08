//! Integration tests for example hook executables.
//!
//! These tests build the example binaries, pipe fixture JSON into stdin,
//! and verify stdout/stderr/exit code behavior.

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

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
    let mut command = Command::new(&binary_path);
    command.args(extra_args);
    configure_hook_environment(&mut command, binary, fixture, extra_args);
    command
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

fn configure_hook_environment(
    command: &mut Command,
    binary: &str,
    fixture: &[u8],
    extra_args: &[&str],
) {
    clear_modeled_hook_environment(command);
    let harness = extra_args
        .iter()
        .find_map(|argument| {
            argument.strip_prefix("--harness=").or_else(|| {
                matches!(*argument, "--claude" | "--codex" | "--antigravity")
                    .then(|| argument.trim_start_matches("--"))
            })
        })
        .or_else(|| binary.split_once('-').map(|(prefix, _)| prefix));

    let Some(harness @ "claude") = harness else {
        return;
    };
    let input: serde_json::Value = serde_json::from_slice(fixture)
        .unwrap_or_else(|error| panic!("invalid {harness} integration fixture: {error}"));
    let field = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| input.get(*name).and_then(serde_json::Value::as_str))
            .unwrap_or_else(|| {
                panic!(
                    "{harness} integration fixture is missing string field {}",
                    names.join(" or ")
                )
            })
    };
    let session_id = field(&["session_id", "sessionId"]);
    let project_dir = field(&["cwd"]);

    match harness {
        "claude" => {
            command
                .env("CLAUDECODE", "1")
                .env("CLAUDE_CODE_CHILD_SESSION", "1")
                .env("CLAUDE_CODE_SESSION_ID", session_id)
                .env("CLAUDE_PROJECT_DIR", project_dir);

            let event = field(&["hook_event_name", "hookEventName"]);
            if matches!(
                event,
                "SessionStart" | "Setup" | "CwdChanged" | "FileChanged"
            ) {
                command.env("CLAUDE_ENV_FILE", format!("{project_dir}/.claude-hook-env"));
            }
        }
        _ => unreachable!(),
    }
}

fn clear_modeled_hook_environment(command: &mut Command) {
    const EXACT_NAMES: &[&str] = &[
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
    ];
    for name in EXACT_NAMES {
        command.env_remove(name);
    }
    for name in std::env::vars_os().filter_map(|(name, _)| name.into_string().ok()) {
        if name.starts_with("CLAUDE_PLUGIN_OPTION_") {
            command.env_remove(name);
        }
    }
}

fn temp_project(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("hookkit-{name}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&path).expect("failed to create temp project");
    path
}

fn ensure_built(binary: &str) {
    static BUILT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let built = BUILT.get_or_init(|| Mutex::new(HashSet::new()));
    let mut guard = match built.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    if guard.contains(binary) {
        return;
    }

    let package = binary;

    let mut command = Command::new("cargo");
    command.args(["build", "-p", package, "--bin", binary]);
    if package == "shared-posttool-autofix" {
        command.args(["--features", "test-support"]);
    }
    let status = command
        .current_dir(workspace_root())
        .status()
        .expect("failed to start cargo build for example binary");
    assert!(status.success(), "failed to build example binary {binary}");

    guard.insert(binary.to_string());
}

#[test]
fn aligned_pre_tool_stdin_helper() {
    if std::env::var_os("HOOKKIT_ALIGNED_PRE_TOOL_STDIN_HELPER").is_none() {
        return;
    }

    let code = hookkit_runtime::aligned::run_aligned_event::<hookkit_runtime::aligned::PreToolUse, _>(
        hookkit_core::HarnessId::CODEX,
        |_, _, context| {
            hookkit_common::PreToolUseOutput::deny(context.harness(), "blocked through stdin")
        },
    );
    std::process::exit(if code == std::process::ExitCode::SUCCESS {
        0
    } else {
        1
    });
}

#[test]
fn aligned_pre_tool_run_path_reads_stdin_and_writes_native_stdout() {
    let fixture = serde_json::json!({
        "session_id": "stdin-session",
        "transcript_path": null,
        "cwd": "/tmp",
        "hook_event_name": "PreToolUse",
        "model": "gpt-test",
        "turn_id": "stdin-turn",
        "permission_mode": "default",
        "tool_name": "Read",
        "tool_use_id": "stdin-call",
        "tool_input": {"path": ".env"}
    });
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "aligned_pre_tool_stdin_helper", "--nocapture"])
        .env("HOOKKIT_ALIGNED_PRE_TOOL_STDIN_HELPER", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    clear_modeled_hook_environment(&mut command);

    let output = command
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(&serde_json::to_vec(&fixture).unwrap())?;
            child.wait_with_output()
        })
        .expect("aligned stdin helper should run");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(
        r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"blocked through stdin"}}"#
    ));
}

// --- codex-bash-guard ---

#[test]
fn codex_bash_guard_allows_safe_command() {
    let fixture = serde_json::json!({
        "session_id": "test",
        "transcript_path": null,
        "cwd": "/tmp",
        "hook_event_name": "PreToolUse",
        "model": "gpt-test",
        "turn_id": "turn-test",
        "permission_mode": "default",
        "tool_name": "Bash",
        "tool_use_id": "call-test",
        "tool_input": {"command": "cargo test"}
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
    assert!(
        output.status.success(),
        "deny should use JSON output on stdout"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("should be JSON");
    let specific = &json["hookSpecificOutput"];
    assert_eq!(specific["hookEventName"], "PreToolUse");
    assert_eq!(specific["permissionDecision"], "deny");
    assert!(
        specific["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .contains("Denied")
    );
}

#[test]
fn codex_bash_guard_rejects_non_pretool() {
    let fixture = fixture_bytes("codex", "session_start.json");
    let output = run_example("codex-bash-guard", &fixture, &[]);
    assert!(
        !output.status.success(),
        "typed hook must reject a wrong event"
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

// --- antigravity-pre-invocation ---

#[test]
fn antigravity_pre_invocation_runs_contract_fixture() {
    let fixture = fixture_bytes("antigravity", "pre_invocation.json");
    let output = run_example("antigravity-pre-invocation", &fixture, &[]);
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        json["injectSteps"][0]["ephemeralMessage"]
            .as_str()
            .unwrap()
            .contains("workspace root")
    );
}

// --- claude-sessionstart-context ---

#[test]
fn claude_context_injects_on_session_start() {
    let fixture = fixture_bytes("claude", "session_start.json");
    let output = run_example("claude-sessionstart-context", &fixture, &[]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("should be JSON");
    assert!(
        json["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .is_some()
    );
    assert!(json["systemMessage"].as_str().is_some());
}

#[test]
fn claude_context_rejects_non_session_start() {
    let fixture = fixture_bytes("claude", "stop.json");
    let output = run_example("claude-sessionstart-context", &fixture, &[]);
    assert!(
        !output.status.success(),
        "typed hook must reject a wrong event"
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

// --- shared-posttool-autofix ---

#[test]
fn shared_autofix_claude_clean_success_stays_quiet() {
    let fixture = fixture_bytes("claude", "post_tool_use.json");
    let output = run_example("shared-posttool-autofix", &fixture, &["--claude"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("should be JSON");
    assert_eq!(json, serde_json::json!({}));
}

#[test]
fn shared_autofix_codex_stays_quiet() {
    let fixture = fixture_bytes("codex", "post_tool_use.json");
    let output = run_example("shared-posttool-autofix", &fixture, &["--codex"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("should be JSON");
    assert_eq!(json, serde_json::json!({}));
}

#[test]
fn shared_autofix_claude_manual_mode_emits_user_and_agent_signals() {
    let fixture = serde_json::json!({
        "session_id": "test-manual",
        "transcript_path": "/tmp/test-manual.jsonl",
        "cwd": "/tmp",
        "hook_event_name": "PostToolUse",
        "tool_name": "Write",
        "tool_input": {
            "file_path": "/tmp/demo.rs",
            "content": "fn main() {}",
            "__hookkit_test_outcome": "manual"
        },
        "tool_use_id": "call-manual",
        "tool_response": {"filePath": "/tmp/demo.rs"}
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
fn shared_autofix_codex_manual_mode_emits_agent_context() {
    let fixture = serde_json::json!({
        "session_id": "test-manual-codex",
        "transcript_path": null,
        "cwd": "/tmp",
        "hook_event_name": "PostToolUse",
        "model": "gpt-test",
        "turn_id": "turn-manual-codex",
        "permission_mode": "default",
        "tool_name": "Bash",
        "tool_use_id": "call-manual-codex",
        "tool_input": {
            "command": "cargo clippy",
            "__hookkit_test_outcome": "manual"
        },
        "tool_response": {"exit_code": 1}
    });
    let output = run_example(
        "shared-posttool-autofix",
        &serde_json::to_vec(&fixture).unwrap(),
        &["--codex"],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("should be JSON");
    assert!(
        json["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("Manual fixes remain")
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Diagnostics:"));
}

// --- codex-claude-rules ---

#[test]
fn codex_claude_rules_injects_each_matching_rule_once() {
    let project = temp_project("codex-claude-rules");
    let rules_dir = project.join(".claude/rules");
    let state_dir = project.join("state");
    std::fs::create_dir_all(&rules_dir).unwrap();
    std::fs::write(
        rules_dir.join("rust.md"),
        "---\npaths: '**/*.rs'\n---\nUse the repository's Rust conventions.\n",
    )
    .unwrap();
    let fixture = serde_json::to_vec(&serde_json::json!({
        "session_id": "rules-session",
        "transcript_path": null,
        "cwd": project.to_string_lossy(),
        "hook_event_name": "PreToolUse",
        "model": "gpt-test",
        "turn_id": "rules-turn",
        "permission_mode": "default",
        "tool_name": "apply_patch",
        "tool_use_id": "rules-call",
        "tool_input": {"patch": "*** Update File: src/lib.rs\n"}
    }))
    .unwrap();
    let project_arg = project.to_string_lossy().into_owned();
    let empty_home_arg = project
        .join("empty-claude-home")
        .to_string_lossy()
        .into_owned();
    let state_arg = state_dir.to_string_lossy().into_owned();
    let args = [
        "--project-root",
        project_arg.as_str(),
        "--claude-home",
        empty_home_arg.as_str(),
        "--state-dir",
        state_arg.as_str(),
    ];

    let first = run_example("codex-claude-rules", &fixture, &args);
    assert!(first.status.success());
    let json: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert!(
        json["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("Use the repository's Rust conventions")
    );

    let second = run_example("codex-claude-rules", &fixture, &args);
    assert!(second.status.success());
    assert!(
        second.stdout.is_empty(),
        "the rule must not be injected twice"
    );
}

// --- forbidden-file-guard ---

#[test]
fn forbidden_file_guard_emits_codex_native_deny() {
    let project = temp_project("forbidden-file-guard");
    let config = project.join("forbidden-files.yaml");
    std::fs::write(&config, "patterns: ['.env', '**/.env']\n").unwrap();
    let fixture = serde_json::to_vec(&serde_json::json!({
        "session_id": "guard-session",
        "transcript_path": null,
        "cwd": project.to_string_lossy(),
        "hook_event_name": "PreToolUse",
        "model": "gpt-test",
        "turn_id": "guard-turn",
        "permission_mode": "default",
        "tool_name": "mcp__filesystem__read_file",
        "tool_use_id": "guard-call",
        "tool_input": {"path": ".env"}
    }))
    .unwrap();
    let config_arg = config.to_string_lossy().into_owned();
    let output = run_example(
        "forbidden-file-guard",
        &fixture,
        &["--harness=codex", "--config", config_arg.as_str()],
    );
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["hookSpecificOutput"]["permissionDecision"], "deny");
}

#[test]
fn forbidden_file_guard_emits_antigravity_native_deny() {
    let project = temp_project("forbidden-file-guard-cross-harness");
    let config = project.join("forbidden-files.yaml");
    std::fs::write(&config, "patterns: ['.env']\n").unwrap();
    let config_arg = config.to_string_lossy().into_owned();
    let fixtures = [(
        "antigravity",
        serde_json::json!({
            "conversationId": "guard-antigravity-session",
            "workspacePaths": [project.to_string_lossy()],
            "transcriptPath": "/tmp/guard-antigravity.jsonl",
            "artifactDirectoryPath": "/tmp/guard-antigravity-artifacts",
            "toolCall": {"name": "read_file", "args": {"path": ".env"}},
            "stepIdx": 1
        }),
    )];

    for (harness, fixture) in fixtures {
        let harness_arg = format!("--harness={harness}");
        let output = run_example(
            "forbidden-file-guard",
            &serde_json::to_vec(&fixture).unwrap(),
            &[harness_arg.as_str(), "--config", config_arg.as_str()],
        );
        assert!(output.status.success(), "{harness} guard should succeed");
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["decision"], "deny", "{harness} should deny");
    }
}
