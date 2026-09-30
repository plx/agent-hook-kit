//! Process-level tests for the aligned stdin/stdout runners:
//! `run_aligned_event`, `run_aligned_event_with_diagnostics`, and
//! `run_aligned_event_with_options`.
//!
//! Each test re-executes this test binary with only [`HELPER`] selected and a
//! scenario name in [`SCENARIO`], so the runner reads real stdin and its
//! stdout, stderr, and exit status are exactly what a harness would observe.

use hookkit_common::{PreToolUseOutput, TurnCompletionOutput, UserPromptSubmitOutput};
use hookkit_core::{Diagnostic, DiagnosticLevel, DiagnosticsSink, HarnessId};
use hookkit_runtime::RunOptions;
use hookkit_runtime::aligned::{
    PreToolUse, TurnCompletion, UserPromptSubmit, run_aligned_event,
    run_aligned_event_with_diagnostics, run_aligned_event_with_options,
};
use std::io::Write;
use std::process::{Command, ExitCode, Stdio};

const HELPER: &str = "aligned_stdin_helper";
const SCENARIO: &str = "HOOKKIT_ALIGNED_STDIN_SCENARIO";
const SINK_FILE: &str = "HOOKKIT_ALIGNED_STDIN_SINK";

/// Appends each diagnostic as one line to a file the parent test reads.
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

/// Runs the selected scenario and exits with the runner's exit code. It does
/// nothing in a normal test run.
#[test]
fn aligned_stdin_helper() {
    let Some(scenario) = std::env::var_os(SCENARIO) else {
        return;
    };
    let fail_closed = RunOptions::new().fail_closed();
    let code = match scenario.to_str().unwrap() {
        "pre-tool-default" => {
            run_aligned_event::<PreToolUse, _>(HarnessId::CODEX, |_, _, context| {
                PreToolUseOutput::pass_through(context.harness())
            })
        }
        "pre-tool-pass-through" => {
            run_aligned_event::<PreToolUse, _>(HarnessId::CLAUDE_CODE, |_, _, context| {
                PreToolUseOutput::pass_through(context.harness())
            })
        }
        "pre-tool-fail-closed-codex" => run_aligned_event_with_options::<PreToolUse, _>(
            HarnessId::CODEX,
            fail_closed,
            |_, _, context| PreToolUseOutput::pass_through(context.harness()),
        ),
        "pre-tool-fail-closed-antigravity" => run_aligned_event_with_options::<PreToolUse, _>(
            HarnessId::ANTIGRAVITY,
            fail_closed,
            |_, _, context| PreToolUseOutput::pass_through(context.harness()),
        ),
        "pre-tool-fail-closed-blank-reason" => run_aligned_event_with_options::<PreToolUse, _>(
            HarnessId::CODEX,
            fail_closed,
            |_, _, context| PreToolUseOutput::deny(context.harness(), " "),
        ),
        "pre-tool-fail-closed-panic" => run_aligned_event_with_options::<PreToolUse, _>(
            HarnessId::CODEX,
            fail_closed,
            |_, _, _| panic!("policy engine exploded"),
        ),
        "user-prompt-fail-closed-claude" => run_aligned_event_with_options::<UserPromptSubmit, _>(
            HarnessId::CLAUDE_CODE,
            fail_closed,
            |_, _, context| UserPromptSubmitOutput::no_op(context.harness()),
        ),
        "turn-completion-fail-closed" => run_aligned_event_with_options::<TurnCompletion, _>(
            HarnessId::CODEX,
            fail_closed,
            |_, _, context| TurnCompletionOutput::allow(context.harness()),
        ),
        "pre-tool-sink" => {
            let sink = FileSink(std::env::var_os(SINK_FILE).unwrap().into());
            run_aligned_event_with_diagnostics::<PreToolUse, _>(
                HarnessId::CODEX,
                &sink,
                |_, _, context| {
                    context.diagnostics().record(Diagnostic::new(
                        DiagnosticLevel::Info,
                        "handler saw the call",
                    ));
                    PreToolUseOutput::deny(context.harness(), "")
                },
            )
        }
        other => panic!("unknown aligned stdin scenario {other}"),
    };
    let code = (0..=u8::MAX)
        .find(|candidate| ExitCode::from(*candidate) == code)
        .expect("runner exit code");
    std::process::exit(i32::from(code));
}

struct Run {
    code: Option<i32>,
    /// Stdout after libtest's preamble: exactly what the runner wrote.
    stdout: String,
    stderr: String,
}

fn spawn(scenario: &str, stdin: &[u8], envs: &[(&str, &str)]) -> Run {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", HELPER, "--nocapture", "--test-threads=1"])
        // No ambient harness variables leak in from the process running the
        // tests, which may itself be a hook harness.
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
        .expect("aligned stdin helper should run");
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

fn codex_pre_tool_use() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "session_id": "aligned-session",
        "transcript_path": null,
        "cwd": "/tmp",
        "hook_event_name": "PreToolUse",
        "model": "gpt-test",
        "turn_id": "aligned-turn",
        "permission_mode": "default",
        "tool_name": "Bash",
        "tool_use_id": "aligned-call",
        "tool_input": {"command": "true"}
    }))
    .unwrap()
}

fn diagnostic_line(stderr: &str) -> &str {
    stderr
        .lines()
        .find(|line| line.starts_with("hookkit: "))
        .unwrap_or_else(|| panic!("no hookkit diagnostic in {stderr:?}"))
}

#[test]
fn default_aligned_runner_fails_open_with_exit_one() {
    let outcome = spawn("pre-tool-default", b"not json", &[]);
    assert_eq!(outcome.code, Some(1));
    assert_eq!(outcome.stdout, "");
    assert!(
        diagnostic_line(&outcome.stderr).contains("codex/PreToolUse failed: invalid JSON"),
        "{:?}",
        outcome.stderr
    );
}

#[test]
fn claude_pass_through_writes_the_empty_native_response() {
    let payload = serde_json::json!({
        "session_id": "s",
        "transcript_path": "/tmp/t.jsonl",
        "cwd": "/repo/src",
        "hook_event_name": "PreToolUse",
        "permission_mode": "default",
        "tool_name": "Bash",
        "tool_input": {"command": "rm -rf build"},
        "tool_use_id": "toolu_1"
    });
    let outcome = spawn(
        "pre-tool-pass-through",
        &serde_json::to_vec(&payload).unwrap(),
        &[
            ("CLAUDECODE", "1"),
            ("CLAUDE_CODE_CHILD_SESSION", "1"),
            ("CLAUDE_CODE_SESSION_ID", "s"),
            ("CLAUDE_PROJECT_DIR", "/repo"),
        ],
    );
    assert_eq!(outcome.code, Some(0), "{:?}", outcome.stderr);
    assert_eq!(outcome.stdout, "{}");
    assert!(outcome.stderr.is_empty());
}

#[test]
fn fail_closed_codex_gate_exits_two_with_the_diagnostic_as_the_reason() {
    for stdin in [
        &b"not json"[..],
        b"",
        b"{\"hook_event_name\":\"PreToolUse\"}",
    ] {
        let outcome = spawn("pre-tool-fail-closed-codex", stdin, &[]);
        assert_eq!(outcome.code, Some(2), "{:?}", outcome.stderr);
        assert_eq!(outcome.stdout, "");
        assert!(
            diagnostic_line(&outcome.stderr).contains("codex/PreToolUse failed"),
            "{:?}",
            outcome.stderr
        );
    }
}

#[test]
fn fail_closed_antigravity_gate_denies_on_stdout_at_exit_zero() {
    let outcome = spawn("pre-tool-fail-closed-antigravity", b"{}", &[]);
    assert_eq!(outcome.code, Some(0), "{:?}", outcome.stderr);
    let response: serde_json::Value = serde_json::from_str(&outcome.stdout).unwrap();
    assert_eq!(response["decision"], "deny");
    assert!(
        response["reason"]
            .as_str()
            .unwrap()
            .contains("antigravity/PreToolUse failed")
    );
}

#[test]
fn fail_closed_denies_when_the_handler_cannot_produce_a_decision() {
    // A blank deny reason is rejected at construction; failing closed turns
    // it into a block rather than letting the tool run.
    let blank = spawn(
        "pre-tool-fail-closed-blank-reason",
        &codex_pre_tool_use(),
        &[],
    );
    assert_eq!(blank.code, Some(2), "{:?}", blank.stderr);
    assert!(
        diagnostic_line(&blank.stderr).contains("deny reason must be non-empty"),
        "{:?}",
        blank.stderr
    );

    let panicked = spawn("pre-tool-fail-closed-panic", &codex_pre_tool_use(), &[]);
    assert_eq!(panicked.code, Some(2), "{:?}", panicked.stderr);
    assert!(
        diagnostic_line(&panicked.stderr).contains("panicked: policy engine exploded"),
        "{:?}",
        panicked.stderr
    );
}

#[test]
fn fail_closed_claude_prompt_gate_blocks_when_the_environment_is_invalid() {
    let payload = serde_json::json!({
        "session_id": "s",
        "transcript_path": "/tmp/t.jsonl",
        "cwd": "/repo",
        "hook_event_name": "UserPromptSubmit",
        "prompt": "ship it"
    });
    // No CLAUDE_* variables: the required baseline environment is missing.
    let outcome = spawn(
        "user-prompt-fail-closed-claude",
        &serde_json::to_vec(&payload).unwrap(),
        &[],
    );
    assert_eq!(outcome.code, Some(2), "{:?}", outcome.stderr);
    assert!(
        diagnostic_line(&outcome.stderr).contains("claude-code/UserPromptSubmit failed"),
        "{:?}",
        outcome.stderr
    );
}

#[test]
fn fail_closed_turn_completion_stays_non_blocking() {
    // Blocking a Stop would keep the agent working, so turn completion keeps
    // the non-blocking response under either policy.
    let outcome = spawn("turn-completion-fail-closed", b"not json", &[]);
    assert_eq!(outcome.code, Some(1));
    assert_eq!(outcome.stdout, "");
    assert!(diagnostic_line(&outcome.stderr).contains("codex/TurnCompletion failed"));
}

#[test]
fn the_sink_receives_handler_records_and_the_runner_failure() {
    let sink = std::env::temp_dir().join(format!(
        "hookkit-aligned-sink-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let sink_path = sink.to_str().unwrap();
    let outcome = spawn(
        "pre-tool-sink",
        &codex_pre_tool_use(),
        &[(SINK_FILE, sink_path)],
    );
    assert_eq!(outcome.code, Some(1), "{:?}", outcome.stderr);
    let recorded = std::fs::read_to_string(&sink).unwrap();
    let _ = std::fs::remove_file(&sink);
    let lines: Vec<_> = recorded.lines().collect();
    assert_eq!(lines.len(), 2, "{recorded:?}");
    assert_eq!(lines[0], "Info handler saw the call");
    assert!(
        lines[1].starts_with("Error hookkit: ") && lines[1].contains("codex/PreToolUse failed"),
        "{recorded:?}"
    );
}
