use hookkit_core::{CommandEnvironmentSpec, EnvironmentVariables};

/// Capture only the exact names and dynamic prefixes declared by `E`.
///
/// This is the ambient-environment convenience used by stdin/stdout runners.
/// In-memory `execute_*` APIs accept an injected [`EnvironmentVariables`] map
/// instead, which keeps tests deterministic and avoids process-global mutation.
pub fn capture_command_environment<E: CommandEnvironmentSpec>()
-> hookkit_core::Result<EnvironmentVariables> {
    let mut captured = EnvironmentVariables::new();

    for name in E::VARIABLE_NAMES {
        let Some(value) = std::env::var_os(name) else {
            continue;
        };
        let value = value.into_string().map_err(|_| {
            hookkit_core::HookkitError::NonUnicodeEnvironmentVariable {
                variable: (*name).to_string(),
            }
        })?;
        captured.insert(*name, value);
    }

    if !E::VARIABLE_PREFIXES.is_empty() {
        for (name, value) in std::env::vars_os() {
            let Some(name) = name.to_str() else {
                continue;
            };
            if captured.contains_key(name)
                || !E::VARIABLE_PREFIXES
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
            {
                continue;
            }
            let value = value.into_string().map_err(|_| {
                hookkit_core::HookkitError::NonUnicodeEnvironmentVariable {
                    variable: name.to_string(),
                }
            })?;
            captured.insert(name, value);
        }
    }

    Ok(captured)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hookkit_core::{EventId, HarnessId};

    struct Nothing;

    impl CommandEnvironmentSpec for Nothing {
        const VARIABLE_NAMES: &'static [&'static str] = &[];

        fn from_variables(
            _event: &EventId,
            _variables: &EnvironmentVariables,
        ) -> hookkit_core::Result<Self> {
            Ok(Self)
        }
    }

    #[test]
    fn empty_spec_captures_no_ambient_values() {
        let captured = capture_command_environment::<Nothing>().unwrap();
        assert!(captured.is_empty());
        let _ = Nothing::from_variables(
            &EventId::builtin(HarnessId::builtin("test"), "Event"),
            &captured,
        )
        .unwrap();
    }
}
