use hookkit_core::{
    ContractId, DISABLED_DIAGNOSTICS, DiagnosticsSink, EventSpec, HandlerKind, ProcessEmission,
    RawInvocation, ResolutionProvenance, RuntimeContext,
};
use std::io::{Read, Write};

/// Parse and execute one exact event. Harness and event resolution are absent by
/// construction: `E` supplies both identities and its associated output type.
/// Returning another event's output does not type-check:
///
/// ```compile_fail
/// use hookkit_runtime::typed::execute_typed;
/// use hookkit_claude::protocol::{SessionStart, WorktreeCreateOutput};
/// let bytes = br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"SessionStart","source":"startup"}"#.to_vec();
/// execute_typed::<SessionStart, _>(bytes, |_, _| {
///     Ok(WorktreeCreateOutput::path("/tmp/w".into())?)
/// });
/// ```
pub fn execute_typed<E, F>(
    bytes: impl Into<Vec<u8>>,
    handler: F,
) -> hookkit_core::Result<ProcessEmission>
where
    E: EventSpec,
    F: FnOnce(E::Input, &RuntimeContext<'_>) -> hookkit_core::Result<E::CommandOutput>,
{
    execute_typed_with_diagnostics::<E, _>(bytes, &DISABLED_DIAGNOSTICS, handler)
}

/// Exact typed execution with an explicit out-of-band diagnostics sink.
pub fn execute_typed_with_diagnostics<E, F>(
    bytes: impl Into<Vec<u8>>,
    diagnostics: &dyn DiagnosticsSink,
    handler: F,
) -> hookkit_core::Result<ProcessEmission>
where
    E: EventSpec,
    F: FnOnce(E::Input, &RuntimeContext<'_>) -> hookkit_core::Result<E::CommandOutput>,
{
    let invocation = RawInvocation::parse(bytes)?;
    let input = E::parse(&invocation)?;
    let context = RuntimeContext::new(
        E::HARNESS,
        E::SNAPSHOT,
        E::EVENT,
        E::CONTRACT,
        ResolutionProvenance::TypedStatic,
        &invocation,
        E::context(&input),
        diagnostics,
    )?;
    validate_command_emission(E::emit(handler(input, &context)?)?, E::CONTRACT)
}

/// Stdin/stdout adapter for one exact command event.
pub fn run_event<E, F>(handler: F) -> std::process::ExitCode
where
    E: EventSpec,
    F: FnOnce(E::Input, &RuntimeContext<'_>) -> hookkit_core::Result<E::CommandOutput>,
{
    run_event_with_diagnostics::<E, _>(&DISABLED_DIAGNOSTICS, handler)
}

/// Stdin/stdout adapter with a configured out-of-band diagnostics sink.
pub fn run_event_with_diagnostics<E, F>(
    diagnostics: &dyn DiagnosticsSink,
    handler: F,
) -> std::process::ExitCode
where
    E: EventSpec,
    F: FnOnce(E::Input, &RuntimeContext<'_>) -> hookkit_core::Result<E::CommandOutput>,
{
    let mut bytes = Vec::new();
    if std::io::stdin().read_to_end(&mut bytes).is_err() {
        return std::process::ExitCode::from(1);
    }
    match execute_typed_with_diagnostics::<E, _>(bytes, diagnostics, handler) {
        Ok(emission) => write_emission(&emission),
        Err(_) => std::process::ExitCode::from(1),
    }
}

/// Backward-compatible spelling for the contract-safe typed runner.
pub fn run_typed<E, F>(handler: F) -> std::process::ExitCode
where
    E: EventSpec,
    F: FnOnce(E::Input, &RuntimeContext<'_>) -> hookkit_core::Result<E::CommandOutput>,
{
    run_event::<E, _>(handler)
}

pub(crate) fn write_emission(emission: &ProcessEmission) -> std::process::ExitCode {
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    if stdout.write_all(emission.stdout()).is_err()
        || stderr.write_all(emission.stderr()).is_err()
        || stdout.flush().is_err()
        || stderr.flush().is_err()
    {
        return std::process::ExitCode::from(1);
    }
    std::process::ExitCode::from(emission.exit_code())
}

pub(crate) fn validate_command_emission(
    emission: ProcessEmission,
    expected_contract: ContractId,
) -> hookkit_core::Result<ProcessEmission> {
    if emission.contract() != expected_contract || emission.binding() != HandlerKind::Command {
        return Err(hookkit_core::HookkitError::InvalidProcessEmission(
            "event encoder returned the wrong contract or binding identity",
        ));
    }
    Ok(emission)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hookkit_core::{
        ContractId, EventCategory, EventId, HarnessId, HookkitError, NativeContext, SnapshotId,
    };

    struct Echo;
    impl EventSpec for Echo {
        type Input = serde_json::Value;
        type CommandOutput = serde_json::Value;
        const HARNESS: HarnessId = HarnessId::builtin("test");
        const SNAPSHOT: SnapshotId = SnapshotId::builtin("v1");
        const EVENT: EventId = EventId::builtin(HarnessId::builtin("test"), "Echo");
        const CATEGORY: EventCategory = EventCategory::Other;
        const CONTRACT: ContractId = ContractId::builtin("test/v1/Echo");
        fn parse(input: &RawInvocation) -> hookkit_core::Result<Self::Input> {
            Ok(input.json().clone())
        }
        fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
            ProcessEmission::command_json(Self::CONTRACT, &output)
        }
        fn context(_input: &Self::Input) -> NativeContext {
            NativeContext {
                workspace_roots: vec!["/repo".into()],
                ..NativeContext::default()
            }
        }
    }

    struct WrongContract;
    impl EventSpec for WrongContract {
        type Input = ();
        type CommandOutput = ();
        const HARNESS: HarnessId = HarnessId::builtin("test");
        const SNAPSHOT: SnapshotId = SnapshotId::builtin("v1");
        const EVENT: EventId = EventId::builtin(HarnessId::builtin("test"), "WrongContract");
        const CATEGORY: EventCategory = EventCategory::Other;
        const CONTRACT: ContractId = ContractId::builtin("test/v1/WrongContract");
        fn parse(_input: &RawInvocation) -> hookkit_core::Result<Self::Input> {
            Ok(())
        }
        fn emit(_: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
            Ok(ProcessEmission::command_empty(ContractId::builtin(
                "test/v1/Other",
            )))
        }
    }

    #[test]
    fn exact_json_emission_has_no_implicit_newline_and_retains_contract() {
        let emission = execute_typed::<Echo, _>(br#"{"ok":true}"#.to_vec(), |value, ctx| {
            assert_eq!(ctx.harness().as_str(), "test");
            assert_eq!(ctx.event().name(), "Echo");
            assert_eq!(
                ctx.workspace_roots(),
                &[hookkit_core::Utf8PathBuf::from("/repo")]
            );
            Ok(value)
        })
        .unwrap();
        assert_eq!(emission.stdout(), br#"{"ok":true}"#);
        assert_eq!(emission.contract(), Echo::CONTRACT);
    }

    #[test]
    fn handler_errors_do_not_become_protocol_decisions() {
        let error = execute_typed::<Echo, _>(br#"{}"#.to_vec(), |_, _| {
            Err(HookkitError::InvalidIdentity("handler failed"))
        })
        .unwrap_err();
        assert!(matches!(error, HookkitError::InvalidIdentity(_)));
    }

    #[test]
    fn event_encoder_cannot_substitute_another_contract_identity() {
        let error =
            execute_typed::<WrongContract, _>(br#"{}"#.to_vec(), |_, _| Ok(())).unwrap_err();
        assert!(matches!(error, HookkitError::InvalidProcessEmission(_)));
    }
}
