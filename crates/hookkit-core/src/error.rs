/// Top-level error type for hookkit.
#[derive(Debug, thiserror::Error)]
pub enum HookkitError {
    /// An open identity value violated a constructor invariant.
    #[error("invalid protocol identity: {0}")]
    InvalidIdentity(&'static str),

    /// Reading or writing hook process data failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A `serde_json` error while decoding an invocation or encoding a response.
    ///
    /// Covers both syntactically invalid JSON and well-formed JSON that does not
    /// match the expected native structure (e.g. missing or mistyped fields
    /// during typed deserialization).
    #[error("invalid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),

    /// A protocol adapter attempted to construct an internally inconsistent
    /// process result.
    #[error("invalid process emission: {0}")]
    InvalidProcessEmission(&'static str),

    /// A declared hook environment variable contained non-Unicode process
    /// data and could not be represented by [`crate::EnvironmentVariables`].
    #[error("environment variable `{variable}` is not valid UTF-8")]
    NonUnicodeEnvironmentVariable {
        /// Name of the variable whose value was not valid UTF-8.
        variable: String,
    },

    /// A declared environment variable was present but malformed for the
    /// selected native event.
    #[error("invalid hook environment for `{event}`: {message}")]
    InvalidHookEnvironment {
        /// Event whose environment contract was being parsed.
        event: crate::EventId,
        /// Adapter-provided explanation of the invalid value.
        message: String,
    },

    /// Redundant values in the environment and invocation payload disagreed.
    #[error("hook environment does not match `{event}` input: {message}")]
    EnvironmentContextMismatch {
        /// Event whose two context representations disagreed.
        event: crate::EventId,
        /// Adapter-provided explanation of the mismatch.
        message: String,
    },

    /// A Claude-compatible input omitted its required event discriminator.
    #[error("missing hook_event_name in payload")]
    MissingHookEventName,

    /// An event was paired with a different harness identity.
    #[error("event `{event}` does not belong to selected harness {harness}")]
    EventHarnessMismatch {
        /// Selected harness.
        harness: crate::HarnessId,
        /// Event belonging to another harness.
        event: crate::EventId,
    },

    /// A caller-supplied event hint belonged to another harness.
    #[error("event hint `{hint}` does not belong to selected harness {selected}")]
    HintHarnessMismatch {
        /// Harness selected for resolution.
        selected: crate::HarnessId,
        /// Hint belonging to a different harness.
        hint: crate::EventId,
    },

    /// A validated hint named a different event than the observed input.
    #[error("event hint `{hint}` contradicts observed event `{actual}`")]
    EventHintMismatch {
        /// Caller-supplied event hint.
        hint: crate::EventId,
        /// Event represented by the decoded input or output.
        actual: crate::EventId,
    },

    /// A hint disagreed with a protocol's authoritative discriminator.
    #[error("event hint `{hint}` contradicts authoritative discriminator `{actual}` for {harness}")]
    HintContradiction {
        /// Harness performing event resolution.
        harness: crate::HarnessId,
        /// Caller-supplied event hint.
        hint: crate::EventId,
        /// Event named by the authoritative wire discriminator.
        actual: crate::EventId,
    },

    /// A dynamic handler returned an output arm for a different event.
    #[error("handler output event `{output}` does not match input event `{input}`")]
    OutputEventMismatch {
        /// Event that triggered the handler.
        input: crate::EventId,
        /// Event represented by the returned output arm.
        output: crate::EventId,
    },

    /// Shape-based resolution left more than one viable event.
    #[error("event resolution is ambiguous for {harness}; candidates: {candidates:?}")]
    AmbiguousEvent {
        /// Harness whose catalog was searched.
        harness: crate::HarnessId,
        /// Viable event candidates in deterministic catalog order.
        candidates: Vec<crate::EventId>,
    },

    /// A hinted catalog event had a native parser, but the parser rejected the
    /// payload.
    #[error("input is invalid for hinted event `{event}` on {harness}: {message}")]
    InvalidForHint {
        /// Harness whose catalog was searched.
        harness: crate::HarnessId,
        /// Caller-supplied event hint.
        event: crate::EventId,
        /// Native parser error rendered for resolution diagnostics.
        message: String,
    },

    /// A compile-time selected adapter rejected input supplied with a dynamic
    /// event hint.
    #[error("input is invalid for hinted event `{event}`: {message}")]
    InvalidInputForHint {
        /// Caller-supplied event hint.
        event: crate::EventId,
        /// Native parser error rendered for diagnostics.
        message: String,
    },

    /// No implemented or catalog-only descriptor could represent the payload.
    #[error("no event candidate for harness {harness}")]
    NoEventCandidate {
        /// Harness whose descriptors were searched.
        harness: crate::HarnessId,
    },

    /// Harness-specific decoding could not recognize the payload.
    #[error("unrecognized event for harness {harness}: {message}")]
    UnrecognizedEvent {
        /// Harness that attempted to decode the payload.
        harness: crate::HarnessId,
        /// Adapter-provided reason the input was not recognized.
        message: String,
    },

    /// Resolution selected a catalog-only event that cannot be executed.
    #[error("no native parser is implemented for catalog event `{event}`")]
    NativeParserUnavailable {
        /// Selected catalog event.
        event: crate::EventId,
    },

    /// The selected built-in does not have a runtime dispatch arm.
    #[error("unsupported built-in harness selection: {0:?}")]
    UnsupportedBuiltinHarness(crate::BuiltinHarness),
}
