//! Stderr diagnostics for the stdin/stdout runtime adapters.
//!
//! [`crate::aligned::run_aligned_event`], [`crate::typed::run_event_with_diagnostics`]
//! (and therefore [`crate::typed::run_typed`]), [`crate::selected::run_harness`], and
//! [`crate::selected::dispatch_builtin_harness`] each read stdin, resolve the hook
//! environment, and execute a handler, folding every failure along the way into a
//! bare `exit 1`. Hook protocols require a clean, machine-parseable stdout on every
//! exit path, so these adapters must never write anything else there — but stderr
//! carries no such constraint, and writing nothing to it turned every misconfigured
//! environment variable, malformed payload, or handler bug into an unexplained,
//! byte-for-byte silent failure. This module gives every one of those adapters a
//! single, concise stderr line identifying the running program and the hook it was
//! invoked for, plus the failing error's full cause chain, before they return that
//! same `exit 1`.

use std::fmt::Display;
use std::io::Write;

/// Renders `error` and its full [`std::error::Error::source`] chain, most specific
/// cause last, the way `anyhow`'s alternate (`{:#}`) `Display` does.
///
/// Some [`hookkit_core::HookkitError`] variants (for example `Io` and `InvalidJson`)
/// already interpolate their wrapped source's message into their own `Display`
/// output via `#[from]`, so a naive walk of `.source()` would print that message
/// twice. This skips a cause whose rendering is already a suffix of what has been
/// accumulated so far.
pub(crate) fn error_chain(error: &dyn std::error::Error) -> String {
    let mut rendered = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !cause_text.is_empty() && !rendered.ends_with(cause_text.as_str()) {
            rendered.push_str(": ");
            rendered.push_str(&cause_text);
        }
        source = cause.source();
    }
    rendered
}

/// Best-effort file-name portion of the running program's `argv[0]`, falling back
/// to `"hookkit"` when it is unavailable or not valid Unicode.
fn program_name() -> String {
    std::env::args()
        .next()
        .as_deref()
        .map(std::path::Path::new)
        .and_then(std::path::Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "hookkit".to_string())
}

/// Writes the one-block failure diagnostic (program identity, hook identity, and
/// the error's full cause chain) to `sink`. Split out from [`report_failure`] so
/// tests can assert on the rendered bytes without touching real process stderr.
pub(crate) fn write_failure_diagnostic(
    sink: &mut dyn Write,
    hook: impl Display,
    error: &dyn std::error::Error,
) -> std::io::Result<()> {
    writeln!(
        sink,
        "hookkit: {program} {hook} failed: {chain}",
        program = program_name(),
        chain = error_chain(error),
    )
}

/// Writes the failure diagnostic to real stderr, then returns the `exit 1` the
/// caller was already going to return. Never touches stdout.
pub(crate) fn report_failure(
    hook: impl Display,
    error: &dyn std::error::Error,
) -> std::process::ExitCode {
    // Best effort: if stderr itself is broken there is nowhere left to report to,
    // but the process must still exit non-zero.
    let _ = write_failure_diagnostic(&mut std::io::stderr(), hook, error);
    std::process::ExitCode::from(1)
}

/// Same as [`report_failure`] for a bare [`std::io::Error`] (for example, reading
/// stdin) that never became a [`hookkit_core::HookkitError`].
pub(crate) fn report_io_failure(
    hook: impl Display,
    error: std::io::Error,
) -> std::process::ExitCode {
    report_failure(hook, &error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Leaf;
    impl std::fmt::Display for Leaf {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "leaf cause")
        }
    }
    impl std::error::Error for Leaf {}

    #[derive(Debug)]
    struct Wrapper(Leaf);
    impl std::fmt::Display for Wrapper {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "wrapper: {}", self.0)
        }
    }
    impl std::error::Error for Wrapper {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    #[derive(Debug)]
    struct Enriching(Leaf);
    impl std::fmt::Display for Enriching {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "enriching context")
        }
    }
    impl std::error::Error for Enriching {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    #[test]
    fn chain_does_not_duplicate_a_message_already_interpolated_by_display() {
        let wrapper = Wrapper(Leaf);
        assert_eq!(error_chain(&wrapper), "wrapper: leaf cause");
    }

    #[test]
    fn chain_appends_a_cause_whose_message_display_did_not_already_include() {
        let enriching = Enriching(Leaf);
        assert_eq!(error_chain(&enriching), "enriching context: leaf cause");
    }

    #[test]
    fn real_hookkit_error_from_conversions_render_without_duplication() {
        let json_error = serde_json::from_str::<serde_json::Value>("not json").unwrap_err();
        let expected_leaf = json_error.to_string();
        let error = hookkit_core::HookkitError::from(json_error);
        let rendered = error_chain(&error);
        assert_eq!(rendered, format!("invalid JSON: {expected_leaf}"));
    }

    #[test]
    fn write_failure_diagnostic_names_the_program_and_hook_and_carries_the_chain() {
        let error = Enriching(Leaf);
        let mut buffer = Vec::new();
        write_failure_diagnostic(&mut buffer, "codex/PreToolUse", &error).unwrap();
        let rendered = String::from_utf8(buffer).unwrap();
        assert!(rendered.starts_with("hookkit: "));
        assert!(rendered.contains("codex/PreToolUse"));
        assert!(rendered.contains("failed: enriching context: leaf cause"));
    }
}
