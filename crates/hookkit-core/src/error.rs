use crate::{Harness, HookEventKey};

/// Top-level error type for hookkit.
#[derive(Debug, thiserror::Error)]
pub enum HookkitError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),

    #[error("missing hook_event_name in payload")]
    MissingHookEventName,

    #[error("unknown event `{event_name}` for harness {harness}")]
    UnknownEvent {
        harness: Harness,
        event_name: String,
    },

    #[error("failed to parse `{event_name}` for harness {harness}: {source}")]
    ParseFailure {
        harness: Harness,
        event_name: String,
        source: serde_json::Error,
    },

    #[error("unsupported capability `{capability}` for {harness} {event:?}")]
    UnsupportedCapability {
        harness: Harness,
        event: HookEventKey,
        capability: &'static str,
    },

    #[error("invalid output for {harness} {event:?}: {message}")]
    InvalidOutputCombination {
        harness: Harness,
        event: HookEventKey,
        message: String,
    },
}
