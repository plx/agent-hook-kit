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
//!   Claude Code and Codex treat exit 1 as a non-blocking hook error, so for
//!   most events the pending action proceeds: a policy hook fails open. Claude
//!   Code `WorktreeCreate` and `WorktreeRemove` are the exception: any
//!   non-zero exit fails worktree creation, or removal while the directory
//!   remains.
//! - [`FailurePolicy::FailClosed`] emits the harness's native blocking
//!   response for events that gate a pending action, so a policy hook that
//!   cannot decide denies instead. See [`failure_response`] for the exact
//!   per-harness lowering.
//!
//! The runners lower a failure for the event the harness actually sent: the
//! event the runner resolved, else the one the payload's discriminator
//! (`hook_event_name`) names, and only then the configured hint or the event
//! the runner was built for. A hook registered under the wrong event therefore
//! gets that event's semantics instead of blocking a `Stop` or letting a
//! `PreToolUse` through.
//!
//! [`RunOptions`] carries the policy and the diagnostics sink into
//! [`crate::typed::run_event_with_options`],
//! [`crate::selected::run_harness_with_options`],
//! [`crate::selected::dispatch_builtin_harness_with_options`], and
//! [`crate::aligned::run_aligned_event_with_options`]. Runners outside this
//! crate's typed, selected, and aligned families can reuse the same lowering
//! through [`report_run_failure`].
//!
//! # Panics
//!
//! The runners catch a panic in the handler, in parsing, or in emission and
//! report it under the failure policy instead of exiting 101. Such a panic
//! does not print the standard panic message or backtrace: the report is
//! still exactly one stderr line, which names the panic's message and
//! location, because Claude Code and Codex show that stderr to the user or
//! hand it to the model as a block reason. To do this, the first runner call
//! installs a process-wide panic hook that is silent only while a runner is
//! catching panics on the calling thread; every other panic, including one on
//! a thread the handler spawned, reaches the hook that was installed before
//! it.
//!
//! Catching a panic requires unwinding. A hook binary built with
//! `panic = "abort"` in its Cargo profile aborts on the first panic, which
//! Claude Code and Codex report as a non-blocking error, so a
//! [`FailurePolicy::FailClosed`] guard then fails open. Keep the default
//! `panic = "unwind"` for such binaries.
//!
//! A [`DiagnosticsSink`] that panics while the runner records a failure or an
//! environment warning is contained the same way: the record is dropped and
//! the failure policy still decides the response.

use hookkit_core::{
    BuiltinHarness, DISABLED_DIAGNOSTICS, Diagnostic, DiagnosticLevel, DiagnosticsSink, EventId,
    HarnessId,
};
use std::cell::{Cell, RefCell};
use std::fmt;
use std::io::Write;
use std::sync::Once;

/// How a stdin/stdout runner responds when it cannot produce the handler's
/// response.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FailurePolicy {
    /// Exit 1 with a one-line stderr diagnostic and empty stdout.
    ///
    /// Claude Code and Codex treat this as a non-blocking hook error: for
    /// most events the pending action proceeds and the user sees a hook-error
    /// notice. Use it for observers, telemetry, and feedback hooks, where
    /// blocking the agent because the hook itself broke would do more harm
    /// than good.
    ///
    /// Claude Code `WorktreeCreate` and `WorktreeRemove` have no non-blocking
    /// failure: any non-zero exit aborts worktree creation, and fails worktree
    /// removal if the directory still exists afterward. A failure there
    /// blocks under either policy, and [`FailureResponse::fails_closed`]
    /// reports it.
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

    /// Reports whether the harness will refuse the pending action.
    ///
    /// This is true for every [`FailurePolicy::FailClosed`] lowering that
    /// blocks, and also for the [`FailurePolicy::NonBlocking`] exit 1 on the
    /// Claude Code worktree events, which any non-zero exit fails.
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

/// Claude Code events that any non-zero exit blocks, so even the
/// [`FailurePolicy::NonBlocking`] exit 1 refuses the pending action.
///
/// Both are also in [`CLAUDE_EXIT_TWO_GATES`].
const CLAUDE_NONZERO_GATES: &[&str] = &["WorktreeCreate", "WorktreeRemove"];

/// Codex events whose exit 2 with non-empty stderr blocks the pending action.
///
/// Codex also honors exit 2 on `PostToolUse` (feedback) and `Stop`
/// (continuation), which are excluded for the same reasons as on Claude Code.
const CODEX_EXIT_TWO_GATES: &[&str] = &["PreToolUse", "PermissionRequest", "UserPromptSubmit"];

/// The built-in harness `harness` names, including the `claude` alias the
/// bundled command-line tools accept, so a runner configured with the alias
/// still lowers failures the way Claude Code reads them.
fn lowering_harness(harness: &HarnessId) -> Option<BuiltinHarness> {
    BuiltinHarness::from_id(harness).or_else(|| harness.as_str().parse().ok())
}

/// The name of `event` when it belongs to `harness`.
fn event_name(harness: BuiltinHarness, event: Option<&EventId>) -> Option<&str> {
    event
        .filter(|event| lowering_harness(event.harness()) == Some(harness))
        .map(EventId::name)
}

fn fail_closed_lowering(harness: &HarnessId, event: Option<&EventId>) -> FailClosedLowering {
    let Some(harness) = lowering_harness(harness) else {
        return FailClosedLowering::NotGating;
    };
    let event = event_name(harness, event);
    match harness {
        BuiltinHarness::ClaudeCode => match event {
            None => FailClosedLowering::ExitTwo,
            Some("PermissionRequest") => FailClosedLowering::ClaudePermissionDeny,
            Some(name) if CLAUDE_EXIT_TWO_GATES.contains(&name) => FailClosedLowering::ExitTwo,
            Some(_) => FailClosedLowering::NotGating,
        },
        BuiltinHarness::Codex => match event {
            None => FailClosedLowering::ExitTwo,
            Some(name) if CODEX_EXIT_TWO_GATES.contains(&name) => FailClosedLowering::ExitTwo,
            Some(_) => FailClosedLowering::NotGating,
        },
        BuiltinHarness::Antigravity => match event {
            None | Some("PreToolUse") => FailClosedLowering::AntigravityDeny,
            Some(_) => FailClosedLowering::NotGating,
        },
        _ => FailClosedLowering::NotGating,
    }
}

/// Whether a non-zero exit other than 2 refuses the pending action.
fn nonzero_exit_blocks(harness: &HarnessId, event: Option<&EventId>) -> bool {
    lowering_harness(harness) == Some(BuiltinHarness::ClaudeCode)
        && event_name(BuiltinHarness::ClaudeCode, event)
            .is_some_and(|name| CLAUDE_NONZERO_GATES.contains(&name))
}

/// Lowers a runner failure to the exact native process response.
///
/// `diagnostic` is the one-line failure description; it is always written to
/// stderr followed by a newline. `event` is the native event the invocation
/// was for, when it is known; pass `None` when the payload could not be
/// identified.
///
/// `harness` may be a canonical built-in identity or the `claude` alias.
///
/// Under [`FailurePolicy::NonBlocking`] the response is exit 1 with empty
/// stdout for every harness. Claude Code and Codex proceed with the pending
/// action, except that Claude Code fails `WorktreeCreate`, and
/// `WorktreeRemove` while the directory remains, on any non-zero exit
/// ([`FailureResponse::fails_closed`] is then true). Under
/// [`FailurePolicy::FailClosed`]:
///
/// | Harness | Event | Response |
/// | :- | :- | :- |
/// | Claude Code | `PreToolUse`, `UserPromptSubmit`, `UserPromptExpansion`, `PostToolBatch`, `PreCompact`, `PreModelSwitch`, `ConfigChange`, `Elicitation`, `ElicitationResult`, `TaskCreated`, `WorktreeCreate`, `WorktreeRemove` | exit 2, diagnostic on stderr |
/// | Claude Code | `PermissionRequest` (ignores exit 2) | exit 0, `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":…}}}` |
/// | Codex | `PreToolUse`, `PermissionRequest`, `UserPromptSubmit` | exit 2, diagnostic on stderr |
/// | Antigravity | `PreToolUse` | exit 0, `{"decision":"deny","reason":…}` |
/// | Claude Code, Codex | unknown (the payload could not be identified) | exit 2, diagnostic on stderr |
/// | Antigravity | unknown | exit 0, `{"decision":"deny","reason":…}` |
/// | any | every other event, or a harness that is not built in | exit 1, as for `NonBlocking` |
///
/// An event is unknown only when nothing names it: the payload is not JSON,
/// or it has no event discriminator and the runner had no hint. The stdin
/// runners identify every other event, including events an adapter does not
/// model yet, by the payload's discriminator (`hook_event_name`). An unknown
/// Claude Code or Codex event exits 2 because that blocks every exit-2 gate.
/// It cannot deny a Claude Code `PermissionRequest`, which ignores exit 2 and
/// falls back to the normal permission prompt; on a `Stop` event it asks the
/// agent to continue, which the next `Stop` reports through
/// `stop_hook_active` (Claude Code also caps consecutive continuations). An
/// unknown Antigravity event gets a `deny` decision: it denies a `PreToolUse`
/// and, because only `"continue"` prevents a stop, it lets a `Stop` end
/// normally.
///
/// Claude Code `PreCompact` is a gate: blocking an automatic compaction that
/// Claude Code triggered to recover from a context-limit error makes that
/// error surface and fails the current request, so fail closed on it only
/// when refusing to compact is worth that.
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
        fails_closed: nonzero_exit_blocks(harness, event),
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
///
/// A panic in the sink is contained: the record is dropped and the response
/// is still written.
pub fn report_run_failure(
    options: &RunOptions<'_>,
    harness: &HarnessId,
    event: Option<&EventId>,
    hook: &dyn fmt::Display,
    error: &dyn std::error::Error,
) -> std::process::ExitCode {
    let diagnostic = crate::report::failure_line(hook, error);
    let response = failure_response(options.failure_policy(), harness, event, &diagnostic);
    record_contained(
        options.diagnostics(),
        Diagnostic::new(DiagnosticLevel::Error, diagnostic),
    );
    response.emit()
}

/// Records `diagnostic` in a sink the runner calls itself, dropping it if the
/// sink panics so that a broken sink cannot change the hook's response.
pub(crate) fn record_contained(diagnostics: &dyn DiagnosticsSink, diagnostic: Diagnostic) {
    let _ = catch_panic(|| diagnostics.record(diagnostic));
}

/// A panic caught while a runner parsed, handled, or emitted an invocation.
#[derive(Debug)]
pub(crate) struct Panicked {
    message: String,
    location: Option<String>,
}

impl fmt::Display for Panicked {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "panicked: {}", self.message)?;
        match &self.location {
            Some(location) => write!(formatter, " (at {location})"),
            None => Ok(()),
        }
    }
}

impl std::error::Error for Panicked {}

/// Runs `operation`, converting a panic into [`Panicked`] so the runner can
/// report it under its failure policy instead of exiting 101.
///
/// A panic on the calling thread while `operation` runs skips the standard
/// panic message and backtrace; its location is kept for the report instead.
/// See [`install_quiet_panic_hook`].
pub(crate) fn catch_panic<T>(operation: impl FnOnce() -> T) -> Result<T, Panicked> {
    install_quiet_panic_hook();
    let outer = CATCHING_PANIC.with(|catching| catching.replace(true));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation));
    CATCHING_PANIC.with(|catching| catching.set(outer));
    let location = PANIC_LOCATION.with(|location| location.borrow_mut().take());
    result.map_err(|payload| {
        let message = payload
            .downcast_ref::<&str>()
            .map(|message| (*message).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic payload".to_owned());
        Panicked { message, location }
    })
}

thread_local! {
    /// Whether this thread is inside [`catch_panic`].
    static CATCHING_PANIC: Cell<bool> = const { Cell::new(false) };
    /// Location of the last panic [`catch_panic`] caught on this thread.
    static PANIC_LOCATION: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Installs, once per process, a panic hook that stays silent for panics
/// [`catch_panic`] is about to catch and hands every other panic to the hook
/// installed before it.
///
/// The default hook would print `thread '...' panicked at ...`, a backtrace
/// note, and with `RUST_BACKTRACE` a whole backtrace ahead of the runner's
/// one-line report, and Claude Code and Codex hand that stderr to the user or
/// the model. The hook is never swapped back, so concurrent runners cannot
/// restore each other's hooks out of order; outside [`catch_panic`] it
/// behaves exactly like the hook it wraps.
fn install_quiet_panic_hook() {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if CATCHING_PANIC.try_with(Cell::get).unwrap_or(false) {
                let _ = PANIC_LOCATION.try_with(|location| {
                    if let Ok(mut location) = location.try_borrow_mut() {
                        *location = info.location().map(ToString::to_string);
                    }
                });
            } else {
                previous(info);
            }
        }));
    });
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
    use std::collections::BTreeSet;
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
        let claude = CLAUDE_EXIT_TWO_GATES
            .iter()
            .map(|name| (HarnessId::CLAUDE_CODE, *name));
        let codex = CODEX_EXIT_TWO_GATES
            .iter()
            .map(|name| (HarnessId::CODEX, *name));
        for (harness, name) in claude.chain(codex) {
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
    fn claude_worktree_events_block_on_any_nonzero_exit() {
        for name in CLAUDE_NONZERO_GATES {
            let gate = event(HarnessId::CLAUDE_CODE, name);
            for policy in [FailurePolicy::NonBlocking, FailurePolicy::FailClosed] {
                let response =
                    failure_response(policy, &HarnessId::CLAUDE_CODE, Some(&gate), "boom");
                assert_ne!(response.exit_code(), 0, "{gate} {policy:?}");
                assert!(response.fails_closed(), "{gate} {policy:?}");
            }
        }
        // Codex has no worktree events; the names gate nothing there.
        let codex = event(HarnessId::CODEX, "WorktreeCreate");
        let response = failure_response(
            FailurePolicy::NonBlocking,
            &HarnessId::CODEX,
            Some(&codex),
            "boom",
        );
        assert!(!response.fails_closed());
    }

    #[test]
    fn the_claude_alias_lowers_like_claude_code() {
        let alias = HarnessId::new("claude").unwrap();
        let gate = event(alias.clone(), "PreToolUse");
        let response = failure_response(FailurePolicy::FailClosed, &alias, Some(&gate), "boom");
        assert_eq!(response.exit_code(), 2);
        assert!(response.fails_closed());
        // A canonical event under the alias, and the reverse, still match.
        let canonical = event(HarnessId::CLAUDE_CODE, "PermissionRequest");
        let response =
            failure_response(FailurePolicy::FailClosed, &alias, Some(&canonical), "boom");
        assert_eq!(response.exit_code(), 0);
        assert!(response.fails_closed());
        let stop = event(alias, "Stop");
        let response = failure_response(
            FailurePolicy::FailClosed,
            &HarnessId::CLAUDE_CODE,
            Some(&stop),
            "boom",
        );
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
    fn panics_are_caught_with_their_message_and_location() {
        let line = line!() + 1;
        let panicked = catch_panic(|| -> u8 { panic!("handler exploded") }).unwrap_err();
        assert_eq!(
            panicked.to_string(),
            format!("panicked: handler exploded (at {}:{line}:47)", file!())
        );
        let formatted = catch_panic(|| -> u8 { panic!("code {}", 7) }).unwrap_err();
        assert!(
            formatted.to_string().starts_with("panicked: code 7 (at "),
            "{formatted}"
        );
        assert_eq!(catch_panic(|| 3).unwrap(), 3);
    }

    struct Panicking;

    impl DiagnosticsSink for Panicking {
        fn record(&self, _diagnostic: Diagnostic) {
            panic!("audit log is read-only");
        }
    }

    #[test]
    fn a_panicking_sink_is_contained() {
        record_contained(&Panicking, Diagnostic::new(DiagnosticLevel::Error, "boom"));
        // The record's panic leaves nothing behind for the next report.
        assert_eq!(
            PANIC_LOCATION.with(|location| location.borrow().clone()),
            None
        );
        assert!(!CATCHING_PANIC.with(Cell::get));
        let caught = catch_panic(|| -> u8 { panic!("later") }).unwrap_err();
        assert!(caught.to_string().starts_with("panicked: later (at "));
    }

    /// Exit-2 effects in the contract ledger that refuse the pending action.
    const BLOCKING_EFFECTS: &[&str] = &[
        "deny",
        "decline",
        "rollback",
        "stop",
        "fail",
        "fail-if-directory-remains",
    ];

    /// Exit-2 effects that refuse nothing: they keep the agent working
    /// (`continue`), feed it context, or are ignored or non-blocking.
    const NON_BLOCKING_EFFECTS: &[&str] = &[
        "continue",
        "provide-context",
        "replace-result",
        "ignored",
        "nonblocking-error",
        "terminal-sequence-only",
        "display-original",
    ];

    /// Events whose exit 2 blocks but that fail-closed lowering deliberately
    /// leaves non-blocking, because blocking them keeps the agent working.
    const DELIBERATELY_NON_GATING: &[(&str, &str)] = &[("claude-code", "TaskCompleted")];

    fn contracts_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts")
    }

    fn read_yaml(path: &std::path::Path) -> serde_yaml_ng::Value {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        serde_yaml_ng::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
    }

    fn exit_range(outcome: &serde_yaml_ng::Value) -> (u64, u64) {
        let exit = &outcome["exit"];
        match exit["exact"].as_u64() {
            Some(code) => (code, code),
            None => (
                exit["range"]["min"].as_u64().expect("exit range min"),
                exit["range"]["max"].as_u64().expect("exit range max"),
            ),
        }
    }

    /// Derives, from `harness`'s registry-selected snapshot, the events whose
    /// unstructured response at `code` (empty stdout, the diagnostic on
    /// stderr) refuses the pending action.
    fn ledger_gates(harness: &str, code: u64) -> BTreeSet<String> {
        let contracts = contracts_dir();
        let registry = read_yaml(&contracts.join("registry.yaml"));
        let snapshot = registry["harnesses"][harness]["current"]
            .as_str()
            .unwrap_or_else(|| panic!("no selected {harness} snapshot"));
        let root = contracts
            .join("harnesses")
            .join(harness)
            .join("snapshots")
            .join(snapshot);
        let index = read_yaml(&root.join("snapshot.yaml"));
        let events = index["events"].as_sequence().expect("snapshot events");
        assert!(!events.is_empty(), "{harness} {snapshot} lists no events");
        let mut gates = BTreeSet::new();
        for entry in events {
            let name = entry["wire_name"].as_str().expect("wire_name");
            let path = entry["path"].as_str().expect("event path");
            let contract = read_yaml(&root.join(path).join("contract.yaml"));
            let outcomes = contract["bindings"]["command"]["outcomes"]
                .as_sequence()
                .unwrap_or_else(|| panic!("{harness}/{name} has no command outcomes"));
            let mut blocks = false;
            for outcome in outcomes {
                let (min, max) = exit_range(outcome);
                if !(min..=max).contains(&code) || outcome["stdout"]["content_kind"] == "json" {
                    continue;
                }
                let effect = outcome["effect"].as_str().expect("outcome effect");
                if BLOCKING_EFFECTS.contains(&effect) {
                    blocks = true;
                } else {
                    assert!(
                        NON_BLOCKING_EFFECTS.contains(&effect),
                        "{harness}/{name}: classify the new exit-{code} effect `{effect}`"
                    );
                }
            }
            if blocks {
                gates.insert(name.to_owned());
            }
        }
        gates
    }

    fn table(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn exit_two_gate_tables_match_the_selected_contract_ledger() {
        for (harness, gates) in [
            ("claude-code", CLAUDE_EXIT_TWO_GATES),
            ("codex", CODEX_EXIT_TWO_GATES),
            ("antigravity", &[][..]),
        ] {
            let mut expected = ledger_gates(harness, 2);
            for (excluded_harness, excluded) in DELIBERATELY_NON_GATING {
                if *excluded_harness == harness {
                    assert!(
                        expected.remove(*excluded),
                        "{harness}/{excluded} no longer blocks on exit 2; drop the exclusion"
                    );
                }
            }
            assert_eq!(table(gates), expected, "{harness} exit-2 gates");
        }
    }

    #[test]
    fn nonzero_gate_table_matches_the_selected_contract_ledger() {
        assert_eq!(
            table(CLAUDE_NONZERO_GATES),
            ledger_gates("claude-code", 1),
            "Claude Code events that exit 1 refuses"
        );
        assert!(ledger_gates("codex", 1).is_empty());
        assert!(ledger_gates("antigravity", 1).is_empty());
    }
}
