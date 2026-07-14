use hookkit_common::{PostToolUseCommandEnvironment, PostToolUseInput, PostToolUseOutput};
use hookkit_core::{
    CommandEnvironmentSpec, ContractId, DISABLED_DIAGNOSTICS, EnvironmentVariables, EventId,
    EventSpec, HarnessId, HookkitError, NativeContext, ProcessEmission, RawInvocation,
    ResolutionProvenance, RuntimeContext, SnapshotId,
};
use std::io::Read;

/// Marker for the aligned post-tool event family.
pub enum PostToolUse {}

mod sealed {
    pub trait Sealed {}
}

impl sealed::Sealed for PostToolUse {}

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

struct Parsed {
    input: PostToolUseInput,
    event: EventId,
    snapshot: SnapshotId,
    contract: ContractId,
    native_context: NativeContext,
    command_environment: PostToolUseCommandEnvironment,
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
}
