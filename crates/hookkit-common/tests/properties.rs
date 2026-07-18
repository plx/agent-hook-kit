//! Properties for the small common message vocabulary.
//!
//! Common helpers sit between hook logic and harness-specific lowering. These
//! tests document that they are deliberately boring value builders: arbitrary
//! user text and paths pass through unchanged, and only the documented framing
//! (such as newline joining) is introduced.

use hookkit_common::message::{AgentContext, TailToolCall};
use hookkit_common::{DiagnosticArtifact, NoticeLevel, UserNotice};
use proptest::prelude::*;
use serde_json::Value;
use std::path::PathBuf;

fn scalar_json() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|value| Value::Number(value.into())),
        any::<String>().prop_map(Value::String),
    ]
}

proptest! {
    /// Property: one pushed line becomes one line in the same position. Joining
    /// is the sole transformation, so callers can predict the exact agent text.
    #[test]
    fn agent_context_is_an_ordered_newline_join(lines in prop::collection::vec(any::<String>(), 0..40)) {
        let context = lines
            .iter()
            .cloned()
            .fold(AgentContext::new(), AgentContext::push);

        prop_assert_eq!(&context.lines, &lines);
        prop_assert_eq!(context.to_string_block(), lines.join("\n"));
    }

    /// Property: user notices preserve arbitrary text and use the documented
    /// lowercase wire spelling for their severity.
    #[test]
    fn user_notice_serialization_preserves_text(text in any::<String>(), level in 0u8..3) {
        let notice = match level {
            0 => UserNotice::info(text.clone()),
            1 => UserNotice::warning(text.clone()),
            _ => UserNotice::error(text.clone()),
        };
        let value = serde_json::to_value(&notice).unwrap();
        let expected_level = match notice.level {
            NoticeLevel::Info => "info",
            NoticeLevel::Warning => "warning",
            NoticeLevel::Error => "error",
        };

        prop_assert_eq!(value["text"].as_str(), Some(text.as_str()));
        prop_assert_eq!(value["level"].as_str(), Some(expected_level));
    }

    /// Property: artifact enrichment is additive. Adding relative-path and
    /// summary metadata cannot alter the original absolute path or media type.
    #[test]
    fn artifact_builders_do_not_rewrite_existing_fields(
        absolute in any::<String>(),
        relative in any::<String>(),
        media_type in any::<String>(),
        summary in any::<String>(),
    ) {
        let artifact = DiagnosticArtifact::new(PathBuf::from(&absolute), media_type.clone())
            .with_project_relative_path(PathBuf::from(&relative))
            .with_summary(summary.clone());

        prop_assert_eq!(artifact.absolute_path, PathBuf::from(absolute));
        prop_assert_eq!(artifact.project_relative_path, Some(PathBuf::from(relative)));
        prop_assert_eq!(artifact.media_type, media_type);
        prop_assert_eq!(artifact.summary, Some(summary));
    }

    /// Property: a tail call is an exact name/arguments pair; arbitrary JSON
    /// arguments survive serialization without being wrapped or flattened.
    #[test]
    fn tail_tool_calls_retain_name_and_arguments(name in any::<String>(), args in scalar_json()) {
        let call = TailToolCall::new(name.clone(), args.clone());
        let value = serde_json::to_value(call).unwrap();

        prop_assert_eq!(value["name"].as_str(), Some(name.as_str()));
        prop_assert_eq!(&value["args"], &args);
    }
}
