/// Top-level error type for hookkit.
///
/// The enum is `#[non_exhaustive]`: new failure classes are added as the
/// supported protocols grow, so downstream matches need a wildcard arm.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
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

    /// A caller-supplied event hint named a different event than the one the
    /// payload's authoritative discriminator, or its unique sound shape,
    /// identifies.
    #[error("event hint `{hint}` contradicts observed event `{actual}`")]
    EventHintMismatch {
        /// Caller-supplied event hint.
        hint: crate::EventId,
        /// Event represented by the decoded input or output.
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

    /// A harness adapter decoded a different input arm than the event that
    /// resolution selected. This is an adapter defect detected before any
    /// handler runs.
    #[error("harness adapter decoded `{decoded}` for resolved event `{resolved}`")]
    DecodedEventMismatch {
        /// Event selected by resolution.
        resolved: crate::EventId,
        /// Event represented by the decoded input arm.
        decoded: crate::EventId,
    },

    /// Shape-based resolution left more than one viable event.
    #[error("event resolution is ambiguous for {harness}; candidates: {candidates:?}")]
    AmbiguousEvent {
        /// Harness whose catalog was searched.
        harness: crate::HarnessId,
        /// Viable event candidates in deterministic catalog order.
        candidates: Vec<crate::EventId>,
    },

    /// A protocol crate's hinted catalog lookup found a native parser, but
    /// that parser rejected the payload.
    ///
    /// This variant carries the harness whose catalog was searched. It and
    /// [`HookkitError::InvalidInputForHint`] both mean "the hinted parser
    /// rejected the input"; protocol crates choose the variant that carries
    /// the context they have.
    #[error("input is invalid for hinted event `{event}` on {harness}: {message}")]
    InvalidForHint {
        /// Harness whose catalog was searched.
        harness: crate::HarnessId,
        /// Caller-supplied event hint.
        event: crate::EventId,
        /// Native parser error rendered for resolution diagnostics.
        message: String,
    },

    /// A caller-supplied event hint was not registered, had no native parser,
    /// or its parser rejected the payload.
    ///
    /// See [`HookkitError::InvalidForHint`] for the harness-qualified form.
    #[error("input is invalid for hinted event `{event}`: {message}")]
    InvalidInputForHint {
        /// Caller-supplied event hint.
        event: crate::EventId,
        /// Native parser error rendered for diagnostics.
        message: String,
    },

    /// An event identified without a hint (for example by its authoritative
    /// discriminator) was rejected by its exact native parser.
    #[error("input is invalid for `{event}`: {source}")]
    InvalidInputForEvent {
        /// Event whose native parser rejected the payload.
        event: crate::EventId,
        /// The native parser's error.
        #[source]
        source: Box<HookkitError>,
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

    /// An open harness identity has no adapter for the requested operation.
    ///
    /// [`crate::HarnessId`] is deliberately open, so APIs that only support
    /// the built-in harnesses report other identities (including aliases
    /// such as `claude` for `claude-code`) with this variant.
    #[error("unsupported harness `{harness}`: {message}")]
    UnsupportedHarness {
        /// The harness identity that has no adapter.
        harness: crate::HarnessId,
        /// What the caller tried to do with it.
        message: String,
    },

    /// An application hook handler failed for a domain-specific reason.
    ///
    /// Construct it with [`HookkitError::handler`]. The boxed error stays
    /// available through [`std::error::Error::source`] and
    /// `downcast_ref`, so application error types are not flattened into
    /// strings or disguised as I/O errors.
    #[error("hook handler failed: {0}")]
    Handler(#[source] Box<dyn std::error::Error + Send + Sync + 'static>),
}

impl HookkitError {
    /// Wraps an application error returned by a hook handler.
    ///
    /// ```
    /// use hookkit_core::HookkitError;
    ///
    /// let error = HookkitError::handler(std::fmt::Error);
    /// assert!(matches!(error, HookkitError::Handler(_)));
    /// assert_eq!(error.to_string(), "hook handler failed: an error occurred when formatting an argument");
    /// ```
    pub fn handler(error: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> Self {
        Self::Handler(error.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    #[derive(Debug, thiserror::Error)]
    #[error("policy file is missing")]
    struct PolicyMissing;

    #[test]
    fn handler_errors_keep_their_type_and_source() {
        let error = HookkitError::handler(PolicyMissing);
        assert_eq!(
            error.to_string(),
            "hook handler failed: policy file is missing"
        );
        let source = error.source().expect("handler source");
        assert!(source.downcast_ref::<PolicyMissing>().is_some());
    }

    #[test]
    fn handler_accepts_plain_messages() {
        let error = HookkitError::handler("configuration is inconsistent");
        assert_eq!(
            error.to_string(),
            "hook handler failed: configuration is inconsistent"
        );
    }

    #[test]
    fn event_qualified_parse_errors_name_the_event_and_keep_the_cause() {
        let parse = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
        let event = crate::EventId::builtin(crate::HarnessId::CLAUDE_CODE, "SessionStart");
        let error = HookkitError::InvalidInputForEvent {
            event,
            source: Box::new(HookkitError::from(parse)),
        };
        let rendered = error.to_string();
        assert!(
            rendered.starts_with("input is invalid for `claude-code/SessionStart`: invalid JSON:")
        );
        let cause = error
            .source()
            .and_then(|source| source.downcast_ref::<Box<HookkitError>>())
            .expect("boxed parser error source");
        assert!(matches!(**cause, HookkitError::InvalidJson(_)));
        assert!(matches!(
            &error,
            HookkitError::InvalidInputForEvent { source, .. }
                if matches!(**source, HookkitError::InvalidJson(_))
        ));
    }

    #[test]
    fn errors_are_thread_safe() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<HookkitError>();
    }
}
