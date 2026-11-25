//! Property tests for Antigravity's discriminator-free native contracts.

use hookkit_antigravity::{
    InjectStep, PostToolUse, PostToolUseOutput, PreInvocation, PreInvocationOutput, PreToolUse,
    PreToolUseOutput, SessionStart, SessionStartOutput, Stop, StopDecision, StopOutput,
};
use hookkit_core::{EventSpec, RawInvocation};
use proptest::prelude::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// Any JSON scalar a harness could plausibly send as `toolCall.id`.
fn tool_call_id() -> impl Strategy<Value = Value> {
    prop_oneof![
        any::<String>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<bool>().prop_map(Value::from),
        Just(Value::Null),
    ]
}

fn raw(value: &Value) -> RawInvocation {
    RawInvocation::parse(serde_json::to_vec(value).unwrap()).unwrap()
}

proptest! {
    /// Property: message injection is a one-element ordered operation and
    /// arbitrary message text is serialized without semantic normalization.
    #[test]
    fn injected_system_messages_are_exact(message in any::<String>()) {
        let output = PreInvocationOutput::inject(InjectStep::system_message(message.clone()));
        let emission = PreInvocation::emit(output).unwrap();
        let value: Value = serde_json::from_slice(emission.stdout()).unwrap();

        prop_assert_eq!(value["injectSteps"].as_array().unwrap().len(), 1);
        prop_assert_eq!(
            value["injectSteps"][0]["systemMessage"]["systemMessage"].as_str(),
            Some(message.as_str())
        );
        prop_assert!(value["injectSteps"][0]["systemMessage"].get("metadata").is_none());
        prop_assert!(value.get("terminationBehavior").is_none());

        let start = SessionStart::emit(SessionStartOutput::inject(
            InjectStep::system_message(message.clone()),
        )).unwrap();
        prop_assert_eq!(start.stdout(), emission.stdout());
    }

    /// Property: the deprecated `ephemeralMessage` step stays exact for
    /// builds that predate `systemMessage`.
    #[test]
    fn injected_deprecated_ephemeral_messages_are_exact(message in any::<String>()) {
        let output = PreInvocationOutput::inject(InjectStep::ephemeral_message(message.clone()));
        let emission = PreInvocation::emit(output).unwrap();
        let value: Value = serde_json::from_slice(emission.stdout()).unwrap();

        prop_assert_eq!(value["injectSteps"][0]["ephemeralMessage"].as_str(), Some(message.as_str()));
    }

    /// Property: a prompt block always pairs exactly one error step with
    /// `terminate`, and the message is shown verbatim.
    #[test]
    fn prompt_blocks_are_exact(message in any::<String>()) {
        let emission = PreInvocation::emit(PreInvocationOutput::block(message.clone())).unwrap();
        let value: Value = serde_json::from_slice(emission.stdout()).unwrap();

        prop_assert_eq!(
            value,
            serde_json::json!({
                "injectSteps": [{"errorMessage": {"userMessage": message}}],
                "terminationBehavior": "terminate",
            })
        );
        prop_assert_eq!(emission.exit_code(), 0);
    }

    /// Property: permission overrides form a set on the native wire contract.
    /// Emission always succeeds and keeps the first occurrence of each entry
    /// in order, so a duplicate never turns a decision into a hook failure.
    #[test]
    fn permission_overrides_are_deduplicated_in_first_occurrence_order(
        overrides in prop::collection::vec("[a-c]{0,2}", 0..30),
    ) {
        let mut output = PreToolUseOutput::deny();
        output.permission_overrides = overrides.clone();
        let emission = PreToolUse::emit(output).unwrap();
        let value: Value = serde_json::from_slice(emission.stdout()).unwrap();

        let mut seen = BTreeSet::new();
        let expected: Vec<&str> = overrides
            .iter()
            .map(String::as_str)
            .filter(|item| seen.insert(*item))
            .collect();
        if expected.is_empty() {
            prop_assert!(value.get("permissionOverrides").is_none());
        } else {
            let actual: Vec<_> = value["permissionOverrides"]
                .as_array().unwrap().iter().map(|item| item.as_str().unwrap()).collect();
            prop_assert_eq!(actual, expected);
        }
    }

    /// Property: successful process diagnostics are orthogonal to the exact
    /// empty-object PostToolUse response.
    #[test]
    fn post_tool_protocol_stderr_preserves_exact_stdout(stderr in any::<String>()) {
        let emission = PostToolUse::emit(
            PostToolUseOutput::no_op()
                .with_protocol_stderr(stderr.clone())
                .unwrap()
        ).unwrap();

        prop_assert_eq!(emission.stdout(), b"{}");
        prop_assert_eq!(emission.stderr(), stderr.as_bytes());
        prop_assert_eq!(emission.exit_code(), 0);
    }

    /// Property: the release-note metadata (`hookEventName`, `cwd`,
    /// `lastUserInput`, and `toolCall.id`) and unknown additions are read
    /// tolerantly: any value of the documented JSON type, and any
    /// `toolCall.id` at all, parses for both tool events, round-trips
    /// losslessly, and never selects or rejects the event. The runtime
    /// context and `ToolCall::id_str` agree on the string identifier.
    #[test]
    fn tool_event_metadata_is_read_tolerantly_and_losslessly(
        hook_event_name in any::<String>(),
        cwd in any::<String>(),
        last_user_input in any::<String>(),
        id in tool_call_id(),
        unknown in "[a-z]{1,8}",
        unknown_value in any::<i64>(),
    ) {
        let mut value = json!({
            "conversationId": "c1",
            "workspacePaths": ["/repo"],
            "transcriptPath": "/tmp/t.jsonl",
            "artifactDirectoryPath": "/tmp/a",
            "hookEventName": hook_event_name,
            "cwd": cwd,
            "lastUserInput": last_user_input,
            "toolCall": {"name": "run_command", "args": {"CommandLine": "ls"}, "id": id.clone()},
            "stepIdx": 4
        });
        let unknown = format!("future_{unknown}");
        value[&unknown] = unknown_value.into();
        let invocation = raw(&value);

        let pre = PreToolUse::parse(&invocation).unwrap();
        prop_assert_eq!(serde_json::to_value(&pre).unwrap(), value.clone());
        prop_assert_eq!(pre.hook_event_name.as_deref(), Some(hook_event_name.as_str()));
        prop_assert_eq!(&pre.extra[&unknown], &Value::from(unknown_value));
        prop_assert_eq!(pre.tool_call.id.as_ref(), Some(&id));
        let expected_id = id.as_str().filter(|id| !id.trim().is_empty());
        prop_assert_eq!(pre.tool_call.id_str(), expected_id);
        let context = PreToolUse::context(&pre);
        prop_assert_eq!(context.tool_call_id.as_ref().map(|id| id.as_str()), expected_id);

        let post = PostToolUse::parse(&invocation).unwrap();
        prop_assert_eq!(serde_json::to_value(&post).unwrap(), value);
        let context = PostToolUse::context(&post);
        prop_assert_eq!(context.tool_call_id.as_ref().map(|id| id.as_str()), expected_id);
    }

    /// Property: `continuation_active_flag` prefers the release-note
    /// snake_case key, falls back to a camelCase one, reports absence as
    /// unknown, and both keys round-trip.
    #[test]
    fn stop_continuation_flag_prefers_the_release_note_spelling(
        snake in proptest::option::of(any::<bool>()),
        camel in proptest::option::of(any::<bool>()),
    ) {
        let mut value = json!({
            "conversationId": "c1",
            "workspacePaths": [],
            "transcriptPath": "/tmp/t.jsonl",
            "artifactDirectoryPath": "/tmp/a",
            "executionNum": 1,
            "terminationReason": "model_stop",
            "fullyIdle": true
        });
        if let Some(snake) = snake {
            value["continuation_active"] = snake.into();
        }
        if let Some(camel) = camel {
            value["continuationActive"] = camel.into();
        }
        let input = Stop::parse(&raw(&value)).unwrap();
        prop_assert_eq!(input.continuation_active, snake);
        prop_assert_eq!(input.continuation_active_flag(), snake.or(camel));
        prop_assert_eq!(serde_json::to_value(&input).unwrap(), value);
    }

    /// Property: `overwrite` is emitted exactly as given, in the reference's
    /// field order, without changing the decision.
    #[test]
    fn overwrite_arguments_are_emitted_exactly(
        arguments in prop::collection::btree_map("[A-Za-z]{1,12}", any::<String>(), 0..6),
    ) {
        let overwrite: serde_json::Map<String, Value> = arguments
            .into_iter()
            .map(|(key, value)| (key, Value::from(value)))
            .collect();
        let emission = PreToolUse::emit(
            PreToolUseOutput::auto_approve()
                .with_overwrite(overwrite.clone())
                .with_permission_override("command(ls)"),
        )
        .unwrap();
        let value: Value = serde_json::from_slice(emission.stdout()).unwrap();
        prop_assert_eq!(
            value,
            json!({
                "decision": "auto_approve",
                "overwrite": overwrite,
                "permissionOverrides": ["command(ls)"],
            })
        );
        let text = String::from_utf8(emission.stdout().to_vec()).unwrap();
        prop_assert!(text.find("\"overwrite\"") < text.find("\"permissionOverrides\""));
    }

    /// Property: Stop's decision is the only required non-empty semantic
    /// value, and it round-trips verbatim. Emission rejects only the empty
    /// string and look-alikes of `continue` that Antigravity would read as
    /// allowing the stop. Reasons remain optional and are omitted instead of
    /// emitted as null.
    #[test]
    fn stop_emits_every_unambiguous_nonempty_decision_verbatim(
        decision in any::<String>(),
        reason in proptest::option::of(any::<String>()),
    ) {
        let lookalike = decision != "continue" && decision.trim().eq_ignore_ascii_case("continue");
        let output = StopOutput { decision: StopDecision::from(decision.clone()), reason: reason.clone() };
        prop_assert_eq!(output.decision.as_str(), decision.as_str());
        let result = Stop::emit(output);
        if decision.is_empty() || lookalike {
            prop_assert!(result.is_err());
        } else {
            let value: Value = serde_json::from_slice(result.unwrap().stdout()).unwrap();
            prop_assert_eq!(value["decision"].as_str(), Some(decision.as_str()));
            match reason {
                Some(reason) => prop_assert_eq!(value["reason"].as_str(), Some(reason.as_str())),
                None => prop_assert!(value.get("reason").is_none()),
            }
        }
    }
}
