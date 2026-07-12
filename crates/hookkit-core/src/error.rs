/// Top-level error type for hookkit.
#[derive(Debug, thiserror::Error)]
pub enum HookkitError {
    #[error("invalid protocol identity: {0}")]
    InvalidIdentity(&'static str),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),

    #[error("invalid process emission: {0}")]
    InvalidProcessEmission(&'static str),

    #[error("missing hook_event_name in payload")]
    MissingHookEventName,

    #[error("event `{event}` does not belong to selected harness {harness}")]
    EventHarnessMismatch {
        harness: crate::HarnessId,
        event: crate::EventId,
    },

    #[error("event hint `{hint}` does not belong to selected harness {selected}")]
    HintHarnessMismatch {
        selected: crate::HarnessId,
        hint: crate::EventId,
    },

    #[error("event hint `{hint}` contradicts observed event `{actual}`")]
    EventHintMismatch {
        hint: crate::EventId,
        actual: crate::EventId,
    },

    #[error("event hint `{hint}` contradicts authoritative discriminator `{actual}` for {harness}")]
    HintContradiction {
        harness: crate::HarnessId,
        hint: crate::EventId,
        actual: crate::EventId,
    },

    #[error("handler output event `{output}` does not match input event `{input}`")]
    OutputEventMismatch {
        input: crate::EventId,
        output: crate::EventId,
    },

    #[error("event resolution is ambiguous for {harness}; candidates: {candidates:?}")]
    AmbiguousEvent {
        harness: crate::HarnessId,
        candidates: Vec<crate::EventId>,
    },

    #[error("input is invalid for hinted event `{event}` on {harness}: {message}")]
    InvalidForHint {
        harness: crate::HarnessId,
        event: crate::EventId,
        message: String,
    },

    #[error("input is invalid for hinted event `{event}`: {message}")]
    InvalidInputForHint {
        event: crate::EventId,
        message: String,
    },

    #[error("no event candidate for harness {harness}")]
    NoEventCandidate { harness: crate::HarnessId },

    #[error("unrecognized event for harness {harness}: {message}")]
    UnrecognizedEvent {
        harness: crate::HarnessId,
        message: String,
    },

    #[error("no native parser is implemented for catalog event `{event}`")]
    NativeParserUnavailable { event: crate::EventId },

    #[error("unsupported built-in harness selection: {0:?}")]
    UnsupportedBuiltinHarness(crate::BuiltinHarness),
}
