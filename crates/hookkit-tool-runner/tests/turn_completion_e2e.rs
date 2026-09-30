//! Hermetic multi-Stop end-to-end tests for the bundled deferred hook suite.
//!
//! Each test drives the real `file-activity-agent-hook` and
//! `turn-completion-agent-hook` binaries through several native Stop events
//! per harness, using a fake checker executable and a private state root, and
//! asserts on exact native stdout. The invariants guard against Stop loops:
//!
//! - every native block carries a non-empty reason;
//! - an allowed Claude Stop never emits `hookSpecificOutput.additionalContext`
//!   (which would continue the conversation);
//! - an unchanged repeat does not block again inside one continuation loop;
//! - coverage gaps and missing tools do not re-block every later Stop.
//!
//! The tests require `pkl` on `PATH` and skip otherwise.

#![cfg(unix)]

use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const HARNESSES: [&str; 3] = ["claude", "codex", "antigravity"];

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

struct Project {
    root: PathBuf,
    state: PathBuf,
    trace: PathBuf,
    harness: &'static str,
}

impl Project {
    fn new(label: &str, harness: &'static str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "hookkit-e2e-{label}-{harness}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        // Canonicalize so native inputs and runner paths agree on macOS.
        let root = std::fs::canonicalize(root).unwrap();
        let checker = root.join("bin/checker");
        std::fs::create_dir_all(checker.parent().unwrap()).unwrap();
        std::fs::write(
            &checker,
            "#!/bin/sh\ntrace=$1\nshift\necho \"$#\" >> \"$trace\"\nfor file in \"$@\"; do\n  if grep -q MANUAL \"$file\"; then exit 1; fi\ndone\nexit 0\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&checker).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&checker, permissions).unwrap();
        Self {
            state: root.join("state"),
            trace: root.join("trace.log"),
            root,
            harness,
        }
    }

    fn checker(&self) -> PathBuf {
        self.root.join("bin/checker")
    }

    /// Write a config with one checker tool over `*.txt` files.
    fn configure(&self, executable: &Path, settings: &str) {
        let config_dir = self.root.join(".agent-hook-kit");
        std::fs::create_dir_all(&config_dir).unwrap();
        let config = format!(
            r#"amends "Config.pkl"

settings {{
{settings}
}}

tools {{
  ["check"] = new ToolSpec {{
    id = "check"
    displayName = "Check"
    executable = "{executable}"
    installHint = "install the checker"
    files {{ include = new Listing {{ "**/*.txt" }} }}
    workflows {{
      ["lint"] = new Workflow {{
        check = new WorkflowCommand {{
          argv = new Listing {{ "{trace}"; new Files {{}} }}
          exitCodes {{ issues = new Listing {{ 1 }} }}
        }}
      }}
    }}
    workflowOrder = new Listing {{ "lint" }}
  }}
}}
run = new Listing {{ "check" }}
"#,
            executable = executable.display(),
            trace = self.trace.display(),
        );
        std::fs::write(config_dir.join("post-tool-use.pkl"), config).unwrap();
    }

    fn write(&self, relative: &str, contents: &str) -> PathBuf {
        let path = self.root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn session(&self) -> String {
        format!("e2e-{}", self.harness)
    }

    fn post_tool_write(&self, relative: &str) -> Value {
        let project = self.root.to_string_lossy();
        match self.harness {
            "claude" => json!({
                "session_id": self.session(),
                "transcript_path": "/tmp/e2e-transcript.jsonl",
                "cwd": project,
                "hook_event_name": "PostToolUse",
                "tool_name": "Write",
                "tool_input": {"file_path": relative, "content": "x"},
                "tool_use_id": format!("write-{relative}"),
                "tool_response": {"filePath": self.root.join(relative).to_string_lossy()}
            }),
            "codex" => json!({
                "session_id": self.session(),
                "transcript_path": "/tmp/e2e-transcript.jsonl",
                "cwd": project,
                "hook_event_name": "PostToolUse",
                "model": "gpt-test",
                "turn_id": "turn-1",
                "permission_mode": "default",
                "tool_name": "Write",
                "tool_use_id": format!("write-{relative}"),
                "tool_input": {"file_path": relative, "content": "x"},
                "tool_response": {"filePath": self.root.join(relative).to_string_lossy()}
            }),
            _ => self.antigravity_command(&format!("printf x > {relative}")),
        }
    }

    fn post_tool_dynamic_shell(&self) -> Value {
        let command = "mystery $TARGET";
        match self.harness {
            "claude" => json!({
                "session_id": self.session(),
                "transcript_path": "/tmp/e2e-transcript.jsonl",
                "cwd": self.root.to_string_lossy(),
                "hook_event_name": "PostToolUse",
                "tool_name": "Bash",
                "tool_input": {"command": command},
                "tool_use_id": "dynamic-shell",
                "tool_response": {"stdout": "", "stderr": "", "interrupted": false}
            }),
            "codex" => json!({
                "session_id": self.session(),
                "transcript_path": "/tmp/e2e-transcript.jsonl",
                "cwd": self.root.to_string_lossy(),
                "hook_event_name": "PostToolUse",
                "model": "gpt-test",
                "turn_id": "turn-1",
                "permission_mode": "default",
                "tool_name": "Bash",
                "tool_use_id": "dynamic-shell",
                "tool_input": {"command": command},
                "tool_response": {"exit_code": 0}
            }),
            _ => self.antigravity_command(command),
        }
    }

    fn antigravity_command(&self, command: &str) -> Value {
        json!({
            "conversationId": self.session(),
            "workspacePaths": [self.root.to_string_lossy()],
            "transcriptPath": self.root.join("transcript.jsonl").to_string_lossy(),
            "artifactDirectoryPath": self.root.join("artifacts").to_string_lossy(),
            "toolCall": {
                "name": "run_command",
                "args": {"CommandLine": command, "Cwd": self.root.to_string_lossy()}
            },
            "stepIdx": 1
        })
    }

    fn stop(&self, stop_hook_active: bool) -> Value {
        let project = self.root.to_string_lossy();
        match self.harness {
            "claude" => json!({
                "session_id": self.session(),
                "transcript_path": "/tmp/e2e-transcript.jsonl",
                "cwd": project,
                "hook_event_name": "Stop",
                "stop_hook_active": stop_hook_active,
                "last_assistant_message": "done"
            }),
            "codex" => json!({
                "session_id": self.session(),
                "transcript_path": "/tmp/e2e-transcript.jsonl",
                "cwd": project,
                "hook_event_name": "Stop",
                "model": "gpt-test",
                "turn_id": "turn-1",
                "permission_mode": "default",
                "stop_hook_active": stop_hook_active,
                "last_assistant_message": "done"
            }),
            _ => json!({
                "conversationId": self.session(),
                "workspacePaths": [project],
                "transcriptPath": self.root.join("transcript.jsonl").to_string_lossy(),
                "artifactDirectoryPath": self.root.join("artifacts").to_string_lossy(),
                "executionNum": 1,
                "terminationReason": "model_stop",
                "fullyIdle": true
            }),
        }
    }

    fn run(&self, binary: &str, input: &Value) -> std::process::Output {
        let mut command = Command::new(binary);
        command
            .arg(format!("--{}", self.harness))
            .arg("--state-dir")
            .arg(&self.state)
            .current_dir(&self.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
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
        ] {
            command.env_remove(name);
        }
        if self.harness == "claude" {
            command
                .env("CLAUDECODE", "1")
                .env("CLAUDE_CODE_CHILD_SESSION", "1")
                .env("CLAUDE_CODE_SESSION_ID", self.session())
                .env("CLAUDE_PROJECT_DIR", &self.root);
        }
        let mut child = command.spawn().expect("spawn hook binary");
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

    fn observe(&self, input: &Value) {
        let output = self.run(env!("CARGO_BIN_EXE_file-activity-agent-hook"), input);
        assert!(
            output.status.success(),
            "observer failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Run one Stop, check the cross-harness invariants, and return stdout.
    ///
    /// Every response is one JSON object, except that Codex's snapshot
    /// defines empty stdout as the `no-op` outcome: an allowed Codex Stop
    /// with nothing to report prints nothing. That case alone is returned as
    /// `Value::Null`, which no harness ever prints and which reads as neither
    /// blocked nor carrying a message.
    fn stop_hook(&self, stop_hook_active: bool) -> Value {
        let output = self.run(
            env!("CARGO_BIN_EXE_turn-completion-agent-hook"),
            &self.stop(stop_hook_active),
        );
        assert_eq!(
            output.status.code(),
            Some(0),
            "{} Stop failed: {}",
            self.harness,
            String::from_utf8_lossy(&output.stderr)
        );
        if output.stdout.is_empty() {
            assert_eq!(
                self.harness, "codex",
                "only the Codex no-op is empty stdout; {} printed nothing",
                self.harness
            );
            return Value::Null;
        }
        let stdout: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "{} Stop stdout is not JSON ({error}): {}",
                self.harness,
                String::from_utf8_lossy(&output.stdout)
            )
        });
        if is_blocked(self.harness, &stdout) {
            let reason = stdout["reason"].as_str().unwrap_or_default();
            assert!(
                !reason.trim().is_empty(),
                "{} block without a reason: {stdout}",
                self.harness
            );
        } else if self.harness == "claude" {
            assert!(
                stdout.get("hookSpecificOutput").is_none(),
                "an allowed Claude Stop must not continue the conversation: {stdout}"
            );
        }
        if self.harness == "antigravity" && !is_blocked(self.harness, &stdout) {
            assert!(
                stdout.get("reason").is_none(),
                "Antigravity ignores reason on an allowed stop: {stdout}"
            );
        }
        stdout
    }

    fn summaries(&self) -> Vec<Value> {
        let mut found = Vec::new();
        collect_named(&self.state, "summary.json", &mut found);
        found.sort();
        found
            .into_iter()
            .map(|path| serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap())
            .collect()
    }

    fn latest_summary(&self) -> Value {
        let summaries = self.summaries();
        summaries
            .into_iter()
            .max_by_key(|summary| summary["run"]["id"].as_str().unwrap_or_default().to_owned())
            .expect("a summary")
    }

    fn checker_invocations(&self) -> Vec<usize> {
        std::fs::read_to_string(&self.trace)
            .unwrap_or_default()
            .lines()
            .map(|line| line.trim().parse().unwrap())
            .collect()
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn collect_named(root: &Path, name: &str, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect_named(&path, name, found);
        } else if path.file_name().and_then(|value| value.to_str()) == Some(name) {
            found.push(path);
        }
    }
}

fn is_blocked(harness: &str, stdout: &Value) -> bool {
    match harness {
        "antigravity" => stdout["decision"] == "continue",
        _ => stdout["decision"] == "block",
    }
}

#[test]
fn claude_post_tool_use_failure_writes_reach_the_stop_runner() {
    require_pkl!();
    let project = Project::new("post-tool-failure", "claude");
    project.configure(
        &project.checker(),
        r#"  fileActivity { filesystemMtime = false }"#,
    );
    // The command wrote the file before its failing test step exited 1, so
    // Claude reports it only through PostToolUseFailure.
    project.write("notes/failing.txt", "MANUAL\n");
    let failure = json!({
        "session_id": project.session(),
        "transcript_path": "/tmp/e2e-transcript.jsonl",
        "cwd": project.root.to_string_lossy(),
        "hook_event_name": "PostToolUseFailure",
        "tool_name": "Bash",
        "tool_input": {"command": "printf MANUAL > notes/failing.txt && false"},
        "tool_use_id": "failed-bash",
        "error": "Exit code 1"
    });
    let output = project.run(env!("CARGO_BIN_EXE_file-activity-agent-hook"), &failure);
    assert!(
        output.status.success(),
        "observer failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The observer stays quiet: the PostToolUseFailure no-op is an empty object.
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({}),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );

    let stop = project.stop_hook(false);
    assert!(is_blocked("claude", &stop), "{stop}");
    assert_eq!(project.checker_invocations(), vec![1]);

    // The observer rejects Claude events that carry no completed tool call.
    let other = project.run(
        env!("CARGO_BIN_EXE_file-activity-agent-hook"),
        &project.stop(false),
    );
    assert_eq!(other.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&other.stderr).contains("PostToolUseFailure"),
        "{}",
        String::from_utf8_lossy(&other.stderr)
    );
}

#[test]
fn unchanged_manual_issues_block_once_per_continuation_loop() {
    require_pkl!();
    for harness in HARNESSES {
        let project = Project::new("manual-loop", harness);
        // An empty agent template must still produce a non-empty block reason.
        project.configure(
            &project.checker(),
            r#"  fileActivity { filesystemMtime = false }
  deferredReporting = new DeferredReporting {
    manualFixesNeeded = new TemplatePair { agent = "" }
  }"#,
        );
        project.write("a.txt", "MANUAL\n");
        project.observe(&project.post_tool_write("a.txt"));

        let first = project.stop_hook(false);
        assert!(is_blocked(harness, &first), "{harness}: {first}");
        assert!(
            first["reason"].as_str().unwrap().contains("summary.json"),
            "{harness}: the synthesized reason points at the summary: {first}"
        );

        // The agent stops again without changing anything.
        let repeat = project.stop_hook(true);
        assert!(
            !is_blocked(harness, &repeat),
            "{harness}: an unchanged repeat must not block again: {repeat}"
        );
        if harness != "antigravity" {
            assert!(
                repeat["systemMessage"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("not blocking Stop again"),
                "{harness}: {repeat}"
            );
        }
        assert_eq!(
            project.latest_summary()["stopDecision"]["suppressedRepeat"],
            true
        );

        if harness != "antigravity" {
            // A new turn (no continuation) still gets one block for a problem
            // that is still present.
            let next_turn = project.stop_hook(false);
            assert!(is_blocked(harness, &next_turn), "{harness}: {next_turn}");
        }

        // Fixing the file lets the next Stop pass.
        project.write("a.txt", "CLEAN\n");
        let fixed = project.stop_hook(true);
        assert!(!is_blocked(harness, &fixed), "{harness}: {fixed}");
    }
}

#[test]
fn coverage_gaps_are_reported_once_instead_of_reblocking_every_stop() {
    require_pkl!();
    for harness in HARNESSES {
        let project = Project::new("gap-once", harness);
        project.configure(
            &project.checker(),
            r#"  fileActivity { filesystemMtime = false; coverageGapPolicy = "strict" }"#,
        );
        project.observe(&project.post_tool_dynamic_shell());

        let first = project.stop_hook(false);
        assert!(
            is_blocked(harness, &first),
            "{harness}: strict blocks the Stop that first reports a gap: {first}"
        );
        let summary = project.latest_summary();
        assert!(summary["counts"]["coverageGaps"].as_u64().unwrap() >= 1);
        assert!(
            summary["stateDisposition"]["retryTargets"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        for stop_hook_active in [true, false] {
            let later = project.stop_hook(stop_hook_active);
            assert!(
                !is_blocked(harness, &later),
                "{harness}: a reported gap must not block later Stops: {later}"
            );
            if harness != "antigravity" {
                assert!(
                    !later
                        .get("systemMessage")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .contains("coverage is incomplete"),
                    "{harness}: the gap is not re-reported: {later}"
                );
            }
        }
    }
}

#[test]
fn missing_tools_are_reported_without_blocking_or_requeueing() {
    require_pkl!();
    for harness in HARNESSES {
        let project = Project::new("missing-tool", harness);
        project.configure(
            &project.root.join("bin/definitely-missing"),
            r#"  fileActivity { filesystemMtime = false }"#,
        );
        project.write("a.txt", "MANUAL\n");
        project.observe(&project.post_tool_write("a.txt"));

        let first = project.stop_hook(false);
        assert!(!is_blocked(harness, &first), "{harness}: {first}");
        if harness != "antigravity" {
            let message = first["systemMessage"].as_str().unwrap_or_default();
            assert!(
                message.contains("definitely-missing") && message.contains("install the checker"),
                "{harness}: the user learns which tool is missing: {first}"
            );
        }
        let summary = project.latest_summary();
        assert_eq!(summary["counts"]["unavailableTools"], 1);
        assert_eq!(summary["counts"]["operationalErrors"], 0);
        assert!(
            summary["stateDisposition"]["retryFiles"]
                .as_array()
                .unwrap()
                .is_empty(),
            "{harness}: files of a missing tool are not retried forever"
        );

        let second = project.stop_hook(true);
        assert!(!is_blocked(harness, &second), "{harness}: {second}");
        assert_eq!(
            project.summaries().len(),
            1,
            "{harness}: nothing was left pending for a second run"
        );
    }
}

#[test]
fn allowed_claude_stop_after_an_auto_fix_does_not_continue_the_conversation() {
    require_pkl!();
    let project = Project::new("claude-allowed", "claude");
    let fixer = project.root.join("bin/fixer");
    std::fs::write(
        &fixer,
        "#!/bin/sh\nmode=$1\nshift\nfor file in \"$@\"; do\n  if [ \"$mode\" = fix ]; then sed 's/MANUAL/CLEAN/' \"$file\" > \"$file.tmp\" && mv \"$file.tmp\" \"$file\"; elif grep -q MANUAL \"$file\"; then exit 1; fi\ndone\nexit 0\n",
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&fixer).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&fixer, permissions).unwrap();
    let config_dir = project.root.join(".agent-hook-kit");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("post-tool-use.pkl"),
        format!(
            r#"amends "Config.pkl"

settings {{ fileActivity {{ filesystemMtime = false }} }}

tools {{
  ["fix"] = new ToolSpec {{
    id = "fix"
    displayName = "Fix"
    executable = "{}"
    files {{ include = new Listing {{ "**/*.txt" }} }}
    workflows {{
      ["lint"] = new Workflow {{
        check = new WorkflowCommand {{
          argv = new Listing {{ "check"; new Files {{}} }}
          exitCodes {{ issues = new Listing {{ 1 }} }}
        }}
        remedy = new WorkflowCommand {{
          argv = new Listing {{ "fix"; new Files {{}} }}
          writes = "target-files"
        }}
      }}
    }}
  }}
}}
run = new Listing {{ "fix" }}
"#,
            fixer.display()
        ),
    )
    .unwrap();
    project.write("a.txt", "MANUAL\n");
    project.observe(&project.post_tool_write("a.txt"));

    let stop = project.stop_hook(false);
    assert!(!is_blocked("claude", &stop), "{stop}");
    let message = stop["systemMessage"].as_str().unwrap();
    assert!(message.contains("Auto-fixed 1 file"), "{message}");
    assert_eq!(
        std::fs::read_to_string(project.root.join("a.txt")).unwrap(),
        "CLEAN\n"
    );
}

#[test]
fn large_candidate_sets_are_chunked_below_the_argument_limit() {
    require_pkl!();
    let project = Project::new("argmax", "claude");
    project.configure(
        &project.checker(),
        r#"  fileActivity { filesystemMtime = false }"#,
    );
    // Well over the 128 KiB file-argument budget, so one batch must become
    // several invocations (unchunked, a large enough set fails with E2BIG).
    let stem = "x".repeat(180);
    let files = (0..900)
        .map(|index| project.write(&format!("generated/{stem}-{index:05}.txt"), "CLEAN\n"))
        .collect::<Vec<_>>();
    // Seed the pending window with one scoped target instead of 900 events.
    let state = hookkit_session_state::SessionState::open(
        hookkit_core::HarnessId::CLAUDE_CODE,
        hookkit_session_state::SessionIdentity::Session(project.session()),
        hookkit_session_state::StateRoot::new(&project.state),
    )
    .unwrap();
    hookkit_file_activity::FileActivityStore::from_state(state)
        .unwrap()
        .requeue_targets(
            "e2e-seed",
            [hookkit_file_activity::FileActivityTarget::Path {
                path: hookkit_core::Utf8PathBuf::from_path_buf(project.root.join("generated"))
                    .unwrap(),
                scope: hookkit_file_activity::FileActivityScope::Descendants,
            }],
        )
        .unwrap();

    let stop = project.stop_hook(false);
    assert!(!is_blocked("claude", &stop), "{stop}");
    let summary = project.latest_summary();
    assert_eq!(summary["counts"]["operationalErrors"], 0, "{summary}");
    assert_eq!(summary["counts"]["clean"], files.len());
    let invocations = project.checker_invocations();
    assert!(invocations.len() > 1, "{invocations:?}");
    assert_eq!(invocations.iter().sum::<usize>(), files.len());
}
