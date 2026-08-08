//! Property tests for Antigravity's discriminator-free native contracts.

use hookkit_antigravity::{
    InjectStep, PostToolUse, PostToolUseOutput, PreInvocation, PreInvocationOutput, PreToolUse,
    PreToolUseOutput, Stop, StopOutput, ToolDecision,
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
        let output = PreInvocationOutput::inject(InjectStep::EphemeralMessage {
            ephemeral_message: message.clone(),
        });
        let emission = PreInvocation::emit(output).unwrap();
        let value: Value = serde_json::from_slice(emission.stdout()).unwrap();

        prop_assert_eq!(value["injectSteps"].as_array().unwrap().len(), 1);
        prop_assert_eq!(value["injectSteps"][0]["ephemeralMessage"].as_str(), Some(message.as_str()));
    }

    /// Property: permission overrides form a set on the native wire contract.
    /// Every unique vector emits successfully; every duplicate is rejected.
    #[test]
    fn permission_overrides_must_be_unique(overrides in prop::collection::vec(any::<String>(), 0..30)) {
        let unique = overrides.iter().collect::<BTreeSet<_>>().len() == overrides.len();
        let output = PreToolUseOutput {
            decision: ToolDecision::Allow,
            reason: None,
            permission_overrides: overrides.clone(),
        };
        let result = PreToolUse::emit(output);

        prop_assert_eq!(result.is_ok(), unique);
        if let Ok(emission) = result {
            let value: Value = serde_json::from_slice(emission.stdout()).unwrap();
            if overrides.is_empty() {
                prop_assert!(value.get("permissionOverrides").is_none());
            } else {
                let actual: Vec<_> = value["permissionOverrides"]
                    .as_array().unwrap().iter().map(|item| item.as_str().unwrap()).collect();
                let expected: Vec<_> = overrides.iter().map(String::as_str).collect();
                prop_assert_eq!(actual, expected);
            }
        }
    }

    /// Property: successful process diagnostics are orthogonal to the exact
    /// empty-object PostToolUse response.
    #[test]
    fn post_tool_protocol_stderr_preserves_exact_stdout(stderr in any::<String>()) {
        let emission = PostToolUse::emit(
            PostToolUseOutput::default()
                .with_protocol_stderr(stderr.clone())
                .unwrap()
        ).unwrap();

        prop_assert_eq!(emission.stdout(), b"{}");
        prop_assert_eq!(emission.stderr(), stderr.as_bytes());
        prop_assert_eq!(emission.exit_code(), 0);
    }

    /// Property: Stop's decision is the only required non-empty semantic
    /// value. Reasons remain optional and are omitted instead of emitted null.
    #[test]
    fn stop_requires_only_a_nonempty_decision(
        decision in any::<String>(),
        reason in proptest::option::of(any::<String>()),
    ) {
        let result = Stop::emit(StopOutput { decision: decision.clone(), reason: reason.clone() });
        if decision.is_empty() {
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
