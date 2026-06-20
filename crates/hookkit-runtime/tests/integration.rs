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
    let (event, tool_response_key) = match harness {
        "claude" => ("PostToolUse", "tool_response"),
        "codex" => ("PostToolUse", "toolResult"),
        "gemini" => ("AfterTool", "toolResponse"),
        _ => panic!("unknown harness {harness}"),
    };
    let mut fixture = serde_json::json!({
        "sessionId": format!("{harness}-ruff-test"),
        "cwd": project.to_string_lossy(),
        "hookEventName": event,
        "toolName": "Write",
        "toolInput": {
            "file_path": rel_path,
            "content": "test fixture"
        }
    });
    fixture.as_object_mut().unwrap().insert(
        tool_response_key.to_string(),
        serde_json::json!({
            "filePath": project.join(rel_path).to_string_lossy()
        }),
    );
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

    let status = Command::new("cargo")
        .args(["build", "-p", package, "--bin", binary])
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
    assert!(
        output.status.success(),
        "deny should use JSON output on stdout"
    );
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
    assert!(
        json["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .is_some()
    );
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
    assert!(
        stdout.trim().is_empty(),
        "codex manual mode should stay stdout-quiet"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Diagnostics:"));
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
    assert!(
        String::from_utf8_lossy(&output.stdout).trim().is_empty(),
        "clean files should not send agent-visible output"
    );
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
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());
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
    assert!(String::from_utf8_lossy(&output.stdout).trim().is_empty());
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
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("run references unknown tool `rff`"));
}

#[test]
fn post_tool_use_codex_runs_but_cannot_emit_posttool_agent_context() {
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
    assert!(
        String::from_utf8_lossy(&output.stdout).trim().is_empty(),
        "current Codex PostToolUse model has no additionalContext output"
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
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unavailable"));
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
