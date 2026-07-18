//! Property tests for Codex's exact hook-output dialect.

use hookkit_codex::protocol::{PostToolUse, PostToolUseOutput, PreToolUse, PreToolUseOutput};
use hookkit_core::EventSpec;
use proptest::prelude::*;
use serde_json::{Map, Value};

proptest! {
    /// Property: optional deny reasons are omitted when absent and preserved
    /// verbatim when present; JSON `null` is not substituted for omission.
    #[test]
    fn deny_reason_presence_controls_wire_field(reason in proptest::option::of(any::<String>())) {
        let emission = PreToolUse::emit(PreToolUseOutput::deny(reason.clone())).unwrap();
        let value: Value = serde_json::from_slice(emission.stdout()).unwrap();
        let specific = &value["hookSpecificOutput"];

        prop_assert_eq!(specific["hookEventName"].as_str(), Some("PreToolUse"));
        prop_assert_eq!(specific["permissionDecision"].as_str(), Some("deny"));
        match reason {
            Some(reason) => prop_assert_eq!(specific["permissionDecisionReason"].as_str(), Some(reason.as_str())),
            None => prop_assert!(specific.get("permissionDecisionReason").is_none()),
        }
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

    /// Property: all successful PostToolUse fields compose into one object and
    /// retain their values, irrespective of builder call data.
    #[test]
    fn post_tool_use_builders_are_additive(
        context in any::<String>(),
        reason in any::<String>(),
        stop_reason in any::<String>(),
        continue_session in any::<bool>(),
        updated in any::<i64>(),
    ) {
        let output = PostToolUseOutput::with_context(context.clone())
            .with_updated_mcp_tool_output(Value::Number(updated.into())).unwrap()
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
        prop_assert_eq!(value["hookSpecificOutput"]["updatedMcpToolOutput"].as_i64(), Some(updated));
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
}
