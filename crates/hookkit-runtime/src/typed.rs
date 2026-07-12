use hookkit_core::{EventSpec, ProcessEmission, RawInvocation};
use std::io::{Read, Write};

/// Parse and execute one exact event. Harness and event resolution are absent by
/// construction: `E` supplies both identities and its associated output type.
/// Returning another event's output does not type-check:
///
/// ```compile_fail
/// use hookkit_runtime::typed::execute_typed;
/// use hookkit_claude::protocol::{SessionStart, WorktreeCreateOutput};
/// let bytes = br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"SessionStart","source":"startup"}"#.to_vec();
/// execute_typed::<SessionStart, _>(bytes, |_| {
///     Ok(WorktreeCreateOutput::path("/tmp/w".into())?)
/// });
/// ```
pub fn execute_typed<E, F>(
    bytes: impl Into<Vec<u8>>,
    handler: F,
) -> hookkit_core::Result<ProcessEmission>
where
    E: EventSpec,
    F: FnOnce(E::Input) -> hookkit_core::Result<E::CommandOutput>,
{
    let invocation = RawInvocation::parse(bytes)?;
    let input = E::parse(&invocation)?;
    E::emit(handler(input)?)
}

/// Stdin/stdout adapter for one exact command event.
pub fn run_typed<E, F>(handler: F) -> std::process::ExitCode
where
    E: EventSpec,
    F: FnOnce(E::Input) -> hookkit_core::Result<E::CommandOutput>,
{
    let mut bytes = Vec::new();
    if let Err(error) = std::io::stdin().read_to_end(&mut bytes) {
        eprintln!("hookkit: failed to read stdin: {error}");
        return std::process::ExitCode::from(1);
    }
    let emission = match execute_typed::<E, _>(bytes, handler) {
        Ok(emission) => emission,
        Err(error) => {
            eprintln!("hookkit: typed hook failed: {error}");
            return std::process::ExitCode::from(1);
        }
    };
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

#[cfg(test)]
mod tests {
    use super::*;
    use hookkit_core::{EventCategory, EventId, HarnessId, HookkitError};

    struct Echo;
    impl EventSpec for Echo {
        type Input = serde_json::Value;
        type CommandOutput = serde_json::Value;
        const HARNESS: HarnessId = HarnessId::builtin("test");
        const EVENT: EventId = EventId::builtin("Echo");
        const CATEGORY: EventCategory = EventCategory::Other;
        const CONTRACT_ID: &'static str = "test/v1/Echo";
        fn parse(input: &RawInvocation) -> hookkit_core::Result<Self::Input> {
            Ok(input.json().clone())
        }
        fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
            ProcessEmission::success_json(&output)
        }
    }

    #[test]
    fn exact_json_emission_has_no_implicit_newline() {
        let emission = execute_typed::<Echo, _>(br#"{"ok":true}"#.to_vec(), Ok).unwrap();
        assert_eq!(emission.stdout(), br#"{"ok":true}"#);
    }

    #[test]
    fn handler_errors_do_not_become_protocol_decisions() {
        let error = execute_typed::<Echo, _>(br#"{}"#.to_vec(), |_| {
            Err(HookkitError::InvalidIdentity("handler failed"))
        })
        .unwrap_err();
        assert!(matches!(error, HookkitError::InvalidIdentity(_)));
    }
}
