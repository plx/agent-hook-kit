use hookkit_common::{
    PostToolUseCommandEnvironment, PostToolUseInput, PostToolUseOutput,
    TurnCompletionCommandEnvironment, TurnCompletionInput, TurnCompletionOutput,
};
use hookkit_core::{
    CommandEnvironmentSpec, ContractId, DISABLED_DIAGNOSTICS, EnvironmentVariables, EventId,
    EventSpec, HarnessId, HookkitError, NativeContext, ProcessEmission, RawInvocation,
    ResolutionProvenance, RuntimeContext, SnapshotId,
};
use std::io::Read;

/// Marker for the aligned post-tool event family.
pub enum PostToolUse {}

/// Marker for the aligned turn-completion event family.
pub enum TurnCompletion {}

mod sealed {
    pub trait Sealed {}
}

impl sealed::Sealed for PostToolUse {}
impl sealed::Sealed for TurnCompletion {}

/// Event-specific aligned execution contract.
pub trait AlignedEventSpec: sealed::Sealed {
    type Input;
    type CommandEnvironment;
    type Output;

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

/// Convenience spelling for the first aligned event family.
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

struct ParsedTurnCompletion {
    input: TurnCompletionInput,
    event: EventId,
    snapshot: SnapshotId,
    contract: ContractId,
    native_context: NativeContext,
    command_environment: TurnCompletionCommandEnvironment,
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
        "gemini-cli" => {
            let input = hookkit_gemini::protocol::AfterTool::parse(invocation)?;
            let command_environment = hookkit_gemini::GeminiCommandEnvironment::from_variables(
                &hookkit_gemini::protocol::AfterTool::EVENT,
                variables,
            )?;
            hookkit_gemini::protocol::AfterTool::validate_command_environment(
                &input,
                &command_environment,
            )?;
            let native_context = hookkit_gemini::protocol::AfterTool::context(&input);
            Ok(Parsed {
                input: PostToolUseInput::Gemini(input),
                event: hookkit_gemini::protocol::AfterTool::EVENT,
                snapshot: hookkit_gemini::protocol::AfterTool::SNAPSHOT,
                contract: hookkit_gemini::protocol::AfterTool::CONTRACT,
                native_context,
                command_environment: PostToolUseCommandEnvironment::Gemini(command_environment),
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
        "gemini-cli" => {
            let input = hookkit_gemini::catalog::AfterAgent::parse(invocation)?;
            let command_environment = hookkit_gemini::GeminiCommandEnvironment::from_variables(
                &hookkit_gemini::catalog::AfterAgent::EVENT,
                variables,
            )?;
            hookkit_gemini::catalog::AfterAgent::validate_command_environment(
                &input,
                &command_environment,
            )?;
            let native_context = hookkit_gemini::catalog::AfterAgent::context(&input);
            Ok(ParsedTurnCompletion {
                input: TurnCompletionInput::Gemini(input),
                event: hookkit_gemini::catalog::AfterAgent::EVENT,
                snapshot: hookkit_gemini::catalog::AfterAgent::SNAPSHOT,
                contract: hookkit_gemini::catalog::AfterAgent::CONTRACT,
                native_context,
                command_environment: TurnCompletionCommandEnvironment::Gemini(command_environment),
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
        "gemini-cli" => crate::environment::capture_command_environment::<
            hookkit_gemini::GeminiCommandEnvironment,
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
        PostToolUseOutput::Gemini(output) => hookkit_gemini::protocol::AfterTool::emit(output),
        PostToolUseOutput::Antigravity(output) => hookkit_antigravity::PostToolUse::emit(output),
        _ => Err(HookkitError::InvalidProcessEmission(
            "unknown aligned output arm cannot be emitted",
        )),
    }
}

fn emit_turn_completion(output: TurnCompletionOutput) -> hookkit_core::Result<ProcessEmission> {
    match output {
        TurnCompletionOutput::Claude(output) => hookkit_claude::catalog::Stop::emit(output),
        TurnCompletionOutput::Codex(output) => hookkit_codex::catalog::Stop::emit(output),
        TurnCompletionOutput::Gemini(output) => hookkit_gemini::catalog::AfterAgent::emit(output),
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

    fn gemini_variables() -> EnvironmentVariables {
        EnvironmentVariables::from_pairs([
            ("GEMINI_PROJECT_DIR", "/repo"),
            ("GEMINI_PLANS_DIR", "/repo/.gemini/plans"),
            ("GEMINI_CWD", "/repo"),
            ("GEMINI_SESSION_ID", "s"),
            ("CLAUDE_PROJECT_DIR", "/repo"),
        ])
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
    fn antigravity_clean_path_needs_no_fabricated_tool_data() {
        let bytes = br#"{"conversationId":"c","workspacePaths":["/repo","/lib"],"transcriptPath":"/tmp/t","artifactDirectoryPath":"/tmp/a","stepIdx":2}"#.to_vec();
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
                HarnessId::GEMINI_CLI,
                br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"AfterAgent","timestamp":"2026-07-15T00:00:00Z","prompt":"do it","prompt_response":"done","stop_hook_active":false}"#.as_slice(),
                gemini_variables(),
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
                        TurnCompletionInput::Gemini(_) => Ok(TurnCompletionOutput::Gemini(
                            hookkit_gemini::catalog::AfterAgentOutput::no_op(),
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
