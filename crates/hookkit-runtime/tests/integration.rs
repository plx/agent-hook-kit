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

fn spawn_example(binary: &str, fixture: &[u8], extra_args: &[&str]) -> std::process::Child {
    use std::io::Write;

    ensure_built(binary);
    let binary_path = format!("{}/target/debug/{binary}", workspace_root());
    let mut command = Command::new(&binary_path);
    command.args(extra_args);
    configure_hook_environment(&mut command, binary, fixture, extra_args);
    let mut child = command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("failed to run {binary_path}: {error}"));
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(fixture)
        .expect("write fixture");
    child
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
                matches!(
                    *argument,
                    "--claude" | "--codex" | "--gemini" | "--antigravity"
                )
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

fn wait_for_path(path: &Path) {
    for _ in 0..500 {
        if path.exists() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("timed out waiting for {}", path.display());
}

fn run_git(project: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(project)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
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
  check=0
  for arg in "$@"; do
    if [[ "$arg" == "--check" ]]; then
      check=1
    fi
  done
  if grep -q "format_crash" "$file"; then
    echo "format crashed" >&2
    exit 2
  fi
  if grep -q "needs_format" "$file"; then
    if [[ "$check" == "1" ]]; then
      echo "Would reformat: $file"
      exit 1
    else
      perl -0pi -e 's/needs_format/formatted/g' "$file"
      echo "1 file reformatted"
    fi
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

  if grep -q "wait_for_release" "$file"; then
    : > "${file}.started"
    for _ in $(seq 1 1000); do
      [[ -e "${file}.release" ]] && break
      sleep 0.01
    done
    [[ -e "${file}.release" ]] || exit 2
  fi

  if grep -q "check_crash" "$file"; then
    echo "check crashed" >&2
    exit 2
  fi

  if grep -q "manual_issue" "$file"; then
    if grep -q "large_diagnostic" "$file"; then
      head -c 131072 /dev/zero | tr '\0' x >&2
      echo >&2
    fi
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

fn add_deferred_reporting_config(project: &Path, reporting_body: &str) {
    let path = project.join(".agent-hook-kit/post-tool-use.pkl");
    let config = std::fs::read_to_string(&path).expect("read generated hook config");
    let replacement = format!(
        "settings {{\n  deferredReporting = new DeferredReporting {{\n{reporting_body}\n  }}"
    );
    let config = config.replacen("settings {", &replacement, 1);
    std::fs::write(path, config).expect("write deferred reporting config");
}

fn add_runner_setting(project: &Path, setting: &str) {
    let path = project.join(".agent-hook-kit/post-tool-use.pkl");
    let config = std::fs::read_to_string(&path).expect("read generated hook config");
    let replacement = format!("settings {{\n  {setting}");
    let config = config.replacen("settings {", &replacement, 1);
    std::fs::write(path, config).expect("write runner setting");
}

fn replace_file_activity_settings(project: &Path, body: &str) {
    let path = project.join(".agent-hook-kit/post-tool-use.pkl");
    let config = std::fs::read_to_string(&path).expect("read generated hook config");
    let config = config.replacen(
        "fileActivity { filesystemMtime = false }",
        &format!("fileActivity {{ {body} }}"),
        1,
    );
    std::fs::write(path, config).expect("write file activity settings");
}

fn write_per_file_ruff_hook_config(project: &Path, fake_ruff: &Path) {
    let config_dir = project.join(".agent-hook-kit");
    std::fs::create_dir_all(&config_dir).expect("failed to create config dir");
    let escaped = fake_ruff.to_string_lossy().replace('\\', "\\\\");
    let config = format!(
        r#"amends "Config.pkl"
import "Builtins.pkl"

settings {{
  fileActivity {{ filesystemMtime = false }}
}}

tools {{
  ["ruff"] = (Builtins.ruff) {{
    executable = "{escaped}"
    workflows {{
      ["lint"] = new Workflow {{
        check = new WorkflowCommand {{
          argv = new Listing {{ "check"; new Files {{}} }}
          exitCodes {{
            clean = new Listing {{ 0 }}
            issues = new Listing {{ 1 }}
            failure = new Listing {{ 2 }}
          }}
        }}
        remedy = new WorkflowCommand {{
          argv = new Listing {{ "check"; "--fix"; new Files {{}} }}
          exitCodes {{
            clean = new Listing {{ 0 }}
            issues = new Listing {{ 1 }}
            failure = new Listing {{ 2 }}
          }}
          writes = "target-files"
        }}
        invocation = "per-file"
      }}
    }}
    workflowOrder = new Listing {{ "lint" }}
  }}
}}
run = new Listing {{ "ruff" }}
"#
    );
    std::fs::write(config_dir.join("post-tool-use.pkl"), config)
        .expect("failed to write post-tool-use.pkl");
}

fn write_selective_operational_hook_config(project: &Path, fake_ruff: &Path) {
    let config_dir = project.join(".agent-hook-kit");
    std::fs::create_dir_all(&config_dir).expect("failed to create config dir");
    let clean_executable = fake_ruff.to_string_lossy().replace('\\', "\\\\");
    let missing_executable = project
        .join("bin/definitely-missing-checker")
        .to_string_lossy()
        .replace('\\', "\\\\");
    let config = format!(
        r#"amends "Config.pkl"

settings {{
  fileActivity {{ filesystemMtime = false }}
}}

tools {{
  ["clean-python"] = new ToolSpec {{
    id = "clean-python"
    displayName = "Clean Python"
    executable = "{clean_executable}"
    files {{ include = new Listing {{ "**/*.py" }} }}
    workflows {{
      ["lint"] = new Workflow {{
        check = new WorkflowCommand {{
          argv = new Listing {{ "check"; new Files {{}} }}
          exitCodes {{ issues = new Listing {{ 1 }}; failure = new Listing {{ 2 }} }}
        }}
        invocation = "per-file"
      }}
    }}
    workflowOrder = new Listing {{ "lint" }}
  }}
  ["missing-rust"] = new ToolSpec {{
    id = "missing-rust"
    displayName = "Missing Rust"
    executable = "{missing_executable}"
    files {{ include = new Listing {{ "**/*.rs" }} }}
    workflows {{
      ["lint"] = new Workflow {{
        check = new WorkflowCommand {{
          argv = new Listing {{ "check"; new Files {{}} }}
          exitCodes {{ issues = new Listing {{ 1 }}; failure = new Listing {{ 2 }} }}
        }}
        invocation = "per-file"
      }}
    }}
    workflowOrder = new Listing {{ "lint" }}
  }}
}}
run = new Listing {{ "clean-python"; "missing-rust" }}
"#
    );
    std::fs::write(config_dir.join("post-tool-use.pkl"), config)
        .expect("failed to write post-tool-use.pkl");
}

fn write_artifact_linking_hook_config(
    project: &Path,
    fake_ruff: &Path,
    tool_ids: &[&str],
    invocation: &str,
) {
    let config_dir = project.join(".agent-hook-kit");
    std::fs::create_dir_all(&config_dir).expect("failed to create config dir");
    let executable = fake_ruff.to_string_lossy().replace('\\', "\\\\");
    let tools = tool_ids
        .iter()
        .map(|id| {
            format!(
                r#"  ["{id}"] = new ToolSpec {{
    id = "{id}"
    displayName = "{id}"
    executable = "{executable}"
    files {{ include = new Listing {{ "**/*.py" }} }}
    workflows {{
      ["lint"] = new Workflow {{
        check = new WorkflowCommand {{
          argv = new Listing {{ "check"; new Files {{}} }}
          exitCodes {{ issues = new Listing {{ 1 }}; failure = new Listing {{ 2 }} }}
        }}
        invocation = "{invocation}"
      }}
    }}
    workflowOrder = new Listing {{ "lint" }}
  }}"#
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let run = tool_ids
        .iter()
        .map(|id| format!("\"{id}\""))
        .collect::<Vec<_>>()
        .join("; ");
    let config = format!(
        r#"amends "Config.pkl"

settings {{ fileActivity {{ filesystemMtime = false }} }}

tools {{
{tools}
}}
run = new Listing {{ {run} }}
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

fn codex_post_tool_case(
    project: &Path,
    tool_name: &str,
    tool_use_id: &str,
    tool_input: serde_json::Value,
) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "session_id": "codex-ruff-test",
        "transcript_path": "/tmp/codex-ruff-test.jsonl",
        "cwd": project.to_string_lossy(),
        "hook_event_name": "PostToolUse",
        "model": "gpt-test",
        "turn_id": "codex-ruff-turn",
        "permission_mode": "default",
        "tool_name": tool_name,
        "tool_use_id": tool_use_id,
        "tool_input": tool_input,
        "tool_response": {"exit_code": 0}
    }))
    .unwrap()
}

fn turn_completion_fixture(harness: &str, project: &Path) -> Vec<u8> {
    let fixture = match harness {
        "claude" => serde_json::json!({
            "session_id": "claude-ruff-test",
            "transcript_path": "/tmp/claude-ruff-test.jsonl",
            "cwd": project.to_string_lossy(),
            "hook_event_name": "Stop",
            "stop_hook_active": false,
            "last_assistant_message": "done"
        }),
        "codex" => serde_json::json!({
            "session_id": "codex-ruff-test",
            "transcript_path": "/tmp/codex-ruff-test.jsonl",
            "cwd": project.to_string_lossy(),
            "hook_event_name": "Stop",
            "model": "gpt-test",
            "turn_id": "codex-ruff-turn",
            "permission_mode": "default",
            "stop_hook_active": false,
            "last_assistant_message": "done"
        }),
        "gemini" => serde_json::json!({
            "session_id": "gemini-ruff-test",
            "transcript_path": "/tmp/gemini-ruff-test.json",
            "cwd": project.to_string_lossy(),
            "hook_event_name": "AfterAgent",
            "timestamp": "2026-07-15T00:00:00Z",
            "prompt": "do it",
            "prompt_response": "done",
            "stop_hook_active": false
        }),
        "antigravity" => serde_json::json!({
            "conversationId": "antigravity-ruff-test",
            "workspacePaths": [project.to_string_lossy()],
            "transcriptPath": project.join("antigravity-transcript.jsonl").to_string_lossy(),
            "artifactDirectoryPath": project.join("antigravity-artifacts").to_string_lossy(),
            "executionNum": 1,
            "terminationReason": "agent-finished",
            "fullyIdle": true
        }),
        _ => panic!("unknown harness {harness}"),
    };
    serde_json::to_vec(&fixture).unwrap()
}

fn seed_pending_file(state_dir: &Path, harness: &str, path: &Path) {
    seed_pending_target(
        state_dir,
        harness,
        hookkit_file_activity::FileActivityTarget::exact(
            hookkit_core::Utf8PathBuf::from_path_buf(path.to_path_buf()).unwrap(),
        ),
    );
}

fn seed_pending_target(
    state_dir: &Path,
    harness: &str,
    target: hookkit_file_activity::FileActivityTarget,
) {
    let (harness, identity) = match harness {
        "claude" => (
            hookkit_core::HarnessId::CLAUDE_CODE,
            hookkit_session_state::SessionIdentity::Session("claude-ruff-test".into()),
        ),
        "codex" => (
            hookkit_core::HarnessId::CODEX,
            hookkit_session_state::SessionIdentity::Session("codex-ruff-test".into()),
        ),
        "gemini" => (
            hookkit_core::HarnessId::GEMINI_CLI,
            hookkit_session_state::SessionIdentity::Session("gemini-ruff-test".into()),
        ),
        "antigravity" => (
            hookkit_core::HarnessId::ANTIGRAVITY,
            hookkit_session_state::SessionIdentity::Conversation("antigravity-ruff-test".into()),
        ),
        _ => panic!("unknown harness {harness}"),
    };
    let state = hookkit_session_state::SessionState::open(
        harness,
        identity,
        hookkit_session_state::StateRoot::new(state_dir),
    )
    .unwrap();
    let store = hookkit_file_activity::FileActivityStore::from_state(state).unwrap();
    store.requeue_targets("integration-test", [target]).unwrap();
}

fn prepare_deferred_ruff_case(
    harness: &str,
    name: &str,
    files: &[(&str, &str)],
) -> (PathBuf, PathBuf, String) {
    let project = temp_project(&format!("{name}-{harness}"));
    let state_dir = project.join("state");
    let state_arg = state_dir.to_string_lossy().into_owned();
    let fake_ruff = write_fake_ruff(&project);
    write_per_file_ruff_hook_config(&project, &fake_ruff);
    for (relative, contents) in files {
        let path = project.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        seed_pending_file(&state_dir, harness, &path);
    }
    (project, state_dir, state_arg)
}

fn run_deferred_case(harness: &str, project: &Path, state_arg: &str) -> std::process::Output {
    let harness_arg = format!("--{harness}");
    run_example(
        "turn-completion-agent-hook",
        &turn_completion_fixture(harness, project),
        &[harness_arg.as_str(), "--state-dir", state_arg],
    )
}

fn only_summary(state_dir: &Path) -> serde_json::Value {
    let summaries = files_named(state_dir, "summary.json");
    assert_eq!(summaries.len(), 1, "expected exactly one deferred summary");
    serde_json::from_slice(&std::fs::read(&summaries[0]).unwrap()).unwrap()
}

fn session_journal_len(state_dir: &Path, harness: &str, session: &str) -> usize {
    hookkit_session_state::SessionState::open(
        hookkit_core::HarnessId::new(harness).unwrap(),
        hookkit_session_state::SessionIdentity::Session(session.into()),
        hookkit_session_state::StateRoot::new(state_dir),
    )
    .map_err(hookkit_file_activity::FileActivityError::from)
    .and_then(hookkit_file_activity::FileActivityStore::from_state)
    .and_then(|store| {
        store
            .pending()
            .with_entity(|view| {
                Ok(hookkit_session_state::EntityOutcome::retain(
                    view.events().len(),
                ))
            })
            .map_err(Into::into)
    })
    .unwrap()
}

fn files_named(root: &Path, name: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return found;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files_named(&path, name));
        } else if path.file_name().and_then(|value| value.to_str()) == Some(name) {
            found.push(path);
        }
    }
    found.sort();
    found
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
        "post-tool-use-agent-hook"
        | "file-activity-agent-hook"
        | "turn-completion-agent-hook"
        | "session-start-state-agent-hook" => "hookkit-tool-runner",
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
fn file_activity_agent_hook_records_all_supported_posttool_paths() {
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
            "file-activity-agent-hook",
            &fixture,
            &[harness_arg.as_str(), "--state-dir", state_arg.as_str()],
        );
        assert!(output.status.success(), "{harness} tracker should succeed");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
            serde_json::json!({})
        );

        let state = hookkit_session_state::SessionState::open(
            hookkit_core::HarnessId::new(harness_id).unwrap(),
            hookkit_session_state::SessionIdentity::Session(session.into()),
            hookkit_session_state::StateRoot::new(&state_dir),
        )
        .unwrap();
        let store = hookkit_file_activity::FileActivityStore::from_state(state).unwrap();
        store
            .pending()
            .with_entity(|view| {
                assert_eq!(view.events().len(), 1);
                assert!(
                    view.state().targets().contains(
                        &hookkit_file_activity::FileActivityTarget::exact(
                            hookkit_core::Utf8PathBuf::from_path_buf(project.join("src/main.rs"))
                                .unwrap()
                        )
                    )
                );
                Ok(hookkit_session_state::EntityOutcome::retain(()))
            })
            .unwrap();
    }

    let compatibility = run_example(
        "session-modified-file-tracker",
        &post_tool_use_fixture("codex", &project, "src/compatibility.rs"),
        &["--harness=codex", "--state-dir", state_arg.as_str()],
    );
    assert!(compatibility.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&compatibility.stdout).unwrap(),
        serde_json::json!({})
    );
}

#[test]
fn file_activity_observer_persists_shared_writer_patch_shell_and_gap_analysis_quietly() {
    let project = temp_project("file-activity-shared-analysis");
    let state_dir = project.join("state");
    let state_arg = state_dir.to_string_lossy().into_owned();
    let cases = [
        (
            "Write",
            "writer",
            serde_json::json!({"file_path": "src/writer.rs", "content": "fn main() {}"}),
        ),
        (
            "apply_patch",
            "patch",
            serde_json::json!({
                "patch": "*** Begin Patch\n*** Update File: src/patched.rs\n@@\n-old\n+new\n*** End Patch"
            }),
        ),
        (
            "Bash",
            "shell",
            serde_json::json!({"command": "echo ok > src/shell.txt"}),
        ),
        (
            "Read",
            "read-only",
            serde_json::json!({"file_path": "src/read-only.rs"}),
        ),
        (
            "Bash",
            "dynamic-gap",
            serde_json::json!({"command": "echo ok > src/known.txt; mystery $TARGET"}),
        ),
    ];
    for (tool, id, input) in cases {
        let output = run_example(
            "file-activity-agent-hook",
            &codex_post_tool_case(&project, tool, id, input),
            &["--codex", "--state-dir", state_arg.as_str()],
        );
        assert!(
            output.status.success(),
            "{id}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
            serde_json::json!({})
        );
        assert!(output.stderr.is_empty(), "{id} observer must remain quiet");
    }

    let state = hookkit_session_state::SessionState::open(
        hookkit_core::HarnessId::CODEX,
        hookkit_session_state::SessionIdentity::Session("codex-ruff-test".into()),
        hookkit_session_state::StateRoot::new(&state_dir),
    )
    .unwrap();
    let store = hookkit_file_activity::FileActivityStore::from_state(state).unwrap();
    store
        .pending()
        .with_entity(|view| {
            let targets = view.state().targets();
            for relative in [
                "src/writer.rs",
                "src/patched.rs",
                "src/shell.txt",
                "src/known.txt",
            ] {
                assert!(
                    targets.contains(&hookkit_file_activity::FileActivityTarget::exact(
                        hookkit_core::Utf8PathBuf::from_path_buf(project.join(relative)).unwrap()
                    ))
                );
            }
            assert!(
                !targets.contains(&hookkit_file_activity::FileActivityTarget::exact(
                    hookkit_core::Utf8PathBuf::from_path_buf(project.join("src/read-only.rs"))
                        .unwrap()
                ))
            );
            assert!(view.state().has_gaps());
            let evidence = view
                .events()
                .iter()
                .filter_map(|record| match record.event() {
                    hookkit_file_activity::FileActivityEvent::Evidence(evidence) => Some(evidence),
                    hookkit_file_activity::FileActivityEvent::Gap(_)
                    | hookkit_file_activity::FileActivityEvent::Retry(_) => None,
                })
                .collect::<Vec<_>>();
            assert!(evidence.iter().any(|item| {
                item.source == hookkit_file_activity::FileActivitySource::StructuredToolInput
            }));
            assert!(
                evidence
                    .iter()
                    .any(|item| item.source == hookkit_file_activity::FileActivitySource::Patch)
            );
            assert!(evidence.iter().any(|item| {
                item.source == hookkit_file_activity::FileActivitySource::ShellInference
            }));
            Ok(hookkit_session_state::EntityOutcome::retain(()))
        })
        .unwrap();
}

#[test]
fn bundled_start_observer_and_turn_runner_share_one_explicit_state_root() {
    require_pkl!();
    let project = temp_project("bundled-deferred-suite-state-root");
    let state_dir = project.join("shared-state");
    let state_arg = state_dir.to_string_lossy().into_owned();
    let fake_ruff = write_fake_ruff(&project);
    write_per_file_ruff_hook_config(&project, &fake_ruff);
    let start = serde_json::to_vec(&serde_json::json!({
        "session_id": "codex-ruff-test",
        "transcript_path": "/tmp/codex-ruff-test.jsonl",
        "cwd": project.to_string_lossy(),
        "hook_event_name": "SessionStart",
        "model": "gpt-test",
        "permission_mode": "default",
        "source": "startup"
    }))
    .unwrap();
    let started = run_example(
        "session-start-state-agent-hook",
        &start,
        &["--harness=codex", &format!("--state-dir={state_arg}")],
    );
    assert!(started.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&started.stdout).unwrap(),
        serde_json::json!({})
    );

    let file = project.join("src/clean.py");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "print('clean')\n").unwrap();
    let observed = run_example(
        "file-activity-agent-hook",
        &post_tool_use_fixture("codex", &project, "src/clean.py"),
        &["--harness=codex", &format!("--state-dir={state_arg}")],
    );
    assert!(observed.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&observed.stdout).unwrap(),
        serde_json::json!({})
    );
    assert_eq!(
        session_journal_len(&state_dir, "codex", "codex-ruff-test"),
        1
    );

    let stopped = run_example(
        "turn-completion-agent-hook",
        &turn_completion_fixture("codex", &project),
        &["--harness=codex", &format!("--state-dir={state_arg}")],
    );
    assert!(stopped.status.success());
    let response: serde_json::Value = serde_json::from_slice(&stopped.stdout).unwrap();
    assert!(
        response["systemMessage"]
            .as_str()
            .unwrap()
            .contains("Checked 1 clean file")
    );
    assert_eq!(
        session_journal_len(&state_dir, "codex", "codex-ruff-test"),
        0
    );
    let summary = only_summary(&state_dir);
    assert!(Path::new(summary["run"]["stateDirectory"].as_str().unwrap()).starts_with(&state_dir));
}

// --- turn-completion-agent-hook consuming session state ---

#[test]
fn turn_completion_no_pending_work_emits_each_exact_native_no_op() {
    require_pkl!();
    for harness in ["claude", "codex", "gemini", "antigravity"] {
        let project = temp_project(&format!("turn-completion-no-pending-{harness}"));
        let state_dir = project.join("state");
        let state_arg = state_dir.to_string_lossy().into_owned();
        let output = run_deferred_case(harness, &project, &state_arg);
        assert!(output.status.success(), "{harness}: {:?}", output.stderr);
        let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        if harness == "antigravity" {
            assert_eq!(response, serde_json::json!({"decision": "stop"}));
        } else {
            assert_eq!(response, serde_json::json!({}));
        }
        assert!(files_named(&state_dir, "summary.json").is_empty());
    }
}

#[test]
fn turn_completion_allowed_bucket_matrix_uses_native_audience_channels() {
    require_pkl!();
    for harness in ["claude", "codex", "gemini", "antigravity"] {
        for (case, files, expected_clean, expected_auto) in [
            ("clean", vec![("src/clean.py", "print('clean')\n")], 1, 0),
            (
                "auto",
                vec![("src/dirty.py", "import os  # unused_import\n")],
                0,
                1,
            ),
            (
                "mixed",
                vec![
                    ("src/clean.py", "print('clean')\n"),
                    ("src/dirty.py", "import os  # unused_import\n"),
                ],
                1,
                1,
            ),
        ] {
            let (project, state_dir, state_arg) = prepare_deferred_ruff_case(
                harness,
                &format!("turn-completion-allowed-{case}"),
                &files,
            );
            let output = run_deferred_case(harness, &project, &state_arg);
            assert!(
                output.status.success(),
                "{harness}/{case}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            if harness == "antigravity" {
                assert_eq!(response["decision"], "stop");
                assert!(
                    response["reason"]
                        .as_str()
                        .unwrap()
                        .contains("omitted user deferred Stop message")
                );
            } else {
                assert!(response.get("decision").is_none());
                let user = response["systemMessage"].as_str().unwrap();
                if expected_clean == 1 {
                    assert!(
                        user.contains("Checked 1 clean file"),
                        "{harness}/{case}: {user:?}"
                    );
                }
                if expected_auto == 1 {
                    assert!(user.contains("Auto-fixed 1 file"));
                    if harness == "claude" {
                        assert!(
                            response["hookSpecificOutput"]["additionalContext"]
                                .as_str()
                                .unwrap()
                                .contains("re-read changed files")
                        );
                    } else {
                        assert!(user.contains("omitted agent deferred Stop message"));
                    }
                }
            }
            let summary = only_summary(&state_dir);
            assert_eq!(summary["status"], "clean");
            assert_eq!(summary["counts"]["clean"], expected_clean);
            assert_eq!(summary["counts"]["autoFixed"], expected_auto);
            assert_eq!(summary["renderedMessages"]["lowering"]["blocked"], false);
        }
    }
}

#[test]
fn turn_completion_one_window_contains_clean_auto_fixed_and_manual_files() {
    require_pkl!();
    let project = temp_project("turn-completion-three-normal-buckets");
    let state_dir = project.join("state");
    let state_arg = state_dir.to_string_lossy().into_owned();
    let fake_ruff = write_fake_ruff(&project);
    write_per_file_ruff_hook_config(&project, &fake_ruff);
    std::fs::create_dir_all(project.join("src")).unwrap();
    for (path, contents) in [
        ("src/clean.py", "print('clean')\n"),
        ("src/auto.py", "import os  # unused_import\n"),
        ("src/manual.py", "print(manual_issue)\n"),
    ] {
        std::fs::write(project.join(path), contents).unwrap();
        let observed = run_example(
            "file-activity-agent-hook",
            &post_tool_use_fixture("codex", &project, path),
            &["--codex", "--state-dir", state_arg.as_str()],
        );
        assert!(observed.status.success());
    }

    let stopped = run_deferred_case("codex", &project, &state_arg);

    assert!(stopped.status.success());
    let response: serde_json::Value = serde_json::from_slice(&stopped.stdout).unwrap();
    assert_eq!(response["decision"], "block");
    let user = response["systemMessage"].as_str().unwrap();
    assert!(user.contains("Checked 1 clean file: src/clean.py"));
    assert!(user.contains("Auto-fixed 1 file: src/auto.py"));
    assert!(user.contains("1 file needs manual fixes"));
    let summary = only_summary(&state_dir);
    assert_eq!(summary["counts"]["clean"], 1);
    assert_eq!(summary["counts"]["autoFixed"], 1);
    assert_eq!(summary["counts"]["manualFixesNeeded"], 1);
    let statuses = summary["result"]["files"]
        .as_object()
        .unwrap()
        .values()
        .map(|file| file["status"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        statuses,
        std::collections::BTreeSet::from(["auto-fixed", "clean", "manual-fixes-needed"])
    );
}

#[test]
fn turn_completion_blocked_manual_and_operational_matrix_is_native() {
    require_pkl!();
    for harness in ["claude", "codex", "gemini", "antigravity"] {
        for (case, contents, expected_status) in [
            ("manual", "print(manual_issue)\n", "issues"),
            (
                "operational",
                "print('check_crash')\n",
                "operational-failure",
            ),
        ] {
            let (project, state_dir, state_arg) = prepare_deferred_ruff_case(
                harness,
                &format!("turn-completion-blocked-{case}"),
                &[("src/result.py", contents)],
            );
            let output = run_deferred_case(harness, &project, &state_arg);
            assert!(
                output.status.success(),
                "{harness}/{case}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            match harness {
                "claude" | "codex" => assert_eq!(response["decision"], "block"),
                "gemini" => assert_eq!(response["decision"], "deny"),
                "antigravity" => assert_eq!(response["decision"], "continue"),
                _ => unreachable!(),
            }
            assert!(!response["reason"].as_str().unwrap().is_empty());
            if harness != "antigravity" {
                assert!(!response["systemMessage"].as_str().unwrap().is_empty());
            }
            let summary = only_summary(&state_dir);
            assert_eq!(summary["status"], expected_status);
            assert_eq!(summary["renderedMessages"]["lowering"]["blocked"], true);
            assert_eq!(
                summary["renderedMessages"]["lowering"]["agent"]["status"],
                "emitted"
            );
        }
    }
}

#[test]
fn turn_completion_lowering_policies_and_empty_agent_are_explicit() {
    require_pkl!();

    let (project, state_dir, state_arg) = prepare_deferred_ruff_case(
        "codex",
        "turn-completion-strict-unrepresentable",
        &[("src/dirty.py", "import os  # unused_import\n")],
    );
    add_runner_setting(&project, r#"loweringPolicy = "strict""#);
    let strict = run_deferred_case("codex", &project, &state_arg);
    assert!(!strict.status.success());
    assert!(
        strict.stdout.is_empty(),
        "strict failure must not corrupt stdout"
    );
    let summary = only_summary(&state_dir);
    assert_eq!(
        summary["renderedMessages"]["lowering"]["agent"]["status"],
        "unrepresentable"
    );
    assert!(summary["renderedMessages"]["lowering"]["strictError"].is_string());
    assert!(session_journal_len(&state_dir, "codex", "codex-ruff-test") >= 1);

    let (project, state_dir, state_arg) = prepare_deferred_ruff_case(
        "codex",
        "turn-completion-best-effort-omission",
        &[("src/dirty.py", "import os  # unused_import\n")],
    );
    add_runner_setting(&project, r#"loweringPolicy = "best-effort""#);
    let best_effort = run_deferred_case("codex", &project, &state_arg);
    assert!(best_effort.status.success());
    let response: serde_json::Value = serde_json::from_slice(&best_effort.stdout).unwrap();
    assert!(
        response["systemMessage"]
            .as_str()
            .unwrap()
            .contains("Auto-fixed")
    );
    assert!(
        !response["systemMessage"]
            .as_str()
            .unwrap()
            .contains("hookkit: omitted")
    );
    let summary = only_summary(&state_dir);
    assert_eq!(
        summary["renderedMessages"]["lowering"]["agent"]["status"],
        "omitted"
    );
    assert!(
        summary["renderedMessages"]["lowering"]["warnings"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let (project, state_dir, state_arg) = prepare_deferred_ruff_case(
        "codex",
        "turn-completion-empty-agent",
        &[("src/manual.py", "print(manual_issue)\n")],
    );
    add_runner_setting(&project, r#"loweringPolicy = "strict""#);
    add_deferred_reporting_config(
        &project,
        r#"    manualFixesNeeded = new TemplatePair { agent = "" }"#,
    );
    let empty_agent = run_deferred_case("codex", &project, &state_arg);
    assert!(
        empty_agent.status.success(),
        "{}",
        String::from_utf8_lossy(&empty_agent.stderr)
    );
    let response: serde_json::Value = serde_json::from_slice(&empty_agent.stdout).unwrap();
    assert_eq!(response["decision"], "block");
    assert_eq!(response["reason"], "");
    let summary = only_summary(&state_dir);
    assert_eq!(
        summary["renderedMessages"]["lowering"]["agent"]["status"],
        "empty"
    );
}

#[test]
fn turn_completion_batch_autofixes_then_acknowledges_the_exact_snapshot() {
    require_pkl!();
    let project = temp_project("turn-completion-autofix");
    let state_dir = project.join("state");
    let state_arg = state_dir.to_string_lossy().into_owned();
    let fake_ruff = write_fake_ruff(&project);
    write_ruff_hook_config(&project, &fake_ruff, "");
    std::fs::create_dir_all(project.join("src")).unwrap();
    let file = project.join("src/dirty.py");
    std::fs::write(&file, "import os  # unused_import\nprint('needs_format')\n").unwrap();

    let tracked = run_example(
        "file-activity-agent-hook",
        &post_tool_use_fixture("claude", &project, "src/dirty.py"),
        &["--harness=claude", "--state-dir", state_arg.as_str()],
    );
    assert!(tracked.status.success());
    assert_eq!(
        session_journal_len(&state_dir, "claude-code", "claude-ruff-test"),
        1
    );

    let stopped = run_example(
        "turn-completion-agent-hook",
        &turn_completion_fixture("claude", &project),
        &["--claude", "--state-dir", state_arg.as_str()],
    );
    assert!(stopped.status.success());
    let response: serde_json::Value = serde_json::from_slice(&stopped.stdout).unwrap();
    assert!(
        response["systemMessage"]
            .as_str()
            .unwrap()
            .contains("Auto-fixed 1 file: src/dirty.py")
    );
    assert_eq!(
        response["hookSpecificOutput"]["additionalContext"],
        "Auto-fixed 1 file; re-read changed files before editing further."
    );
    let rewritten = std::fs::read_to_string(file).unwrap();
    assert!(rewritten.contains("formatted"));
    assert!(!rewritten.contains("unused_import"));
    assert_eq!(
        session_journal_len(&state_dir, "claude-code", "claude-ruff-test"),
        0
    );
    let summaries = files_named(&state_dir, "summary.json");
    assert_eq!(summaries.len(), 1);
    let summary: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&summaries[0]).unwrap()).unwrap();
    assert_eq!(summary["status"], "clean");
    assert_eq!(
        summary["stateDisposition"]["source"],
        "acknowledge-sealed-window"
    );

    let second = run_example(
        "turn-completion-agent-hook",
        &turn_completion_fixture("claude", &project),
        &["--claude", "--state-dir", state_arg.as_str()],
    );
    assert!(second.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&second.stdout).unwrap(),
        serde_json::json!({})
    );
    assert_eq!(
        files_named(&state_dir, "summary.json").len(),
        1,
        "the runner's own writes must not resurrect an auto-fixed file"
    );
}

#[test]
fn turn_completion_handled_baseline_suppresses_unchanged_git_dirty_fallback() {
    require_pkl!();
    let project = temp_project("turn-completion-git-dirty-baseline");
    let state_dir = project.join("state");
    let state_arg = state_dir.to_string_lossy().into_owned();
    let fake_ruff = write_fake_ruff(&project);
    write_per_file_ruff_hook_config(&project, &fake_ruff);
    replace_file_activity_settings(&project, r#"filesystemMtime = false; vcs = "git-dirty""#);
    std::fs::create_dir_all(project.join("src")).unwrap();
    let file = project.join("src/tracked.py");
    std::fs::write(&file, "print('original')\n").unwrap();
    run_git(&project, &["init", "-q"]);
    run_git(&project, &["add", "."]);
    run_git(
        &project,
        &[
            "-c",
            "user.name=HookKit",
            "-c",
            "user.email=hookkit@example.invalid",
            "commit",
            "-qm",
            "baseline",
        ],
    );
    std::fs::write(&file, "print('agent edit')\n").unwrap();
    let observed = run_example(
        "file-activity-agent-hook",
        &post_tool_use_fixture("codex", &project, "src/tracked.py"),
        &["--codex", "--state-dir", state_arg.as_str()],
    );
    assert!(observed.status.success());

    let first = run_deferred_case("codex", &project, &state_arg);
    assert!(first.status.success());
    assert_eq!(only_summary(&state_dir)["counts"]["clean"], 1);
    assert!(
        String::from_utf8_lossy(
            &Command::new("git")
                .arg("-C")
                .arg(&project)
                .args(["status", "--short", "--", "src/tracked.py"])
                .output()
                .unwrap()
                .stdout
        )
        .contains("src/tracked.py")
    );

    let second = run_deferred_case("codex", &project, &state_arg);

    assert!(second.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&second.stdout).unwrap(),
        serde_json::json!({})
    );
    assert_eq!(
        files_named(&state_dir, "summary.json").len(),
        1,
        "an unchanged handled Git-dirty file must not run checks again"
    );
}

#[test]
fn invalid_deferred_template_syntax_fails_before_any_remedy_runs() {
    require_pkl!();
    let project = temp_project("turn-completion-invalid-reporting-syntax");
    let state_dir = project.join("state");
    let state_arg = state_dir.to_string_lossy().into_owned();
    let fake_ruff = write_fake_ruff(&project);
    write_ruff_hook_config(&project, &fake_ruff, "");
    add_deferred_reporting_config(&project, r#"    clean = new TemplatePair { user = "{{" }"#);
    std::fs::create_dir_all(project.join("src")).unwrap();
    let file = project.join("src/dirty.py");
    std::fs::write(&file, "import os  # unused_import\n").unwrap();
    let tracked = run_example(
        "file-activity-agent-hook",
        &post_tool_use_fixture("codex", &project, "src/dirty.py"),
        &["--harness=codex", "--state-dir", state_arg.as_str()],
    );
    assert!(tracked.status.success());

    let stopped = run_example(
        "turn-completion-agent-hook",
        &turn_completion_fixture("codex", &project),
        &["--codex", "--state-dir", state_arg.as_str()],
    );
    assert!(stopped.status.success());
    let response: serde_json::Value = serde_json::from_slice(&stopped.stdout).unwrap();
    assert_eq!(response["decision"], "block");
    assert!(
        std::fs::read_to_string(file)
            .unwrap()
            .contains("unused_import")
    );
    assert_eq!(files_named(&state_dir, "config-error.log").len(), 1);
    let summary_path = files_named(&state_dir, "summary.json").pop().unwrap();
    let summary: serde_json::Value =
        serde_json::from_slice(&std::fs::read(summary_path).unwrap()).unwrap();
    assert_eq!(summary["status"], "operational-failure");
    assert_eq!(summary["result"]["artifacts"].as_object().unwrap().len(), 1);
}

#[test]
fn deferred_template_render_failure_is_a_durable_operational_error() {
    require_pkl!();
    let project = temp_project("turn-completion-reporting-render-failure");
    let state_dir = project.join("state");
    let state_arg = state_dir.to_string_lossy().into_owned();
    let fake_ruff = write_fake_ruff(&project);
    write_ruff_hook_config(&project, &fake_ruff, "");
    add_deferred_reporting_config(
        &project,
        r#"    masterUser = "{{ unavailable_reporting_function() }}""#,
    );
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("src/clean.py"), "print('clean')\n").unwrap();
    let tracked = run_example(
        "file-activity-agent-hook",
        &post_tool_use_fixture("codex", &project, "src/clean.py"),
        &["--harness=codex", "--state-dir", state_arg.as_str()],
    );
    assert!(tracked.status.success());

    let stopped = run_example(
        "turn-completion-agent-hook",
        &turn_completion_fixture("codex", &project),
        &["--codex", "--state-dir", state_arg.as_str()],
    );
    assert!(stopped.status.success());
    let response: serde_json::Value = serde_json::from_slice(&stopped.stdout).unwrap();
    assert_eq!(response["decision"], "block");
    let reporting_logs = files_named(&state_dir, "reporting-error.log");
    assert_eq!(reporting_logs.len(), 1);
    assert!(
        !std::fs::read_to_string(&reporting_logs[0])
            .unwrap()
            .is_empty()
    );
    let summary_path = files_named(&state_dir, "summary.json").pop().unwrap();
    let summary: serde_json::Value =
        serde_json::from_slice(&std::fs::read(summary_path).unwrap()).unwrap();
    assert_eq!(summary["status"], "operational-failure");
    assert_eq!(summary["result"]["files"].as_object().unwrap().len(), 1);
    let artifacts = summary["result"]["artifacts"].as_object().unwrap();
    assert!(
        artifacts.len() >= 2,
        "tool artifacts must survive rendering failure"
    );
    assert!(artifacts.values().any(|artifact| {
        artifact["classification"] == "configuration-error"
            && artifact["absolutePath"]
                .as_str()
                .unwrap()
                .ends_with("reporting-error.log")
    }));
    assert!(session_journal_len(&state_dir, "codex", "codex-ruff-test") >= 1);
}

#[test]
fn turn_completion_mtime_fallback_finds_files_without_tool_observations() {
    require_pkl!();
    let project = temp_project("turn-completion-mtime-fallback");
    let state_dir = project.join("state");
    let state_arg = state_dir.to_string_lossy().into_owned();
    let start = serde_json::to_vec(&serde_json::json!({
        "session_id": "claude-ruff-test",
        "transcript_path": "/tmp/claude-ruff-test.jsonl",
        "cwd": project.to_string_lossy(),
        "hook_event_name": "SessionStart",
        "source": "startup",
        "model": "claude-test"
    }))
    .unwrap();
    let started = run_example(
        "session-start-state-agent-hook",
        &start,
        &["--claude", "--state-dir", state_arg.as_str()],
    );
    assert!(started.status.success());

    let fake_ruff = write_fake_ruff(&project);
    write_ruff_hook_config(&project, &fake_ruff, "");
    std::fs::create_dir_all(project.join("src")).unwrap();
    let file = project.join("src/unobserved.py");
    std::fs::write(&file, "import os  # unused_import\nprint('needs_format')\n").unwrap();

    let stopped = run_example(
        "turn-completion-agent-hook",
        &turn_completion_fixture("claude", &project),
        &["--claude", "--state-dir", state_arg.as_str()],
    );
    assert!(stopped.status.success());
    let rewritten = std::fs::read_to_string(file).unwrap();
    assert!(rewritten.contains("formatted"));
    assert!(!rewritten.contains("unused_import"));
    assert_eq!(
        session_journal_len(&state_dir, "claude-code", "claude-ruff-test"),
        0
    );
}

#[test]
fn turn_completion_batch_retains_issues_and_points_at_detailed_logs() {
    require_pkl!();
    let project = temp_project("turn-completion-issues");
    let state_dir = project.join("state");
    let state_arg = state_dir.to_string_lossy().into_owned();
    let fake_ruff = write_fake_ruff(&project);
    write_ruff_hook_config(&project, &fake_ruff, "");
    std::fs::create_dir_all(project.join("src")).unwrap();
    let file = project.join("src/broken.py");
    std::fs::write(&file, "print(manual_issue)\n").unwrap();

    let tracked = run_example(
        "file-activity-agent-hook",
        &post_tool_use_fixture("codex", &project, "src/broken.py"),
        &["--harness=codex", "--state-dir", state_arg.as_str()],
    );
    assert!(tracked.status.success());

    let stopped = run_example(
        "turn-completion-agent-hook",
        &turn_completion_fixture("codex", &project),
        &["--codex", "--state-dir", state_arg.as_str()],
    );
    assert!(stopped.status.success());
    let response: serde_json::Value = serde_json::from_slice(&stopped.stdout).unwrap();
    assert_eq!(response["decision"], "block");
    assert!(
        response["systemMessage"]
            .as_str()
            .unwrap()
            .contains("manual fixes")
    );
    assert!(
        response["reason"]
            .as_str()
            .unwrap()
            .contains("Python: src/broken.py")
    );
    assert!(
        session_journal_len(&state_dir, "codex", "codex-ruff-test") >= 1,
        "retained window includes tool evidence and fallback observations"
    );
    let summaries = files_named(&state_dir, "summary.json");
    assert_eq!(summaries.len(), 1);
    let summary: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&summaries[0]).unwrap()).unwrap();
    assert_eq!(summary["status"], "issues");
    assert!(summary.get("acknowledged").is_none());
    assert_eq!(
        summary["stateDisposition"]["source"],
        "acknowledge-sealed-window"
    );
    assert_eq!(
        summary["renderedMessages"]["lowering"]["policy"],
        "best-effort-with-warnings"
    );
    assert_eq!(
        summary["renderedMessages"]["lowering"]["user"]["status"],
        "emitted"
    );
    assert_eq!(
        summary["renderedMessages"]["lowering"]["agent"]["status"],
        "emitted"
    );
    assert!(
        summary["renderedMessages"]["user"]
            .as_str()
            .unwrap()
            .contains("manual fixes")
    );
    assert!(
        summary["renderedMessages"]["agent"]
            .as_str()
            .unwrap()
            .contains("Python: src/broken.py")
    );
    let artifacts = summary["result"]["artifacts"].as_object().unwrap();
    assert!(artifacts.len() >= 4);
    assert!(artifacts.values().any(|artifact| {
        artifact["contents"]
            .as_str()
            .unwrap()
            .contains("F821 undefined name manual_issue")
    }));
    for artifact in artifacts.values() {
        let path = artifact["absolutePath"].as_str().unwrap();
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            artifact["contents"].as_str().unwrap()
        );
    }

    std::fs::write(&file, "print('fixed')\n").unwrap();
    let retried = run_example(
        "turn-completion-agent-hook",
        &turn_completion_fixture("codex", &project),
        &["--codex", "--state-dir", state_arg.as_str()],
    );
    assert!(retried.status.success());
    let retried_response: serde_json::Value = serde_json::from_slice(&retried.stdout).unwrap();
    assert!(
        retried_response["systemMessage"]
            .as_str()
            .unwrap()
            .contains("Checked 1 clean file: src/broken.py")
    );
    assert_eq!(
        session_journal_len(&state_dir, "codex", "codex-ruff-test"),
        0
    );
}

#[test]
fn turn_completion_selectively_discharges_clean_and_retries_manual_files() {
    require_pkl!();
    let project = temp_project("turn-completion-selective");
    let state_dir = project.join("state");
    let state_arg = state_dir.to_string_lossy().into_owned();
    let fake_ruff = write_fake_ruff(&project);
    write_per_file_ruff_hook_config(&project, &fake_ruff);
    std::fs::create_dir_all(project.join("src")).unwrap();
    let clean = project.join("src/clean.py");
    let manual = project.join("src/manual.py");
    std::fs::write(&clean, "print('clean')\n").unwrap();
    std::fs::write(&manual, "print(manual_issue)\n").unwrap();

    for file in ["src/clean.py", "src/manual.py"] {
        let tracked = run_example(
            "file-activity-agent-hook",
            &post_tool_use_fixture("codex", &project, file),
            &["--harness=codex", "--state-dir", state_arg.as_str()],
        );
        assert!(tracked.status.success());
    }
    let stopped = run_example(
        "turn-completion-agent-hook",
        &turn_completion_fixture("codex", &project),
        &["--codex", "--state-dir", state_arg.as_str()],
    );
    assert!(stopped.status.success());
    let response: serde_json::Value = serde_json::from_slice(&stopped.stdout).unwrap();
    assert_eq!(response["decision"], "block");

    let state = hookkit_session_state::SessionState::open(
        hookkit_core::HarnessId::CODEX,
        hookkit_session_state::SessionIdentity::Session("codex-ruff-test".into()),
        hookkit_session_state::StateRoot::new(&state_dir),
    )
    .unwrap();
    let store = hookkit_file_activity::FileActivityStore::from_state(state).unwrap();
    store
        .pending()
        .with_entity(|view| {
            assert_eq!(view.state().targets().len(), 1);
            assert!(
                view.state()
                    .targets()
                    .contains(&hookkit_file_activity::FileActivityTarget::exact(
                        hookkit_core::Utf8PathBuf::from_path_buf(
                            std::fs::canonicalize(&manual).unwrap()
                        )
                        .unwrap()
                    ))
            );
            Ok(hookkit_session_state::EntityOutcome::retain(()))
        })
        .unwrap();

    let summaries = files_named(&state_dir, "summary.json");
    assert_eq!(summaries.len(), 1);
    let first: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&summaries[0]).unwrap()).unwrap();
    let files = first["result"]["files"].as_object().unwrap();
    assert_eq!(files.len(), 2);
    assert!(files.values().any(|file| file["status"] == "clean"));
    assert!(
        files
            .values()
            .any(|file| file["status"] == "manual-fixes-needed")
    );

    std::fs::write(&manual, "print('fixed')\n").unwrap();
    let retried = run_example(
        "turn-completion-agent-hook",
        &turn_completion_fixture("codex", &project),
        &["--codex", "--state-dir", state_arg.as_str()],
    );
    assert!(retried.status.success());
    let retried_response: serde_json::Value = serde_json::from_slice(&retried.stdout).unwrap();
    assert!(
        retried_response["systemMessage"]
            .as_str()
            .unwrap()
            .contains("Checked 1 clean file: src/manual.py")
    );
    assert_eq!(
        session_journal_len(&state_dir, "codex", "codex-ruff-test"),
        0
    );
    let summaries = files_named(&state_dir, "summary.json");
    assert_eq!(summaries.len(), 2);
    let second: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&summaries[1]).unwrap()).unwrap();
    assert_eq!(second["candidateFiles"].as_array().unwrap().len(), 1);
    assert!(
        second["candidateFiles"][0]
            .as_str()
            .unwrap()
            .ends_with("manual.py")
    );
}

#[test]
fn turn_completion_preserves_observation_appended_while_stop_is_running() {
    require_pkl!();
    let project = temp_project("turn-completion-concurrent-observation");
    let state_dir = project.join("state");
    let state_arg = state_dir.to_string_lossy().into_owned();
    let fake_ruff = write_fake_ruff(&project);
    write_per_file_ruff_hook_config(&project, &fake_ruff);
    std::fs::create_dir_all(project.join("src")).unwrap();
    let slow = project.join("src/slow.py");
    let concurrent = project.join("src/concurrent.py");
    std::fs::write(&slow, "print('wait_for_release')\n").unwrap();
    std::fs::write(&concurrent, "print('concurrent')\n").unwrap();
    let observed = run_example(
        "file-activity-agent-hook",
        &post_tool_use_fixture("codex", &project, "src/slow.py"),
        &["--codex", "--state-dir", state_arg.as_str()],
    );
    assert!(observed.status.success());

    let fixture = turn_completion_fixture("codex", &project);
    let child = spawn_example(
        "turn-completion-agent-hook",
        &fixture,
        &["--codex", "--state-dir", state_arg.as_str()],
    );
    wait_for_path(&project.join("src/slow.py.started"));
    let concurrent_observation = run_example(
        "file-activity-agent-hook",
        &post_tool_use_fixture("codex", &project, "src/concurrent.py"),
        &["--codex", "--state-dir", state_arg.as_str()],
    );
    assert!(concurrent_observation.status.success());
    std::fs::write(project.join("src/slow.py.release"), "release\n").unwrap();
    let first = child.wait_with_output().expect("wait for running Stop");

    assert!(first.status.success());
    let first_summary = only_summary(&state_dir);
    assert_eq!(first_summary["candidateFiles"].as_array().unwrap().len(), 1);
    assert!(
        first_summary["candidateFiles"][0]
            .as_str()
            .unwrap()
            .ends_with("slow.py")
    );
    assert_eq!(
        session_journal_len(&state_dir, "codex", "codex-ruff-test"),
        1,
        "the post-seal observation must remain in the active generation"
    );

    let second = run_deferred_case("codex", &project, &state_arg);

    assert!(second.status.success());
    let summaries = files_named(&state_dir, "summary.json");
    assert_eq!(summaries.len(), 2);
    let second_summary: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&summaries[1]).unwrap()).unwrap();
    assert_eq!(
        second_summary["candidateFiles"].as_array().unwrap().len(),
        1
    );
    assert!(
        second_summary["candidateFiles"][0]
            .as_str()
            .unwrap()
            .ends_with("concurrent.py")
    );
}

#[test]
fn turn_completion_operational_failure_retries_only_affected_files() {
    require_pkl!();
    let project = temp_project("turn-completion-selective-operational");
    let state_dir = project.join("state");
    let state_arg = state_dir.to_string_lossy().into_owned();
    let fake_ruff = write_fake_ruff(&project);
    write_selective_operational_hook_config(&project, &fake_ruff);
    std::fs::create_dir_all(project.join("src")).unwrap();
    let clean = project.join("src/clean.py");
    let operational = project.join("src/operational.rs");
    std::fs::write(&clean, "print('clean')\n").unwrap();
    std::fs::write(&operational, "fn main() {}\n").unwrap();

    for file in ["src/clean.py", "src/operational.rs"] {
        let tracked = run_example(
            "file-activity-agent-hook",
            &post_tool_use_fixture("codex", &project, file),
            &["--harness=codex", "--state-dir", state_arg.as_str()],
        );
        assert!(tracked.status.success());
    }
    let stopped = run_example(
        "turn-completion-agent-hook",
        &turn_completion_fixture("codex", &project),
        &["--codex", "--state-dir", state_arg.as_str()],
    );
    assert!(stopped.status.success());
    let response: serde_json::Value = serde_json::from_slice(&stopped.stdout).unwrap();
    assert_eq!(response["decision"], "block");

    let state = hookkit_session_state::SessionState::open(
        hookkit_core::HarnessId::CODEX,
        hookkit_session_state::SessionIdentity::Session("codex-ruff-test".into()),
        hookkit_session_state::StateRoot::new(&state_dir),
    )
    .unwrap();
    let store = hookkit_file_activity::FileActivityStore::from_state(state).unwrap();
    store
        .pending()
        .with_entity(|view| {
            assert_eq!(view.state().targets().len(), 1);
            assert!(
                view.state()
                    .targets()
                    .contains(&hookkit_file_activity::FileActivityTarget::exact(
                        hookkit_core::Utf8PathBuf::from_path_buf(
                            std::fs::canonicalize(&operational).unwrap()
                        )
                        .unwrap()
                    ))
            );
            Ok(hookkit_session_state::EntityOutcome::retain(()))
        })
        .unwrap();

    let summaries = files_named(&state_dir, "summary.json");
    assert_eq!(summaries.len(), 1);
    let summary: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&summaries[0]).unwrap()).unwrap();
    assert_eq!(summary["status"], "operational-failure");
    assert_eq!(summary["result"]["files"].as_object().unwrap().len(), 1);
    assert_eq!(
        summary["result"]["operationalProblems"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
    let artifacts = summary["result"]["artifacts"].as_object().unwrap();
    assert_eq!(artifacts.len(), 2);
    assert!(
        artifacts
            .values()
            .any(|artifact| artifact["classification"] == "clean")
    );
    assert!(
        artifacts
            .values()
            .any(|artifact| artifact["classification"] == "spawn-error")
    );
    for artifact in artifacts.values() {
        assert_eq!(
            std::fs::read_to_string(artifact["absolutePath"].as_str().unwrap()).unwrap(),
            artifact["contents"].as_str().unwrap()
        );
    }
}

#[test]
fn turn_completion_per_file_batch_isolates_one_operational_failure() {
    require_pkl!();
    let project = temp_project("turn-completion-partial-batch-failure");
    let state_dir = project.join("state");
    let state_arg = state_dir.to_string_lossy().into_owned();
    let fake_ruff = write_fake_ruff(&project);
    write_per_file_ruff_hook_config(&project, &fake_ruff);
    std::fs::create_dir_all(project.join("src")).unwrap();
    let clean = project.join("src/clean.py");
    let failed = project.join("src/failed.py");
    std::fs::write(&clean, "print('clean')\n").unwrap();
    std::fs::write(&failed, "print('check_crash')\n").unwrap();
    for path in [&clean, &failed] {
        seed_pending_file(&state_dir, "codex", path);
    }

    let stopped = run_deferred_case("codex", &project, &state_arg);

    assert!(stopped.status.success());
    let summary = only_summary(&state_dir);
    assert_eq!(summary["result"]["files"].as_object().unwrap().len(), 2);
    let normal_paths = summary["result"]["files"]
        .as_object()
        .unwrap()
        .values()
        .map(|file| file["path"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(normal_paths.iter().any(|path| path.ends_with("clean.py")));
    assert!(
        normal_paths.iter().any(|path| path.ends_with("failed.py")),
        "a successful independent format check remains a normal result"
    );
    let problem = summary["result"]["operationalProblems"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    assert_eq!(problem["affectedFiles"].as_array().unwrap().len(), 1);
    assert!(
        problem["affectedFiles"][0]
            .as_str()
            .unwrap()
            .ends_with("failed.py")
    );
    let state = hookkit_session_state::SessionState::open(
        hookkit_core::HarnessId::CODEX,
        hookkit_session_state::SessionIdentity::Session("codex-ruff-test".into()),
        hookkit_session_state::StateRoot::new(&state_dir),
    )
    .unwrap();
    let store = hookkit_file_activity::FileActivityStore::from_state(state).unwrap();
    store
        .pending()
        .with_entity(|view| {
            assert_eq!(
                view.state().targets(),
                &std::collections::BTreeSet::from([
                    hookkit_file_activity::FileActivityTarget::exact(
                        hookkit_core::Utf8PathBuf::from_path_buf(
                            std::fs::canonicalize(&failed).unwrap(),
                        )
                        .unwrap(),
                    ),
                ])
            );
            Ok(hookkit_session_state::EntityOutcome::retain(()))
        })
        .unwrap();
}

#[test]
fn turn_completion_records_uncovered_deleted_unresolved_and_truncated_activity() {
    require_pkl!();

    for case in ["uncovered", "deleted", "unresolved", "truncated"] {
        let project = temp_project(&format!("turn-completion-coverage-{case}"));
        let state_dir = project.join("state");
        let state_arg = state_dir.to_string_lossy().into_owned();
        let fake_ruff = write_fake_ruff(&project);
        write_per_file_ruff_hook_config(&project, &fake_ruff);
        if case == "truncated" {
            replace_file_activity_settings(&project, "filesystemMtime = false; maxEntries = 1");
        }
        std::fs::create_dir_all(project.join("src/nested")).unwrap();
        std::fs::write(project.join("src/nested/one.py"), "print('one')\n").unwrap();
        std::fs::write(project.join("src/nested/two.py"), "print('two')\n").unwrap();

        match case {
            "uncovered" => {
                let path = project.join("src/note.unknown");
                std::fs::write(&path, "not covered\n").unwrap();
                seed_pending_file(&state_dir, "codex", &path);
            }
            "deleted" => {
                let path = project.join("src/deleted.py");
                std::fs::write(&path, "print('gone')\n").unwrap();
                seed_pending_file(&state_dir, "codex", &path);
                std::fs::remove_file(path).unwrap();
            }
            "unresolved" => seed_pending_target(
                &state_dir,
                "codex",
                hookkit_file_activity::FileActivityTarget::Path {
                    path: hookkit_core::Utf8PathBuf::from_path_buf(
                        project.join("missing-directory"),
                    )
                    .unwrap(),
                    scope: hookkit_file_activity::FileActivityScope::Descendants,
                },
            ),
            "truncated" => seed_pending_target(
                &state_dir,
                "codex",
                hookkit_file_activity::FileActivityTarget::Workspace {
                    root: Some(hookkit_core::Utf8PathBuf::from_path_buf(project.clone()).unwrap()),
                },
            ),
            _ => unreachable!(),
        }

        let stopped = run_deferred_case("codex", &project, &state_arg);

        assert!(
            stopped.status.success(),
            "{case}: {}",
            String::from_utf8_lossy(&stopped.stderr)
        );
        let summary = only_summary(&state_dir);
        match case {
            "uncovered" => assert_eq!(
                summary["result"]["uncoveredFiles"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            ),
            "deleted" => assert_eq!(
                summary["result"]["notApplicableFiles"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            ),
            "unresolved" => {
                assert!(summary["counts"]["coverageGaps"].as_u64().unwrap() >= 1);
                assert!(
                    summary["stateDisposition"]["retryTargets"]
                        .as_array()
                        .is_some_and(|targets| !targets.is_empty())
                );
            }
            "truncated" => {
                assert!(summary["counts"]["coverageGaps"].as_u64().unwrap() >= 1);
                assert!(
                    summary["stateDisposition"]["retryGaps"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|gap| gap.as_str().unwrap().contains("traversal budget"))
                );
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn turn_completion_links_distinct_tool_artifacts_to_one_file() {
    require_pkl!();
    let project = temp_project("turn-completion-multi-tool-artifacts");
    let state_dir = project.join("state");
    let state_arg = state_dir.to_string_lossy().into_owned();
    let fake_ruff = write_fake_ruff(&project);
    write_artifact_linking_hook_config(
        &project,
        &fake_ruff,
        &["first-check", "second-check"],
        "per-file",
    );
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("src/shared.py"), "print('clean')\n").unwrap();
    let tracked = run_example(
        "file-activity-agent-hook",
        &post_tool_use_fixture("codex", &project, "src/shared.py"),
        &["--harness=codex", "--state-dir", state_arg.as_str()],
    );
    assert!(tracked.status.success());
    let stopped = run_example(
        "turn-completion-agent-hook",
        &turn_completion_fixture("codex", &project),
        &["--codex", "--state-dir", state_arg.as_str()],
    );
    assert!(stopped.status.success());

    let summary_path = files_named(&state_dir, "summary.json").pop().unwrap();
    let summary: serde_json::Value =
        serde_json::from_slice(&std::fs::read(summary_path).unwrap()).unwrap();
    let file = summary["result"]["files"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    let reports = file["reports"].as_array().unwrap();
    assert_eq!(reports.len(), 2);
    let artifact_ids = reports
        .iter()
        .flat_map(|report| report["artifactIds"].as_array().unwrap())
        .map(|id| id.as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(artifact_ids.len(), 2);
    assert_eq!(summary["artifactPaths"].as_array().unwrap().len(), 2);
    assert_eq!(summary["artifactContents"].as_object().unwrap().len(), 2);
}

#[test]
fn turn_completion_applies_custom_group_bucket_and_master_templates() {
    require_pkl!();
    let project = temp_project("turn-completion-custom-reporting");
    let state_dir = project.join("state");
    let state_arg = state_dir.to_string_lossy().into_owned();
    let fake_ruff = write_fake_ruff(&project);
    write_per_file_ruff_hook_config(&project, &fake_ruff);
    add_deferred_reporting_config(
        &project,
        r#"    groups = new Listing<FileGroup> {
      new FileGroup { id = "special-python"; displayName = "Special Python"; include = new Listing { "**/*.py" } }
      new FileGroup { id = "other"; displayName = "Other"; include = new Listing { "**" } }
    }
    manualFixesNeeded = new TemplatePair {
      user = "BUCKET-U {{ manual_fix_files[0].displayPath }} {{ groups[0].display_name }}"
      agent = "BUCKET-A {{ manual_fix_files[0].groupId }} {{ artifact_paths | length }}"
    }
    masterUser = "MASTER-U {{ rendered_buckets.manual_fixes_needed.user }}"
    masterAgent = "MASTER-A {{ rendered_buckets.manual_fixes_needed.agent }}""#,
    );
    std::fs::create_dir_all(project.join("src")).unwrap();
    let file = project.join("src/custom.py");
    std::fs::write(&file, "print(manual_issue)\n").unwrap();
    seed_pending_file(&state_dir, "codex", &file);

    let stopped = run_deferred_case("codex", &project, &state_arg);

    assert!(stopped.status.success());
    let response: serde_json::Value = serde_json::from_slice(&stopped.stdout).unwrap();
    assert_eq!(
        response["systemMessage"],
        "MASTER-U BUCKET-U src/custom.py Special Python"
    );
    assert!(
        response["reason"]
            .as_str()
            .unwrap()
            .starts_with("MASTER-A BUCKET-A special-python ")
    );
    let summary = only_summary(&state_dir);
    let file = summary["result"]["files"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    assert_eq!(file["groupId"], "special-python");
}

#[test]
fn turn_completion_keeps_large_diagnostics_in_artifacts_not_native_context() {
    require_pkl!();
    let project = temp_project("turn-completion-large-diagnostics");
    let state_dir = project.join("state");
    let state_arg = state_dir.to_string_lossy().into_owned();
    let fake_ruff = write_fake_ruff(&project);
    write_per_file_ruff_hook_config(&project, &fake_ruff);
    std::fs::create_dir_all(project.join("src")).unwrap();
    let file = project.join("src/large.py");
    std::fs::write(&file, "print(manual_issue)  # large_diagnostic\n").unwrap();
    seed_pending_file(&state_dir, "codex", &file);

    let stopped = run_deferred_case("codex", &project, &state_arg);

    assert!(stopped.status.success());
    assert!(
        stopped.stdout.len() < 16_384,
        "native output must stay concise"
    );
    let response: serde_json::Value = serde_json::from_slice(&stopped.stdout).unwrap();
    assert!(!response["systemMessage"].as_str().unwrap().contains("xxxx"));
    assert!(!response["reason"].as_str().unwrap().contains("xxxx"));
    let summary = only_summary(&state_dir);
    assert!(
        summary["result"]["artifacts"]
            .as_object()
            .unwrap()
            .values()
            .any(|artifact| artifact["contents"]
                .as_str()
                .is_some_and(|text| text.len() > 100_000))
    );
}

#[test]
fn turn_completion_reuses_one_batch_artifact_for_multiple_files() {
    require_pkl!();
    let project = temp_project("turn-completion-batch-artifact");
    let state_dir = project.join("state");
    let state_arg = state_dir.to_string_lossy().into_owned();
    let fake_ruff = write_fake_ruff(&project);
    write_artifact_linking_hook_config(&project, &fake_ruff, &["batch-check"], "batch");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(project.join("src/first.py"), "print('first')\n").unwrap();
    std::fs::write(project.join("src/second.py"), "print('second')\n").unwrap();
    for file in ["src/first.py", "src/second.py"] {
        let tracked = run_example(
            "file-activity-agent-hook",
            &post_tool_use_fixture("codex", &project, file),
            &["--harness=codex", "--state-dir", state_arg.as_str()],
        );
        assert!(tracked.status.success());
    }
    let stopped = run_example(
        "turn-completion-agent-hook",
        &turn_completion_fixture("codex", &project),
        &["--codex", "--state-dir", state_arg.as_str()],
    );
    assert!(stopped.status.success());

    let summary_path = files_named(&state_dir, "summary.json").pop().unwrap();
    let summary: serde_json::Value =
        serde_json::from_slice(&std::fs::read(summary_path).unwrap()).unwrap();
    let files = summary["result"]["files"].as_object().unwrap();
    assert_eq!(files.len(), 2);
    let artifact_ids = files
        .values()
        .map(|file| file["reports"][0]["artifactIds"][0].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(artifact_ids.len(), 1);
    let artifact = summary["result"]["artifacts"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap();
    assert_eq!(artifact["candidateFiles"].as_array().unwrap().len(), 2);
    assert_eq!(artifact["classification"], "clean");
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
