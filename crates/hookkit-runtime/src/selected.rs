use crate::resolution::resolve_event;
use hookkit_core::{
    BuiltinHarness, CommandEnvironmentSpec, DISABLED_DIAGNOSTICS, DiagnosticsSink,
    EnvironmentVariables, EventId, EventSelector, HarnessSpec, ProcessEmission, RawInvocation,
    RuntimeContext,
};
use std::io::Read;

/// Execute a dynamically selected event within one compile-time selected harness.
pub fn execute_harness<H, F>(
    bytes: impl Into<Vec<u8>>,
    hint: Option<H::EventSelector>,
    variables: &EnvironmentVariables,
    handler: F,
) -> hookkit_core::Result<ProcessEmission>
where
    H: HarnessSpec,
    F: FnOnce(
        H::AnyInput,
        &H::CommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<H::AnyCommandOutput>,
{
    execute_harness_with_diagnostics::<H, _>(bytes, hint, variables, &DISABLED_DIAGNOSTICS, handler)
}

/// Executes a dynamically selected event and sends out-of-band diagnostics to
/// `diagnostics`.
///
/// Parsing, event resolution, environment validation, input/output arm
/// agreement, and emitted contract identity are checked before bytes are
/// returned. The handler is invoked only after all input-side checks pass.
pub fn execute_harness_with_diagnostics<H, F>(
    bytes: impl Into<Vec<u8>>,
    hint: Option<H::EventSelector>,
    variables: &EnvironmentVariables,
    diagnostics: &dyn DiagnosticsSink,
    handler: F,
) -> hookkit_core::Result<ProcessEmission>
where
    H: HarnessSpec,
    F: FnOnce(
        H::AnyInput,
        &H::CommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<H::AnyCommandOutput>,
{
    execute_harness_with_event_id::<H, _>(
        bytes,
        hint.as_ref().map(EventSelector::event_id),
        variables,
        diagnostics,
        handler,
    )
}

fn execute_harness_with_event_id<H, F>(
    bytes: impl Into<Vec<u8>>,
    hint: Option<EventId>,
    variables: &EnvironmentVariables,
    diagnostics: &dyn DiagnosticsSink,
    handler: F,
) -> hookkit_core::Result<ProcessEmission>
where
    H: HarnessSpec,
    F: FnOnce(
        H::AnyInput,
        &H::CommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<H::AnyCommandOutput>,
{
    if let Some(hint) = hint.as_ref().filter(|hint| hint.harness() != &H::ID) {
        return Err(hookkit_core::HookkitError::HintHarnessMismatch {
            selected: H::ID,
            hint: hint.clone(),
        });
    }
    let invocation = RawInvocation::parse(bytes)?;
    let resolved = resolve_event(&H::identification_descriptors(), H::ID, &invocation, hint)?;
    let input = H::decode(&resolved.event, &invocation)?;
    let input_event = H::input_event(&input);
    if input_event != resolved.event {
        return Err(hookkit_core::HookkitError::OutputEventMismatch {
            input: resolved.event,
            output: input_event,
        });
    }
    let environment =
        <H::CommandEnvironment as CommandEnvironmentSpec>::from_variables(&input_event, variables)?;
    H::validate_command_environment(&input, &environment)?;
    let context = RuntimeContext::new(
        H::ID,
        resolved.snapshot,
        input_event.clone(),
        resolved.contract,
        resolved.provenance,
        &invocation,
        H::context(&input),
        diagnostics,
    )?;
    let output = handler(input, &environment, &context)?;
    let output_event = H::output_event(&output);
    if output_event != input_event {
        return Err(hookkit_core::HookkitError::OutputEventMismatch {
            input: input_event,
            output: output_event,
        });
    }
    crate::typed::validate_command_emission(
        H::encode_command(&input_event, output)?,
        resolved.contract,
    )
}

/// Stdin/stdout adapter for a compile-time selected harness.
pub fn run_harness<H, F>(hint: Option<H::EventSelector>, handler: F) -> std::process::ExitCode
where
    H: HarnessSpec,
    F: FnOnce(
        H::AnyInput,
        &H::CommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<H::AnyCommandOutput>,
{
    let mut bytes = Vec::new();
    if let Err(error) = std::io::stdin().read_to_end(&mut bytes) {
        return crate::report::report_io_failure(H::ID, error);
    }
    let variables = match crate::environment::capture_command_environment::<H::CommandEnvironment>()
    {
        Ok(variables) => variables,
        Err(error) => return crate::report::report_failure(H::ID, &error),
    };
    match execute_harness::<H, _>(bytes, hint, &variables, handler) {
        Ok(emission) => crate::typed::write_emission(&emission),
        Err(error) => crate::report::report_failure(H::ID, &error),
    }
}

#[derive(Debug)]
/// Lossless sum type over inputs from all built-in harness adapters.
pub enum BuiltinInput {
    /// Claude Code input.
    Claude(hookkit_claude::protocol::AnyInput),
    /// Codex input.
    Codex(hookkit_codex::protocol::AnyInput),
    /// Antigravity input.
    Antigravity(hookkit_antigravity::AnyInput),
}

#[derive(Debug)]
/// Sum type over outputs from all built-in harness adapters.
pub enum BuiltinOutput {
    /// Claude Code output.
    Claude(hookkit_claude::protocol::AnyCommandOutput),
    /// Codex output.
    Codex(hookkit_codex::protocol::AnyCommandOutput),
    /// Antigravity output.
    Antigravity(hookkit_antigravity::AnyCommandOutput),
}

#[derive(Debug, Clone)]
/// Lossless sum type over native environments from all built-in harnesses.
pub enum BuiltinCommandEnvironment {
    /// Claude Code environment.
    Claude(hookkit_claude::ClaudeCommandEnvironment),
    /// Codex environment.
    Codex(hookkit_codex::CodexCommandEnvironment),
    /// Antigravity environment.
    Antigravity(hookkit_antigravity::AntigravityCommandEnvironment),
}

/// Execute through the separate runtime-selected built-in umbrella model.
pub fn execute_builtin_harness<F>(
    harness: BuiltinHarness,
    bytes: impl Into<Vec<u8>>,
    hint: Option<EventId>,
    variables: &EnvironmentVariables,
    handler: F,
) -> hookkit_core::Result<ProcessEmission>
where
    F: FnOnce(
        BuiltinInput,
        &BuiltinCommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<BuiltinOutput>,
{
    let bytes = bytes.into();
    match harness {
        BuiltinHarness::ClaudeCode => {
            execute_harness_with_event_id::<hookkit_claude::protocol::ClaudeCode, _>(
                bytes,
                hint,
                variables,
                &DISABLED_DIAGNOSTICS,
                |input, environment, context| {
                    let environment = BuiltinCommandEnvironment::Claude(environment.clone());
                    match handler(BuiltinInput::Claude(input), &environment, context)? {
                        BuiltinOutput::Claude(output) => Ok(output),
                        output => Err(builtin_harness_mismatch(context, &output)),
                    }
                },
            )
        }
        BuiltinHarness::Codex => {
            execute_harness_with_event_id::<hookkit_codex::protocol::Codex, _>(
                bytes,
                hint,
                variables,
                &DISABLED_DIAGNOSTICS,
                |input, environment, context| {
                    let environment = BuiltinCommandEnvironment::Codex(environment.clone());
                    match handler(BuiltinInput::Codex(input), &environment, context)? {
                        BuiltinOutput::Codex(output) => Ok(output),
                        output => Err(builtin_harness_mismatch(context, &output)),
                    }
                },
            )
        }
        BuiltinHarness::Antigravity => {
            execute_harness_with_event_id::<hookkit_antigravity::Antigravity, _>(
                bytes,
                hint,
                variables,
                &DISABLED_DIAGNOSTICS,
                |input, environment, context| {
                    let environment = BuiltinCommandEnvironment::Antigravity(*environment);
                    match handler(BuiltinInput::Antigravity(input), &environment, context)? {
                        BuiltinOutput::Antigravity(output) => Ok(output),
                        output => Err(builtin_harness_mismatch(context, &output)),
                    }
                },
            )
        }
        _ => Err(hookkit_core::HookkitError::UnsupportedBuiltinHarness(
            harness,
        )),
    }
}

/// Stdin/stdout adapter for runtime-selected built-in dispatch.
pub fn dispatch_builtin_harness<F>(
    harness: BuiltinHarness,
    hint: Option<EventId>,
    handler: F,
) -> std::process::ExitCode
where
    F: FnOnce(
        BuiltinInput,
        &BuiltinCommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<BuiltinOutput>,
{
    let mut bytes = Vec::new();
    if let Err(error) = std::io::stdin().read_to_end(&mut bytes) {
        return crate::report::report_io_failure(harness.id(), error);
    }
    let variables = match capture_builtin_command_environment(&harness) {
        Ok(variables) => variables,
        Err(error) => return crate::report::report_failure(harness.id(), &error),
    };
    match execute_builtin_harness(harness, bytes, hint, &variables, handler) {
        Ok(emission) => crate::typed::write_emission(&emission),
        Err(error) => crate::report::report_failure(harness.id(), &error),
    }
}

fn capture_builtin_command_environment(
    harness: &BuiltinHarness,
) -> hookkit_core::Result<EnvironmentVariables> {
    match harness {
        BuiltinHarness::ClaudeCode => crate::environment::capture_command_environment::<
            hookkit_claude::ClaudeCommandEnvironment,
        >(),
        BuiltinHarness::Codex => crate::environment::capture_command_environment::<
            hookkit_codex::CodexCommandEnvironment,
        >(),
        BuiltinHarness::Antigravity => crate::environment::capture_command_environment::<
            hookkit_antigravity::AntigravityCommandEnvironment,
        >(),
        _ => Err(hookkit_core::HookkitError::UnsupportedBuiltinHarness(
            *harness,
        )),
    }
}

fn builtin_harness_mismatch(
    context: &RuntimeContext<'_>,
    output: &BuiltinOutput,
) -> hookkit_core::HookkitError {
    let event = match output {
        BuiltinOutput::Claude(output) => hookkit_claude::protocol::ClaudeCode::output_event(output),
        BuiltinOutput::Codex(output) => hookkit_codex::protocol::Codex::output_event(output),
        BuiltinOutput::Antigravity(output) => {
            hookkit_antigravity::Antigravity::output_event(output)
        }
    };
    hookkit_core::HookkitError::EventHarnessMismatch {
        harness: context.harness().clone(),
        event,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hookkit_core::{ContractId, HarnessId, ResolutionProvenance, SnapshotId, Utf8Path};

    fn codex_pre_tool_use() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "session_id": "session-123",
            "transcript_path": "/tmp/codex-transcript.jsonl",
            "cwd": "/workspace",
            "hook_event_name": "PreToolUse",
            "model": "gpt-test",
            "turn_id": "turn-456",
            "permission_mode": "default",
            "tool_name": "shell",
            "tool_use_id": "call-789",
            "tool_input": {"command": "cargo test"}
        }))
        .unwrap()
    }

    #[test]
    fn compile_time_selected_harness_executes_with_exact_context() {
        let emission = execute_harness::<hookkit_codex::protocol::Codex, _>(
            codex_pre_tool_use(),
            None,
            &EnvironmentVariables::new(),
            |input, environment, context| {
                assert!(environment.plugin.is_none());
                let hookkit_codex::protocol::AnyInput::PreToolUse(input) = input else {
                    panic!("resolved the wrong Codex event")
                };
                assert_eq!(input.tool_name, "shell");
                assert_eq!(context.harness(), &HarnessId::CODEX);
                assert_eq!(context.snapshot(), SnapshotId::builtin("commit-1e59dc5-r1"));
                assert_eq!(
                    context.event(),
                    &EventId::builtin(HarnessId::CODEX, "PreToolUse")
                );
                assert_eq!(
                    context.contract(),
                    ContractId::builtin("codex/commit-1e59dc5-r1/PreToolUse")
                );
                assert_eq!(
                    context.provenance(),
                    ResolutionProvenance::DefinitiveDiscriminator
                );
                assert_eq!(context.workspace_roots(), [Utf8Path::new("/workspace")]);
                assert_eq!(context.session_id().unwrap().as_str(), "session-123");
                assert_eq!(context.turn_id().unwrap().as_str(), "turn-456");
                assert_eq!(context.tool_call_id().unwrap().as_str(), "call-789");
                assert_eq!(
                    context.transcript_path(),
                    Some(Utf8Path::new("/tmp/codex-transcript.jsonl"))
                );
                assert_eq!(context.raw().json()["hook_event_name"], "PreToolUse");

                Ok(hookkit_codex::protocol::AnyCommandOutput::PreToolUse(
                    hookkit_codex::protocol::PreToolUseOutput::deny("blocked by test"),
                ))
            },
        )
        .unwrap();

        assert_eq!(emission.exit_code(), 0);
        assert!(emission.stderr().is_empty());
        let output: serde_json::Value = serde_json::from_slice(emission.stdout()).unwrap();
        assert_eq!(output["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert_eq!(output["hookSpecificOutput"]["permissionDecision"], "deny");
    }

    #[test]
    fn compile_time_selected_harness_rejects_wrong_event_output() {
        let error = execute_harness::<hookkit_codex::protocol::Codex, _>(
            codex_pre_tool_use(),
            None,
            &EnvironmentVariables::new(),
            |_input, _environment, _context| {
                Ok(hookkit_codex::protocol::AnyCommandOutput::PostToolUse(
                    hookkit_codex::protocol::PostToolUseOutput::no_op(),
                ))
            },
        )
        .unwrap_err();

        assert!(matches!(
            error,
            hookkit_core::HookkitError::OutputEventMismatch { input, output }
                if input == EventId::builtin(HarnessId::CODEX, "PreToolUse")
                    && output == EventId::builtin(HarnessId::CODEX, "PostToolUse")
        ));
    }

    #[test]
    fn checked_dynamic_encoder_requires_the_triggering_event() {
        let error = hookkit_codex::protocol::Codex::encode_command(
            &EventId::builtin(HarnessId::CODEX, "PreToolUse"),
            hookkit_codex::protocol::AnyCommandOutput::PostToolUse(
                hookkit_codex::protocol::PostToolUseOutput::no_op(),
            ),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            hookkit_core::HookkitError::OutputEventMismatch { .. }
        ));
    }

    #[test]
    fn runtime_selected_builtin_executes_matching_harness() {
        let emission = execute_builtin_harness(
            BuiltinHarness::Codex,
            codex_pre_tool_use(),
            None,
            &EnvironmentVariables::new(),
            |input, environment, context| {
                assert_eq!(context.harness(), &HarnessId::CODEX);
                assert!(matches!(environment, BuiltinCommandEnvironment::Codex(_)));
                assert!(matches!(
                    input,
                    BuiltinInput::Codex(hookkit_codex::protocol::AnyInput::PreToolUse(_))
                ));
                Ok(BuiltinOutput::Codex(
                    hookkit_codex::protocol::AnyCommandOutput::PreToolUse(
                        hookkit_codex::protocol::PreToolUseOutput::no_op(),
                    ),
                ))
            },
        )
        .unwrap();

        assert!(emission.stdout().is_empty());
    }

    #[test]
    fn runtime_selected_builtin_rejects_cross_harness_output() {
        let error = execute_builtin_harness(
            BuiltinHarness::Codex,
            codex_pre_tool_use(),
            None,
            &EnvironmentVariables::new(),
            |_input, _environment, _context| {
                Ok(BuiltinOutput::Claude(
                    hookkit_claude::protocol::AnyCommandOutput::SessionStart(
                        hookkit_claude::protocol::SessionStartOutput::no_op(),
                    ),
                ))
            },
        )
        .unwrap_err();

        assert!(matches!(
            error,
            hookkit_core::HookkitError::EventHarnessMismatch { harness, event }
                if harness == HarnessId::CODEX
                    && event == EventId::builtin(HarnessId::CLAUDE_CODE, "SessionStart")
        ));
    }

    #[test]
    fn cross_harness_hint_precedes_invalid_payload_parsing() {
        let error = execute_builtin_harness(
            BuiltinHarness::Codex,
            b"not json".to_vec(),
            Some(EventId::builtin(HarnessId::CLAUDE_CODE, "SessionStart")),
            &EnvironmentVariables::new(),
            |_input, _environment, _context| {
                unreachable!("a foreign hint must fail before parsing")
            },
        )
        .unwrap_err();

        assert!(matches!(
            error,
            hookkit_core::HookkitError::HintHarnessMismatch { selected, hint }
                if selected == HarnessId::CODEX
                    && hint == EventId::builtin(HarnessId::CLAUDE_CODE, "SessionStart")
        ));
    }

    #[test]
    fn ambiguous_native_shape_requires_and_validates_event_hint() {
        let payload = serde_json::to_vec(&serde_json::json!({
            "conversationId": "conversation-1",
            "workspacePaths": ["/workspace"],
            "transcriptPath": "/tmp/antigravity-transcript.jsonl",
            "artifactDirectoryPath": "/tmp/artifacts",
            "invocationNum": 1,
            "initialNumSteps": 2
        }))
        .unwrap();

        let error = execute_harness::<hookkit_antigravity::Antigravity, _>(
            payload.clone(),
            None,
            &EnvironmentVariables::new(),
            |_input, _environment, _context| {
                Ok(hookkit_antigravity::AnyCommandOutput::PreInvocation(
                    hookkit_antigravity::PreInvocationOutput::no_op(),
                ))
            },
        )
        .unwrap_err();
        assert!(matches!(
            error,
            hookkit_core::HookkitError::AmbiguousEvent { harness, candidates }
                if harness == HarnessId::ANTIGRAVITY && candidates.len() == 2
        ));

        let emission = execute_harness::<hookkit_antigravity::Antigravity, _>(
            payload,
            Some(hookkit_antigravity::Event::PreInvocation),
            &EnvironmentVariables::new(),
            |input, _environment, context| {
                assert!(matches!(
                    input,
                    hookkit_antigravity::AnyInput::PreInvocation(_)
                ));
                assert_eq!(context.provenance(), ResolutionProvenance::HintValidated);
                Ok(hookkit_antigravity::AnyCommandOutput::PreInvocation(
                    hookkit_antigravity::PreInvocationOutput::no_op(),
                ))
            },
        )
        .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(emission.stdout()).unwrap(),
            serde_json::json!({})
        );
    }
}
