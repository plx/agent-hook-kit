//! Integration tests for example hook executables.
//!
//! These tests build the example binaries, pipe fixture JSON into stdin,
//! and verify stdout/stderr/exit code behavior.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

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
                matches!(*argument, "--claude" | "--codex" | "--gemini")
                    .then(|| argument.trim_start_matches("--"))
            })
        })
        .or_else(|| binary.split_once('-').map(|(prefix, _)| prefix));

    let Some(harness @ ("claude" | "gemini")) = harness else {
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
        "gemini" => {
            command
                .env("GEMINI_PROJECT_DIR", project_dir)
                .env("GEMINI_PLANS_DIR", format!("{project_dir}/.gemini/plans"))
                .env("GEMINI_CWD", project_dir)
                .env("GEMINI_SESSION_ID", session_id)
                .env("CLAUDE_PROJECT_DIR", project_dir);
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
        "GEMINI_PROJECT_DIR",
        "GEMINI_PLANS_DIR",
        "GEMINI_CWD",
        "GEMINI_SESSION_ID",
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

fn write_executable(project: &Path, name: &str, body: &str) -> PathBuf {
    let bin_dir = project.join("bin");
    std::fs::create_dir_all(&bin_dir).expect("failed to create bin dir");
    let path = bin_dir.join(name);
    std::fs::write(&path, body).unwrap_or_else(|e| panic!("failed to write {name}: {e}"));

    #[cfg(unix)]
    {
        let mut perms = std::fs::metadata(&path)
            .unwrap_or_else(|e| panic!("{name} metadata: {e}"))
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap_or_else(|e| panic!("chmod {name}: {e}"));
    }

    path
}

fn write_fake_ruff(project: &Path) -> PathBuf {
    let bin_dir = project.join("bin");
    std::fs::create_dir_all(&bin_dir).expect("failed to create bin dir");
    let fake = bin_dir.join("ruff");
    std::fs::write(
        &fake,
        r#"#!/usr/bin/env bash
set -u
mode="${1:-}"
shift || true
file="${@: -1}"

if [[ "$mode" == "format" ]]; then
  if grep -q "format_crash" "$file"; then
    echo "format crashed" >&2
    exit 2
  fi
  if grep -q "needs_format" "$file"; then
    perl -0pi -e 's/needs_format/formatted/g' "$file"
    echo "1 file reformatted"
  else
    echo "1 file left unchanged"
  fi
  exit 0
fi

if [[ "$mode" == "check" ]]; then
  fix=0
  unfixable_f401=0
  prev=""
  for arg in "$@"; do
    if [[ "$arg" == "--fix" ]]; then
      fix=1
    fi
    if [[ "$prev" == "--unfixable" && "$arg" == "F401" ]]; then
      unfixable_f401=1
    fi
    prev="$arg"
  done

  if grep -q "manual_issue" "$file"; then
    echo "${file}:1:1: F821 undefined name manual_issue" >&2
    exit 1
  fi

  if grep -q "unused_import" "$file"; then
    if [[ "$fix" == "1" && "$unfixable_f401" == "0" ]]; then
      perl -0pi -e 's/^.*unused_import.*\n?//mg' "$file"
      echo "Found 1 error (1 fixed)"
      exit 0
    fi
    echo "${file}:1:1: F401 unused import" >&2
    exit 1
  fi

  echo "All checks passed!"
  exit 0
fi

echo "unknown fake ruff mode: $mode" >&2
exit 2
"#,
    )
    .expect("failed to write fake ruff");

    #[cfg(unix)]
    {
        let mut perms = std::fs::metadata(&fake)
            .expect("fake ruff metadata")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake, perms).expect("failed to chmod fake ruff");
    }

    fake
}

/// Write a Pkl config that wires the embedded ruff builtin to a custom
/// executable (typically a fake bash script). `extra_phase` is an optional
/// Pkl snippet inserted inside the `phases { ... }` block.
fn write_ruff_hook_config(project: &Path, fake_ruff: &Path, extra_phase: &str) {
    let config_dir = project.join(".agent-hook-kit");
    std::fs::create_dir_all(&config_dir).expect("failed to create config dir");
    let escaped = fake_ruff.to_string_lossy().replace('\\', "\\\\");
    let config = format!(
        r#"amends "Config.pkl"
import "Builtins.pkl"

settings {{
  diagnosticsDirectory = ".agent-hook-kit/ruff-agent-hook"
}}

tools {{
  ["ruff"] = (Builtins.ruff) {{
    executable = "{escaped}"
    phases {{
{extra_phase}
    }}
    messages {{
      issuesAgent = "fix {{{{ issue_files | join(\", \") }}}}; diagnostics {{{{ diagnostics_rel_path }}}}"
      issuesChangedAgent = "re-read {{{{ changed_files | join(\", \") }}}}, then fix {{{{ issue_files | join(\", \") }}}}; diagnostics {{{{ diagnostics_rel_path }}}}"
    }}
  }}
}}
run = new Listing {{ "ruff" }}
"#
    );
    std::fs::write(config_dir.join("post-tool-use.pkl"), config)
        .expect("failed to write post-tool-use.pkl");
}

fn post_tool_use_fixture(harness: &str, project: &Path, rel_path: &str) -> Vec<u8> {
    let fixture = match harness {
        "claude" => serde_json::json!({
            "session_id": "claude-ruff-test",
            "transcript_path": "/tmp/claude-ruff-test.jsonl",
            "cwd": project.to_string_lossy(),
            "hook_event_name": "PostToolUse",
            "tool_name": "Write",
            "tool_input": {
                "file_path": rel_path,
                "content": "test fixture"
            },
            "tool_use_id": "claude-ruff-tool",
            "tool_response": {
                "filePath": project.join(rel_path).to_string_lossy()
            }
        }),
        "codex" => serde_json::json!({
            "session_id": "codex-ruff-test",
            "transcript_path": "/tmp/codex-ruff-test.jsonl",
            "cwd": project.to_string_lossy(),
            "hook_event_name": "PostToolUse",
            "model": "gpt-test",
            "turn_id": "codex-ruff-turn",
            "permission_mode": "default",
            "tool_name": "Write",
            "tool_use_id": "codex-ruff-tool",
            "tool_input": {
                "file_path": rel_path,
                "content": "test fixture"
            },
            "tool_response": {
                "filePath": project.join(rel_path).to_string_lossy()
            }
        }),
        "gemini" => serde_json::json!({
            "session_id": "gemini-ruff-test",
            "transcript_path": "/tmp/gemini-ruff-test.json",
            "cwd": project.to_string_lossy(),
            "hook_event_name": "AfterTool",
            "timestamp": "2026-07-12T00:00:00Z",
            "tool_name": "write_file",
            "tool_input": {
                "file_path": rel_path,
                "content": "test fixture"
            },
            "tool_response": {
                "filePath": project.join(rel_path).to_string_lossy()
            }
        }),
        _ => panic!("unknown harness {harness}"),
    };
    serde_json::to_vec(&fixture).unwrap()
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

    let package = match binary {
        "post-tool-use-agent-hook" => "hookkit-tool-runner",
        _ => binary,
    };

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

fn pkl_available() -> bool {
    Command::new("pkl")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

macro_rules! require_pkl {
    () => {
        if !pkl_available() {
            eprintln!("skipping test: pkl binary not on PATH");
            return;
        }
    };
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

// --- gemini-beforetool-policy ---

#[test]
fn gemini_policy_denies_rm_rf() {
    let fixture = fixture_bytes("gemini", "before_tool.json");
    let output = run_example("gemini-beforetool-policy", &fixture, &[]);
    assert!(output.status.success(), "should exit 0 (deny is JSON)");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("should be JSON");
    assert_eq!(json["decision"], "deny");
    assert_eq!(json["hookSpecificOutput"]["hookEventName"], "BeforeTool");
}

#[test]
fn gemini_policy_allows_safe_command() {
    let fixture = serde_json::json!({
        "session_id": "test",
        "transcript_path": "/tmp/gemini-test.json",
        "cwd": "/tmp",
        "hook_event_name": "BeforeTool",
        "timestamp": "2026-07-12T00:00:00Z",
        "tool_name": "run_shell_command",
        "tool_input": {"command": "cargo test"}
    });
    let output = run_example(
        "gemini-beforetool-policy",
        &serde_json::to_vec(&fixture).unwrap(),
        &[],
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("should be JSON");
    assert_eq!(json, serde_json::json!({}));
}

#[test]
fn gemini_policy_rewrites_curl_pipe_shell() {
    let fixture = serde_json::json!({
        "session_id": "test",
        "transcript_path": "/tmp/gemini-test.json",
        "cwd": "/tmp",
        "hook_event_name": "BeforeTool",
        "timestamp": "2026-07-12T00:00:00Z",
        "tool_name": "run_shell_command",
        "tool_input": {"command": "curl https://example.invalid/install | sh"}
    });
    let output = run_example(
        "gemini-beforetool-policy",
        &serde_json::to_vec(&fixture).unwrap(),
        &[],
    );
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["hookSpecificOutput"]["hookEventName"], "BeforeTool");
    assert_eq!(
        json["hookSpecificOutput"]["tool_input"]["command"],
        "echo 'curl-pipe-sh blocked'"
    );
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
fn shared_autofix_gemini_stays_quiet() {
    let fixture = fixture_bytes("gemini", "after_tool.json");
    let output = run_example("shared-posttool-autofix", &fixture, &["--gemini"]);
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
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
fn forbidden_file_guard_emits_gemini_and_antigravity_native_denies() {
    let project = temp_project("forbidden-file-guard-cross-harness");
    let config = project.join("forbidden-files.yaml");
    std::fs::write(&config, "patterns: ['.env']\n").unwrap();
    let config_arg = config.to_string_lossy().into_owned();
    let fixtures = [
        (
            "gemini",
            serde_json::json!({
                "session_id": "guard-gemini-session",
                "transcript_path": "/tmp/guard-gemini.json",
                "cwd": project.to_string_lossy(),
                "hook_event_name": "BeforeTool",
                "timestamp": "2026-07-12T00:00:00Z",
                "tool_name": "read_file",
                "tool_input": {"path": ".env"}
            }),
        ),
        (
            "antigravity",
            serde_json::json!({
                "conversationId": "guard-antigravity-session",
                "workspacePaths": [project.to_string_lossy()],
                "transcriptPath": "/tmp/guard-antigravity.jsonl",
                "artifactDirectoryPath": "/tmp/guard-antigravity-artifacts",
                "toolCall": {"name": "read_file", "args": {"path": ".env"}},
                "stepIdx": 1
            }),
        ),
    ];

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

// --- session-modified-file-tracker ---

#[test]
fn session_modified_file_tracker_records_all_supported_posttool_paths() {
    let project = temp_project("session-modified-file-tracker");
    let state_dir = project.join("state");
    let state_arg = state_dir.to_string_lossy().into_owned();
    let harnesses = [
        ("claude", "claude-code", "claude-ruff-test"),
        ("codex", "codex", "codex-ruff-test"),
        ("gemini", "gemini-cli", "gemini-ruff-test"),
    ];

    for (harness, harness_id, session) in harnesses {
        let fixture = post_tool_use_fixture(harness, &project, "src/main.rs");
        let harness_arg = format!("--harness={harness}");
        let output = run_example(
            "session-modified-file-tracker",
            &fixture,
            &[harness_arg.as_str(), "--state-dir", state_arg.as_str()],
        );
        assert!(output.status.success(), "{harness} tracker should succeed");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
            serde_json::json!({})
        );

        let markers =
            std::fs::read_dir(state_dir.join(format!("{harness_id}/{session}/modified-files")))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
        assert_eq!(markers.len(), 1);
        assert_eq!(
            std::fs::read_to_string(markers[0].path()).unwrap(),
            format!("{}\n", project.join("src/main.rs").display())
        );
    }
}

// --- post-tool-use-agent-hook driven by Pkl configs ---

#[test]
fn post_tool_use_clean_python_file_is_quiet() {
    require_pkl!();
    let project = temp_project("ruff-clean");
    let fake_ruff = write_fake_ruff(&project);
    write_ruff_hook_config(&project, &fake_ruff, "");

    let src = project.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("clean.py"), "print('ok')\n").unwrap();

    let output = run_example(
        "post-tool-use-agent-hook",
        &post_tool_use_fixture("claude", &project, "src/clean.py"),
        &["--claude"],
    );

    assert!(output.status.success());
    let stdout: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("clean output should be JSON");
    assert_eq!(stdout, serde_json::json!({}));
    assert!(
        String::from_utf8_lossy(&output.stderr).trim().is_empty(),
        "clean unchanged files should stay quiet"
    );
}

#[test]
fn post_tool_use_autofix_sends_concise_agent_feedback_when_supported() {
    require_pkl!();
    let project = temp_project("ruff-autofix");
    let fake_ruff = write_fake_ruff(&project);
    write_ruff_hook_config(&project, &fake_ruff, "");

    let src = project.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("dirty.py"),
        "import os  # unused_import\nprint('needs_format')\n",
    )
    .unwrap();

    let output = run_example(
        "post-tool-use-agent-hook",
        &post_tool_use_fixture("claude", &project, "src/dirty.py"),
        &["--claude"],
    );

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("should be JSON");
    assert!(
        json["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("Ruff changed src/dirty.py")
    );
    let rewritten = std::fs::read_to_string(src.join("dirty.py")).unwrap();
    assert!(rewritten.contains("formatted"));
    assert!(!rewritten.contains("unused_import"));
}

#[test]
fn post_tool_use_manual_issues_write_diagnostics_and_render_template() {
    require_pkl!();
    let project = temp_project("ruff-manual");
    let fake_ruff = write_fake_ruff(&project);
    write_ruff_hook_config(&project, &fake_ruff, "");

    let src = project.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("broken.py"), "print(manual_issue)\n").unwrap();

    let output = run_example(
        "post-tool-use-agent-hook",
        &post_tool_use_fixture("gemini", &project, "src/broken.py"),
        &["--gemini"],
    );

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("should be JSON");
    let context = json["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(context.contains("fix src/broken.py"));
    assert!(context.contains(".agent-hook-kit/ruff-agent-hook"));

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("F821 undefined name manual_issue"));
    assert!(
        project
            .join(".agent-hook-kit/ruff-agent-hook/gemini-ruff-test_ruff-tool-issues.txt")
            .is_file()
    );
}

#[test]
fn post_tool_use_can_pass_phase_extra_args_for_unfixable_rules() {
    require_pkl!();
    let project = temp_project("ruff-unfixable");
    let fake_ruff = write_fake_ruff(&project);
    write_ruff_hook_config(
        &project,
        &fake_ruff,
        r#"      ["fix"] {
        extraArgs = new Listing<String> { "--unfixable"; "F401" }
      }
      ["verify"] {
        extraArgs = new Listing<String> { "--unfixable"; "F401" }
      }
"#,
    );

    let src = project.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("imports.py"), "import os  # unused_import\n").unwrap();

    let output = run_example(
        "post-tool-use-agent-hook",
        &post_tool_use_fixture("claude", &project, "src/imports.py"),
        &["--claude"],
    );

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("should be JSON");
    assert!(
        json["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("fix src/imports.py")
    );
    assert!(
        std::fs::read_to_string(src.join("imports.py"))
            .unwrap()
            .contains("unused_import")
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("F401 unused import"));
}

#[test]
fn post_tool_use_reports_changed_files_and_remaining_issues() {
    require_pkl!();
    let project = temp_project("ruff-changed-issues");
    let fake_ruff = write_fake_ruff(&project);
    write_ruff_hook_config(&project, &fake_ruff, "");

    let src = project.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("broken_dirty.py"),
        "print('needs_format')\nprint(manual_issue)\n",
    )
    .unwrap();

    let output = run_example(
        "post-tool-use-agent-hook",
        &post_tool_use_fixture("claude", &project, "src/broken_dirty.py"),
        &["--claude"],
    );

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("should be JSON");
    let context = json["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(context.contains("re-read src/broken_dirty.py"));
    assert!(context.contains("fix src/broken_dirty.py"));
    assert!(
        std::fs::read_to_string(src.join("broken_dirty.py"))
            .unwrap()
            .contains("formatted")
    );
}

#[test]
fn post_tool_use_reports_missing_tool_to_user_without_failing_hook() {
    require_pkl!();
    let project = temp_project("ruff-missing");
    write_ruff_hook_config(
        &project,
        &project.join("bin").join("definitely-missing-ruff"),
        "",
    );

    let src = project.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("dirty.py"), "print('needs_format')\n").unwrap();

    let output = run_example(
        "post-tool-use-agent-hook",
        &post_tool_use_fixture("claude", &project, "src/dirty.py"),
        &["--claude"],
    );

    assert!(output.status.success());
    let stdout: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("no-op output should be JSON");
    assert_eq!(stdout, serde_json::json!({}));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unavailable"));
    assert!(stderr.contains("definitely-missing-ruff"));
}

#[test]
fn post_tool_use_reports_tool_failure_with_diagnostics() {
    require_pkl!();
    let project = temp_project("ruff-failure");
    let fake_ruff = write_fake_ruff(&project);
    write_ruff_hook_config(&project, &fake_ruff, "");

    let src = project.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("crash.py"), "print('format_crash')\n").unwrap();

    let output = run_example(
        "post-tool-use-agent-hook",
        &post_tool_use_fixture("gemini", &project, "src/crash.py"),
        &["--gemini"],
    );

    assert!(output.status.success());
    let stdout: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("no-op output should be JSON");
    assert_eq!(stdout, serde_json::json!({}));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("phase `format` failed"));
    assert!(stderr.contains("format crashed"));
    assert!(
        project
            .join(".agent-hook-kit/ruff-agent-hook/gemini-ruff-test_ruff-tool-failure.txt")
            .is_file()
    );
}

#[test]
fn post_tool_use_reports_changes_made_before_later_phase_failure() {
    require_pkl!();
    let project = temp_project("changed-before-failure");
    let changer = write_executable(
        &project,
        "changer",
        r#"#!/usr/bin/env bash
file="${@: -1}"
printf "changed\n" >> "$file"
exit 0
"#,
    );
    let failer = write_executable(
        &project,
        "failer",
        r#"#!/usr/bin/env bash
echo "verify crashed" >&2
exit 2
"#,
    );

    let config_dir = project.join(".agent-hook-kit");
    std::fs::create_dir_all(&config_dir).unwrap();
    let changer = changer.to_string_lossy().replace('\\', "\\\\");
    let failer = failer.to_string_lossy().replace('\\', "\\\\");
    std::fs::write(
        config_dir.join("post-tool-use.pkl"),
        format!(
            r#"amends "Config.pkl"

settings {{
  diagnosticsDirectory = ".agent-hook-kit/post-tool-use"
}}

tools {{
  ["combo"] = new ToolSpec {{
    id = "combo"
    displayName = "Combo"
    executable = "{changer}"
    files {{ include = new Listing<String> {{ "*.py"; "**/*.py" }} }}
    phases {{
      ["format"] = new Phase {{
        mode = "format"
        program = "{changer}"
        argv = new Listing<String | ArgToken> {{ new Files {{}} }}
        writes = "target-files"
      }}
      ["verify"] = new Phase {{
        mode = "verify"
        program = "{failer}"
        argv = new Listing<String | ArgToken> {{ new Files {{}} }}
        exitCodes {{ clean = new Listing<Int> {{ 0 }}; failure = new Listing<Int> {{ 2 }} }}
      }}
    }}
    phaseOrder = new Listing<String> {{ "format"; "verify" }}
  }}
}}
run = new Listing<String> {{ "combo" }}
"#
        ),
    )
    .unwrap();

    let src = project.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.py"), "original\n").unwrap();

    let output = run_example(
        "post-tool-use-agent-hook",
        &post_tool_use_fixture("claude", &project, "src/a.py"),
        &["--claude"],
    );

    assert!(output.status.success());
    assert!(
        std::fs::read_to_string(src.join("a.py"))
            .unwrap()
            .contains("changed")
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("should be JSON");
    assert!(
        json["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("Combo changed src/a.py")
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Combo: phase `verify` failed"));
    assert!(stderr.contains("verify crashed"));
}

#[test]
fn post_tool_use_fail_fast_stops_after_operational_failure() {
    require_pkl!();
    let project = temp_project("fail-fast");
    let failer = write_executable(
        &project,
        "failer",
        r#"#!/usr/bin/env bash
echo "tool crashed" >&2
exit 2
"#,
    );
    let changer = write_executable(
        &project,
        "changer",
        r#"#!/usr/bin/env bash
file="${@: -1}"
printf "changed\n" >> "$file"
exit 0
"#,
    );

    let config_dir = project.join(".agent-hook-kit");
    std::fs::create_dir_all(&config_dir).unwrap();
    let failer = failer.to_string_lossy().replace('\\', "\\\\");
    let changer = changer.to_string_lossy().replace('\\', "\\\\");
    std::fs::write(
        config_dir.join("post-tool-use.pkl"),
        format!(
            r#"amends "Config.pkl"

settings {{
  failFast = true
  diagnosticsDirectory = ".agent-hook-kit/post-tool-use"
}}

tools {{
  ["failer"] = new ToolSpec {{
    id = "failer"
    displayName = "Failer"
    executable = "{failer}"
    files {{ include = new Listing<String> {{ "*.py"; "**/*.py" }} }}
    phases {{
      ["verify"] = new Phase {{
        mode = "verify"
        argv = new Listing<String | ArgToken> {{ new Files {{}} }}
        exitCodes {{ clean = new Listing<Int> {{ 0 }}; failure = new Listing<Int> {{ 2 }} }}
      }}
    }}
  }}
  ["changer"] = new ToolSpec {{
    id = "changer"
    displayName = "Changer"
    executable = "{changer}"
    files {{ include = new Listing<String> {{ "*.py"; "**/*.py" }} }}
    phases {{
      ["format"] = new Phase {{
        mode = "format"
        argv = new Listing<String | ArgToken> {{ new Files {{}} }}
        writes = "target-files"
      }}
    }}
  }}
}}
run = new Listing<String> {{ "failer"; "changer" }}
"#
        ),
    )
    .unwrap();

    let src = project.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.py"), "original\n").unwrap();

    let output = run_example(
        "post-tool-use-agent-hook",
        &post_tool_use_fixture("claude", &project, "src/a.py"),
        &["--claude"],
    );

    assert!(output.status.success());
    assert_eq!(
        std::fs::read_to_string(src.join("a.py")).unwrap(),
        "original\n"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Failer: phase `verify` failed"));
    assert!(!stderr.contains("Changer: changed"));
}

#[test]
fn post_tool_use_continue_after_issues_false_stops_later_tools() {
    require_pkl!();
    let project = temp_project("stop-after-issues");
    let issuer = write_executable(
        &project,
        "issuer",
        r#"#!/usr/bin/env bash
echo "${1}: issue" >&2
exit 1
"#,
    );
    let changer = write_executable(
        &project,
        "changer",
        r#"#!/usr/bin/env bash
file="${@: -1}"
printf "changed\n" >> "$file"
exit 0
"#,
    );

    let config_dir = project.join(".agent-hook-kit");
    std::fs::create_dir_all(&config_dir).unwrap();
    let issuer = issuer.to_string_lossy().replace('\\', "\\\\");
    let changer = changer.to_string_lossy().replace('\\', "\\\\");
    std::fs::write(
        config_dir.join("post-tool-use.pkl"),
        format!(
            r#"amends "Config.pkl"

settings {{
  continueAfterIssues = false
  diagnosticsDirectory = ".agent-hook-kit/post-tool-use"
}}

tools {{
  ["issuer"] = new ToolSpec {{
    id = "issuer"
    displayName = "Issuer"
    executable = "{issuer}"
    files {{ include = new Listing<String> {{ "*.py"; "**/*.py" }} }}
    phases {{
      ["verify"] = new Phase {{
        mode = "verify"
        argv = new Listing<String | ArgToken> {{ new Files {{}} }}
        exitCodes {{ clean = new Listing<Int> {{ 0 }}; issues = new Listing<Int> {{ 1 }} }}
      }}
    }}
  }}
  ["changer"] = new ToolSpec {{
    id = "changer"
    displayName = "Changer"
    executable = "{changer}"
    files {{ include = new Listing<String> {{ "*.py"; "**/*.py" }} }}
    phases {{
      ["format"] = new Phase {{
        mode = "format"
        argv = new Listing<String | ArgToken> {{ new Files {{}} }}
        writes = "target-files"
      }}
    }}
  }}
}}
run = new Listing<String> {{ "issuer"; "changer" }}
"#
        ),
    )
    .unwrap();

    let src = project.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.py"), "original\n").unwrap();

    let output = run_example(
        "post-tool-use-agent-hook",
        &post_tool_use_fixture("claude", &project, "src/a.py"),
        &["--claude"],
    );

    assert!(output.status.success());
    assert_eq!(
        std::fs::read_to_string(src.join("a.py")).unwrap(),
        "original\n"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Issuer: issues remain"));
    assert!(!stderr.contains("Changer: changed"));
}

#[test]
fn post_tool_use_unknown_run_entry_fails_hook() {
    require_pkl!();
    let project = temp_project("unknown-run-entry");
    let config_dir = project.join(".agent-hook-kit");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("post-tool-use.pkl"),
        r#"amends "Config.pkl"
run = new Listing<String> { "rff" }
"#,
    )
    .unwrap();

    let src = project.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.py"), "print('ok')\n").unwrap();

    let output = run_example(
        "post-tool-use-agent-hook",
        &post_tool_use_fixture("claude", &project, "src/a.py"),
        &["--claude"],
    );

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        output.stderr.is_empty(),
        "runtime diagnostics are disabled unless a sink is configured"
    );
}

#[test]
fn post_tool_use_codex_emits_posttool_agent_context() {
    require_pkl!();
    let project = temp_project("ruff-codex");
    let fake_ruff = write_fake_ruff(&project);
    write_ruff_hook_config(&project, &fake_ruff, "");

    let src = project.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("dirty.py"), "import os  # unused_import\n").unwrap();

    let output = run_example(
        "post-tool-use-agent-hook",
        &post_tool_use_fixture("codex", &project, "src/dirty.py"),
        &["--codex"],
    );

    assert!(output.status.success());
    let stdout: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("should emit structured JSON");
    assert_eq!(stdout["hookSpecificOutput"]["hookEventName"], "PostToolUse");
    assert!(
        stdout["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("Ruff changed src/dirty.py")
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("Ruff: changed src/dirty.py"));
}

#[test]
fn post_tool_use_hard_failure_policy_fails_the_hook() {
    require_pkl!();
    let project = temp_project("ruff-hard-failure");
    let config_dir = project.join(".agent-hook-kit");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("post-tool-use.pkl"),
        r#"amends "Config.pkl"
import "Builtins.pkl"

settings {
  missingToolPolicy = "hard-failure"
}

tools {
  ["ruff"] = (Builtins.ruff) {
    executable = "definitely-missing-ruff"
  }
}
run = new Listing { "ruff" }
"#,
    )
    .unwrap();

    let src = project.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("dirty.py"), "print('needs_format')\n").unwrap();

    let output = run_example(
        "post-tool-use-agent-hook",
        &post_tool_use_fixture("claude", &project, "src/dirty.py"),
        &["--claude"],
    );

    assert!(
        !output.status.success(),
        "hard-failure should fail the hook"
    );
    assert!(output.stdout.is_empty());
    assert!(
        output.stderr.is_empty(),
        "operational failures use the runtime diagnostics sink"
    );
}

#[test]
fn post_tool_use_harness_block_policy_emits_blocking_exit_code() {
    require_pkl!();
    let project = temp_project("ruff-harness-block");
    let config_dir = project.join(".agent-hook-kit");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("post-tool-use.pkl"),
        r#"amends "Config.pkl"
import "Builtins.pkl"

settings {
  missingToolPolicy = "harness-block"
}

tools {
  ["ruff"] = (Builtins.ruff) {
    executable = "definitely-missing-ruff"
  }
}
run = new Listing { "ruff" }
"#,
    )
    .unwrap();

    let src = project.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("dirty.py"), "print('needs_format')\n").unwrap();

    let output = run_example(
        "post-tool-use-agent-hook",
        &post_tool_use_fixture("claude", &project, "src/dirty.py"),
        &["--claude"],
    );

    assert_eq!(
        output.status.code(),
        Some(2),
        "harness-block should exit 2 (blocking) instead of 0 or 1"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unavailable"));
}
