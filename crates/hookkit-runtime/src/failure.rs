//! What the stdin/stdout runners do when they cannot produce a handler's
//! response.
//!
//! A runner fails when stdin cannot be read, the declared environment cannot
//! be captured, the payload does not parse or resolve, the environment
//! contradicts the payload, the handler returns an error or panics, or the
//! handler's output cannot be emitted. Every failure is reported as one
//! stderr line of the form `hookkit: <program> <hook> failed: <cause chain>`
//! and recorded as an [`DiagnosticLevel::Error`] diagnostic in the configured
//! [`DiagnosticsSink`]. What else happens depends on the [`FailurePolicy`]:
//!
//! - [`FailurePolicy::NonBlocking`], the default, exits 1 with empty stdout.
//!   Claude Code and Codex treat exit 1 as a non-blocking hook error, so the
//!   pending action proceeds: a policy hook fails open.
//! - [`FailurePolicy::FailClosed`] emits the harness's native blocking
//!   response for events that gate a pending action, so a policy hook that
//!   cannot decide denies instead. See [`failure_response`] for the exact
//!   per-harness lowering.
//!
//! [`RunOptions`] carries the policy and the diagnostics sink into
//! [`crate::typed::run_event_with_options`],
//! [`crate::selected::run_harness_with_options`], and
//! [`crate::selected::dispatch_builtin_harness_with_options`]. Runners outside
//! this crate's typed and selected families can reuse the same lowering
//! through [`report_run_failure`].

use hookkit_core::{
    DISABLED_DIAGNOSTICS, Diagnostic, DiagnosticLevel, DiagnosticsSink, EventId, HarnessId,
};
use std::fmt;
use std::io::Write;

/// How a stdin/stdout runner responds when it cannot produce the handler's
/// response.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FailurePolicy {
    /// Exit 1 with a one-line stderr diagnostic and empty stdout.
    ///
    /// Claude Code and Codex treat this as a non-blocking hook error: the
    /// pending action proceeds and the user sees a hook-error notice. Use it
    /// for observers, telemetry, and feedback hooks, where blocking the agent
    /// because the hook itself broke would do more harm than good.
    #[default]
    NonBlocking,
    /// Deny the pending action when the event can gate one.
    ///
    /// Claude Code and Codex exit 2 with the diagnostic on stderr, except
    /// that Claude Code `PermissionRequest`, which ignores exit 2, receives a
    /// JSON `deny` decision. Antigravity documents no exit-code semantics, so
    /// it receives a JSON `deny` decision on stdout at exit 0. Events that
    /// gate nothing (observers, and turn-completion events where blocking
    /// would force the agent to keep working) keep the
    /// [`FailurePolicy::NonBlocking`] response. See [`failure_response`].
    FailClosed,
}

/// Options shared by the stdin/stdout runners.
///
/// ```
/// use hookkit_runtime::failure::{FailurePolicy, RunOptions};
///
/// let options = RunOptions::new().fail_closed();
/// assert_eq!(options.failure_policy(), FailurePolicy::FailClosed);
/// ```
#[derive(Clone, Copy)]
pub struct RunOptions<'a> {
    diagnostics: &'a dyn DiagnosticsSink,
    failure_policy: FailurePolicy,
}

impl RunOptions<'static> {
    /// Options with diagnostics disabled and the
    /// [`FailurePolicy::NonBlocking`] policy, the convenience runners' defaults.
    pub fn new() -> Self {
        Self {
            diagnostics: &DISABLED_DIAGNOSTICS,
            failure_policy: FailurePolicy::NonBlocking,
        }
    }
}

impl Default for RunOptions<'static> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> RunOptions<'a> {
    /// Sends handler and runtime diagnostics, including every runner failure,
    /// to `diagnostics`.
    pub fn with_diagnostics<'b>(self, diagnostics: &'b dyn DiagnosticsSink) -> RunOptions<'b> {
        RunOptions {
            diagnostics,
            failure_policy: self.failure_policy,
        }
    }

    /// Selects how runner failures are reported to the harness.
    pub fn with_failure_policy(mut self, policy: FailurePolicy) -> Self {
        self.failure_policy = policy;
        self
    }

    /// Shorthand for [`FailurePolicy::FailClosed`].
    pub fn fail_closed(self) -> Self {
        self.with_failure_policy(FailurePolicy::FailClosed)
    }

    /// Returns the configured diagnostics sink.
    pub fn diagnostics(&self) -> &'a dyn DiagnosticsSink {
        self.diagnostics
    }

    /// Returns the configured failure policy.
    pub fn failure_policy(&self) -> FailurePolicy {
        self.failure_policy
    }
}

impl fmt::Debug for RunOptions<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RunOptions")
            .field("failure_policy", &self.failure_policy)
            .finish_non_exhaustive()
    }
}

/// The exact process response for a runner failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureResponse {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_code: u8,
    fails_closed: bool,
}

impl FailureResponse {
    /// Returns the bytes to write to stdout.
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    /// Returns the bytes to write to stderr.
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }

    /// Returns the process exit code.
    pub fn exit_code(&self) -> u8 {
        self.exit_code
    }

    /// Reports whether the response denies the pending action.
    pub fn fails_closed(&self) -> bool {
        self.fails_closed
    }

    /// Writes the response to the process's stdout and stderr and returns
    /// its exit code.
    ///
    /// Writing is best effort: if a stream is broken there is nowhere left to
    /// report to, and the exit code is still returned.
    pub fn emit(&self) -> std::process::ExitCode {
        {
            let mut stdout = std::io::stdout().lock();
            let _ = stdout.write_all(&self.stdout);
            let _ = stdout.flush();
        }
        {
            let mut stderr = std::io::stderr().lock();
            let _ = stderr.write_all(&self.stderr);
            let _ = stderr.flush();
        }
        std::process::ExitCode::from(self.exit_code)
    }
}

/// Native fail-closed lowering for one harness event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailClosedLowering {
    /// Exit 2 with the diagnostic as the (non-empty) blocking reason.
    ExitTwo,
    /// Claude Code `PermissionRequest` JSON `deny` decision at exit 0.
    ClaudePermissionDeny,
    /// Antigravity JSON `{"decision":"deny"}` at exit 0.
    AntigravityDeny,
    /// The event gates nothing; report without blocking.
    NotGating,
}

/// Claude Code events whose exit 2 blocks the pending action without forcing
/// the agent to keep working.
///
/// `PermissionRequest` ignores exit 2 and is lowered to JSON instead.
/// `Stop`, `SubagentStop`, `TeammateIdle`, and `TaskCompleted` are excluded
/// because blocking them keeps the agent running. Observer and feedback
/// events (`PostToolUse`, `SessionStart`, and so on) are excluded because
/// they gate nothing.
const CLAUDE_EXIT_TWO_GATES: &[&str] = &[
    "PreToolUse",
    "UserPromptSubmit",
    "UserPromptExpansion",
    "PostToolBatch",
    "PreCompact",
    "PreModelSwitch",
    "ConfigChange",
    "Elicitation",
    "ElicitationResult",
    "TaskCreated",
    "WorktreeCreate",
    "WorktreeRemove",
];

/// Codex events whose exit 2 with non-empty stderr blocks the pending action.
///
/// Codex also honors exit 2 on `PostToolUse` (feedback) and `Stop`
/// (continuation), which are excluded for the same reasons as on Claude Code.
const CODEX_EXIT_TWO_GATES: &[&str] = &["PreToolUse", "PermissionRequest", "UserPromptSubmit"];

fn fail_closed_lowering(harness: &HarnessId, event: Option<&EventId>) -> FailClosedLowering {
    let event = event
        .filter(|event| event.harness() == harness)
        .map(EventId::name);
    if harness == &HarnessId::CLAUDE_CODE {
        match event {
            None => FailClosedLowering::ExitTwo,
            Some("PermissionRequest") => FailClosedLowering::ClaudePermissionDeny,
            Some(name) if CLAUDE_EXIT_TWO_GATES.contains(&name) => FailClosedLowering::ExitTwo,
            Some(_) => FailClosedLowering::NotGating,
        }
    } else if harness == &HarnessId::CODEX {
        match event {
            None => FailClosedLowering::ExitTwo,
            Some(name) if CODEX_EXIT_TWO_GATES.contains(&name) => FailClosedLowering::ExitTwo,
            Some(_) => FailClosedLowering::NotGating,
        }
    } else if harness == &HarnessId::ANTIGRAVITY {
        match event {
            None | Some("PreToolUse") => FailClosedLowering::AntigravityDeny,
            Some(_) => FailClosedLowering::NotGating,
        }
    } else {
        FailClosedLowering::NotGating
    }
}

/// Lowers a runner failure to the exact native process response.
///
/// `diagnostic` is the one-line failure description; it is always written to
/// stderr followed by a newline. `event` is the native event the invocation
/// was for, when it is known; pass `None` when the payload could not be
/// identified.
///
/// Under [`FailurePolicy::NonBlocking`] the response is exit 1 with empty
/// stdout for every harness. Under [`FailurePolicy::FailClosed`]:
///
/// | Harness | Event | Response |
/// | :- | :- | :- |
/// | Claude Code | `PreToolUse`, `UserPromptSubmit`, `UserPromptExpansion`, `PostToolBatch`, `PreCompact`, `PreModelSwitch`, `ConfigChange`, `Elicitation`, `ElicitationResult`, `TaskCreated`, `WorktreeCreate`, `WorktreeRemove` | exit 2, diagnostic on stderr |
/// | Claude Code | `PermissionRequest` (ignores exit 2) | exit 0, `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":…}}}` |
/// | Codex | `PreToolUse`, `PermissionRequest`, `UserPromptSubmit` | exit 2, diagnostic on stderr |
/// | Antigravity | `PreToolUse` | exit 0, `{"decision":"deny","reason":…}` |
/// | Claude Code, Codex | unknown (the payload could not be identified) | exit 2, diagnostic on stderr |
/// | Antigravity | unknown | exit 0, `{"decision":"deny","reason":…}` |
/// | any | every other event, or an unknown harness | exit 1, as for `NonBlocking` |
///
/// An event is unknown only when nothing names it: the payload is not JSON,
/// or it has no event discriminator and the runner had no hint. The stdin
/// runners identify every other event, including events an adapter does not
/// model yet, by the payload's discriminator (`hook_event_name`). An unknown
/// Claude Code or Codex event exits 2 because that blocks every gating event;
/// on a `Stop` event it also asks the agent to continue, which the next
/// `Stop` reports through `stop_hook_active` (Claude Code also caps
/// consecutive continuations). An unknown Antigravity event gets a `deny`
/// decision: it denies a `PreToolUse` and, because only `"continue"`
/// prevents a stop, it lets a `Stop` end normally.
pub fn failure_response(
    policy: FailurePolicy,
    harness: &HarnessId,
    event: Option<&EventId>,
    diagnostic: &str,
) -> FailureResponse {
    let mut stderr = diagnostic.as_bytes().to_vec();
    stderr.push(b'\n');
    let non_blocking = |stderr| FailureResponse {
        stdout: Vec::new(),
        stderr,
        exit_code: 1,
        fails_closed: false,
    };
    if policy == FailurePolicy::NonBlocking {
        return non_blocking(stderr);
    }
    // A blocking reason must be non-empty after trimming for Codex.
    let reason = if diagnostic.trim().is_empty() {
        "hookkit: hook failed"
    } else {
        diagnostic
    };
    match fail_closed_lowering(harness, event) {
        FailClosedLowering::NotGating => non_blocking(stderr),
        FailClosedLowering::ExitTwo => FailureResponse {
            stdout: Vec::new(),
            stderr: format!("{reason}\n").into_bytes(),
            exit_code: 2,
            fails_closed: true,
        },
        FailClosedLowering::ClaudePermissionDeny => json_response(
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": {"behavior": "deny", "message": reason},
                }
            }),
            stderr,
        ),
        FailClosedLowering::AntigravityDeny => json_response(
            serde_json::json!({"decision": "deny", "reason": reason}),
            stderr,
        ),
    }
}

fn json_response(value: serde_json::Value, stderr: Vec<u8>) -> FailureResponse {
    FailureResponse {
        stdout: value.to_string().into_bytes(),
        stderr,
        exit_code: 0,
        fails_closed: true,
    }
}

/// Reports a runner failure and returns the exit code the runner must return.
///
/// The one-line diagnostic `hookkit: <program> <hook> failed: <cause chain>`
/// is recorded in `options`' diagnostics sink as an error, then the
/// [`failure_response`] for `options`' policy is written to the process's
/// stdout and stderr. `hook` labels the invocation in the diagnostic (for
/// example the event, or `harness/family` for an aligned runner); `event` is
/// the exact native event when it is known.
pub fn report_run_failure(
    options: &RunOptions<'_>,
    harness: &HarnessId,
    event: Option<&EventId>,
    hook: &dyn fmt::Display,
    error: &dyn std::error::Error,
) -> std::process::ExitCode {
    let diagnostic = crate::report::failure_line(hook, error);
    options
        .diagnostics()
        .record(Diagnostic::new(DiagnosticLevel::Error, diagnostic.clone()));
    failure_response(options.failure_policy(), harness, event, &diagnostic).emit()
}

/// A panic caught while a runner parsed, handled, or emitted an invocation.
#[derive(Debug)]
pub(crate) struct Panicked(String);

impl fmt::Display for Panicked {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "panicked: {}", self.0)
    }
}

impl std::error::Error for Panicked {}

/// Runs `operation`, converting a panic into [`Panicked`] so the runner can
/// report it under its failure policy instead of exiting 101.
pub(crate) fn catch_panic<T>(operation: impl FnOnce() -> T) -> Result<T, Panicked> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)).map_err(|payload| {
        let message = payload
            .downcast_ref::<&str>()
            .map(|message| (*message).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic payload".to_owned());
        Panicked(message)
    })
}

/// Reads all of stdin.
pub(crate) fn read_stdin() -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::io::stdin().read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn event(harness: HarnessId, name: &'static str) -> EventId {
        EventId::builtin(harness, name)
    }

    #[test]
    fn non_blocking_is_exit_one_for_every_harness() {
        for harness in [
            HarnessId::CLAUDE_CODE,
            HarnessId::CODEX,
            HarnessId::ANTIGRAVITY,
        ] {
            let pre_tool = event(harness.clone(), "PreToolUse");
            let response = failure_response(
                FailurePolicy::NonBlocking,
                &harness,
                Some(&pre_tool),
                "boom",
            );
            assert_eq!(response.exit_code(), 1);
            assert!(response.stdout().is_empty());
            assert_eq!(response.stderr(), b"boom\n");
            assert!(!response.fails_closed());
        }
    }

    #[test]
    fn claude_and_codex_gates_fail_closed_with_exit_two_and_a_reason() {
        for (harness, name) in [
            (HarnessId::CLAUDE_CODE, "PreToolUse"),
            (HarnessId::CLAUDE_CODE, "UserPromptSubmit"),
            (HarnessId::CLAUDE_CODE, "PreModelSwitch"),
            (HarnessId::CODEX, "PreToolUse"),
            (HarnessId::CODEX, "PermissionRequest"),
            (HarnessId::CODEX, "UserPromptSubmit"),
        ] {
            let gate = event(harness.clone(), name);
            let response =
                failure_response(FailurePolicy::FailClosed, &harness, Some(&gate), "boom");
            assert_eq!(response.exit_code(), 2, "{gate}");
            assert!(response.stdout().is_empty(), "{gate}");
            assert_eq!(response.stderr(), b"boom\n", "{gate}");
            assert!(response.fails_closed(), "{gate}");
        }
    }

    #[test]
    fn claude_permission_request_fails_closed_with_a_json_deny() {
        let gate = event(HarnessId::CLAUDE_CODE, "PermissionRequest");
        let response = failure_response(
            FailurePolicy::FailClosed,
            &HarnessId::CLAUDE_CODE,
            Some(&gate),
            "boom",
        );
        assert_eq!(response.exit_code(), 0);
        assert!(response.fails_closed());
        let json: serde_json::Value = serde_json::from_slice(response.stdout()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PermissionRequest",
                    "decision": {"behavior": "deny", "message": "boom"}
                }
            })
        );
    }

    #[test]
    fn antigravity_fails_closed_with_a_json_deny_at_exit_zero() {
        let gate = event(HarnessId::ANTIGRAVITY, "PreToolUse");
        for event in [Some(&gate), None] {
            let response = failure_response(
                FailurePolicy::FailClosed,
                &HarnessId::ANTIGRAVITY,
                event,
                "boom",
            );
            assert_eq!(response.exit_code(), 0);
            assert!(response.fails_closed());
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(response.stdout()).unwrap(),
                serde_json::json!({"decision": "deny", "reason": "boom"})
            );
        }
    }

    #[test]
    fn non_gating_events_keep_the_non_blocking_response_when_failing_closed() {
        for (harness, name) in [
            (HarnessId::CLAUDE_CODE, "PostToolUse"),
            (HarnessId::CLAUDE_CODE, "Stop"),
            (HarnessId::CLAUDE_CODE, "SessionStart"),
            (HarnessId::CODEX, "Stop"),
            (HarnessId::CODEX, "PostToolUse"),
            (HarnessId::ANTIGRAVITY, "Stop"),
            (HarnessId::ANTIGRAVITY, "PostToolUse"),
        ] {
            let observer = event(harness.clone(), name);
            let response =
                failure_response(FailurePolicy::FailClosed, &harness, Some(&observer), "boom");
            assert_eq!(response.exit_code(), 1, "{observer}");
            assert!(response.stdout().is_empty(), "{observer}");
            assert!(!response.fails_closed(), "{observer}");
        }
        let custom = HarnessId::builtin("custom");
        let response = failure_response(FailurePolicy::FailClosed, &custom, None, "boom");
        assert_eq!(response.exit_code(), 1);
    }

    #[test]
    fn unknown_claude_and_codex_events_fail_closed_with_exit_two() {
        for harness in [HarnessId::CLAUDE_CODE, HarnessId::CODEX] {
            let response = failure_response(FailurePolicy::FailClosed, &harness, None, "boom");
            assert_eq!(response.exit_code(), 2);
        }
        // An event from another harness is treated as unknown.
        let foreign = event(HarnessId::ANTIGRAVITY, "Stop");
        let response = failure_response(
            FailurePolicy::FailClosed,
            &HarnessId::CODEX,
            Some(&foreign),
            "boom",
        );
        assert_eq!(response.exit_code(), 2);
    }

    #[test]
    fn blank_diagnostics_still_produce_a_non_empty_blocking_reason() {
        let gate = event(HarnessId::CODEX, "PreToolUse");
        let response = failure_response(
            FailurePolicy::FailClosed,
            &HarnessId::CODEX,
            Some(&gate),
            " ",
        );
        assert!(!String::from_utf8_lossy(response.stderr()).trim().is_empty());
    }

    #[derive(Default)]
    struct Recording(Mutex<Vec<Diagnostic>>);

    impl DiagnosticsSink for Recording {
        fn record(&self, diagnostic: Diagnostic) {
            self.0.lock().unwrap().push(diagnostic);
        }
    }

    #[test]
    fn options_carry_the_sink_and_policy() {
        let sink = Recording::default();
        let options = RunOptions::new().with_diagnostics(&sink).fail_closed();
        assert_eq!(options.failure_policy(), FailurePolicy::FailClosed);
        options
            .diagnostics()
            .record(Diagnostic::new(DiagnosticLevel::Info, "hello"));
        assert_eq!(sink.0.lock().unwrap().len(), 1);
        assert_eq!(
            RunOptions::default().failure_policy(),
            FailurePolicy::NonBlocking
        );
        assert!(format!("{options:?}").contains("FailClosed"));
    }

    #[test]
    fn panics_are_caught_with_their_message() {
        let panicked = catch_panic(|| -> u8 { panic!("handler exploded") }).unwrap_err();
        assert_eq!(panicked.to_string(), "panicked: handler exploded");
        let formatted = catch_panic(|| -> u8 { panic!("code {}", 7) }).unwrap_err();
        assert_eq!(formatted.to_string(), "panicked: code 7");
        assert_eq!(catch_panic(|| 3).unwrap(), 3);
    }
}
