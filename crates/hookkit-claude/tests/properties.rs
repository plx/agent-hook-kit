//! Property tests for Claude Code's native output contract.
//!
//! Builder composition is intentionally exercised with arbitrary text. These
//! properties document which bytes are opaque protocol text and which fields
//! are placed in structured JSON, including the fixed event discriminator.

use hookkit_claude::events::{
    UserPromptExpansion, UserPromptExpansionOutput, UserPromptSubmit, UserPromptSubmitOutput,
};
use hookkit_claude::protocol::{PostToolUse, PostToolUseOutput, SessionStart, SessionStartOutput};
use hookkit_core::EventSpec;
use proptest::prelude::*;
use serde_json::Value;

/// Code points ECMAScript's `String.prototype.trim` removes: the WhiteSpace
/// and LineTerminator productions (TAB, VT, FF, ZWNBSP, every `Zs` space
/// separator, LF, CR, LS, PS).
///
/// Written out from the ECMAScript grammar rather than derived from Rust's
/// `char::is_whitespace`, so the properties below check the crate against
/// Claude Code's rule instead of against its own predicate.
fn js_trims(c: char) -> bool {
    matches!(
        c,
        '\u{9}'
            | '\u{b}'
            | '\u{c}'
            | '\u{feff}'
            // Zs: SPACE, NO-BREAK SPACE, OGHAM SPACE MARK, EN QUAD through
            // HAIR SPACE, NARROW NO-BREAK SPACE, MEDIUM MATHEMATICAL SPACE,
            // IDEOGRAPHIC SPACE.
            | ' '
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'
            ..='\u{200a}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\n'
                | '\r'
                | '\u{2028}'
                | '\u{2029}'
    )
}

/// Claude Code (v2.1.248 or later) parses stdout as JSON when its text,
/// trimmed as JavaScript trims it, starts with `{` and ends with `}`.
///
/// Claude Code reads such text as plain text only when it is several lines
/// that each parse as JSON and none sets an output field. Structured context
/// reaches Claude in that case too, so the properties require the structured
/// form for all brace-delimited text.
fn claude_parses_as_json(text: &str) -> bool {
    let trimmed = text.trim_matches(js_trims);
    trimmed.starts_with('{') && trimmed.ends_with('}')
}

/// Whether `text` is brace-delimited only after also trimming code points
/// JavaScript keeps, such as NEXT LINE (U+0085). Routing such text to
/// structured context is safe, because the context still reaches Claude, so
/// either form is acceptable for it.
fn brace_delimited_after_wider_trim(text: &str) -> bool {
    let trimmed = text.trim_matches(|c: char| c.is_whitespace() || js_trims(c));
    trimmed.starts_with('{') && trimmed.ends_with('}')
}

/// Code points around a brace-delimited body: JavaScript trims some of them,
/// keeps others, and U+0085 is whitespace to Rust but not to JavaScript.
fn edge_char() -> impl Strategy<Value = char> {
    prop::sample::select(vec![
        ' ', '\t', '\n', '\r', '\u{b}', '\u{c}', '\u{a0}', '\u{feff}', '\u{1680}', '\u{2003}',
        '\u{2028}', '\u{2029}', '\u{202f}', '\u{3000}', '\u{85}', '\u{180e}', '\u{200b}', 'x',
    ])
}

fn text_context_input() -> impl Strategy<Value = String> {
    prop_oneof![
        any::<String>(),
        any::<String>().prop_map(|inner| format!(" {{{inner}}}\n")),
        (
            prop::collection::vec(edge_char(), 0..3),
            any::<String>(),
            prop::collection::vec(edge_char(), 0..3),
        )
            .prop_map(|(before, inner, after)| {
                let before: String = before.into_iter().collect();
                let after: String = after.into_iter().collect();
                format!("{before}{{{inner}}}{after}")
            }),
    ]
}

/// Asserts the text-context routing rule for one emission of `text`.
fn assert_text_context_routing(
    stdout: &[u8],
    text: &str,
    event: &str,
) -> Result<(), TestCaseError> {
    let structured = serde_json::from_slice::<Value>(stdout)
        .ok()
        .filter(|value| {
            value["hookSpecificOutput"]["hookEventName"].as_str() == Some(event)
                && value["hookSpecificOutput"]["additionalContext"].as_str() == Some(text)
        });
    if claude_parses_as_json(text) {
        prop_assert!(
            structured.is_some(),
            "{text:?} would be parsed as JSON by Claude Code but was written verbatim"
        );
    } else if brace_delimited_after_wider_trim(text) {
        prop_assert!(structured.is_some() || stdout == text.as_bytes());
    } else {
        prop_assert_eq!(stdout, text.as_bytes());
    }
    Ok(())
}

#[test]
fn text_context_routing_follows_javascript_trim_at_the_edges() {
    // JavaScript trims these, so Claude Code parses the text as JSON.
    for text in [
        "\u{2028}{\"a\":1}\u{2029}",
        "\u{3000}{}\u{3000}",
        "\u{feff}{ a }",
        "\u{a0}{}\u{1680}",
        "\u{b}{}\u{c}",
    ] {
        assert!(claude_parses_as_json(text), "{text:?}");
        let emission = SessionStart::emit(SessionStartOutput::text_context(text)).unwrap();
        let value: Value = serde_json::from_slice(emission.stdout()).unwrap();
        assert_eq!(value["hookSpecificOutput"]["additionalContext"], text);
    }
    // JavaScript keeps these, so Claude Code reads the text as plain text.
    for text in ["\u{200b}{}", "{}\u{180e}", "x{}"] {
        assert!(!claude_parses_as_json(text), "{text:?}");
        let emission = SessionStart::emit(SessionStartOutput::text_context(text)).unwrap();
        assert_eq!(emission.stdout(), text.as_bytes());
    }
    // NEXT LINE is whitespace to Rust but not to JavaScript; either routing
    // delivers the context.
    let text = "\u{85}{}";
    assert!(!claude_parses_as_json(text));
    let emission = SessionStart::emit(SessionStartOutput::text_context(text)).unwrap();
    assert_text_context_routing(emission.stdout(), text, "SessionStart").unwrap();
}

proptest! {
    /// Property: the text SessionStart variant is an opaque successful stdout
    /// payload. It is neither JSON-quoted nor newline-normalized, except that
    /// text Claude Code would parse as JSON is delivered as structured
    /// `additionalContext` so it is never dropped.
    #[test]
    fn session_start_text_is_byte_exact_or_structured(text in text_context_input()) {
        let emission = SessionStart::emit(SessionStartOutput::text_context(text.clone())).unwrap();
        assert_text_context_routing(emission.stdout(), &text, "SessionStart")?;
        prop_assert!(emission.stderr().is_empty());
        prop_assert_eq!(emission.exit_code(), 0);
    }

    /// Property: prompt events route text context by the same rule.
    #[test]
    fn prompt_text_context_is_byte_exact_or_structured(text in text_context_input()) {
        let emission =
            UserPromptSubmit::emit(UserPromptSubmitOutput::text_context(text.clone())).unwrap();
        assert_text_context_routing(emission.stdout(), &text, "UserPromptSubmit")?;
        let emission = UserPromptExpansion::emit(
            UserPromptExpansionOutput::text_context(text.clone()),
        )
        .unwrap();
        assert_text_context_routing(emission.stdout(), &text, "UserPromptExpansion")?;
    }

    /// Property: structured SessionStart context always carries Claude's exact
    /// `SessionStart` discriminator while preserving both arbitrary messages.
    #[test]
    fn session_start_structured_context_has_fixed_identity(
        context in any::<String>(),
        system_message in any::<String>(),
    ) {
        let output = SessionStartOutput::with_context_and_system_message(
            context.clone(),
            system_message.clone(),
        );
        let emission = SessionStart::emit(output).unwrap();
        let value: Value = serde_json::from_slice(emission.stdout()).unwrap();

        prop_assert_eq!(value["hookSpecificOutput"]["hookEventName"].as_str(), Some("SessionStart"));
        prop_assert_eq!(value["hookSpecificOutput"]["additionalContext"].as_str(), Some(context.as_str()));
        prop_assert_eq!(value["systemMessage"].as_str(), Some(system_message.as_str()));
    }

    /// Property: PostToolUse builder methods compose without losing fields;
    /// arbitrary JSON tool output remains a JSON value rather than text.
    #[test]
    fn post_tool_use_builders_are_additive(
        context in any::<String>(),
        reason in any::<String>(),
        message in any::<String>(),
        updated in any::<i64>(),
        continue_session in any::<bool>(),
        classifier in any::<String>(),
    ) {
        let output = PostToolUseOutput::with_context(context.clone())
            .with_updated_tool_output(Value::Number(updated.into())).unwrap()
            .with_block(reason.clone()).unwrap()
            .with_continue(continue_session).unwrap()
            .with_classifier_context(classifier.clone()).unwrap()
            .with_system_message(message.clone()).unwrap();
        let emission = PostToolUse::emit(output).unwrap();
        let value: Value = serde_json::from_slice(emission.stdout()).unwrap();

        prop_assert_eq!(value["decision"].as_str(), Some("block"));
        prop_assert_eq!(value["reason"].as_str(), Some(reason.as_str()));
        prop_assert_eq!(value["continue"].as_bool(), Some(continue_session));
        prop_assert_eq!(value["hookSpecificOutput"]["classifierContext"].as_str(), Some(classifier.as_str()));
        prop_assert_eq!(value["systemMessage"].as_str(), Some(message.as_str()));
        prop_assert_eq!(value["hookSpecificOutput"]["hookEventName"].as_str(), Some("PostToolUse"));
        prop_assert_eq!(value["hookSpecificOutput"]["additionalContext"].as_str(), Some(context.as_str()));
        prop_assert_eq!(value["hookSpecificOutput"]["updatedToolOutput"].as_i64(), Some(updated));
    }

    /// Property: protocol stderr is an independent successful channel. Adding
    /// it cannot change the JSON stdout generated by the wrapped output.
    #[test]
    fn post_tool_protocol_stderr_preserves_successful_stdout(
        context in any::<String>(),
        stderr in any::<String>(),
    ) {
        let plain = PostToolUse::emit(PostToolUseOutput::with_context(context.clone())).unwrap();
        let wrapped = PostToolUse::emit(
            PostToolUseOutput::with_context(context)
                .with_protocol_stderr(stderr.clone()).unwrap()
        ).unwrap();

        prop_assert_eq!(wrapped.stdout(), plain.stdout());
        prop_assert_eq!(wrapped.stderr(), stderr.as_bytes());
        prop_assert_eq!(wrapped.exit_code(), 0);
    }
}
