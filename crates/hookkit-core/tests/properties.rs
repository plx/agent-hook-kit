//! Executable documentation for the protocol-independent value semantics.
//!
//! These properties deliberately range over arbitrary Unicode and nested JSON:
//! hook payloads and identifiers originate outside this crate, so preserving
//! their exact values is part of the public contract rather than an incidental
//! implementation detail.

use hookkit_core::{
    ContractId, EnvironmentVariables, EventId, HarnessId, ProcessEmission, RawInvocation, SessionId,
};
use proptest::prelude::*;
use serde_json::Value;
use std::collections::BTreeMap;

fn json_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(|value| Value::Number(value.into())),
        any::<String>().prop_map(Value::String),
    ];

    leaf.prop_recursive(4, 64, 8, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..8).prop_map(Value::Array),
            prop::collection::btree_map("[a-zA-Z_][a-zA-Z0-9_]{0,12}", inner, 0..8)
                .prop_map(|entries| Value::Object(entries.into_iter().collect())),
        ]
    })
}

proptest! {
    /// Property: parsing JSON is lossless in two independent senses. The exact
    /// input bytes remain available for forwarding/auditing, while the parsed
    /// tree has precisely the value serialized into those bytes.
    #[test]
    fn raw_invocations_retain_exact_bytes_and_json(value in json_value()) {
        let bytes = serde_json::to_vec(&value).unwrap();
        let raw = RawInvocation::parse(bytes.clone()).unwrap();

        prop_assert_eq!(raw.bytes(), bytes.as_slice());
        prop_assert_eq!(raw.json(), &value);
    }

    /// Property: open identifiers reject exactly the empty string. Non-empty
    /// identifiers are opaque, including whitespace and non-ASCII text; the
    /// library must not silently normalize data owned by a native harness.
    #[test]
    fn open_identifiers_preserve_every_nonempty_string(value in any::<String>()) {
        let harness = HarnessId::new(value.clone());
        let session = SessionId::new(value.clone());

        if value.is_empty() {
            prop_assert!(harness.is_err());
            prop_assert!(session.is_err());
        } else {
            let harness = harness.unwrap();
            let session = session.unwrap();
            prop_assert_eq!(harness.as_str(), value.as_str());
            prop_assert_eq!(session.as_str(), value.as_str());
        }
    }

    /// Property: an event's display form is only a view. Constructing an event
    /// never changes either the harness identity or the native event name.
    #[test]
    fn event_identity_retains_both_scoped_parts(
        harness in any::<String>().prop_filter("nonempty harness", |s| !s.is_empty()),
        event in any::<String>().prop_filter("nonempty event", |s| !s.is_empty()),
    ) {
        let harness = HarnessId::new(harness.clone()).unwrap();
        let identity = EventId::new(harness.clone(), event.clone()).unwrap();

        prop_assert_eq!(identity.harness(), &harness);
        prop_assert_eq!(identity.name(), event.as_str());
        prop_assert_eq!(identity.to_string(), format!("{harness}/{event}"));
    }

    /// Property: environment capture behaves like a deterministic map. Later
    /// occurrences of a name win and iteration is lexicographically ordered,
    /// independent of the order supplied by the operating system.
    #[test]
    fn environment_variables_are_a_sorted_last_write_wins_map(
        pairs in prop::collection::vec(("[A-Z_][A-Z0-9_]{0,10}", any::<String>()), 0..40),
    ) {
        let expected: BTreeMap<_, _> = pairs.iter().cloned().collect();
        let variables = EnvironmentVariables::from_pairs(pairs);
        let actual: Vec<_> = variables.iter().collect();
        let expected_refs: Vec<_> = expected
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();

        prop_assert_eq!(actual, expected_refs);
        prop_assert_eq!(variables.len(), expected.len());
    }

    /// Property: JSON command emissions are exact transport envelopes: success
    /// is exit zero, stderr is empty, and stdout decodes to the original value.
    #[test]
    fn json_emissions_preserve_arbitrary_json(value in json_value()) {
        let contract = ContractId::builtin("property/json");
        let emission = ProcessEmission::command_json(contract, &value).unwrap();

        prop_assert_eq!(emission.contract(), contract);
        prop_assert_eq!(emission.exit_code(), 0);
        prop_assert!(emission.stderr().is_empty());
        prop_assert_eq!(serde_json::from_slice::<Value>(emission.stdout()).unwrap(), value);
    }

    /// Property: stderr-only failures preserve arbitrary bytes and every
    /// non-zero protocol exit code, while exit zero is rejected as contradictory.
    #[test]
    fn stderr_only_emissions_require_and_preserve_failure(
        message in prop::collection::vec(any::<u8>(), 0..256),
        exit_code in any::<u8>(),
    ) {
        let result = ProcessEmission::command_stderr(
            ContractId::builtin("property/stderr"),
            message.clone(),
            exit_code,
        );
        if exit_code == 0 {
            prop_assert!(result.is_err());
        } else {
            let emission = result.unwrap();
            prop_assert!(emission.stdout().is_empty());
            prop_assert_eq!(emission.stderr(), message.as_slice());
            prop_assert_eq!(emission.exit_code(), exit_code);
        }
    }
}
