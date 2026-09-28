//! Explicit-harness execution for lossless aligned lifecycle-event families.
//!
//! The sealed marker types select only library-defined alignments. Each path
//! parses, validates, and emits the exact native contract for the explicitly
//! selected harness, and rejects a handler output arm for another harness.

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
    CommandEnvironmentSpec, ContractId, DISABLED_DIAGNOSTICS, EnvironmentVariables, EventId,
    EventSpec, HarnessId, HookkitError, NativeContext, ProcessEmission, RawInvocation,
    ResolutionProvenance, RuntimeContext, SnapshotId,
};
use std::io::Read;

/// Marker for the aligned pre-tool event family.
pub enum PreToolUse {}

/// Marker for the aligned post-tool event family.
pub enum PostToolUse {}

/// Marker for the aligned turn-completion event family.
pub enum TurnCompletion {}

/// Marker for the Claude/Codex aligned permission-request event family.
pub enum PermissionRequest {}

/// Marker for the Claude/Codex aligned pre-compaction event family.
pub enum PreCompact {}

/// Marker for the Claude/Codex aligned post-compaction event family.
pub enum PostCompact {}

/// Marker for the Claude/Codex aligned session-start event family.
pub enum SessionStart {}

/// Marker for the Claude/Codex aligned session-end event family.
pub enum SessionEnd {}

/// Marker for the Claude/Codex aligned subagent-start event family.
pub enum SubagentStart {}

/// Marker for the Claude/Codex aligned subagent-stop event family.
pub enum SubagentStop {}

/// Marker for the Claude/Codex aligned user-prompt-submit event family.
pub enum UserPromptSubmit {}

mod sealed {
    pub trait Sealed {}
}

impl sealed::Sealed for PreToolUse {}
impl sealed::Sealed for PostToolUse {}
impl sealed::Sealed for TurnCompletion {}
impl sealed::Sealed for PermissionRequest {}
impl sealed::Sealed for PreCompact {}
impl sealed::Sealed for PostCompact {}
impl sealed::Sealed for SessionStart {}
impl sealed::Sealed for SessionEnd {}
impl sealed::Sealed for SubagentStart {}
impl sealed::Sealed for SubagentStop {}
impl sealed::Sealed for UserPromptSubmit {}

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

    /// Parses, validates, handles, and emits one aligned event for `harness`.
    ///
    /// The implementation selects an exact native contract from the explicit
    /// harness identity. The handler must return the output arm for the same
    /// harness or execution fails before emission.
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
        ) -> hookkit_core::Result<Self::Output>;
}

impl AlignedEventSpec for PreToolUse {
    type Input = PreToolUseInput;
    type CommandEnvironment = PreToolUseCommandEnvironment;
    type Output = PreToolUseOutput;
    const FAMILY: &'static str = "PreToolUse";

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
        execute_pre_tool_use_inner(harness, bytes, variables, handler)
    }
}

impl AlignedEventSpec for PostToolUse {
    type Input = PostToolUseInput;
    type CommandEnvironment = PostToolUseCommandEnvironment;
    type Output = PostToolUseOutput;
    const FAMILY: &'static str = "PostToolUse";

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
        execute_post_tool_use_inner(harness, bytes, variables, handler)
    }
}

impl AlignedEventSpec for TurnCompletion {
    type Input = TurnCompletionInput;
    type CommandEnvironment = TurnCompletionCommandEnvironment;
    type Output = TurnCompletionOutput;
    const FAMILY: &'static str = "TurnCompletion";

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
        execute_turn_completion_inner(harness, bytes, variables, handler)
    }
}

macro_rules! claude_codex_runtime_alignment {
    (
        marker: $marker:ident,
        input: $input:ident,
        environment: $environment:ident,
        output: $output:ident,
        execute: $execute:ident,
        inner: $inner:ident,
        parsed: $parsed:ident,
        parse: $parse:ident,
        emit: $emit:ident,
        family: $family:literal,
        claude: $claude_event:ty,
        codex: $codex_event:ty
    ) => {
        impl AlignedEventSpec for $marker {
            type Input = $input;
            type CommandEnvironment = $environment;
            type Output = $output;
            const FAMILY: &'static str = $family;

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
                $inner(harness, bytes, variables, handler)
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

        fn $inner<F>(
            harness: HarnessId,
            bytes: Vec<u8>,
            variables: &EnvironmentVariables,
            handler: F,
        ) -> hookkit_core::Result<ProcessEmission>
        where
            F: FnOnce($input, &$environment, &RuntimeContext<'_>) -> hookkit_core::Result<$output>,
        {
            let invocation = RawInvocation::parse(bytes)?;
            let parsed = $parse(&harness, &invocation, variables)?;
            let expected_harness = parsed.input.harness();
            let context = RuntimeContext::new(
                harness,
                parsed.snapshot,
                parsed.event.clone(),
                parsed.contract,
                ResolutionProvenance::TypedStatic,
                &invocation,
                parsed.native_context,
                &DISABLED_DIAGNOSTICS,
            )?;
            let output = handler(parsed.input, &parsed.command_environment, &context)?;
            if output.harness() != expected_harness {
                return Err(HookkitError::EventHarnessMismatch {
                    harness: expected_harness,
                    event: output.event_id(),
                });
            }
            crate::typed::validate_command_emission($emit(output)?, context.contract())
        }

        struct $parsed {
            input: $input,
            event: EventId,
            snapshot: SnapshotId,
            contract: ContractId,
            native_context: NativeContext,
            command_environment: $environment,
        }

        fn $parse(
            harness: &HarnessId,
            invocation: &RawInvocation,
            variables: &EnvironmentVariables,
        ) -> hookkit_core::Result<$parsed> {
            match harness.as_str() {
                "claude-code" => {
                    let input = <$claude_event as EventSpec>::parse(invocation)?;
                    let command_environment =
                        hookkit_claude::ClaudeCommandEnvironment::from_variables(
                            &<$claude_event as EventSpec>::EVENT,
                            variables,
                        )?;
                    <$claude_event as EventSpec>::validate_command_environment(
                        &input,
                        &command_environment,
                    )?;
                    let native_context = <$claude_event as EventSpec>::context(&input);
                    Ok($parsed {
                        input: $input::Claude(input),
                        event: <$claude_event as EventSpec>::EVENT,
                        snapshot: <$claude_event as EventSpec>::SNAPSHOT,
                        contract: <$claude_event as EventSpec>::CONTRACT,
                        native_context,
                        command_environment: $environment::Claude(command_environment),
                    })
                }
                "codex" => {
                    let input = <$codex_event as EventSpec>::parse(invocation)?;
                    let command_environment =
                        hookkit_codex::CodexCommandEnvironment::from_variables(
                            &<$codex_event as EventSpec>::EVENT,
                            variables,
                        )?;
                    <$codex_event as EventSpec>::validate_command_environment(
                        &input,
                        &command_environment,
                    )?;
                    let native_context = <$codex_event as EventSpec>::context(&input);
                    Ok($parsed {
                        input: $input::Codex(input),
                        event: <$codex_event as EventSpec>::EVENT,
                        snapshot: <$codex_event as EventSpec>::SNAPSHOT,
                        contract: <$codex_event as EventSpec>::CONTRACT,
                        native_context,
                        command_environment: $environment::Codex(command_environment),
                    })
                }
                _ => Err(HookkitError::UnrecognizedEvent {
                    harness: harness.clone(),
                    message: concat!("no aligned ", $family, " adapter is registered").into(),
                }),
            }
        }

        fn $emit(output: $output) -> hookkit_core::Result<ProcessEmission> {
            match output {
                $output::Claude(output) => <$claude_event as EventSpec>::emit(output),
                $output::Codex(output) => <$codex_event as EventSpec>::emit(output),
                _ => Err(HookkitError::InvalidProcessEmission(
                    "unknown aligned output arm cannot be emitted",
                )),
            }
        }
    };
}

claude_codex_runtime_alignment!(
    marker: PermissionRequest,
    input: PermissionRequestInput,
    environment: PermissionRequestCommandEnvironment,
    output: PermissionRequestOutput,
    execute: execute_permission_request,
    inner: execute_permission_request_inner,
    parsed: ParsedPermissionRequest,
    parse: parse_selected_permission_request,
    emit: emit_permission_request,
    family: "PermissionRequest",
    claude: hookkit_claude::catalog::PermissionRequest,
    codex: hookkit_codex::catalog::PermissionRequest
);

claude_codex_runtime_alignment!(
    marker: PreCompact,
    input: PreCompactInput,
    environment: PreCompactCommandEnvironment,
    output: PreCompactOutput,
    execute: execute_pre_compact,
    inner: execute_pre_compact_inner,
    parsed: ParsedPreCompact,
    parse: parse_selected_pre_compact,
    emit: emit_pre_compact,
    family: "PreCompact",
    claude: hookkit_claude::catalog::PreCompact,
    codex: hookkit_codex::catalog::PreCompact
);

claude_codex_runtime_alignment!(
    marker: PostCompact,
    input: PostCompactInput,
    environment: PostCompactCommandEnvironment,
    output: PostCompactOutput,
    execute: execute_post_compact,
    inner: execute_post_compact_inner,
    parsed: ParsedPostCompact,
    parse: parse_selected_post_compact,
    emit: emit_post_compact,
    family: "PostCompact",
    claude: hookkit_claude::catalog::PostCompact,
    codex: hookkit_codex::catalog::PostCompact
);

claude_codex_runtime_alignment!(
    marker: SessionStart,
    input: SessionStartInput,
    environment: SessionStartCommandEnvironment,
    output: SessionStartOutput,
    execute: execute_session_start,
    inner: execute_session_start_inner,
    parsed: ParsedSessionStart,
    parse: parse_selected_session_start,
    emit: emit_session_start,
    family: "SessionStart",
    claude: hookkit_claude::protocol::SessionStart,
    codex: hookkit_codex::catalog::SessionStart
);

claude_codex_runtime_alignment!(
    marker: SessionEnd,
    input: SessionEndInput,
    environment: SessionEndCommandEnvironment,
    output: SessionEndOutput,
    execute: execute_session_end,
    inner: execute_session_end_inner,
    parsed: ParsedSessionEnd,
    parse: parse_selected_session_end,
    emit: emit_session_end,
    family: "SessionEnd",
    claude: hookkit_claude::catalog::SessionEnd,
    codex: hookkit_codex::catalog::SessionEnd
);

claude_codex_runtime_alignment!(
    marker: SubagentStart,
    input: SubagentStartInput,
    environment: SubagentStartCommandEnvironment,
    output: SubagentStartOutput,
    execute: execute_subagent_start,
    inner: execute_subagent_start_inner,
    parsed: ParsedSubagentStart,
    parse: parse_selected_subagent_start,
    emit: emit_subagent_start,
    family: "SubagentStart",
    claude: hookkit_claude::catalog::SubagentStart,
    codex: hookkit_codex::catalog::SubagentStart
);

claude_codex_runtime_alignment!(
    marker: SubagentStop,
    input: SubagentStopInput,
    environment: SubagentStopCommandEnvironment,
    output: SubagentStopOutput,
    execute: execute_subagent_stop,
    inner: execute_subagent_stop_inner,
    parsed: ParsedSubagentStop,
    parse: parse_selected_subagent_stop,
    emit: emit_subagent_stop,
    family: "SubagentStop",
    claude: hookkit_claude::catalog::SubagentStop,
    codex: hookkit_codex::catalog::SubagentStop
);

claude_codex_runtime_alignment!(
    marker: UserPromptSubmit,
    input: UserPromptSubmitInput,
    environment: UserPromptSubmitCommandEnvironment,
    output: UserPromptSubmitOutput,
    execute: execute_user_prompt_submit,
    inner: execute_user_prompt_submit_inner,
    parsed: ParsedUserPromptSubmit,
    parse: parse_selected_user_prompt_submit,
    emit: emit_user_prompt_submit,
    family: "UserPromptSubmit",
    claude: hookkit_claude::catalog::UserPromptSubmit,
    codex: hookkit_codex::catalog::UserPromptSubmit
);

/// Execute one aligned event for an explicitly selected harness.
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

/// Stdin/stdout adapter for an aligned event and explicit harness.
pub fn run_aligned_event<K, F>(harness: HarnessId, handler: F) -> std::process::ExitCode
where
    K: AlignedEventSpec,
    F: FnOnce(
        K::Input,
        &K::CommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<K::Output>,
{
    let hook = format!("{harness}/{}", K::FAMILY);
    let mut bytes = Vec::new();
    if let Err(error) = std::io::stdin().read_to_end(&mut bytes) {
        return crate::report::report_io_failure(&hook, error);
    }
    let variables = match capture_aligned_command_environment(&harness) {
        Ok(variables) => variables,
        Err(error) => return crate::report::report_failure(&hook, &error),
    };
    match execute_aligned_event::<K, _>(harness, bytes, &variables, handler) {
        Ok(emission) => crate::typed::write_emission(&emission),
        Err(error) => crate::report::report_failure(&hook, &error),
    }
}

/// Convenience spelling for aligned pre-tool execution.
pub fn execute_pre_tool_use<F>(
    harness: HarnessId,
    bytes: impl Into<Vec<u8>>,
    variables: &EnvironmentVariables,
    handler: F,
) -> hookkit_core::Result<ProcessEmission>
where
    F: FnOnce(
        PreToolUseInput,
        &PreToolUseCommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<PreToolUseOutput>,
{
    execute_aligned_event::<PreToolUse, _>(harness, bytes, variables, handler)
}

/// Convenience spelling for aligned post-tool execution.
pub fn execute_post_tool_use<F>(
    harness: HarnessId,
    bytes: impl Into<Vec<u8>>,
    variables: &EnvironmentVariables,
    handler: F,
) -> hookkit_core::Result<ProcessEmission>
where
    F: FnOnce(
        PostToolUseInput,
        &PostToolUseCommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<PostToolUseOutput>,
{
    execute_aligned_event::<PostToolUse, _>(harness, bytes, variables, handler)
}

/// Convenience spelling for aligned turn completion.
pub fn execute_turn_completion<F>(
    harness: HarnessId,
    bytes: impl Into<Vec<u8>>,
    variables: &EnvironmentVariables,
    handler: F,
) -> hookkit_core::Result<ProcessEmission>
where
    F: FnOnce(
        TurnCompletionInput,
        &TurnCompletionCommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<TurnCompletionOutput>,
{
    execute_aligned_event::<TurnCompletion, _>(harness, bytes, variables, handler)
}

fn execute_pre_tool_use_inner<F>(
    harness: HarnessId,
    bytes: Vec<u8>,
    variables: &EnvironmentVariables,
    handler: F,
) -> hookkit_core::Result<ProcessEmission>
where
    F: FnOnce(
        PreToolUseInput,
        &PreToolUseCommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<PreToolUseOutput>,
{
    let invocation = RawInvocation::parse(bytes)?;
    let parsed = parse_selected_pre_tool_use(&harness, &invocation, variables)?;
    let expected_harness = parsed.input.harness();
    let context = RuntimeContext::new(
        harness,
        parsed.snapshot,
        parsed.event.clone(),
        parsed.contract,
        ResolutionProvenance::TypedStatic,
        &invocation,
        parsed.native_context,
        &DISABLED_DIAGNOSTICS,
    )?;
    let output = handler(parsed.input, &parsed.command_environment, &context)?;
    if output.harness() != expected_harness {
        return Err(HookkitError::EventHarnessMismatch {
            harness: expected_harness,
            event: output.event_id(),
        });
    }
    crate::typed::validate_command_emission(emit_pre_tool_use(output)?, context.contract())
}

fn execute_post_tool_use_inner<F>(
    harness: HarnessId,
    bytes: Vec<u8>,
    variables: &EnvironmentVariables,
    handler: F,
) -> hookkit_core::Result<ProcessEmission>
where
    F: FnOnce(
        PostToolUseInput,
        &PostToolUseCommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<PostToolUseOutput>,
{
    let invocation = RawInvocation::parse(bytes)?;
    let parsed = parse_selected(&harness, &invocation, variables)?;
    let expected_harness = parsed.input.harness();
    let context = RuntimeContext::new(
        harness,
        parsed.snapshot,
        parsed.event.clone(),
        parsed.contract,
        ResolutionProvenance::TypedStatic,
        &invocation,
        parsed.native_context,
        &DISABLED_DIAGNOSTICS,
    )?;
    let output = handler(parsed.input, &parsed.command_environment, &context)?;
    if output.harness() != expected_harness {
        return Err(HookkitError::EventHarnessMismatch {
            harness: expected_harness,
            event: output.event_id(),
        });
    }
    crate::typed::validate_command_emission(emit(output)?, context.contract())
}

fn execute_turn_completion_inner<F>(
    harness: HarnessId,
    bytes: Vec<u8>,
    variables: &EnvironmentVariables,
    handler: F,
) -> hookkit_core::Result<ProcessEmission>
where
    F: FnOnce(
        TurnCompletionInput,
        &TurnCompletionCommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<TurnCompletionOutput>,
{
    let invocation = RawInvocation::parse(bytes)?;
    let parsed = parse_selected_turn_completion(&harness, &invocation, variables)?;
    let expected_harness = parsed.input.harness();
    let context = RuntimeContext::new(
        harness,
        parsed.snapshot,
        parsed.event.clone(),
        parsed.contract,
        ResolutionProvenance::TypedStatic,
        &invocation,
        parsed.native_context,
        &DISABLED_DIAGNOSTICS,
    )?;
    let output = handler(parsed.input, &parsed.command_environment, &context)?;
    if output.harness() != expected_harness {
        return Err(HookkitError::EventHarnessMismatch {
            harness: expected_harness,
            event: output.event_id(),
        });
    }
    crate::typed::validate_command_emission(emit_turn_completion(output)?, context.contract())
}

struct Parsed {
    input: PostToolUseInput,
    event: EventId,
    snapshot: SnapshotId,
    contract: ContractId,
    native_context: NativeContext,
    command_environment: PostToolUseCommandEnvironment,
}

struct ParsedPreToolUse {
    input: PreToolUseInput,
    event: EventId,
    snapshot: SnapshotId,
    contract: ContractId,
    native_context: NativeContext,
    command_environment: PreToolUseCommandEnvironment,
}

struct ParsedTurnCompletion {
    input: TurnCompletionInput,
    event: EventId,
    snapshot: SnapshotId,
    contract: ContractId,
    native_context: NativeContext,
    command_environment: TurnCompletionCommandEnvironment,
}

fn parse_selected_pre_tool_use(
    harness: &HarnessId,
    invocation: &RawInvocation,
    variables: &EnvironmentVariables,
) -> hookkit_core::Result<ParsedPreToolUse> {
    match harness.as_str() {
        "claude-code" => {
            let input = hookkit_claude::catalog::PreToolUse::parse(invocation)?;
            let command_environment = hookkit_claude::ClaudeCommandEnvironment::from_variables(
                &hookkit_claude::catalog::PreToolUse::EVENT,
                variables,
            )?;
            hookkit_claude::catalog::PreToolUse::validate_command_environment(
                &input,
                &command_environment,
            )?;
            let native_context = hookkit_claude::catalog::PreToolUse::context(&input);
            Ok(ParsedPreToolUse {
                input: PreToolUseInput::Claude(input),
                event: hookkit_claude::catalog::PreToolUse::EVENT,
                snapshot: hookkit_claude::catalog::PreToolUse::SNAPSHOT,
                contract: hookkit_claude::catalog::PreToolUse::CONTRACT,
                native_context,
                command_environment: PreToolUseCommandEnvironment::Claude(command_environment),
            })
        }
        "codex" => {
            let input = hookkit_codex::protocol::PreToolUse::parse(invocation)?;
            let command_environment = hookkit_codex::CodexCommandEnvironment::from_variables(
                &hookkit_codex::protocol::PreToolUse::EVENT,
                variables,
            )?;
            hookkit_codex::protocol::PreToolUse::validate_command_environment(
                &input,
                &command_environment,
            )?;
            let native_context = hookkit_codex::protocol::PreToolUse::context(&input);
            Ok(ParsedPreToolUse {
                input: PreToolUseInput::Codex(input),
                event: hookkit_codex::protocol::PreToolUse::EVENT,
                snapshot: hookkit_codex::protocol::PreToolUse::SNAPSHOT,
                contract: hookkit_codex::protocol::PreToolUse::CONTRACT,
                native_context,
                command_environment: PreToolUseCommandEnvironment::Codex(command_environment),
            })
        }
        "antigravity" => {
            let input = hookkit_antigravity::PreToolUse::parse(invocation)?;
            let command_environment =
                hookkit_antigravity::AntigravityCommandEnvironment::from_variables(
                    &hookkit_antigravity::PreToolUse::EVENT,
                    variables,
                )?;
            hookkit_antigravity::PreToolUse::validate_command_environment(
                &input,
                &command_environment,
            )?;
            let native_context = hookkit_antigravity::PreToolUse::context(&input);
            Ok(ParsedPreToolUse {
                input: PreToolUseInput::Antigravity(input),
                event: hookkit_antigravity::PreToolUse::EVENT,
                snapshot: hookkit_antigravity::PreToolUse::SNAPSHOT,
                contract: hookkit_antigravity::PreToolUse::CONTRACT,
                native_context,
                command_environment: PreToolUseCommandEnvironment::Antigravity(command_environment),
            })
        }
        _ => Err(HookkitError::UnrecognizedEvent {
            harness: harness.clone(),
            message: "no aligned PreToolUse adapter is registered".into(),
        }),
    }
}

fn parse_selected(
    harness: &HarnessId,
    invocation: &RawInvocation,
    variables: &EnvironmentVariables,
) -> hookkit_core::Result<Parsed> {
    match harness.as_str() {
        "claude-code" => {
            let input = hookkit_claude::protocol::PostToolUse::parse(invocation)?;
            let command_environment = hookkit_claude::ClaudeCommandEnvironment::from_variables(
                &hookkit_claude::protocol::PostToolUse::EVENT,
                variables,
            )?;
            hookkit_claude::protocol::PostToolUse::validate_command_environment(
                &input,
                &command_environment,
            )?;
            let native_context = hookkit_claude::protocol::PostToolUse::context(&input);
            Ok(Parsed {
                input: PostToolUseInput::Claude(input),
                event: hookkit_claude::protocol::PostToolUse::EVENT,
                snapshot: hookkit_claude::protocol::PostToolUse::SNAPSHOT,
                contract: hookkit_claude::protocol::PostToolUse::CONTRACT,
                native_context,
                command_environment: PostToolUseCommandEnvironment::Claude(command_environment),
            })
        }
        "codex" => {
            let input = hookkit_codex::protocol::PostToolUse::parse(invocation)?;
            let command_environment = hookkit_codex::CodexCommandEnvironment::from_variables(
                &hookkit_codex::protocol::PostToolUse::EVENT,
                variables,
            )?;
            hookkit_codex::protocol::PostToolUse::validate_command_environment(
                &input,
                &command_environment,
            )?;
            let native_context = hookkit_codex::protocol::PostToolUse::context(&input);
            Ok(Parsed {
                input: PostToolUseInput::Codex(input),
                event: hookkit_codex::protocol::PostToolUse::EVENT,
                snapshot: hookkit_codex::protocol::PostToolUse::SNAPSHOT,
                contract: hookkit_codex::protocol::PostToolUse::CONTRACT,
                native_context,
                command_environment: PostToolUseCommandEnvironment::Codex(command_environment),
            })
        }
        "antigravity" => {
            let input = hookkit_antigravity::PostToolUse::parse(invocation)?;
            let command_environment =
                hookkit_antigravity::AntigravityCommandEnvironment::from_variables(
                    &hookkit_antigravity::PostToolUse::EVENT,
                    variables,
                )?;
            hookkit_antigravity::PostToolUse::validate_command_environment(
                &input,
                &command_environment,
            )?;
            let native_context = hookkit_antigravity::PostToolUse::context(&input);
            Ok(Parsed {
                input: PostToolUseInput::Antigravity(input),
                event: hookkit_antigravity::PostToolUse::EVENT,
                snapshot: hookkit_antigravity::PostToolUse::SNAPSHOT,
                contract: hookkit_antigravity::PostToolUse::CONTRACT,
                native_context,
                command_environment: PostToolUseCommandEnvironment::Antigravity(
                    command_environment,
                ),
            })
        }
        _ => Err(HookkitError::UnrecognizedEvent {
            harness: harness.clone(),
            message: "no aligned PostToolUse adapter is registered".into(),
        }),
    }
}

fn parse_selected_turn_completion(
    harness: &HarnessId,
    invocation: &RawInvocation,
    variables: &EnvironmentVariables,
) -> hookkit_core::Result<ParsedTurnCompletion> {
    match harness.as_str() {
        "claude-code" => {
            let input = hookkit_claude::catalog::Stop::parse(invocation)?;
            let command_environment = hookkit_claude::ClaudeCommandEnvironment::from_variables(
                &hookkit_claude::catalog::Stop::EVENT,
                variables,
            )?;
            hookkit_claude::catalog::Stop::validate_command_environment(
                &input,
                &command_environment,
            )?;
            let native_context = hookkit_claude::catalog::Stop::context(&input);
            Ok(ParsedTurnCompletion {
                input: TurnCompletionInput::Claude(input),
                event: hookkit_claude::catalog::Stop::EVENT,
                snapshot: hookkit_claude::catalog::Stop::SNAPSHOT,
                contract: hookkit_claude::catalog::Stop::CONTRACT,
                native_context,
                command_environment: TurnCompletionCommandEnvironment::Claude(command_environment),
            })
        }
        "codex" => {
            let input = hookkit_codex::catalog::Stop::parse(invocation)?;
            let command_environment = hookkit_codex::CodexCommandEnvironment::from_variables(
                &hookkit_codex::catalog::Stop::EVENT,
                variables,
            )?;
            hookkit_codex::catalog::Stop::validate_command_environment(
                &input,
                &command_environment,
            )?;
            let native_context = hookkit_codex::catalog::Stop::context(&input);
            Ok(ParsedTurnCompletion {
                input: TurnCompletionInput::Codex(input),
                event: hookkit_codex::catalog::Stop::EVENT,
                snapshot: hookkit_codex::catalog::Stop::SNAPSHOT,
                contract: hookkit_codex::catalog::Stop::CONTRACT,
                native_context,
                command_environment: TurnCompletionCommandEnvironment::Codex(command_environment),
            })
        }
        "antigravity" => {
            let input = hookkit_antigravity::Stop::parse(invocation)?;
            let command_environment =
                hookkit_antigravity::AntigravityCommandEnvironment::from_variables(
                    &hookkit_antigravity::Stop::EVENT,
                    variables,
                )?;
            hookkit_antigravity::Stop::validate_command_environment(&input, &command_environment)?;
            let native_context = hookkit_antigravity::Stop::context(&input);
            Ok(ParsedTurnCompletion {
                input: TurnCompletionInput::Antigravity(input),
                event: hookkit_antigravity::Stop::EVENT,
                snapshot: hookkit_antigravity::Stop::SNAPSHOT,
                contract: hookkit_antigravity::Stop::CONTRACT,
                native_context,
                command_environment: TurnCompletionCommandEnvironment::Antigravity(
                    command_environment,
                ),
            })
        }
        _ => Err(HookkitError::UnrecognizedEvent {
            harness: harness.clone(),
            message: "no aligned TurnCompletion adapter is registered".into(),
        }),
    }
}

fn capture_aligned_command_environment(
    harness: &HarnessId,
) -> hookkit_core::Result<EnvironmentVariables> {
    match harness.as_str() {
        "claude-code" => crate::environment::capture_command_environment::<
            hookkit_claude::ClaudeCommandEnvironment,
        >(),
        "codex" => crate::environment::capture_command_environment::<
            hookkit_codex::CodexCommandEnvironment,
        >(),
        "antigravity" => crate::environment::capture_command_environment::<
            hookkit_antigravity::AntigravityCommandEnvironment,
        >(),
        _ => Err(HookkitError::UnrecognizedEvent {
            harness: harness.clone(),
            message: "no aligned command environment adapter is registered".into(),
        }),
    }
}

fn emit(output: PostToolUseOutput) -> hookkit_core::Result<ProcessEmission> {
    match output {
        PostToolUseOutput::Claude(output) => hookkit_claude::protocol::PostToolUse::emit(output),
        PostToolUseOutput::Codex(output) => hookkit_codex::protocol::PostToolUse::emit(output),
        PostToolUseOutput::Antigravity(output) => hookkit_antigravity::PostToolUse::emit(output),
        _ => Err(HookkitError::InvalidProcessEmission(
            "unknown aligned output arm cannot be emitted",
        )),
    }
}

fn emit_pre_tool_use(output: PreToolUseOutput) -> hookkit_core::Result<ProcessEmission> {
    match output {
        PreToolUseOutput::Claude(output) => hookkit_claude::catalog::PreToolUse::emit(output),
        PreToolUseOutput::Codex(output) => hookkit_codex::protocol::PreToolUse::emit(output),
        PreToolUseOutput::Antigravity(output) => hookkit_antigravity::PreToolUse::emit(output),
        _ => Err(HookkitError::InvalidProcessEmission(
            "unknown aligned output arm cannot be emitted",
        )),
    }
}

fn emit_turn_completion(output: TurnCompletionOutput) -> hookkit_core::Result<ProcessEmission> {
    match output {
        TurnCompletionOutput::Claude(output) => hookkit_claude::catalog::Stop::emit(output),
        TurnCompletionOutput::Codex(output) => hookkit_codex::catalog::Stop::emit(output),
        TurnCompletionOutput::Antigravity(output) => hookkit_antigravity::Stop::emit(output),
        _ => Err(HookkitError::InvalidProcessEmission(
            "unknown aligned output arm cannot be emitted",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claude_variables() -> EnvironmentVariables {
        EnvironmentVariables::from_pairs([
            ("CLAUDECODE", "1"),
            ("CLAUDE_CODE_CHILD_SESSION", "1"),
            ("CLAUDE_CODE_SESSION_ID", "s"),
            ("CLAUDE_PROJECT_DIR", "/repo"),
            ("CLAUDE_ENV_FILE", "/tmp/claude-env"),
        ])
    }

    fn pre_tool_cases() -> Vec<(HarnessId, &'static [u8], EnvironmentVariables)> {
        vec![
            (
                HarnessId::CLAUDE_CODE,
                br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"PreToolUse","permission_mode":"default","tool_name":"Read","tool_input":{"path":".env"},"tool_use_id":"u","claude_only":{"retained":true}}"#,
                claude_variables(),
            ),
            (
                HarnessId::CODEX,
                br#"{"session_id":"s","transcript_path":null,"cwd":"/repo","hook_event_name":"PreToolUse","model":"gpt-5","turn_id":"t","permission_mode":"default","tool_name":"Read","tool_input":{"path":".env"},"tool_use_id":"u","codex_only":"retained"}"#,
                EnvironmentVariables::new(),
            ),
            (
                HarnessId::ANTIGRAVITY,
                br#"{"conversationId":"s","workspacePaths":["/repo","/lib"],"transcriptPath":"/tmp/t","artifactDirectoryPath":"/tmp/a","toolCall":{"name":"read_file","args":{"path":".env"},"nativeFlag":true},"stepIdx":7,"antigravityOnly":"retained"}"#,
                EnvironmentVariables::from_pairs([("AMBIENT_ONLY", "ignored")]),
            ),
        ]
    }

    fn pair_inputs(
        event: &'static str,
        claude_fields: serde_json::Value,
        codex_fields: serde_json::Value,
    ) -> Vec<(HarnessId, Vec<u8>, EnvironmentVariables)> {
        fn merged(mut base: serde_json::Value, fields: serde_json::Value) -> serde_json::Value {
            base.as_object_mut()
                .expect("base is an object")
                .extend(fields.as_object().expect("fields are an object").clone());
            base
        }

        let claude = merged(
            serde_json::json!({
                "session_id": "s",
                "transcript_path": "/tmp/claude.jsonl",
                "cwd": "/repo",
                "hook_event_name": event,
                "native_only": {"claude": true}
            }),
            claude_fields,
        );
        let mut codex = serde_json::json!({
            "session_id": "s",
            "transcript_path": null,
            "cwd": "/repo",
            "hook_event_name": event,
            "native_only": {"codex": true}
        });
        if event != "SessionEnd" {
            codex
                .as_object_mut()
                .expect("base is an object")
                .insert("model".into(), "gpt-test".into());
        }
        let codex = merged(codex, codex_fields);

        vec![
            (
                HarnessId::CLAUDE_CODE,
                serde_json::to_vec(&claude).unwrap(),
                claude_variables(),
            ),
            (
                HarnessId::CODEX,
                serde_json::to_vec(&codex).unwrap(),
                EnvironmentVariables::new(),
            ),
        ]
    }

    macro_rules! assert_pair_family {
        (
            marker: $marker:ty,
            input: $input:ident,
            event: $event:literal,
            claude: $claude_fields:expr,
            codex: $codex_fields:expr,
            output: $output:expr
        ) => {{
            let build_output = $output;
            for (harness, bytes, variables) in pair_inputs($event, $claude_fields, $codex_fields) {
                let expected_harness = harness.clone();
                let handler_harness = harness.clone();
                let emission = execute_aligned_event::<$marker, _>(
                    harness,
                    bytes,
                    &variables,
                    move |input, environment, context| {
                        assert_eq!(input.harness(), handler_harness);
                        assert_eq!(input.event_id().name(), $event);
                        assert_eq!(input.event_id().harness(), &handler_harness);
                        assert_eq!(input.session_id(), "s");
                        assert_eq!(environment.harness(), handler_harness);
                        assert_eq!(context.harness(), &handler_harness);
                        assert_eq!(context.event(), &input.event_id());
                        assert_eq!(input.workspace_roots(), context.workspace_roots());
                        assert_eq!(input.cwd().as_str(), "/repo");
                        match input {
                            $input::Claude(_) => {
                                assert_eq!(handler_harness, HarnessId::CLAUDE_CODE)
                            }
                            $input::Codex(_) => assert_eq!(handler_harness, HarnessId::CODEX),
                            _ => unreachable!("only Claude and Codex are aligned here"),
                        }
                        build_output(&handler_harness)
                    },
                )
                .unwrap();
                assert_eq!(emission.exit_code(), 0, "{} {}", expected_harness, $event);
                assert!(emission.stderr().is_empty());
            }
        }};
    }

    #[test]
    fn claude_codex_pair_families_parse_and_emit_their_portable_floors() {
        assert_pair_family!(
            marker: PermissionRequest,
            input: PermissionRequestInput,
            event: "PermissionRequest",
            claude: serde_json::json!({"tool_name": "Bash", "tool_input": {"command": "true"}}),
            codex: serde_json::json!({
                "turn_id": "t",
                "permission_mode": "default",
                "tool_name": "Bash",
                "tool_input": {"command": "true"}
            }),
            output: |harness: &HarnessId| PermissionRequestOutput::allow(harness)
        );
        assert_pair_family!(
            marker: PreCompact,
            input: PreCompactInput,
            event: "PreCompact",
            claude: serde_json::json!({"trigger": "auto", "custom_instructions": "keep tests"}),
            codex: serde_json::json!({"turn_id": "t", "trigger": "auto"}),
            output: |harness: &HarnessId| PreCompactOutput::no_op(harness)
        );
        assert_pair_family!(
            marker: PostCompact,
            input: PostCompactInput,
            event: "PostCompact",
            claude: serde_json::json!({"trigger": "auto", "compact_summary": "summary"}),
            codex: serde_json::json!({"turn_id": "t", "trigger": "auto"}),
            output: |harness: &HarnessId| PostCompactOutput::with_system_notice(harness, "done")
        );
        assert_pair_family!(
            marker: SessionStart,
            input: SessionStartInput,
            event: "SessionStart",
            claude: serde_json::json!({"source": "startup"}),
            codex: serde_json::json!({"permission_mode": "default", "source": "startup"}),
            output: |harness: &HarnessId| SessionStartOutput::with_context(harness, "context")
        );
        assert_pair_family!(
            marker: SessionEnd,
            input: SessionEndInput,
            event: "SessionEnd",
            claude: serde_json::json!({"reason": "prompt_input_exit"}),
            codex: serde_json::json!({"reason": "other"}),
            output: |harness: &HarnessId| SessionEndOutput::no_op(harness)
        );
        assert_pair_family!(
            marker: SubagentStart,
            input: SubagentStartInput,
            event: "SubagentStart",
            claude: serde_json::json!({"agent_id": "a", "agent_type": "Explore"}),
            codex: serde_json::json!({
                "turn_id": "t",
                "permission_mode": "default",
                "agent_id": "a",
                "agent_type": "Explore"
            }),
            output: |harness: &HarnessId| SubagentStartOutput::with_context(harness, "context")
        );
        assert_pair_family!(
            marker: SubagentStop,
            input: SubagentStopInput,
            event: "SubagentStop",
            claude: serde_json::json!({
                "stop_hook_active": false,
                "agent_id": "a",
                "agent_type": "Explore",
                "agent_transcript_path": "/tmp/a.jsonl",
                "last_assistant_message": "done"
            }),
            codex: serde_json::json!({
                "turn_id": "t",
                "permission_mode": "default",
                "stop_hook_active": false,
                "agent_id": "a",
                "agent_type": "Explore",
                "agent_transcript_path": "/tmp/a.jsonl",
                "last_assistant_message": "done"
            }),
            output: |harness: &HarnessId| SubagentStopOutput::block(harness, "continue")
        );
        assert_pair_family!(
            marker: UserPromptSubmit,
            input: UserPromptSubmitInput,
            event: "UserPromptSubmit",
            claude: serde_json::json!({"prompt": "ship it"}),
            codex: serde_json::json!({
                "turn_id": "t",
                "permission_mode": "default",
                "prompt": "ship it"
            }),
            output: |harness: &HarnessId| UserPromptSubmitOutput::with_context(harness, "context")
        );
    }

    #[test]
    fn pair_only_portable_block_and_deny_helpers_use_exact_native_shapes() {
        for (harness, bytes, variables) in pair_inputs(
            "PermissionRequest",
            serde_json::json!({"tool_name": "Bash", "tool_input": {"command": "true"}}),
            serde_json::json!({
                "turn_id": "t",
                "permission_mode": "default",
                "tool_name": "Bash",
                "tool_input": {"command": "true"}
            }),
        ) {
            let selected = harness.clone();
            let emission =
                execute_permission_request(harness, bytes, &variables, move |_, _, _| {
                    PermissionRequestOutput::deny(&selected, "denied")
                })
                .unwrap();
            let output: serde_json::Value = serde_json::from_slice(emission.stdout()).unwrap();
            assert_eq!(output["hookSpecificOutput"]["decision"]["behavior"], "deny");
            assert_eq!(
                output["hookSpecificOutput"]["decision"]["message"],
                "denied"
            );
        }

        for (harness, bytes, variables) in pair_inputs(
            "UserPromptSubmit",
            serde_json::json!({"prompt": "ship it"}),
            serde_json::json!({
                "turn_id": "t",
                "permission_mode": "default",
                "prompt": "ship it"
            }),
        ) {
            let selected = harness.clone();
            let emission =
                execute_user_prompt_submit(harness, bytes, &variables, move |_, _, _| {
                    UserPromptSubmitOutput::block(&selected, "blocked")
                })
                .unwrap();
            assert_eq!(emission.exit_code(), 2);
            assert!(emission.stdout().is_empty());
            assert_eq!(emission.stderr(), b"blocked");
        }
    }

    #[test]
    fn pair_only_markers_reject_antigravity_and_mismatched_output_arms() {
        let unsupported = execute_aligned_event::<SessionStart, _>(
            HarnessId::ANTIGRAVITY,
            br#"{}"#,
            &EnvironmentVariables::new(),
            |_, _, _| panic!("unsupported harness must not invoke the handler"),
        );
        assert!(matches!(
            unsupported,
            Err(HookkitError::UnrecognizedEvent { .. })
        ));

        let (_, bytes, variables) = pair_inputs(
            "SessionStart",
            serde_json::json!({"source": "startup"}),
            serde_json::json!({"permission_mode": "default", "source": "startup"}),
        )
        .remove(1);
        let mismatched = execute_session_start(HarnessId::CODEX, bytes, &variables, |_, _, _| {
            Ok(SessionStartOutput::Claude(
                hookkit_claude::protocol::SessionStartOutput::no_op(),
            ))
        });
        assert!(matches!(
            mismatched,
            Err(HookkitError::EventHarnessMismatch { .. })
        ));
    }

    #[test]
    fn aligned_pre_tool_parses_and_allows_every_native_contract() {
        for (harness, bytes, variables) in pre_tool_cases() {
            let expected_harness = harness.clone();
            let handler_harness = harness.clone();
            let emission = execute_pre_tool_use(
                harness,
                bytes,
                &variables,
                move |input, environment, context| {
                    assert_eq!(input.harness(), handler_harness);
                    assert_eq!(environment.harness(), handler_harness);
                    assert_eq!(context.harness(), &handler_harness);
                    assert_eq!(input.event_id().harness(), &handler_harness);
                    assert_eq!(input.workspace_roots(), context.workspace_roots());
                    assert_eq!(
                        input.cwd().is_none(),
                        handler_harness == HarnessId::ANTIGRAVITY
                    );
                    assert_eq!(
                        input.tool_input().and_then(|input| input.get("path")),
                        Some(&serde_json::json!(".env"))
                    );
                    assert!(input.tool_name().is_some());

                    match (&input, environment) {
                        (
                            PreToolUseInput::Claude(input),
                            PreToolUseCommandEnvironment::Claude(environment),
                        ) => {
                            assert_eq!(
                                input.field("claude_only"),
                                Some(&serde_json::json!({"retained": true}))
                            );
                            assert_eq!(environment.project_dir, "/repo");
                            assert_eq!(input.cwd, "/repo");
                        }
                        (
                            PreToolUseInput::Codex(input),
                            PreToolUseCommandEnvironment::Codex(environment),
                        ) => {
                            assert_eq!(
                                input.extra.get("codex_only"),
                                Some(&serde_json::json!("retained"))
                            );
                            assert!(environment.plugin.is_none());
                            assert_eq!(input.cwd, "/repo");
                        }
                        (
                            PreToolUseInput::Antigravity(input),
                            PreToolUseCommandEnvironment::Antigravity(_),
                        ) => {
                            assert_eq!(input.step_idx, 7);
                            assert_eq!(
                                input.tool_call.extra.get("nativeFlag"),
                                Some(&serde_json::json!(true))
                            );
                            assert_eq!(
                                input.extra.get("antigravityOnly"),
                                Some(&serde_json::json!("retained"))
                            );
                            assert!(input.workspace_paths.len() == 2);
                        }
                        _ => panic!("input and command-environment arms must match"),
                    }

                    let output = PreToolUseOutput::allow(&handler_harness)?;
                    assert_eq!(output.event_id(), input.event_id());
                    Ok(output)
                },
            )
            .unwrap();

            assert_eq!(emission.exit_code(), 0);
            assert!(emission.stderr().is_empty());
            if expected_harness == HarnessId::CODEX {
                assert!(emission.stdout().is_empty());
                continue;
            }
            let output: serde_json::Value = serde_json::from_slice(emission.stdout()).unwrap();
            match expected_harness.as_str() {
                "claude-code" => {
                    assert_eq!(output["hookSpecificOutput"]["hookEventName"], "PreToolUse");
                    assert_eq!(output["hookSpecificOutput"]["permissionDecision"], "allow");
                }
                "antigravity" => assert_eq!(output, serde_json::json!({"decision": "allow"})),
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn aligned_pre_tool_denies_with_each_native_shape() {
        for (harness, bytes, variables) in pre_tool_cases() {
            let expected_harness = harness.clone();
            let emission = execute_aligned_event::<PreToolUse, _>(
                harness,
                bytes,
                &variables,
                move |_, _, _| PreToolUseOutput::deny(&expected_harness, "blocked by policy"),
            )
            .unwrap();

            assert_eq!(emission.exit_code(), 0);
            assert!(emission.stderr().is_empty());
            let output: serde_json::Value = serde_json::from_slice(emission.stdout()).unwrap();
            assert_eq!(
                output["decision"]
                    .as_str()
                    .or_else(|| output["hookSpecificOutput"]["permissionDecision"].as_str()),
                Some("deny")
            );
            assert_eq!(
                output["reason"]
                    .as_str()
                    .or_else(|| output["hookSpecificOutput"]["permissionDecisionReason"].as_str()),
                Some("blocked by policy")
            );
        }
    }

    #[test]
    fn aligned_pre_tool_rejects_wrong_output_arm_before_emission() {
        let (_, bytes, variables) = pre_tool_cases().remove(1);
        let result =
            execute_pre_tool_use(HarnessId::CODEX, bytes, &variables, |_, environment, _| {
                assert!(matches!(
                    environment,
                    PreToolUseCommandEnvironment::Codex(_)
                ));
                Ok(PreToolUseOutput::Claude(
                    hookkit_claude::catalog::PreToolUseOutput::no_op(),
                ))
            });
        assert!(matches!(
            result,
            Err(HookkitError::EventHarnessMismatch { .. })
        ));
    }

    #[test]
    fn aligned_pre_tool_validates_required_native_environments() {
        let (claude, claude_bytes, _) = pre_tool_cases().remove(0);
        let claude_result = execute_pre_tool_use(
            claude,
            claude_bytes,
            &EnvironmentVariables::new(),
            |_, _, _| panic!("handler must not run for an invalid environment"),
        );
        assert!(matches!(
            claude_result,
            Err(HookkitError::InvalidHookEnvironment { .. })
        ));
    }

    #[test]
    fn mismatched_harness_arms_fail_before_emission() {
        let bytes = br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"PostToolUse","tool_name":"Bash","tool_input":{},"tool_use_id":"u","tool_response":{}}"#.to_vec();
        let result = execute_post_tool_use(
            HarnessId::CLAUDE_CODE,
            bytes,
            &claude_variables(),
            |_, environment, _| {
                assert!(matches!(
                    environment,
                    PostToolUseCommandEnvironment::Claude(_)
                ));
                Ok(PostToolUseOutput::Codex(
                    hookkit_codex::protocol::PostToolUseOutput::no_op(),
                ))
            },
        );
        assert!(matches!(
            result,
            Err(HookkitError::EventHarnessMismatch { .. })
        ));
    }

    #[test]
    fn antigravity_post_tool_preserves_typed_tool_data() {
        let bytes = br#"{"conversationId":"c","workspacePaths":["/repo","/lib"],"transcriptPath":"/tmp/t","artifactDirectoryPath":"/tmp/a","toolCall":{"name":"run_command","args":{"CommandLine":"cargo test","Cwd":"/repo"}},"stepIdx":2}"#.to_vec();
        let emission = execute_post_tool_use(
            HarnessId::ANTIGRAVITY,
            bytes,
            &EnvironmentVariables::new(),
            |input, environment, context| {
                assert_eq!(input.workspace_roots().len(), 2);
                assert_eq!(context.workspace_roots().len(), 2);
                assert!(matches!(
                    environment,
                    PostToolUseCommandEnvironment::Antigravity(_)
                ));
                Ok(PostToolUseOutput::Antigravity(Default::default()))
            },
        )
        .unwrap();
        assert_eq!(emission.stdout(), b"{}");
    }

    #[test]
    fn turn_completion_preserves_each_native_contract() {
        let cases = [
            (
                HarnessId::CLAUDE_CODE,
                br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"Stop","stop_hook_active":false,"last_assistant_message":"done"}"#.as_slice(),
                claude_variables(),
            ),
            (
                HarnessId::CODEX,
                br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"Stop","model":"gpt-5","turn_id":"t","permission_mode":"default","stop_hook_active":false,"last_assistant_message":"done"}"#.as_slice(),
                EnvironmentVariables::new(),
            ),
            (
                HarnessId::ANTIGRAVITY,
                br#"{"conversationId":"s","workspacePaths":["/repo"],"transcriptPath":"/tmp/t","artifactDirectoryPath":"/tmp/a","executionNum":1,"terminationReason":"completed","fullyIdle":true}"#.as_slice(),
                EnvironmentVariables::new(),
            ),
        ];

        for (harness, bytes, variables) in cases {
            let expected = harness.clone();
            let handler_harness = expected.clone();
            let emission = execute_turn_completion(
                harness,
                bytes,
                &variables,
                move |input, environment, context| {
                    assert_eq!(input.harness(), handler_harness);
                    assert_eq!(environment.harness(), handler_harness);
                    assert_eq!(context.workspace_roots()[0].as_str(), "/repo");
                    match input {
                        TurnCompletionInput::Claude(_) => Ok(TurnCompletionOutput::Claude(
                            hookkit_claude::catalog::StopOutput::no_op(),
                        )),
                        TurnCompletionInput::Codex(_) => Ok(TurnCompletionOutput::Codex(
                            hookkit_codex::catalog::StopOutput::no_op(),
                        )),
                        TurnCompletionInput::Antigravity(_) => Ok(
                            TurnCompletionOutput::Antigravity(hookkit_antigravity::StopOutput {
                                decision: "stop".into(),
                                reason: None,
                            }),
                        ),
                        _ => unreachable!(),
                    }
                },
            )
            .unwrap();
            let json: serde_json::Value = serde_json::from_slice(emission.stdout()).unwrap();
            if expected == HarnessId::ANTIGRAVITY {
                assert_eq!(json, serde_json::json!({"decision": "stop"}));
            } else {
                assert_eq!(json, serde_json::json!({}));
            }
        }
    }
}
