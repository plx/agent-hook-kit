use hookkit_core::{
    CommandEnvironmentSpec, ContractId, DISABLED_DIAGNOSTICS, DiagnosticsSink,
    EnvironmentVariables, EventSpec, HandlerKind, ProcessEmission, RawInvocation,
    ResolutionProvenance, RuntimeContext,
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
/// execute_typed::<SessionStart, _>(bytes, &Default::default(), |_, _, _| {
///     Ok(WorktreeCreateOutput::path("/tmp/w".into())?)
/// });
/// ```
pub fn execute_typed<E, F>(
    bytes: impl Into<Vec<u8>>,
    variables: &EnvironmentVariables,
    handler: F,
) -> hookkit_core::Result<ProcessEmission>
where
    E: EventSpec,
    F: FnOnce(
        E::Input,
        &E::CommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<E::CommandOutput>,
{
    execute_typed_with_diagnostics::<E, _>(bytes, variables, &DISABLED_DIAGNOSTICS, handler)
}

/// Exact typed execution with an explicit out-of-band diagnostics sink.
pub fn execute_typed_with_diagnostics<E, F>(
    bytes: impl Into<Vec<u8>>,
    variables: &EnvironmentVariables,
    diagnostics: &dyn DiagnosticsSink,
    handler: F,
) -> hookkit_core::Result<ProcessEmission>
where
    E: EventSpec,
    F: FnOnce(
        E::Input,
        &E::CommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<E::CommandOutput>,
{
    let invocation = RawInvocation::parse(bytes)?;
    let input = E::parse(&invocation)?;
    let environment =
        <E::CommandEnvironment as CommandEnvironmentSpec>::from_variables(&E::EVENT, variables)?;
    E::validate_command_environment(&input, &environment)?;
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
    validate_command_emission(
        E::emit(handler(input, &environment, &context)?)?,
        E::CONTRACT,
    )
}

/// Stdin/stdout adapter for one exact command event.
pub fn run_event<E, F>(handler: F) -> std::process::ExitCode
where
    E: EventSpec,
    F: FnOnce(
        E::Input,
        &E::CommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<E::CommandOutput>,
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
    F: FnOnce(
        E::Input,
        &E::CommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<E::CommandOutput>,
{
    let mut bytes = Vec::new();
    if let Err(error) = std::io::stdin().read_to_end(&mut bytes) {
        return crate::report::report_io_failure(E::EVENT, error);
    }
    let variables = match crate::environment::capture_command_environment::<E::CommandEnvironment>()
    {
        Ok(variables) => variables,
        Err(error) => return crate::report::report_failure(E::EVENT, &error),
    };
    match execute_typed_with_diagnostics::<E, _>(bytes, &variables, diagnostics, handler) {
        Ok(emission) => write_emission(&emission),
        Err(error) => crate::report::report_failure(E::EVENT, &error),
    }
}

/// Backward-compatible spelling for the contract-safe typed runner.
pub fn run_typed<E, F>(handler: F) -> std::process::ExitCode
where
    E: EventSpec,
    F: FnOnce(
        E::Input,
        &E::CommandEnvironment,
        &RuntimeContext<'_>,
    ) -> hookkit_core::Result<E::CommandOutput>,
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
        ContractId, EventCategory, EventId, HarnessId, HookkitError, NativeContext,
        NoCommandEnvironment, SnapshotId,
    };
    use std::cell::Cell;

    struct Echo;
    impl EventSpec for Echo {
        type Input = serde_json::Value;
        type CommandEnvironment = NoCommandEnvironment;
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
        type CommandEnvironment = NoCommandEnvironment;
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
        let variables = EnvironmentVariables::new();
        let emission = execute_typed::<Echo, _>(
            br#"{"ok":true}"#.to_vec(),
            &variables,
            |value, _environment, ctx| {
                assert_eq!(ctx.harness().as_str(), "test");
                assert_eq!(ctx.event().name(), "Echo");
                assert_eq!(
                    ctx.workspace_roots(),
                    &[hookkit_core::Utf8PathBuf::from("/repo")]
                );
                Ok(value)
            },
        )
        .unwrap();
        assert_eq!(emission.stdout(), br#"{"ok":true}"#);
        assert_eq!(emission.contract(), Echo::CONTRACT);
    }

    #[test]
    fn handler_errors_do_not_become_protocol_decisions() {
        let error = execute_typed::<Echo, _>(
            br#"{}"#.to_vec(),
            &EnvironmentVariables::new(),
            |_, _, _| Err(HookkitError::InvalidIdentity("handler failed")),
        )
        .unwrap_err();
        assert!(matches!(error, HookkitError::InvalidIdentity(_)));
    }

    #[test]
    fn event_encoder_cannot_substitute_another_contract_identity() {
        let error = execute_typed::<WrongContract, _>(
            br#"{}"#.to_vec(),
            &EnvironmentVariables::new(),
            |_, _, _| Ok(()),
        )
        .unwrap_err();
        assert!(matches!(error, HookkitError::InvalidProcessEmission(_)));
    }

    #[test]
    fn invalid_command_environment_fails_before_the_handler_runs() {
        let called = Cell::new(false);
        let error = execute_typed::<hookkit_claude::protocol::SessionStart, _>(
            br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"SessionStart","source":"startup"}"#
                .to_vec(),
            &EnvironmentVariables::new(),
            |_, _, _| {
                called.set(true);
                Ok(hookkit_claude::protocol::SessionStartOutput::no_op())
            },
        )
        .unwrap_err();

        assert!(!called.get());
        assert!(matches!(error, HookkitError::InvalidHookEnvironment { .. }));
    }
}
