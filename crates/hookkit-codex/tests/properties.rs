//! Property tests for Codex's exact hook-output dialect.

use hookkit_codex::catalog::{
    SessionStart, SessionStartOutput, SessionStartSource, Stop, StopOutput, SubagentStop,
    SubagentStopOutput, UserPromptSubmit, UserPromptSubmitOutput,
};
use hookkit_codex::protocol::{
    PermissionMode, PostToolUse, PostToolUseOutput, PreToolUse, PreToolUseOutput,
};
use hookkit_core::EventSpec;
use proptest::prelude::*;
use serde_json::{Map, Value};

/// Strings that Codex treats as a missing reason: empty after trimming.
fn blank() -> impl Strategy<Value = String> {
    "[ \t\r\n]{0,8}"
}

/// Strings that contain at least one non-whitespace character.
fn nonblank() -> impl Strategy<Value = String> {
    ".*\\S.*"
}

proptest! {
    /// Property: non-blank denial reasons are preserved verbatim.
    #[test]
    fn deny_reason_is_preserved(reason in nonblank()) {
        let emission = PreToolUse::emit(PreToolUseOutput::deny(reason.clone())).unwrap();
        let value: Value = serde_json::from_slice(emission.stdout()).unwrap();
        let specific = &value["hookSpecificOutput"];

        prop_assert_eq!(specific["hookEventName"].as_str(), Some("PreToolUse"));
        prop_assert_eq!(specific["permissionDecision"].as_str(), Some("deny"));
        prop_assert_eq!(specific["permissionDecisionReason"].as_str(), Some(reason.as_str()));
    }

    /// Property: rewrite output always grants the rewritten call and embeds
    /// the complete map without dropping unfamiliar keys.
    #[test]
    fn rewrite_preserves_arbitrary_input_entries(
        entries in prop::collection::btree_map("x_[a-z]{1,8}", any::<i64>(), 0..20),
    ) {
        let expected: Map<String, Value> = entries
            .into_iter()
            .map(|(key, value)| (key, Value::Number(value.into())))
            .collect();
        let emission = PreToolUse::emit(PreToolUseOutput::rewrite(expected.clone())).unwrap();
        let value: Value = serde_json::from_slice(emission.stdout()).unwrap();

        prop_assert_eq!(value["hookSpecificOutput"]["permissionDecision"].as_str(), Some("allow"));
        prop_assert_eq!(value["hookSpecificOutput"]["updatedInput"].as_object(), Some(&expected));
    }

    /// Property: user- and agent-facing messages compose with any PreToolUse
    /// decision without changing the decision.
    #[test]
    fn pre_tool_use_messages_compose_with_decisions(
        reason in nonblank(),
        context in any::<String>(),
        message in any::<String>(),
    ) {
        let emission = PreToolUse::emit(
            PreToolUseOutput::deny(reason.clone())
                .with_additional_context(context.clone()).unwrap()
                .with_system_message(message.clone()).unwrap(),
        ).unwrap();
        let value: Value = serde_json::from_slice(emission.stdout()).unwrap();

        prop_assert_eq!(value["hookSpecificOutput"]["permissionDecisionReason"].as_str(), Some(reason.as_str()));
        prop_assert_eq!(value["hookSpecificOutput"]["additionalContext"].as_str(), Some(context.as_str()));
        prop_assert_eq!(value["systemMessage"].as_str(), Some(message.as_str()));
    }

    /// Property: all successful PostToolUse fields compose into one object and
    /// retain their values, irrespective of builder call data.
    #[test]
    fn post_tool_use_builders_are_additive(
        context in any::<String>(),
        reason in nonblank(),
        stop_reason in any::<String>(),
        continue_session in any::<bool>(),
    ) {
        let output = PostToolUseOutput::with_context(context.clone())
            .with_block(reason.clone()).unwrap()
            .with_continue(continue_session).unwrap()
            .with_stop_reason(stop_reason.clone()).unwrap();
        let emission = PostToolUse::emit(output).unwrap();
        let value: Value = serde_json::from_slice(emission.stdout()).unwrap();

        prop_assert_eq!(value["decision"].as_str(), Some("block"));
        prop_assert_eq!(value["reason"].as_str(), Some(reason.as_str()));
        prop_assert_eq!(value["continue"].as_bool(), Some(continue_session));
        prop_assert_eq!(value["stopReason"].as_str(), Some(stop_reason.as_str()));
        prop_assert_eq!(value["hookSpecificOutput"]["additionalContext"].as_str(), Some(context.as_str()));
    }

    /// Property: Codex fails a block whose reason or exit-2 stderr is blank
    /// after trimming (the block fails open), so no such output is emitted.
    #[test]
    fn blank_block_reasons_are_never_emitted(reason in blank()) {
        prop_assert!(PreToolUse::emit(PreToolUseOutput::deny(reason.clone())).is_err());
        prop_assert!(PreToolUse::emit(PreToolUseOutput::block(reason.clone())).is_err());
        prop_assert!(PreToolUse::emit(PreToolUseOutput::deny_stderr(reason.clone())).is_err());
        prop_assert!(PostToolUse::emit(PostToolUseOutput::block(reason.clone())).is_err());
        prop_assert!(PostToolUse::emit(PostToolUseOutput::blocking_error(reason.clone())).is_err());
        prop_assert!(Stop::emit(StopOutput::block(reason.clone())).is_err());
        prop_assert!(Stop::emit(StopOutput::blocking_error(reason.clone())).is_err());
        prop_assert!(SubagentStop::emit(SubagentStopOutput::block(reason.clone())).is_err());
        prop_assert!(UserPromptSubmit::emit(UserPromptSubmitOutput::block(reason.clone())).is_err());
        prop_assert!(UserPromptSubmit::emit(UserPromptSubmitOutput::blocking_error(reason)).is_err());
    }

    /// Property: plain-text context is never emitted in a form Codex would
    /// parse as JSON; JSON-looking text is carried losslessly as structured
    /// additional context instead.
    #[test]
    fn text_context_never_looks_like_json(prefix in "[ \t\n]{0,3}", body in any::<String>(), bracket in any::<bool>()) {
        let text = format!("{prefix}{}{body}", if bracket { '[' } else { '{' });
        for (emission, event) in [
            (UserPromptSubmit::emit(UserPromptSubmitOutput::text_context(text.clone())).unwrap(), "UserPromptSubmit"),
            (SessionStart::emit(SessionStartOutput::text_context(text.clone())).unwrap(), "SessionStart"),
        ] {
            let value: Value = serde_json::from_slice(emission.stdout()).unwrap();
            prop_assert_eq!(value["hookSpecificOutput"]["hookEventName"].as_str(), Some(event));
            prop_assert_eq!(value["hookSpecificOutput"]["additionalContext"].as_str(), Some(text.as_str()));
        }
    }

    /// Property: any other text is emitted byte-for-byte as plain stdout.
    /// (Codex trims Unicode whitespace, so the first character is drawn from
    /// neither whitespace nor the JSON openers.)
    #[test]
    fn plain_text_context_is_verbatim(text in "[^\\s{\\[].*") {
        let emission = UserPromptSubmit::emit(UserPromptSubmitOutput::text_context(text.clone())).unwrap();
        prop_assert_eq!(emission.stdout(), text.as_bytes());
    }

    /// Property: Codex protocol stderr augments, but never mutates, successful
    /// JSON stdout.
    #[test]
    fn post_tool_protocol_stderr_is_orthogonal(context in any::<String>(), stderr in any::<String>()) {
        let plain = PostToolUse::emit(PostToolUseOutput::with_context(context.clone())).unwrap();
        let wrapped = PostToolUse::emit(
            PostToolUseOutput::with_context(context)
                .with_protocol_stderr(stderr.clone()).unwrap()
        ).unwrap();

        prop_assert_eq!(wrapped.stdout(), plain.stdout());
        prop_assert_eq!(wrapped.stderr(), stderr.as_bytes());
        prop_assert_eq!(wrapped.exit_code(), 0);
    }

    /// Property: harness-sent enum values round-trip whether or not the
    /// snapshot documents them.
    #[test]
    fn open_enums_round_trip_any_string(value in any::<String>()) {
        let json = Value::String(value.clone());
        let mode: PermissionMode = serde_json::from_value(json.clone()).unwrap();
        prop_assert_eq!(mode.as_str(), value.as_str());
        prop_assert_eq!(serde_json::to_value(&mode).unwrap(), json.clone());
        let source: SessionStartSource = serde_json::from_value(json.clone()).unwrap();
        prop_assert_eq!(source.as_str(), value.as_str());
        prop_assert_eq!(serde_json::to_value(&source).unwrap(), json);
    }
}
