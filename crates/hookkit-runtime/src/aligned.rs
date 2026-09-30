//! Explicit-harness execution for lossless aligned lifecycle-event families.
//!
//! The sealed marker types select only library-defined alignments. Each path
//! parses, validates, and emits the exact native contract for the explicitly
//! selected harness, and rejects a handler output arm for another harness.
//! See [`hookkit_common::aligned`] for what each family's portable helpers
//! mean on each harness.
//!
//! # Failure policy
//!
//! [`run_aligned_event`] reports every failure (unreadable stdin, an invalid
//! environment or payload, a handler error or panic, or an output that cannot
//! be emitted) as one stderr line and exits 1. Claude Code and Codex treat
//! exit 1 as a non-blocking hook error, so the pending action proceeds: a
//! policy hook built on a gating family (`PreToolUse`, `PermissionRequest`,
//! `UserPromptSubmit`, `PreCompact`) *fails open* by default.
//!
//! A guard that must deny when it cannot decide should run through
//! [`run_aligned_event_with_options`] with
//! [`RunOptions::fail_closed`](crate::failure::RunOptions::fail_closed). Every
//! failure is then lowered to the harness's native blocking response for the
//! family's native event, as [`crate::failure::failure_response`] describes:
//!
//! | Family | Claude Code | Codex | Antigravity |
//! | :- | :- | :- | :- |
//! | [`PreToolUse`] | exit 2, reason on stderr | exit 2, reason on stderr | `{"decision":"deny"}` at exit 0 |
//! | [`PermissionRequest`] | JSON `behavior: "deny"` at exit 0 | exit 2 | not aligned |
//! | [`UserPromptSubmit`] | exit 2 | exit 2 | not aligned |
//! | [`PreCompact`] | exit 2 | exit 1 (Codex's stop is broader than a compaction block) | not aligned |
//! | every other family | exit 1 | exit 1 | exit 1 |
//!
//! Observer and turn-completion families keep the non-blocking response under
//! either policy, because blocking them would keep the agent working rather
//! than stop an action. A [`crate::failure::RunOptions`] diagnostics sink also
//! reaches the handler through [`RuntimeContext::diagnostics`] and records
//! every runner failure.

use crate::failure::{RunOptions, catch_panic, read_stdin, report_run_failure};
use hookkit_common::{
    PermissionRequestCommandEnvironment, PermissionRequestInput, PermissionRequestOutput,
    PostCompactCommandEnvironment, PostCompactInput, PostCompactOutput,
    PostToolUseCommandEnvironment, PostToolUseInput, PostToolUseOutput,
    PreCompactCommandEnvironment, PreCompactInput, PreCompactOutput, PreToolUseCommandEnvironment,
    PreToolUseInput, PreToolUseOutput, SessionEndCommandEnvironment, SessionEndInput,
    SessionEndOutput, SessionStartCommandEnvironment, SessionStartInput, SessionStartOutput,
    SubagentStartCommandEnvironment, SubagentStartInput, SubagentStartOutput,
    SubagentStopCommandEnvironment, SubagentStopInput, SubagentStopOutput,
    TurnCompletionCommandEnvironment, TurnCompletionInput, TurnCompletionOutput,
    UserPromptSubmitCommandEnvironment, UserPromptSubmitInput, UserPromptSubmitOutput,
};
use hookkit_core::{
    BuiltinHarness, CommandEnvironmentSpec, ContractId, DISABLED_DIAGNOSTICS, DiagnosticsSink,
    EnvironmentVariables, EventId, EventSpec, HarnessId, HookkitError, NativeContext,
    ProcessEmission, RawInvocation, ResolutionProvenance, RuntimeContext, SnapshotId,
};

mod sealed {
    pub trait Sealed {
        /// Native event name every harness in the family uses.
        const NATIVE_EVENT: &'static str;
    }
}

/// Event-specific aligned execution contract.
pub trait AlignedEventSpec: sealed::Sealed {
    /// Lossless cross-harness input wrapper for the event family.
    type Input;
    /// Lossless cross-harness command-environment wrapper.
    type CommandEnvironment;
    /// Lossless cross-harness output wrapper for the event family.
    type Output;

    /// Human-readable event family name, used only for diagnostics (for example,
    /// naming the hook a stderr failure report was for before the payload has
    /// been parsed enough to know its exact native event).
    const FAMILY: &'static str;

    /// Parses, validates, handles, and emits one aligned event for `harness`
    /// with diagnostics disabled.
    ///
    /// Equivalent to [`Self::execute_with_diagnostics`] with
    /// [`DISABLED_DIAGNOSTICS`].
    fn execute<F>(
        harness: HarnessId,
        bytes: Vec<u8>,
        variables: &EnvironmentVariables,
        handler: F,
    ) -> hookkit_core::Result<ProcessEmission>
    where
        F: FnOnce(
            Self::Input,
            &Self::CommandEnvironment,
            &RuntimeContext<'_>,
        ) -> hookkit_core::Result<Self::Output>,
    {
        Self::execute_with_diagnostics(harness, bytes, variables, &DISABLED_DIAGNOSTICS, handler)
    }

    /// Parses, validates, handles, and emits one aligned event for `harness`.
    ///
    /// The implementation selects an exact native contract from the explicit
    /// harness identity. The handler must return the output arm for the same
    /// harness or execution fails before emission. `diagnostics` is the sink
    /// the handler reaches through [`RuntimeContext::diagnostics`]. A harness
    /// with no adapter for the family fails with
    /// [`HookkitError::UnsupportedHarness`] before the payload is parsed.
    fn execute_with_diagnostics<F>(
        harness: HarnessId,
        bytes: Vec<u8>,
        variables: &EnvironmentVariables,
        diagnostics: &dyn DiagnosticsSink,
        handler: F,
    ) -> hookkit_core::Result<ProcessEmission>
    where
        F: FnOnce(
            Self::Input,
            &Self::CommandEnvironment,
            &RuntimeContext<'_>,
        ) -> hookkit_core::Result<Self::Output>;
}

/// Harness and event identity shared by every aligned input and output arm.
trait ArmIdentity {
    fn arm_harness(&self) -> HarnessId;
    fn arm_event(&self) -> EventId;
}

/// One native invocation parsed and cross-checked against its environment.
struct Parsed<I, E> {
    input: I,
    event: EventId,
    snapshot: SnapshotId,
    contract: ContractId,
    native_context: NativeContext,
    command_environment: E,
}

/// Parses `invocation` as native event `N`, captures and cross-checks its
/// command environment, and wraps both in the family's aligned arms.
fn parse_arm<N, I, E>(
    invocation: &RawInvocation,
    variables: &EnvironmentVariables,
    wrap_input: fn(N::Input) -> I,
    wrap_environment: fn(N::CommandEnvironment) -> E,
) -> hookkit_core::Result<Parsed<I, E>>
where
    N: EventSpec,
{
    let input = N::parse(invocation)?;
    let environment =
        <N::CommandEnvironment as CommandEnvironmentSpec>::from_variables(&N::EVENT, variables)?;
    N::validate_command_environment(&input, &environment)?;
    let native_context = N::context(&input);
    Ok(Parsed {
        input: wrap_input(input),
        event: N::EVENT,
        snapshot: N::SNAPSHOT,
        contract: N::CONTRACT,
        native_context,
        command_environment: wrap_environment(environment),
    })
}

/// Runs the handler on a parsed invocation and emits its output, rejecting an
/// output arm for another harness before emission.
fn execute_parsed<I, E, O, F>(
    harness: HarnessId,
    invocation: &RawInvocation,
    parsed: Parsed<I, E>,
    diagnostics: &dyn DiagnosticsSink,
    handler: F,
    emit: impl FnOnce(O) -> hookkit_core::Result<ProcessEmission>,
) -> hookkit_core::Result<ProcessEmission>
where
    I: ArmIdentity,
    O: ArmIdentity,
    F: FnOnce(I, &E, &RuntimeContext<'_>) -> hookkit_core::Result<O>,
{
    let expected_harness = parsed.input.arm_harness();
    let context = RuntimeContext::new(
        harness,
        parsed.snapshot,
        parsed.event,
        parsed.contract,
        ResolutionProvenance::TypedStatic,
        invocation,
        parsed.native_context,
        diagnostics,
    )?;
    let output = handler(parsed.input, &parsed.command_environment, &context)?;
    if output.arm_harness() != expected_harness {
        return Err(HookkitError::EventHarnessMismatch {
            harness: expected_harness,
            event: output.arm_event(),
        });
    }
    crate::typed::validate_command_emission(emit(output)?, context.contract())
}

fn unsupported_harness(harness: &HarnessId, family: &'static str) -> HookkitError {
    HookkitError::UnsupportedHarness {
        harness: harness.clone(),
        message: format!("no aligned {family} adapter is registered for this harness"),
    }
}

macro_rules! aligned_family {
    (
        $(#[$marker_meta:meta])*
        marker: $marker:ident,
        input: $input:ident,
        environment: $environment:ident,
        output: $output:ident,
        execute: $execute:ident,
        family: $family:literal,
        native_event: $native_event:literal,
        arms: [$($arm:ident($builtin:path, $native:ty)),+ $(,)?]
    ) => {
        $(#[$marker_meta])*
        #[derive(Debug)]
        pub enum $marker {}

        impl sealed::Sealed for $marker {
            const NATIVE_EVENT: &'static str = $native_event;
        }

        impl AlignedEventSpec for $marker {
            type Input = $input;
            type CommandEnvironment = $environment;
            type Output = $output;
            const FAMILY: &'static str = $family;

            fn execute_with_diagnostics<F>(
                harness: HarnessId,
                bytes: Vec<u8>,
                variables: &EnvironmentVariables,
                diagnostics: &dyn DiagnosticsSink,
                handler: F,
            ) -> hookkit_core::Result<ProcessEmission>
            where
                F: FnOnce(
                    Self::Input,
                    &Self::CommandEnvironment,
                    &RuntimeContext<'_>,
                ) -> hookkit_core::Result<Self::Output>,
            {
                let selected = BuiltinHarness::from_id(&harness);
                if !matches!(selected, $(Some($builtin))|+) {
                    return Err(unsupported_harness(&harness, $family));
                }
                let invocation = RawInvocation::parse(bytes)?;
                let parsed = match selected {
                    $(
                        Some($builtin) => parse_arm::<$native, _, _>(
                            &invocation,
                            variables,
                            $input::$arm,
                            $environment::$arm,
                        )?,
                    )+
                    _ => return Err(unsupported_harness(&harness, $family)),
                };
                execute_parsed(harness, &invocation, parsed, diagnostics, handler, |output: $output| {
                    match output {
                        $($output::$arm(output) => <$native as EventSpec>::emit(output),)+
                        _ => Err(HookkitError::InvalidProcessEmission(
                            "unknown aligned output arm cannot be emitted",
                        )),
                    }
                })
            }
        }

        impl ArmIdentity for $input {
            fn arm_harness(&self) -> HarnessId {
                self.harness()
            }

            fn arm_event(&self) -> EventId {
                self.event_id()
            }
        }

        impl ArmIdentity for $output {
            fn arm_harness(&self) -> HarnessId {
                self.harness()
            }

            fn arm_event(&self) -> EventId {
                self.event_id()
            }
        }

        #[doc = concat!("Convenience spelling for aligned `", $family, "` execution.")]
        pub fn $execute<F>(
            harness: HarnessId,
            bytes: impl Into<Vec<u8>>,
            variables: &EnvironmentVariables,
            handler: F,
        ) -> hookkit_core::Result<ProcessEmission>
        where
            F: FnOnce($input, &$environment, &RuntimeContext<'_>) -> hookkit_core::Result<$output>,
        {
            execute_aligned_event::<$marker, _>(harness, bytes, variables, handler)
        }
    };
}

aligned_family!(
    /// Marker for the aligned pre-tool event family (`PreToolUse` on every
    /// harness).
    marker: PreToolUse,
    input: PreToolUseInput,
    environment: PreToolUseCommandEnvironment,
    output: PreToolUseOutput,
    execute: execute_pre_tool_use,
    family: "PreToolUse",
    native_event: "PreToolUse",
    arms: [
        Claude(BuiltinHarness::ClaudeCode, hookkit_claude::catalog::PreToolUse),
        Codex(BuiltinHarness::Codex, hookkit_codex::protocol::PreToolUse),
        Antigravity(BuiltinHarness::Antigravity, hookkit_antigravity::PreToolUse),
    ]
);

aligned_family!(
    /// Marker for the aligned post-tool event family (`PostToolUse` on every
    /// harness).
    marker: PostToolUse,
    input: PostToolUseInput,
    environment: PostToolUseCommandEnvironment,
    output: PostToolUseOutput,
    execute: execute_post_tool_use,
    family: "PostToolUse",
    native_event: "PostToolUse",
    arms: [
        Claude(BuiltinHarness::ClaudeCode, hookkit_claude::protocol::PostToolUse),
        Codex(BuiltinHarness::Codex, hookkit_codex::protocol::PostToolUse),
        Antigravity(BuiltinHarness::Antigravity, hookkit_antigravity::PostToolUse),
    ]
);

aligned_family!(
    /// Marker for the aligned turn-completion event family (`Stop` on every
    /// harness).
    marker: TurnCompletion,
    input: TurnCompletionInput,
    environment: TurnCompletionCommandEnvironment,
    output: TurnCompletionOutput,
    execute: execute_turn_completion,
    family: "TurnCompletion",
    native_event: "Stop",
    arms: [
        Claude(BuiltinHarness::ClaudeCode, hookkit_claude::catalog::Stop),
        Codex(BuiltinHarness::Codex, hookkit_codex::catalog::Stop),
        Antigravity(BuiltinHarness::Antigravity, hookkit_antigravity::Stop),
    ]
);

aligned_family!(
    /// Marker for the Claude/Codex aligned permission-request event family.
    marker: PermissionRequest,
    input: PermissionRequestInput,
    environment: PermissionRequestCommandEnvironment,
    output: PermissionRequestOutput,
    execute: execute_permission_request,
    family: "PermissionRequest",
    native_event: "PermissionRequest",
    arms: [
        Claude(BuiltinHarness::ClaudeCode, hookkit_claude::catalog::PermissionRequest),
        Codex(BuiltinHarness::Codex, hookkit_codex::catalog::PermissionRequest),
    ]
);

aligned_family!(
    /// Marker for the Claude/Codex aligned pre-compaction event family.
    marker: PreCompact,
    input: PreCompactInput,
    environment: PreCompactCommandEnvironment,
    output: PreCompactOutput,
    execute: execute_pre_compact,
    family: "PreCompact",
    native_event: "PreCompact",
    arms: [
        Claude(BuiltinHarness::ClaudeCode, hookkit_claude::catalog::PreCompact),
        Codex(BuiltinHarness::Codex, hookkit_codex::catalog::PreCompact),
    ]
);

aligned_family!(
    /// Marker for the Claude/Codex aligned post-compaction event family.
    marker: PostCompact,
    input: PostCompactInput,
    environment: PostCompactCommandEnvironment,
    output: PostCompactOutput,
    execute: execute_post_compact,
    family: "PostCompact",
    native_event: "PostCompact",
    arms: [
        Claude(BuiltinHarness::ClaudeCode, hookkit_claude::catalog::PostCompact),
        Codex(BuiltinHarness::Codex, hookkit_codex::catalog::PostCompact),
    ]
);

aligned_family!(
    /// Marker for the Claude/Codex aligned session-start event family.
    marker: SessionStart,
    input: SessionStartInput,
    environment: SessionStartCommandEnvironment,
    output: SessionStartOutput,
    execute: execute_session_start,
    family: "SessionStart",
    native_event: "SessionStart",
    arms: [
        Claude(BuiltinHarness::ClaudeCode, hookkit_claude::protocol::SessionStart),
        Codex(BuiltinHarness::Codex, hookkit_codex::catalog::SessionStart),
    ]
);

aligned_family!(
    /// Marker for the Claude/Codex aligned session-end event family.
    marker: SessionEnd,
    input: SessionEndInput,
    environment: SessionEndCommandEnvironment,
    output: SessionEndOutput,
    execute: execute_session_end,
    family: "SessionEnd",
    native_event: "SessionEnd",
    arms: [
        Claude(BuiltinHarness::ClaudeCode, hookkit_claude::catalog::SessionEnd),
        Codex(BuiltinHarness::Codex, hookkit_codex::catalog::SessionEnd),
    ]
);

aligned_family!(
    /// Marker for the Claude/Codex aligned subagent-start event family.
    marker: SubagentStart,
    input: SubagentStartInput,
    environment: SubagentStartCommandEnvironment,
    output: SubagentStartOutput,
    execute: execute_subagent_start,
    family: "SubagentStart",
    native_event: "SubagentStart",
    arms: [
        Claude(BuiltinHarness::ClaudeCode, hookkit_claude::catalog::SubagentStart),
        Codex(BuiltinHarness::Codex, hookkit_codex::catalog::SubagentStart),
    ]
);

aligned_family!(
    /// Marker for the Claude/Codex aligned subagent-stop event family.
    marker: SubagentStop,
    input: SubagentStopInput,
    environment: SubagentStopCommandEnvironment,
    output: SubagentStopOutput,
    execute: execute_subagent_stop,
    family: "SubagentStop",
    native_event: "SubagentStop",
    arms: [
        Claude(BuiltinHarness::ClaudeCode, hookkit_claude::catalog::SubagentStop),
        Codex(BuiltinHarness::Codex, hookkit_codex::catalog::SubagentStop),
    ]
);

aligned_family!(
    /// Marker for the Claude/Codex aligned user-prompt-submit event family.
    marker: UserPromptSubmit,
    input: UserPromptSubmitInput,
    environment: UserPromptSubmitCommandEnvironment,
    output: UserPromptSubmitOutput,
    execute: execute_user_prompt_submit,
    family: "UserPromptSubmit",
    native_event: "UserPromptSubmit",
    arms: [
        Claude(BuiltinHarness::ClaudeCode, hookkit_claude::catalog::UserPromptSubmit),
        Codex(BuiltinHarness::Codex, hookkit_codex::catalog::UserPromptSubmit),
    ]
);

/// Execute one aligned event for an explicitly selected harness with
/// diagnostics disabled.
pub fn execute_aligned_event<K, F>(
    harness: HarnessId,
    bytes: impl Into<Vec<u8>>,
    variables: &EnvironmentVariables,
    handler: F,
) -> hookkit_core::Result<ProcessEmission>
where
    K: AlignedEventSpec,
    F: FnOnce(
        K::Input,
        &K::CommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<K::Output>,
{
    K::execute(harness, bytes.into(), variables, handler)
}

/// Execute one aligned event for an explicitly selected harness, handing the
/// handler `diagnostics` through [`RuntimeContext::diagnostics`].
pub fn execute_aligned_event_with_diagnostics<K, F>(
    harness: HarnessId,
    bytes: impl Into<Vec<u8>>,
    variables: &EnvironmentVariables,
    diagnostics: &dyn DiagnosticsSink,
    handler: F,
) -> hookkit_core::Result<ProcessEmission>
where
    K: AlignedEventSpec,
    F: FnOnce(
        K::Input,
        &K::CommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<K::Output>,
{
    K::execute_with_diagnostics(harness, bytes.into(), variables, diagnostics, handler)
}

/// Stdin/stdout adapter for an aligned event and explicit harness.
///
/// Equivalent to [`run_aligned_event_with_options`] with
/// [`RunOptions::new`]: diagnostics are disabled and every failure exits 1,
/// which Claude Code and Codex treat as non-blocking. A guard built on a
/// gating family therefore fails open; see the
/// [module documentation](self#failure-policy).
pub fn run_aligned_event<K, F>(harness: HarnessId, handler: F) -> std::process::ExitCode
where
    K: AlignedEventSpec,
    F: FnOnce(
        K::Input,
        &K::CommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<K::Output>,
{
    run_aligned_event_with_options::<K, _>(harness, RunOptions::new(), handler)
}

/// Stdin/stdout adapter with a configured out-of-band diagnostics sink.
///
/// Equivalent to [`run_aligned_event_with_options`] with
/// [`RunOptions::with_diagnostics`]. Runner failures are recorded in the sink
/// as well as reported on stderr.
pub fn run_aligned_event_with_diagnostics<K, F>(
    harness: HarnessId,
    diagnostics: &dyn DiagnosticsSink,
    handler: F,
) -> std::process::ExitCode
where
    K: AlignedEventSpec,
    F: FnOnce(
        K::Input,
        &K::CommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<K::Output>,
{
    run_aligned_event_with_options::<K, _>(
        harness,
        RunOptions::new().with_diagnostics(diagnostics),
        handler,
    )
}

/// Stdin/stdout adapter for an aligned event with explicit options.
///
/// Reads the payload from stdin, captures the selected harness's declared
/// environment, executes the handler, and writes the native emission.
/// `options` selects the diagnostics sink handed to the handler and the
/// [`crate::failure::FailurePolicy`] applied when stdin, the environment,
/// parsing, the handler (including a panic), or emission fails. The failure
/// is reported for the family's native event, so
/// [`RunOptions::fail_closed`] denies a pending tool call, prompt, or
/// permission instead of letting it proceed; see the
/// [module documentation](self#failure-policy).
pub fn run_aligned_event_with_options<K, F>(
    harness: HarnessId,
    options: RunOptions<'_>,
    handler: F,
) -> std::process::ExitCode
where
    K: AlignedEventSpec,
    F: FnOnce(
        K::Input,
        &K::CommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<K::Output>,
{
    let hook = format!("{harness}/{}", K::FAMILY);
    let event = EventId::builtin(harness.clone(), <K as sealed::Sealed>::NATIVE_EVENT);
    let fail = |error: &dyn std::error::Error| {
        report_run_failure(&options, &harness, Some(&event), &hook, error)
    };
    let bytes = match read_stdin() {
        Ok(bytes) => bytes,
        Err(error) => return fail(&error),
    };
    let variables =
        match capture_aligned_command_environment(&harness, K::FAMILY, options.diagnostics()) {
            Ok(variables) => variables,
            Err(error) => return fail(&error),
        };
    let executed = catch_panic(|| {
        execute_aligned_event_with_diagnostics::<K, _>(
            harness.clone(),
            bytes,
            &variables,
            options.diagnostics(),
            handler,
        )
    });
    match executed {
        Ok(Ok(emission)) => match crate::typed::try_write_emission(&emission) {
            Ok(code) => code,
            Err(error) => fail(&error),
        },
        Ok(Err(error)) => fail(&error),
        Err(panic) => fail(&panic),
    }
}

fn capture_aligned_command_environment(
    harness: &HarnessId,
    family: &'static str,
    diagnostics: &dyn DiagnosticsSink,
) -> hookkit_core::Result<EnvironmentVariables> {
    use crate::environment::capture_command_environment_with_diagnostics as capture;
    match BuiltinHarness::from_id(harness) {
        Some(BuiltinHarness::ClaudeCode) => {
            capture::<hookkit_claude::ClaudeCommandEnvironment>(diagnostics)
        }
        Some(BuiltinHarness::Codex) => {
            capture::<hookkit_codex::CodexCommandEnvironment>(diagnostics)
        }
        Some(BuiltinHarness::Antigravity) => {
            capture::<hookkit_antigravity::AntigravityCommandEnvironment>(diagnostics)
        }
        _ => Err(unsupported_harness(harness, family)),
    }
}

#[cfg(test)]
mod tests;
