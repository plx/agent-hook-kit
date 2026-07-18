//! Property tests for Gemini CLI's native hook-output semantics.

use hookkit_core::EventSpec;
use hookkit_gemini::protocol::{AfterTool, AfterToolOutput, BeforeTool, BeforeToolOutput};
use proptest::prelude::*;
use serde_json::{Map, Value};

proptest! {
    /// Property: deny-and-rewrite is one atomic structured response: both the
    /// decision and every rewritten input field survive emission.
    #[test]
    fn before_tool_deny_and_rewrite_preserves_both_intents(
        reason in any::<String>(),
        entries in prop::collection::btree_map("x_[a-z]{1,8}", any::<i64>(), 0..20),
    ) {
        let expected: Map<String, Value> = entries
            .into_iter()
            .map(|(key, value)| (key, Value::Number(value.into())))
            .collect();
        let output = BeforeToolOutput::deny_and_rewrite(reason.clone(), expected.clone());
        let emission = BeforeTool::emit(output).unwrap();
        let value: Value = serde_json::from_slice(emission.stdout()).unwrap();

        prop_assert_eq!(value["decision"].as_str(), Some("deny"));
        prop_assert_eq!(value["reason"].as_str(), Some(reason.as_str()));
        prop_assert_eq!(value["hookSpecificOutput"]["hookEventName"].as_str(), Some("BeforeTool"));
        prop_assert_eq!(value["hookSpecificOutput"]["tool_input"].as_object(), Some(&expected));
    }

    /// Property: the tail-call request retains arbitrary names and arguments
    /// and remains associated with the exact `AfterTool` discriminator.
    #[test]
    fn after_tool_tail_call_is_lossless(
        context in any::<String>(),
        name in any::<String>(),
        entries in prop::collection::btree_map("x_[a-z]{1,8}", any::<i64>(), 0..20),
    ) {
        let args: Map<String, Value> = entries
            .into_iter()
            .map(|(key, value)| (key, Value::Number(value.into())))
            .collect();
        let output = AfterToolOutput::with_context(context.clone())
            .and_tail_tool_call(name.clone(), args.clone()).unwrap();
        let emission = AfterTool::emit(output).unwrap();
        let value: Value = serde_json::from_slice(emission.stdout()).unwrap();
        let specific = &value["hookSpecificOutput"];

        prop_assert_eq!(specific["hookEventName"].as_str(), Some("AfterTool"));
        prop_assert_eq!(specific["additionalContext"].as_str(), Some(context.as_str()));
        prop_assert_eq!(specific["tailToolCallRequest"]["name"].as_str(), Some(name.as_str()));
        prop_assert_eq!(specific["tailToolCallRequest"]["args"].as_object(), Some(&args));
    }

    /// Property: protocol stderr is transported alongside the same successful
    /// stdout rather than changing the structured result.
    #[test]
    fn after_tool_protocol_stderr_preserves_stdout(context in any::<String>(), stderr in any::<String>()) {
        let plain = AfterTool::emit(AfterToolOutput::with_context(context.clone())).unwrap();
        let wrapped = AfterTool::emit(
            AfterToolOutput::with_context(context)
                .with_protocol_stderr(stderr.clone()).unwrap()
        ).unwrap();

        prop_assert_eq!(wrapped.stdout(), plain.stdout());
        prop_assert_eq!(wrapped.stderr(), stderr.as_bytes());
        prop_assert_eq!(wrapped.exit_code(), 0);
    }
}
