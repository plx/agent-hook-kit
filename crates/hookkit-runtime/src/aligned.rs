//! Explicit-harness execution for lossless aligned lifecycle-event families.
//!
//! The sealed marker types select only library-defined alignments. Each path
//! parses, validates, and emits the exact native contract for the explicitly
//! selected harness, and rejects a handler output arm for another harness.

use hookkit_common::{
    PostToolUseCommandEnvironment, PostToolUseInput, PostToolUseOutput,
    PreToolUseCommandEnvironment, PreToolUseInput, PreToolUseOutput,
    TurnCompletionCommandEnvironment, TurnCompletionInput, TurnCompletionOutput,
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

mod sealed {
    pub trait Sealed {}
}

impl sealed::Sealed for PreToolUse {}
impl sealed::Sealed for PostToolUse {}
impl sealed::Sealed for TurnCompletion {}

/// Event-specific aligned execution contract.
pub trait AlignedEventSpec: sealed::Sealed {
    /// Lossless cross-harness input wrapper for the event family.
    type Input;
    /// Lossless cross-harness command-environment wrapper.
    type CommandEnvironment;
    /// Lossless cross-harness output wrapper for the event family.
    type Output;

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
    let mut bytes = Vec::new();
    if std::io::stdin().read_to_end(&mut bytes).is_err() {
        return std::process::ExitCode::from(1);
    }
    let variables = match capture_aligned_command_environment(&harness) {
        Ok(variables) => variables,
        Err(_) => return std::process::ExitCode::from(1),
    };
    match execute_aligned_event::<K, _>(harness, bytes, &variables, handler) {
        Ok(emission) => crate::typed::write_emission(&emission),
        Err(_) => std::process::ExitCode::from(1),
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
