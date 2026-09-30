//! Stderr diagnostics for the stdin/stdout runtime adapters.
//!
//! [`crate::aligned::run_aligned_event_with_options`],
//! [`crate::typed::run_event_with_options`] (and therefore their convenience
//! spellings such as [`crate::aligned::run_aligned_event`],
//! [`crate::typed::run_event`], and [`crate::typed::run_typed`]),
//! [`crate::selected::run_harness_with_options`], and
//! [`crate::selected::dispatch_builtin_harness_with_options`] each read stdin,
//! capture the hook environment, and execute a handler. Hook protocols require
//! clean, machine-parseable stdout, so a failure anywhere along the way never
//! writes a partial response there. Instead every adapter writes one concise
//! stderr line, `hookkit: <program> <hook> failed: <cause chain>`, naming the
//! running program, the hook it was invoked for, and the error's full cause
//! chain. By default the adapter then exits 1, which Claude Code and Codex treat
//! as a non-blocking hook error: the pending action proceeds. See
//! [`crate::failure`] for the fail-closed alternative.

use std::fmt::Display;

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
/// to `"hookkit"` when it is unavailable. A name that is not valid Unicode is
/// rendered lossily; it never panics.
fn program_name() -> String {
    program_name_from(std::env::args_os().next())
}

fn program_name_from(argv0: Option<std::ffi::OsString>) -> String {
    argv0
        .as_deref()
        .map(std::path::Path::new)
        .and_then(std::path::Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "hookkit".to_string())
}

/// Renders the one-line failure diagnostic, without a trailing newline.
pub(crate) fn failure_line(hook: impl Display, error: &dyn std::error::Error) -> String {
    format!(
        "hookkit: {program} {hook} failed: {chain}",
        program = program_name(),
        chain = error_chain(error),
    )
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

    #[cfg(unix)]
    #[test]
    fn non_unicode_program_names_render_lossily_instead_of_panicking() {
        use std::os::unix::ffi::OsStringExt;
        let argv0 = std::ffi::OsString::from_vec(b"/opt/hooks/bad\xff-guard".to_vec());
        assert_eq!(program_name_from(Some(argv0)), "bad\u{FFFD}-guard");
        assert_eq!(program_name_from(None), "hookkit");
    }

    #[test]
    fn failure_line_names_the_program_and_hook_and_carries_the_chain() {
        let error = Enriching(Leaf);
        let rendered = failure_line("codex/PreToolUse", &error);
        assert!(!rendered.contains('\n'));
        assert!(rendered.starts_with("hookkit: "));
        assert!(rendered.contains("codex/PreToolUse"));
        assert!(rendered.contains("failed: enriching context: leaf cause"));
    }
}
