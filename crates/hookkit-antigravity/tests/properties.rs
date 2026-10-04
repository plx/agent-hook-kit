//! Property tests for Antigravity's discriminator-free native contracts.

use hookkit_antigravity::{
    InjectStep, PostToolUse, PostToolUseOutput, PreInvocation, PreInvocationOutput, PreToolUse,
    PreToolUseOutput, Stop, StopDecision, StopOutput, ToolDecision,
};
use hookkit_core::EventSpec;
use proptest::prelude::*;
use serde_json::Value;
use std::collections::BTreeSet;

proptest! {
    /// Property: message injection is a one-element ordered operation and
    /// arbitrary message text is serialized without semantic normalization.
    #[test]
    fn injected_ephemeral_messages_are_exact(message in any::<String>()) {
        let output = PreInvocationOutput::inject(InjectStep::ephemeral_message(message.clone()));
        let emission = PreInvocation::emit(output).unwrap();
        let value: Value = serde_json::from_slice(emission.stdout()).unwrap();

        prop_assert_eq!(value["injectSteps"].as_array().unwrap().len(), 1);
        prop_assert_eq!(value["injectSteps"][0]["ephemeralMessage"].as_str(), Some(message.as_str()));
    }

    /// Property: permission overrides form a set on the native wire contract.
    /// Emission always succeeds and keeps the first occurrence of each entry
    /// in order, so a duplicate never turns a decision into a hook failure.
    #[test]
    fn permission_overrides_are_deduplicated_in_first_occurrence_order(
        overrides in prop::collection::vec("[a-c]{0,2}", 0..30),
    ) {
        let output = PreToolUseOutput {
            decision: ToolDecision::Deny,
            reason: None,
            permission_overrides: overrides.clone(),
        };
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
