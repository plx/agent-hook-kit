//! Shared wire helpers for Codex input values and command emission.

use hookkit_core::{ContractId, HookkitError, ProcessEmission};

/// Declares a forward-compatible string enum for a harness-sent input value.
///
/// Values documented by the implemented snapshot get named variants. Any other
/// string deserializes to `Unknown` with the raw value retained, so a newer
/// Codex build that adds a value does not make every hook fail to parse, and
/// serialization writes the original string back unchanged. Strict validation
/// of the documented value set belongs to the snapshot's input schemas.
macro_rules! open_string_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $(
                $(#[$variant_meta:meta])*
                $variant:ident => $wire:literal,
            )+
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        #[non_exhaustive]
        pub enum $name {
            $(
                $(#[$variant_meta])*
                $variant,
            )+
            /// A value this snapshot does not document, retained verbatim.
            ///
            /// Deserialization produces this arm only for undocumented values;
            /// construct values with [`From<&str>`] to get the same
            /// normalization.
            Unknown(String),
        }

        impl $name {
            /// Returns the exact native wire string.
            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$variant => $wire,)+
                    Self::Unknown(value) => value,
                }
            }

            /// Reports whether the value is documented by the implemented
            /// snapshot, that is, whether it is not [`Self::Unknown`].
            pub fn is_known(&self) -> bool {
                !matches!(self, Self::Unknown(_))
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                match value {
                    $($wire => Self::$variant,)+
                    other => Self::Unknown(other.to_owned()),
                }
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                match Self::from(value.as_str()) {
                    Self::Unknown(_) => Self::Unknown(value),
                    known => known,
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                <String as serde::Deserialize>::deserialize(deserializer).map(Self::from)
            }
        }
    };
}

pub(crate) use open_string_enum;

/// Expands to the implemented Codex snapshot id as a string literal, so the
/// snapshot constant and every contract id are spelled in one place.
macro_rules! snapshot_literal {
    () => {
        "commit-ff6aec9-r2"
    };
}

pub(crate) use snapshot_literal;

/// Expands to the builtin [`ContractId`] of one event in the implemented
/// snapshot.
macro_rules! contract_id {
    ($event:literal) => {
        hookkit_core::ContractId::builtin(concat!(
            "codex/",
            $crate::wire::snapshot_literal!(),
            "/",
            $event
        ))
    };
}

pub(crate) use contract_id;

/// Codex treats exit-0 stdout whose first non-whitespace character is `{` or
/// `[` as JSON (`output_parser::looks_like_json`), never as plain text.
pub(crate) fn looks_like_json(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with('{') || text.starts_with('[')
}

/// Rejects text that Codex would treat as missing after trimming.
pub(crate) fn require_nonblank(value: &str, what: &'static str) -> hookkit_core::Result<()> {
    if value.trim().is_empty() {
        return Err(HookkitError::InvalidProcessEmission(what));
    }
    Ok(())
}

/// Emits an exit-2 outcome whose stderr Codex reads as the blocking reason.
///
/// Codex only honors exit 2 when stderr is non-empty after trimming; a blank
/// message is a failed run that does not block, so it is rejected here.
pub(crate) fn blocking_stderr(
    contract: ContractId,
    message: String,
) -> hookkit_core::Result<ProcessEmission> {
    require_nonblank(
        &message,
        "Codex exit-2 blocking stderr must be non-empty after trimming",
    )?;
    ProcessEmission::command_required_stderr(contract, message, 2)
}

/// Rejects a structured `decision: "block"` that Codex would fail without
/// blocking: the reason must be a string that is non-empty after trimming,
/// unless `continue: false` takes precedence.
pub(crate) fn validate_block_reason(
    output: &serde_json::Map<String, serde_json::Value>,
) -> hookkit_core::Result<()> {
    if output.get("decision").and_then(serde_json::Value::as_str) != Some("block")
        || output.get("continue") == Some(&serde_json::Value::Bool(false))
    {
        return Ok(());
    }
    match output.get("reason").and_then(serde_json::Value::as_str) {
        Some(reason) => require_nonblank(
            reason,
            "Codex decision:block requires a reason that is non-empty after trimming",
        ),
        None => Err(HookkitError::InvalidProcessEmission(
            "Codex decision:block requires a reason that is non-empty after trimming",
        )),
    }
}

/// Reports whether `output` pairs `continue: false` with a `decision:
/// "block"` whose reason is missing or blank after trimming.
///
/// Codex applies `continue: false` before the block, so such a block has no
/// effect of its own. Codex still marks its reason invalid, and on
/// `PostToolUse` and `UserPromptSubmit` an invalid block reason makes it
/// discard the response's `additionalContext` without reporting an error.
fn blank_block_under_stop(output: &serde_json::Map<String, serde_json::Value>) -> bool {
    output.get("continue") == Some(&serde_json::Value::Bool(false))
        && output.get("decision").and_then(serde_json::Value::as_str) == Some("block")
        && output
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .is_none_or(|reason| reason.trim().is_empty())
}

/// Encodes a structured object, using empty stdout for an object with no
/// members because Codex's `no-op` outcome is exit 0 with empty stdout.
///
/// A blank block under `continue: false` is left out, because Codex would
/// otherwise drop the additional context while the stop is unchanged.
pub(crate) fn structured(
    contract: ContractId,
    output: &serde_json::Map<String, serde_json::Value>,
) -> hookkit_core::Result<ProcessEmission> {
    if output.is_empty() {
        return Ok(ProcessEmission::command_empty(contract));
    }
    if blank_block_under_stop(output) {
        let mut output = output.clone();
        output.remove("decision");
        output.remove("reason");
        return ProcessEmission::command_json(contract, &output);
    }
    validate_block_reason(output)?;
    ProcessEmission::command_json(contract, output)
}

#[cfg(test)]
mod tests {
    use super::*;

    open_string_enum! {
        /// Test enum.
        pub enum Sample {
            /// First.
            First => "first",
        }
    }

    #[test]
    fn open_enums_round_trip_unknown_values() {
        let known: Sample = serde_json::from_str(r#""first""#).unwrap();
        assert_eq!(known, Sample::First);
        assert!(known.is_known());
        let unknown: Sample = serde_json::from_str(r#""second""#).unwrap();
        assert_eq!(unknown, Sample::Unknown("second".into()));
        assert_eq!(unknown.as_str(), "second");
        assert_eq!(serde_json::to_string(&unknown).unwrap(), r#""second""#);
        assert_eq!(Sample::from(String::from("first")), Sample::First);
        assert!(serde_json::from_str::<Sample>("1").is_err());
    }

    #[test]
    fn json_lookalike_detection_matches_codex() {
        assert!(looks_like_json("{}"));
        assert!(looks_like_json(" \n[repo] policy"));
        assert!(!looks_like_json("repo {policy}"));
        assert!(!looks_like_json(""));
    }

    #[test]
    fn block_reason_rules_follow_codex_parser() {
        let object = |value: serde_json::Value| value.as_object().unwrap().clone();
        assert!(validate_block_reason(&object(serde_json::json!({"decision":"block"}))).is_err());
        assert!(
            validate_block_reason(&object(
                serde_json::json!({"decision":"block","reason":" \t"})
            ))
            .is_err()
        );
        assert!(
            validate_block_reason(&object(
                serde_json::json!({"decision":"block","continue":false})
            ))
            .is_ok()
        );
        assert!(
            validate_block_reason(&object(
                serde_json::json!({"decision":"block","reason":"x"})
            ))
            .is_ok()
        );
    }

    #[test]
    fn a_blank_block_under_stop_is_left_out_of_the_json() {
        let contract = ContractId::builtin("codex/test/PostToolUse");
        let emit = |value: serde_json::Value| {
            let emission = structured(contract, value.as_object().unwrap()).unwrap();
            serde_json::from_slice::<serde_json::Value>(emission.stdout()).unwrap()
        };
        for blank in [
            serde_json::json!({"decision":"block","continue":false}),
            serde_json::json!({"decision":"block","reason":" \n","continue":false}),
        ] {
            assert_eq!(emit(blank), serde_json::json!({"continue": false}));
        }
        let kept = serde_json::json!({"decision":"block","reason":"why","continue":false});
        assert_eq!(emit(kept.clone()), kept);
    }
}
