//! Process-level tests for the stdin/stdout runners: `run_event*`,
//! `run_harness*`, and `dispatch_builtin_harness*`.
//!
//! Each test re-executes this test binary with only [`HELPER`] selected and a
//! scenario name in [`SCENARIO`]. The helper runs the scenario's runner
//! against real stdin, stdout, stderr, and exit status, which is the only way
//! to observe what a harness would see.

use hookkit_core::{BuiltinHarness, Diagnostic, DiagnosticsSink, EventId, HarnessId, HookkitError};
use hookkit_runtime::{
    FailurePolicy, RunOptions, dispatch_builtin_harness_with_options, run_event,
    run_event_with_options, run_harness, run_harness_with_options,
};
use std::io::Write;
use std::process::{Command, ExitCode, Stdio};

const HELPER: &str = "stdin_runner_helper";
const SCENARIO: &str = "HOOKKIT_STDIN_RUNNER_SCENARIO";
const SINK_FILE: &str = "HOOKKIT_STDIN_RUNNER_SINK";

/// A diagnostics sink that appends each diagnostic as one line to a file, so
/// the parent test can read what the child recorded.
struct FileSink(std::path::PathBuf);

impl DiagnosticsSink for FileSink {
    fn record(&self, diagnostic: Diagnostic) {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.0)
            .unwrap();
        writeln!(file, "{:?} {}", diagnostic.level, diagnostic.message).unwrap();
    }
}

fn fail_closed() -> RunOptions<'static> {
    RunOptions::new().with_failure_policy(FailurePolicy::FailClosed)
}

fn codex_deny(reason: String) -> hookkit_core::Result<hookkit_codex::protocol::PreToolUseOutput> {
    Ok(hookkit_codex::protocol::PreToolUseOutput::deny(reason))
}

/// Runs the selected scenario and exits with the runner's exit code. It does
/// nothing in a normal test run.
#[test]
fn stdin_runner_helper() {
    let Some(scenario) = std::env::var_os(SCENARIO) else {
        return;
    };
    let code = match scenario.to_str().unwrap() {
        "typed-default" => run_event::<hookkit_codex::protocol::PreToolUse, _>(|_, _, _| {
            codex_deny("unreachable".into())
        }),
        "typed-fail-closed" => run_event_with_options::<hookkit_codex::protocol::PreToolUse, _>(
            fail_closed(),
            |_, _, _| codex_deny("unreachable".into()),
        ),
        "typed-fail-closed-handler-error" => {
            run_event_with_options::<hookkit_codex::protocol::PreToolUse, _>(
                fail_closed(),
                |_, _, _| Err(HookkitError::handler("policy file is missing")),
            )
        }
        "typed-fail-closed-panic" => {
            run_event_with_options::<hookkit_codex::protocol::PreToolUse, _>(
                fail_closed(),
                |_, _, _| panic!("policy engine exploded"),
            )
        }
        "typed-fail-closed-observer" => {
            run_event_with_options::<hookkit_codex::protocol::PostToolUse, _>(
                fail_closed(),
                |_, _, _| unreachable!("the payload is invalid"),
            )
        }
        "typed-echo-command" => {
            run_event::<hookkit_codex::protocol::PreToolUse, _>(|_, _, context| {
                let command = context.raw().json()["tool_input"]["command"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                codex_deny(format!("saw {command}"))
            })
        }
        "typed-sink" => {
            let sink = FileSink(std::env::var_os(SINK_FILE).unwrap().into());
            run_event_with_options::<hookkit_codex::protocol::PreToolUse, _>(
                RunOptions::new().with_diagnostics(&sink),
                |_, _, _| codex_deny("unreachable".into()),
            )
        }
        "claude-session-start" => {
            run_event::<hookkit_claude::protocol::SessionStart, _>(|_, _, _| {
                Ok(hookkit_claude::protocol::SessionStartOutput::no_op())
            })
        }
        "harness-deny" => run_harness::<hookkit_codex::protocol::Codex, _>(None, |_, _, _| {
            Ok(hookkit_codex::protocol::AnyCommandOutput::PreToolUse(
                hookkit_codex::protocol::PreToolUseOutput::deny("blocked by run_harness"),
            ))
        }),
        "harness-fail-closed" => run_harness_with_options::<hookkit_codex::protocol::Codex, _>(
            None,
            fail_closed(),
            |_, _, _| unreachable!("the payload is invalid"),
        ),
        "dispatch-codex-deny" => dispatch_builtin_harness_with_options(
            BuiltinHarness::Codex,
            None,
            RunOptions::new(),
            |_, _, _| {
                Ok(hookkit_runtime::BuiltinOutput::Codex(
                    hookkit_codex::protocol::AnyCommandOutput::PreToolUse(
                        hookkit_codex::protocol::PreToolUseOutput::deny("blocked by dispatch"),
                    ),
                ))
            },
        ),
        "dispatch-antigravity-fail-closed" => dispatch_builtin_harness_with_options(
            BuiltinHarness::Antigravity,
            Some(EventId::builtin(HarnessId::ANTIGRAVITY, "PreToolUse")),
            fail_closed(),
            |_, _, _| unreachable!("the payload is invalid"),
        ),
        "dispatch-claude-fail-closed" => dispatch_builtin_harness_with_options(
            BuiltinHarness::ClaudeCode,
            None,
            fail_closed(),
            |_, _, _| unreachable!("the payload is invalid"),
        ),
        other => panic!("unknown stdin runner scenario {other}"),
    };
    let code = (0..=u8::MAX)
        .find(|candidate| ExitCode::from(*candidate) == code)
        .expect("runner exit code");
    std::process::exit(i32::from(code));
}

struct Run {
    code: Option<i32>,
    /// Stdout after libtest's `running 1 test` / `test <name> ... ` preamble:
    /// exactly what the runner wrote.
    stdout: String,
    stderr: String,
}

fn spawn(scenario: &str, stdin: &[u8], envs: &[(&str, &std::ffi::OsStr)]) -> Run {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", HELPER, "--nocapture", "--test-threads=1"])
        // A hermetic environment: no ambient harness variables leak in from
        // the process running the tests (which may itself be a hook harness).
        .env_clear()
        .env(SCENARIO, scenario)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in envs {
        command.env(name, value);
    }
    let output = command
        .spawn()
        .and_then(|mut child| {
            child.stdin.take().unwrap().write_all(stdin)?;
            child.wait_with_output()
        })
        .expect("stdin runner helper should run");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let preamble = format!("running 1 test\ntest {HELPER} ... ");
    let stdout = stdout
        .split_once(&preamble)
        .unwrap_or_else(|| panic!("unexpected libtest output {stdout:?}"))
        .1
        .to_owned();
    Run {
        code: output.status.code(),
        stdout,
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

fn codex_pre_tool_use(command: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "session_id": "stdin-session",
        "transcript_path": null,
        "cwd": "/tmp",
        "hook_event_name": "PreToolUse",
        "model": "gpt-test",
        "turn_id": "stdin-turn",
        "permission_mode": "default",
        "tool_name": "Bash",
        "tool_use_id": "stdin-call",
        "tool_input": {"command": command}
    }))
    .unwrap()
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("{error}: {text:?}"))
}

/// The runner's own diagnostic line, ignoring the panic message the default
/// panic hook may print first.
fn diagnostic_line(stderr: &str) -> &str {
    stderr
        .lines()
        .find(|line| line.starts_with("hookkit: "))
        .unwrap_or_else(|| panic!("no hookkit diagnostic in {stderr:?}"))
}

#[test]
fn default_policy_exits_one_with_one_diagnostic_line_and_empty_stdout() {
    let outcome = spawn("typed-default", b"not json", &[]);
    assert_eq!(outcome.code, Some(1));
    assert_eq!(outcome.stdout, "");
    assert_eq!(outcome.stderr.lines().count(), 1, "{:?}", outcome.stderr);
    assert!(
        outcome
            .stderr
            .contains("codex/PreToolUse failed: invalid JSON: expected ident"),
        "{:?}",
        outcome.stderr
    );
}

#[test]
fn fail_closed_gate_exits_two_with_the_diagnostic_as_the_reason() {
    for stdin in [
        &b"not json"[..],
        b"",
        b"{\"hook_event_name\":\"PreToolUse\"}",
    ] {
        let outcome = spawn("typed-fail-closed", stdin, &[]);
        assert_eq!(outcome.code, Some(2), "{:?}", outcome.stderr);
        assert_eq!(outcome.stdout, "");
        assert!(
            outcome.stderr.starts_with("hookkit: ")
                && outcome.stderr.contains("codex/PreToolUse failed"),
            "{:?}",
            outcome.stderr
        );
    }
}

#[test]
fn fail_closed_gate_denies_on_handler_errors_and_panics() {
    let error = spawn(
        "typed-fail-closed-handler-error",
        &codex_pre_tool_use("ls"),
        &[],
    );
    assert_eq!(error.code, Some(2));
    assert_eq!(error.stdout, "");
    assert!(
        error
            .stderr
            .contains("hook handler failed: policy file is missing"),
        "{:?}",
        error.stderr
    );

    let panicked = spawn("typed-fail-closed-panic", &codex_pre_tool_use("ls"), &[]);
    assert_eq!(panicked.code, Some(2), "a panic must not exit 101");
    assert_eq!(panicked.stdout, "");
    assert!(
        diagnostic_line(&panicked.stderr).contains("panicked: policy engine exploded"),
        "{:?}",
        panicked.stderr
    );
}

#[test]
fn fail_closed_observers_stay_non_blocking() {
    let outcome = spawn("typed-fail-closed-observer", b"not json", &[]);
    assert_eq!(outcome.code, Some(1));
    assert_eq!(outcome.stdout, "");
    assert!(outcome.stderr.contains("codex/PostToolUse failed"));
}

#[test]
fn lone_surrogate_escapes_reach_the_handler_as_replacement_characters() {
    let payload = String::from_utf8(codex_pre_tool_use("echo X")).unwrap();
    let payload = payload.replace("echo X", r"echo \ud83d");
    let outcome = spawn("typed-echo-command", payload.as_bytes(), &[]);
    assert_eq!(outcome.code, Some(0), "{:?}", outcome.stderr);
    assert_eq!(
        json(&outcome.stdout)["hookSpecificOutput"]["permissionDecisionReason"],
        "saw echo \u{FFFD}"
    );
}

#[test]
fn runner_failures_are_recorded_in_the_diagnostics_sink() {
    let sink = std::env::temp_dir().join(format!(
        "hookkit-stdin-runner-sink-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let outcome = spawn("typed-sink", b"not json", &[(SINK_FILE, sink.as_os_str())]);
    assert_eq!(outcome.code, Some(1));
    let recorded = std::fs::read_to_string(&sink).unwrap();
    let _ = std::fs::remove_file(&sink);
    assert_eq!(recorded.lines().count(), 1, "{recorded:?}");
    assert!(recorded.starts_with("Error hookkit: "), "{recorded:?}");
    assert!(recorded.contains("codex/PreToolUse failed: invalid JSON"));
}

#[cfg(unix)]
#[test]
fn a_non_utf8_prefix_variable_does_not_disable_the_hook() {
    use std::os::unix::ffi::OsStrExt;
    let payload = serde_json::to_vec(&serde_json::json!({
        "session_id": "stdin-session",
        "transcript_path": "/tmp/stdin-session.jsonl",
        "cwd": "/tmp",
        "hook_event_name": "SessionStart",
        "source": "startup"
    }))
    .unwrap();
    let stray = std::ffi::OsStr::from_bytes(b"stray\xff");
    let outcome = spawn(
        "claude-session-start",
        &payload,
        &[
            ("CLAUDECODE", "1".as_ref()),
            ("CLAUDE_CODE_CHILD_SESSION", "1".as_ref()),
            ("CLAUDE_CODE_SESSION_ID", "stdin-session".as_ref()),
            ("CLAUDE_PROJECT_DIR", "/tmp".as_ref()),
            ("CLAUDE_ENV_FILE", "/tmp/hookkit-stdin-runner-env".as_ref()),
            ("CLAUDE_PLUGIN_OPTION_STRAY", stray),
        ],
    );
    assert_eq!(outcome.code, Some(0), "{:?}", outcome.stderr);
    assert_eq!(outcome.stderr, "");
}

#[test]
fn run_harness_resolves_the_event_and_writes_native_stdout() {
    let outcome = spawn("harness-deny", &codex_pre_tool_use("rm -rf /"), &[]);
    assert_eq!(outcome.code, Some(0), "{:?}", outcome.stderr);
    assert_eq!(outcome.stderr, "");
    let output = json(&outcome.stdout);
    assert_eq!(output["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(
        output["hookSpecificOutput"]["permissionDecisionReason"],
        "blocked by run_harness"
    );
}

#[test]
fn run_harness_fail_closed_uses_the_discriminated_event() {
    // The discriminator names a gate whose parser rejects the payload.
    let gate = spawn(
        "harness-fail-closed",
        br#"{"hook_event_name":"PreToolUse"}"#,
        &[],
    );
    assert_eq!(gate.code, Some(2), "{:?}", gate.stderr);
    assert!(
        gate.stderr.contains("codex/PreToolUse failed"),
        "{:?}",
        gate.stderr
    );

    // The same failure on an observer event stays non-blocking.
    let observer = spawn(
        "harness-fail-closed",
        br#"{"hook_event_name":"PostToolUse"}"#,
        &[],
    );
    assert_eq!(observer.code, Some(1), "{:?}", observer.stderr);
    assert_eq!(observer.stdout, "");

    // Nothing identifies the event, so a Codex runner denies.
    let unknown = spawn("harness-fail-closed", b"not json", &[]);
    assert_eq!(unknown.code, Some(2));
    assert!(
        unknown.stderr.contains(" codex failed: invalid JSON"),
        "{:?}",
        unknown.stderr
    );
}

#[test]
fn dispatch_builtin_harness_writes_native_stdout() {
    let outcome = spawn("dispatch-codex-deny", &codex_pre_tool_use("rm -rf /"), &[]);
    assert_eq!(outcome.code, Some(0), "{:?}", outcome.stderr);
    assert_eq!(
        json(&outcome.stdout)["hookSpecificOutput"]["permissionDecisionReason"],
        "blocked by dispatch"
    );
}

#[test]
fn antigravity_fails_closed_with_a_json_deny_at_exit_zero() {
    let outcome = spawn("dispatch-antigravity-fail-closed", b"not json", &[]);
    assert_eq!(outcome.code, Some(0), "{:?}", outcome.stderr);
    let output = json(&outcome.stdout);
    assert_eq!(output["decision"], "deny");
    let reason = output["reason"].as_str().unwrap();
    assert!(
        reason.starts_with("hookkit: ") && reason.contains("antigravity/PreToolUse failed"),
        "{reason:?}"
    );
    assert!(outcome.stderr.contains("antigravity/PreToolUse failed"));
}

#[test]
fn claude_permission_request_fails_closed_with_a_json_deny() {
    let outcome = spawn(
        "dispatch-claude-fail-closed",
        br#"{"hook_event_name":"PermissionRequest"}"#,
        &[],
    );
    assert_eq!(outcome.code, Some(0), "{:?}", outcome.stderr);
    let output = json(&outcome.stdout);
    assert_eq!(
        output["hookSpecificOutput"]["hookEventName"],
        "PermissionRequest"
    );
    assert_eq!(output["hookSpecificOutput"]["decision"]["behavior"], "deny");

    // Claude's gating events deny with exit 2.
    let gate = spawn(
        "dispatch-claude-fail-closed",
        br#"{"hook_event_name":"UserPromptSubmit"}"#,
        &[],
    );
    assert_eq!(gate.code, Some(2), "{:?}", gate.stderr);
    assert_eq!(gate.stdout, "");
    assert!(gate.stderr.contains("claude-code/UserPromptSubmit failed"));

    // Blocking a Claude Stop would keep the agent working, so it stays
    // non-blocking even when failing closed.
    let stop = spawn(
        "dispatch-claude-fail-closed",
        br#"{"hook_event_name":"Stop"}"#,
        &[],
    );
    assert_eq!(stop.code, Some(1), "{:?}", stop.stderr);
}
