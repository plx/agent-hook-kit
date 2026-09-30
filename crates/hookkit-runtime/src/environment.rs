use hookkit_core::{
    CommandEnvironmentSpec, DISABLED_DIAGNOSTICS, Diagnostic, DiagnosticLevel, DiagnosticsSink,
    EnvironmentVariables,
};

/// Capture only the exact names and dynamic prefixes declared by `E`.
///
/// This is the ambient-environment convenience used by stdin/stdout runners.
/// In-memory `execute_*` APIs accept an injected [`EnvironmentVariables`] map
/// instead, which keeps tests deterministic and avoids process-global mutation.
///
/// A declared exact name whose value is not valid UTF-8 is an error. A
/// variable discovered only through one of `E`'s prefixes, or named in
/// [`CommandEnvironmentSpec::LENIENT_VARIABLE_NAMES`], is skipped instead:
/// such names can match whatever the parent process happened to export, so a
/// stray non-UTF-8 value must not disable every hook. Use
/// [`capture_command_environment_with_diagnostics`] to learn about skipped
/// variables.
pub fn capture_command_environment<E: CommandEnvironmentSpec>()
-> hookkit_core::Result<EnvironmentVariables> {
    capture_command_environment_with_diagnostics::<E>(&DISABLED_DIAGNOSTICS)
}

/// [`capture_command_environment`] that records a warning in `diagnostics`
/// for each prefix-matched or lenient variable skipped because its value is
/// not UTF-8.
///
/// The warning names the variable but never includes its value.
pub fn capture_command_environment_with_diagnostics<E: CommandEnvironmentSpec>(
    diagnostics: &dyn DiagnosticsSink,
) -> hookkit_core::Result<EnvironmentVariables> {
    capture_from(
        Declared {
            names: E::VARIABLE_NAMES,
            lenient: E::LENIENT_VARIABLE_NAMES,
            prefixes: E::VARIABLE_PREFIXES,
        },
        |name| std::env::var_os(name),
        std::env::vars_os(),
        diagnostics,
    )
}

/// The variable names an environment contract declares.
struct Declared<'a> {
    names: &'a [&'a str],
    lenient: &'a [&'a str],
    prefixes: &'a [&'a str],
}

fn skipped_non_utf8(diagnostics: &dyn DiagnosticsSink, name: &str) {
    diagnostics.record(Diagnostic::new(
        DiagnosticLevel::Warning,
        format!(
            "hookkit: ignored environment variable `{name}` because its value is not valid UTF-8"
        ),
    ));
}

fn capture_from(
    declared: Declared<'_>,
    lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
    ambient: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
    diagnostics: &dyn DiagnosticsSink,
) -> hookkit_core::Result<EnvironmentVariables> {
    let Declared {
        names,
        lenient,
        prefixes,
    } = declared;
    let mut captured = EnvironmentVariables::new();

    for name in names {
        let Some(value) = lookup(name) else {
            continue;
        };
        if lenient.contains(name) {
            match value.into_string() {
                Ok(value) => {
                    captured.insert(*name, value);
                }
                Err(_) => skipped_non_utf8(diagnostics, name),
            }
            continue;
        }
        let value = value.into_string().map_err(|_| {
            hookkit_core::HookkitError::NonUnicodeEnvironmentVariable {
                variable: (*name).to_string(),
            }
        })?;
        captured.insert(*name, value);
    }

    if !prefixes.is_empty() {
        for (name, value) in ambient {
            let Some(name) = name.to_str() else {
                continue;
            };
            if captured.contains_key(name)
                || !prefixes.iter().any(|prefix| name.starts_with(prefix))
            {
                continue;
            }
            match value.into_string() {
                Ok(value) => {
                    captured.insert(name, value);
                }
                Err(_) => skipped_non_utf8(diagnostics, name),
            }
        }
    }

    Ok(captured)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hookkit_core::{EventId, HarnessId};
    use std::ffi::OsString;
    use std::sync::Mutex;

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

    #[derive(Default)]
    struct Recording(Mutex<Vec<Diagnostic>>);

    impl DiagnosticsSink for Recording {
        fn record(&self, diagnostic: Diagnostic) {
            self.0.lock().unwrap().push(diagnostic);
        }
    }

    #[cfg(unix)]
    fn non_utf8() -> OsString {
        use std::os::unix::ffi::OsStringExt;
        OsString::from_vec(b"secret\xff".to_vec())
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

    #[cfg(unix)]
    #[test]
    fn non_utf8_prefix_variables_are_skipped_with_a_warning() {
        let sink = Recording::default();
        let captured = capture_from(
            Declared {
                names: &["EXACT"],
                lenient: &[],
                prefixes: &["PREFIX_"],
            },
            |name| (name == "EXACT").then(|| OsString::from("ok")),
            [
                (OsString::from("PREFIX_GOOD"), OsString::from("value")),
                (OsString::from("PREFIX_BAD"), non_utf8()),
                (OsString::from("UNRELATED"), non_utf8()),
            ],
            &sink,
        )
        .unwrap();
        assert_eq!(captured.get("EXACT"), Some("ok"));
        assert_eq!(captured.get("PREFIX_GOOD"), Some("value"));
        assert!(!captured.contains_key("PREFIX_BAD"));
        let recorded = sink.0.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].level, DiagnosticLevel::Warning);
        assert!(recorded[0].message.contains("PREFIX_BAD"));
        assert!(!recorded[0].message.contains("secret"));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_exact_variables_remain_errors() {
        let error = capture_from(
            Declared {
                names: &["EXACT"],
                lenient: &[],
                prefixes: &[],
            },
            |_| Some(non_utf8()),
            [],
            &DISABLED_DIAGNOSTICS,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            hookkit_core::HookkitError::NonUnicodeEnvironmentVariable { variable } if variable == "EXACT"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_lenient_exact_variables_are_skipped_with_a_warning() {
        let sink = Recording::default();
        let captured = capture_from(
            Declared {
                names: &["PLUGIN_ROOT", "STRICT"],
                lenient: &["PLUGIN_ROOT"],
                prefixes: &[],
            },
            |name| match name {
                "PLUGIN_ROOT" => Some(non_utf8()),
                _ => Some(OsString::from("kept")),
            },
            [],
            &sink,
        )
        .unwrap();
        assert!(!captured.contains_key("PLUGIN_ROOT"));
        assert_eq!(captured.get("STRICT"), Some("kept"));
        let recorded = sink.0.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert!(recorded[0].message.contains("PLUGIN_ROOT"));
    }
}
