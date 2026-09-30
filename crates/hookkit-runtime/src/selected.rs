use crate::failure::{RunOptions, catch_panic, read_stdin, report_run_failure};
use crate::resolution::{ParserCheck, resolve_event, resolve_event_with};
use hookkit_core::{
    BuiltinHarness, CommandEnvironmentSpec, DISABLED_DIAGNOSTICS, DiagnosticsSink,
    EnvironmentVariables, EventId, EventSelector, HarnessId, HarnessSpec, IdentificationDescriptor,
    ProcessEmission, RawInvocation, RuntimeContext,
};

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
    let hint = hint.as_ref().map(EventSelector::event_id);
    let invocation = parse_for_harness(&H::ID, hint.as_ref(), bytes.into())?;
    execute_invocation::<H, _>(
        &invocation,
        hint,
        variables,
        diagnostics,
        &mut None,
        handler,
    )
}

/// Rejects a hint for another harness before any payload analysis, then
/// parses the payload.
fn parse_for_harness(
    harness: &HarnessId,
    hint: Option<&EventId>,
    bytes: Vec<u8>,
) -> hookkit_core::Result<RawInvocation> {
    if let Some(hint) = hint.filter(|hint| hint.harness() != harness) {
        return Err(hookkit_core::HookkitError::HintHarnessMismatch {
            selected: harness.clone(),
            hint: hint.clone(),
        });
    }
    RawInvocation::parse(bytes)
}

/// Resolves, decodes, handles, and emits one invocation.
///
/// `resolved` receives the event once the payload has decoded as it, so a
/// runner can lower a later handler or emission failure for that event.
fn execute_invocation<H, F>(
    invocation: &RawInvocation,
    hint: Option<EventId>,
    variables: &EnvironmentVariables,
    diagnostics: &dyn DiagnosticsSink,
    resolved: &mut Option<EventId>,
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
    let descriptors = H::identification_descriptors();
    // A hint or discriminator fixes the event without running its parser;
    // `H::decode` then parses the payload exactly once. Only a decoding
    // failure re-runs the validating resolver, to classify the error the same
    // way `resolve_event` does.
    let selected = resolve_event_with(
        &descriptors,
        H::ID,
        invocation,
        hint.clone(),
        ParserCheck::DeferToDecode,
    )?;
    let input = match H::decode(&selected.event, invocation) {
        Ok(input) => input,
        Err(error) => {
            resolve_event(&descriptors, H::ID, invocation, hint)?;
            return Err(hookkit_core::HookkitError::InvalidInputForEvent {
                event: selected.event,
                source: Box::new(error),
            });
        }
    };
    let input_event = H::input_event(&input);
    if input_event != selected.event {
        return Err(hookkit_core::HookkitError::DecodedEventMismatch {
            resolved: selected.event,
            decoded: input_event,
        });
    }
    *resolved = Some(input_event.clone());
    let environment =
        <H::CommandEnvironment as CommandEnvironmentSpec>::from_variables(&input_event, variables)?;
    H::validate_command_environment(&input, &environment)?;
    let context = RuntimeContext::new(
        H::ID,
        selected.snapshot,
        input_event.clone(),
        selected.contract,
        selected.provenance,
        invocation,
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
        selected.contract,
    )
}

/// Stdin/stdout adapter for a compile-time selected harness.
///
/// On any failure it writes one `hookkit: <program> <hook> failed: ...` line
/// to stderr and exits 1, which Claude Code and Codex treat as a non-blocking
/// error: the pending action proceeds (Claude Code `WorktreeCreate` and
/// `WorktreeRemove` fail on any non-zero exit). Use
/// [`run_harness_with_options`] to install a diagnostics sink or to fail
/// closed.
pub fn run_harness<H, F>(hint: Option<H::EventSelector>, handler: F) -> std::process::ExitCode
where
    H: HarnessSpec,
    F: FnOnce(
        H::AnyInput,
        &H::CommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<H::AnyCommandOutput>,
{
    run_harness_with_options::<H, _>(hint, RunOptions::new(), handler)
}

/// Stdin/stdout adapter for a compile-time selected harness with explicit
/// options.
///
/// `options` selects the diagnostics sink handed to the handler and the
/// [`crate::failure::FailurePolicy`] applied to failures, including a handler
/// panic. Every failure is recorded in the sink and reported on stderr.
///
/// A failure is lowered for the event the harness actually sent: the event
/// the runner resolved when the failure came later (a handler error, a
/// panic, or an emission failure), otherwise the event the payload's
/// authoritative discriminator names, and only when the payload names none,
/// `hint`. No parser runs to classify a failure. A hint that contradicts the
/// payload's discriminator is itself a failure, lowered for the payload's
/// event, so a hook registered under the wrong event does not let a
/// `PreToolUse` through or block a `Stop`.
pub fn run_harness_with_options<H, F>(
    hint: Option<H::EventSelector>,
    options: RunOptions<'_>,
    handler: F,
) -> std::process::ExitCode
where
    H: HarnessSpec,
    F: FnOnce(
        H::AnyInput,
        &H::CommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<H::AnyCommandOutput>,
{
    let hint = hint.as_ref().map(EventSelector::event_id);
    let mut resolved = None;
    let fail = |invocation: Option<&RawInvocation>,
                resolved: Option<&EventId>,
                error: &dyn std::error::Error| {
        let event = failure_event(
            &H::identification_descriptors(),
            &H::ID,
            resolved,
            invocation,
            hint.as_ref(),
        );
        report_failure_for(&options, &H::ID, event.as_ref(), error)
    };
    let bytes = match read_stdin() {
        Ok(bytes) => bytes,
        Err(error) => return fail(None, None, &error),
    };
    let parsed = parse_for_harness(&H::ID, hint.as_ref(), bytes);
    let variables = match crate::environment::capture_command_environment_with_diagnostics::<
        H::CommandEnvironment,
    >(options.diagnostics())
    {
        Ok(variables) => variables,
        Err(error) => return fail(parsed.as_ref().ok(), None, &error),
    };
    let invocation = match parsed {
        Ok(invocation) => invocation,
        Err(error) => return fail(None, None, &error),
    };
    let executed = catch_panic(|| {
        execute_invocation::<H, _>(
            &invocation,
            hint.clone(),
            &variables,
            options.diagnostics(),
            &mut resolved,
            handler,
        )
    });
    let resolved = resolved.as_ref();
    match executed {
        Ok(Ok(emission)) => match crate::typed::try_write_emission(&emission) {
            Ok(code) => code,
            Err(error) => fail(Some(&invocation), resolved, &error),
        },
        Ok(Err(error)) => fail(Some(&invocation), resolved, &error),
        Err(panic) => fail(Some(&invocation), resolved, &panic),
    }
}

/// Best-effort native event for a failed invocation, without running any
/// parser: the event execution resolved, else the event an authoritative
/// discriminator in the payload names, else the hint when it belongs to
/// `harness`.
///
/// The harness interprets the exit code for the event it actually sent, so
/// the payload outranks a hint that contradicts it.
pub(crate) fn failure_event(
    descriptors: &[IdentificationDescriptor],
    harness: &HarnessId,
    resolved: Option<&EventId>,
    invocation: Option<&RawInvocation>,
    hint: Option<&EventId>,
) -> Option<EventId> {
    resolved
        .cloned()
        .or_else(|| {
            invocation.and_then(|invocation| {
                crate::resolution::discriminated_event(descriptors, harness, invocation)
            })
        })
        .or_else(|| hint.filter(|hint| hint.harness() == harness).cloned())
}

fn report_failure_for(
    options: &RunOptions<'_>,
    harness: &HarnessId,
    event: Option<&EventId>,
    error: &dyn std::error::Error,
) -> std::process::ExitCode {
    match event {
        Some(event) => report_run_failure(options, harness, Some(event), event, error),
        None => report_run_failure(options, harness, None, harness, error),
    }
}

#[derive(Debug)]
#[non_exhaustive]
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
#[non_exhaustive]
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
#[non_exhaustive]
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
    execute_builtin_harness_with_diagnostics(
        harness,
        bytes,
        hint,
        variables,
        &DISABLED_DIAGNOSTICS,
        handler,
    )
}

/// [`execute_builtin_harness`] with an explicit out-of-band diagnostics sink,
/// which the handler reaches through [`RuntimeContext::diagnostics`].
pub fn execute_builtin_harness_with_diagnostics<F>(
    harness: BuiltinHarness,
    bytes: impl Into<Vec<u8>>,
    hint: Option<EventId>,
    variables: &EnvironmentVariables,
    diagnostics: &dyn DiagnosticsSink,
    handler: F,
) -> hookkit_core::Result<ProcessEmission>
where
    F: FnOnce(
        BuiltinInput,
        &BuiltinCommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<BuiltinOutput>,
{
    let invocation = parse_for_harness(&harness.id(), hint.as_ref(), bytes.into())?;
    execute_builtin_invocation(
        harness,
        &invocation,
        hint,
        variables,
        diagnostics,
        &mut None,
        handler,
    )
}

fn execute_builtin_invocation<F>(
    harness: BuiltinHarness,
    invocation: &RawInvocation,
    hint: Option<EventId>,
    variables: &EnvironmentVariables,
    diagnostics: &dyn DiagnosticsSink,
    resolved: &mut Option<EventId>,
    handler: F,
) -> hookkit_core::Result<ProcessEmission>
where
    F: FnOnce(
        BuiltinInput,
        &BuiltinCommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<BuiltinOutput>,
{
    match harness {
        BuiltinHarness::ClaudeCode => {
            execute_invocation::<hookkit_claude::protocol::ClaudeCode, _>(
                invocation,
                hint,
                variables,
                diagnostics,
                resolved,
                |input, environment, context| {
                    let environment = BuiltinCommandEnvironment::Claude(environment.clone());
                    match handler(BuiltinInput::Claude(input), &environment, context)? {
                        BuiltinOutput::Claude(output) => Ok(output),
                        output => Err(builtin_harness_mismatch(context, &output)),
                    }
                },
            )
        }
        BuiltinHarness::Codex => execute_invocation::<hookkit_codex::protocol::Codex, _>(
            invocation,
            hint,
            variables,
            diagnostics,
            resolved,
            |input, environment, context| {
                let environment = BuiltinCommandEnvironment::Codex(environment.clone());
                match handler(BuiltinInput::Codex(input), &environment, context)? {
                    BuiltinOutput::Codex(output) => Ok(output),
                    output => Err(builtin_harness_mismatch(context, &output)),
                }
            },
        ),
        BuiltinHarness::Antigravity => execute_invocation::<hookkit_antigravity::Antigravity, _>(
            invocation,
            hint,
            variables,
            diagnostics,
            resolved,
            |input, environment, context| {
                let environment = BuiltinCommandEnvironment::Antigravity(*environment);
                match handler(BuiltinInput::Antigravity(input), &environment, context)? {
                    BuiltinOutput::Antigravity(output) => Ok(output),
                    output => Err(builtin_harness_mismatch(context, &output)),
                }
            },
        ),
        _ => Err(hookkit_core::HookkitError::UnsupportedBuiltinHarness(
            harness,
        )),
    }
}

/// Stdin/stdout adapter for runtime-selected built-in dispatch.
///
/// On any failure it writes one `hookkit: <program> <hook> failed: ...` line
/// to stderr and exits 1, which Claude Code and Codex treat as a non-blocking
/// error: the pending action proceeds (Claude Code `WorktreeCreate` and
/// `WorktreeRemove` fail on any non-zero exit). Use
/// [`dispatch_builtin_harness_with_options`] to install a diagnostics sink or
/// to fail closed.
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
    dispatch_builtin_harness_with_options(harness, hint, RunOptions::new(), handler)
}

/// Stdin/stdout adapter for runtime-selected built-in dispatch with explicit
/// options. See [`run_harness_with_options`] for how failures are reported.
pub fn dispatch_builtin_harness_with_options<F>(
    harness: BuiltinHarness,
    hint: Option<EventId>,
    options: RunOptions<'_>,
    handler: F,
) -> std::process::ExitCode
where
    F: FnOnce(
        BuiltinInput,
        &BuiltinCommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<BuiltinOutput>,
{
    let id = harness.id();
    let mut resolved = None;
    let fail = |invocation: Option<&RawInvocation>,
                resolved: Option<&EventId>,
                error: &dyn std::error::Error| {
        let event = failure_event(
            &builtin_harness_descriptors(harness),
            &id,
            resolved,
            invocation,
            hint.as_ref(),
        );
        report_failure_for(&options, &id, event.as_ref(), error)
    };
    let bytes = match read_stdin() {
        Ok(bytes) => bytes,
        Err(error) => return fail(None, None, &error),
    };
    let parsed = parse_for_harness(&id, hint.as_ref(), bytes);
    let variables = match capture_builtin_command_environment(&harness, options.diagnostics()) {
        Ok(variables) => variables,
        Err(error) => return fail(parsed.as_ref().ok(), None, &error),
    };
    let invocation = match parsed {
        Ok(invocation) => invocation,
        Err(error) => return fail(None, None, &error),
    };
    let executed = catch_panic(|| {
        execute_builtin_invocation(
            harness,
            &invocation,
            hint.clone(),
            &variables,
            options.diagnostics(),
            &mut resolved,
            handler,
        )
    });
    let resolved = resolved.as_ref();
    match executed {
        Ok(Ok(emission)) => match crate::typed::try_write_emission(&emission) {
            Ok(code) => code,
            Err(error) => fail(Some(&invocation), resolved, &error),
        },
        Ok(Err(error)) => fail(Some(&invocation), resolved, &error),
        Err(panic) => fail(Some(&invocation), resolved, &panic),
    }
}

/// The event an authoritative discriminator in `invocation` names, when
/// `harness` is a built-in harness. No parser runs.
pub(crate) fn builtin_payload_event(
    harness: &HarnessId,
    invocation: Option<&RawInvocation>,
) -> Option<EventId> {
    let builtin = BuiltinHarness::from_id(harness)?;
    crate::resolution::discriminated_event(
        &builtin_harness_descriptors(builtin),
        harness,
        invocation?,
    )
}

/// The identification descriptors of one built-in harness adapter.
pub(crate) fn builtin_harness_descriptors(
    harness: BuiltinHarness,
) -> Vec<IdentificationDescriptor> {
    match harness {
        BuiltinHarness::ClaudeCode => hookkit_claude::protocol::identification_descriptors(),
        BuiltinHarness::Codex => hookkit_codex::protocol::identification_descriptors(),
        BuiltinHarness::Antigravity => hookkit_antigravity::identification_descriptors(),
        _ => Vec::new(),
    }
}

fn capture_builtin_command_environment(
    harness: &BuiltinHarness,
    diagnostics: &dyn DiagnosticsSink,
) -> hookkit_core::Result<EnvironmentVariables> {
    use crate::environment::capture_command_environment_with_diagnostics as capture;
    match harness {
        BuiltinHarness::ClaudeCode => {
            capture::<hookkit_claude::ClaudeCommandEnvironment>(diagnostics)
        }
        BuiltinHarness::Codex => capture::<hookkit_codex::CodexCommandEnvironment>(diagnostics),
        BuiltinHarness::Antigravity => {
            capture::<hookkit_antigravity::AntigravityCommandEnvironment>(diagnostics)
        }
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
                assert_eq!(context.snapshot(), SnapshotId::builtin("commit-ff6aec9-r1"));
                assert_eq!(
                    context.event(),
                    &EventId::builtin(HarnessId::CODEX, "PreToolUse")
                );
                assert_eq!(
                    context.contract(),
                    ContractId::builtin("codex/commit-ff6aec9-r1/PreToolUse")
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
    fn selected_harnesses_dispatch_events_added_by_the_current_snapshots() {
        // Claude Code PreModelSwitch, added in docs-2026-09-29-r1.
        let pre_model_switch = serde_json::to_vec(&serde_json::json!({
            "session_id": "session-1",
            "transcript_path": "/tmp/claude-transcript.jsonl",
            "cwd": "/workspace",
            "hook_event_name": "PreModelSwitch",
            "from_model": "claude-sonnet-5",
            "to_model": "claude-opus-5",
            "requested_model": "opus",
            "source": "command",
            "context_tokens": 182340,
            "prompt_cache_warm": true,
            "cache_ttl": "5m",
            "estimated_cache_write_usd": 1.1396,
            "pricing": "catalog"
        }))
        .unwrap();
        let claude_variables = EnvironmentVariables::from_pairs([
            ("CLAUDECODE", "1"),
            ("CLAUDE_CODE_CHILD_SESSION", "1"),
            ("CLAUDE_CODE_SESSION_ID", "session-1"),
            ("CLAUDE_PROJECT_DIR", "/workspace"),
        ]);
        let emission = execute_harness::<hookkit_claude::protocol::ClaudeCode, _>(
            pre_model_switch,
            None,
            &claude_variables,
            |input, _environment, context| {
                assert_eq!(
                    context.event(),
                    &EventId::builtin(HarnessId::CLAUDE_CODE, "PreModelSwitch")
                );
                assert_eq!(
                    context.provenance(),
                    ResolutionProvenance::DefinitiveDiscriminator
                );
                let hookkit_claude::protocol::AnyInput::PreModelSwitch(input) = input else {
                    panic!("resolved the wrong Claude Code event")
                };
                assert_eq!(input.to_model, "claude-opus-5");
                Ok(hookkit_claude::protocol::AnyCommandOutput::PreModelSwitch(
                    hookkit_claude::events::PreModelSwitchOutput::block("Opus is not approved."),
                ))
            },
        )
        .unwrap();
        assert_eq!(emission.exit_code(), 0);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(emission.stdout()).unwrap(),
            serde_json::json!({"decision": "block", "reason": "Opus is not approved."})
        );

        // Claude Code PostModelSwitch, added in docs-2026-09-29-r1: an
        // automatic fallback, which has no requested model.
        let post_model_switch = serde_json::to_vec(&serde_json::json!({
            "session_id": "session-1",
            "transcript_path": "/tmp/claude-transcript.jsonl",
            "cwd": "/workspace",
            "hook_event_name": "PostModelSwitch",
            "from_model": "claude-opus-5",
            "to_model": "claude-sonnet-5",
            "requested_model": null,
            "source": "auto",
            "context_tokens": 0,
            "prompt_cache_warm": false,
            "cache_ttl": "5m",
            "estimated_cache_write_usd": 0,
            "pricing": "catalog"
        }))
        .unwrap();
        let emission = execute_harness::<hookkit_claude::protocol::ClaudeCode, _>(
            post_model_switch,
            None,
            &claude_variables,
            |input, _environment, context| {
                assert_eq!(
                    context.event(),
                    &EventId::builtin(HarnessId::CLAUDE_CODE, "PostModelSwitch")
                );
                assert_eq!(
                    context.provenance(),
                    ResolutionProvenance::DefinitiveDiscriminator
                );
                let hookkit_claude::protocol::AnyInput::PostModelSwitch(input) = input else {
                    panic!("resolved the wrong Claude Code event")
                };
                assert_eq!(input.to_model, "claude-sonnet-5");
                Ok(hookkit_claude::protocol::AnyCommandOutput::PostModelSwitch(
                    hookkit_claude::events::PostModelSwitchOutput::with_context(
                        "On Sonnet, keep diffs small.",
                    ),
                ))
            },
        )
        .unwrap();
        assert_eq!(emission.exit_code(), 0);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(emission.stdout()).unwrap(),
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PostModelSwitch",
                    "additionalContext": "On Sonnet, keep diffs small."
                }
            })
        );

        // Codex Interrupt, added in commit-ff6aec9-r1.
        let interrupt = serde_json::to_vec(&serde_json::json!({
            "session_id": "session-123",
            "transcript_path": null,
            "cwd": "/workspace",
            "hook_event_name": "Interrupt",
            "model": "gpt-test",
            "turn_id": "turn-456",
            "permission_mode": "default"
        }))
        .unwrap();
        let emission = execute_builtin_harness(
            BuiltinHarness::Codex,
            interrupt,
            None,
            &EnvironmentVariables::new(),
            |input, _environment, context| {
                assert_eq!(
                    context.event(),
                    &EventId::builtin(HarnessId::CODEX, "Interrupt")
                );
                assert_eq!(context.turn_id().unwrap().as_str(), "turn-456");
                assert!(matches!(input, BuiltinInput::Codex(_)));
                Ok(BuiltinOutput::Codex(
                    hookkit_codex::catalog::InterruptOutput::system_message("Turn interrupted.")
                        .into(),
                ))
            },
        )
        .unwrap();
        assert_eq!(emission.exit_code(), 0);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(emission.stdout()).unwrap(),
            serde_json::json!({"systemMessage": "Turn interrupted."})
        );
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
