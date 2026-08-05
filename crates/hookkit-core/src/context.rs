use crate::{ContractId, EventId, HarnessId, RawInvocation, SnapshotId, Utf8PathBuf};
use std::fmt;

/// How an invocation's exact event identity was established.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionProvenance {
    /// The caller selected a concrete [`crate::EventSpec`] at compile time.
    TypedStatic,
    /// A documented JSON discriminator uniquely identified the event.
    DefinitiveDiscriminator,
    /// The payload shape uniquely matched an event whose catalog declares the
    /// shape sound for identification.
    SoundShape,
    /// A caller-supplied event hint was accepted after validating the payload.
    HintValidated,
}

/// Diagnostic severity for messages sent outside protocol stdout/stderr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticLevel {
    /// Fine-grained information useful when tracing runtime decisions.
    Trace,
    /// Informational context that does not indicate a problem.
    Info,
    /// A recoverable condition that may require attention.
    Warning,
    /// A failure that prevented an operation from completing as intended.
    Error,
}

/// One runtime or handler diagnostic. A sink decides where it is recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Severity assigned by the diagnostic producer.
    pub level: DiagnosticLevel,
    /// Human-readable diagnostic text.
    pub message: String,
}

impl Diagnostic {
    /// Creates a diagnostic with the given severity and message.
    pub fn new(level: DiagnosticLevel, message: impl Into<String>) -> Self {
        Self {
            level,
            message: message.into(),
        }
    }
}

/// A protocol-independent diagnostic destination.
pub trait DiagnosticsSink: Send + Sync {
    /// Records one diagnostic.
    ///
    /// Implementations must be safe to call concurrently. A sink decides its
    /// own buffering, persistence, and redaction policy.
    fn record(&self, diagnostic: Diagnostic);
}

/// Default sink used by convenience runners. It intentionally writes nowhere.
#[derive(Debug, Default)]
pub struct DisabledDiagnostics;

impl DiagnosticsSink for DisabledDiagnostics {
    fn record(&self, _diagnostic: Diagnostic) {}
}

/// Shared no-op diagnostic sink used by convenience runners.
pub static DISABLED_DIAGNOSTICS: DisabledDiagnostics = DisabledDiagnostics;

macro_rules! string_identifier {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        ///
        /// Values are opaque and are validated only to be non-empty.
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            /// Creates an identifier, rejecting the empty string.
            pub fn new(value: impl Into<String>) -> crate::Result<Self> {
                let value = value.into();
                if value.is_empty() {
                    return Err(crate::HookkitError::InvalidIdentity(concat!(
                        "empty ",
                        stringify!($name)
                    )));
                }
                Ok(Self(value))
            }

            /// Returns the identifier exactly as supplied by the harness.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

string_identifier!(SessionId, "A harness-native session identifier.");
string_identifier!(
    ConversationId,
    "A harness-native conversation identifier, used when no session ID exists."
);
string_identifier!(TurnId, "A harness-native model/agent turn identifier.");
string_identifier!(ToolCallId, "A harness-native tool invocation identifier.");

/// A native lifecycle boundary that starts a new session epoch.
///
/// Adapters populate this only from typed, documented event fields. The
/// timestamp remains a string here so protocol crates do not need to agree on
/// a date-time library; session-state validates and normalizes it when present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionBoundaryContext {
    /// Harness-neutral cause of the boundary.
    pub kind: SessionBoundaryKind,
    /// Timestamp supplied by the native protocol, before validation or
    /// normalization by a consumer.
    pub native_timestamp: Option<String>,
    /// Native value that distinguishes repeated observations of this boundary.
    ///
    /// Session-state hashes this value before persisting it and uses the hash
    /// to deduplicate observations. `None` means the harness supplied no safe
    /// occurrence identity.
    pub occurrence_key: Option<String>,
}

impl SessionBoundaryContext {
    /// Creates a boundary observation without a timestamp or occurrence key.
    pub fn observed(kind: SessionBoundaryKind) -> Self {
        Self {
            kind,
            native_timestamp: None,
            occurrence_key: None,
        }
    }

    /// Attaches the timestamp exactly as supplied by the native protocol.
    pub fn with_native_timestamp(mut self, timestamp: impl Into<String>) -> Self {
        self.native_timestamp = Some(timestamp.into());
        self
    }

    /// Attaches a native occurrence identity for downstream deduplication.
    pub fn with_occurrence_key(mut self, key: impl Into<String>) -> Self {
        self.occurrence_key = Some(key.into());
        self
    }
}

/// Harness-neutral session-start causes retained by [`NativeContext`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SessionBoundaryKind {
    /// A newly started native session.
    Startup,
    /// A previously persisted session was resumed.
    Resume,
    /// A new native session was forked from an existing session.
    Fork,
    /// The harness cleared the current conversation context.
    Clear,
    /// The harness compacted the current conversation context.
    Compact,
    /// The first invocation in an invocation-numbered harness session.
    InvocationStart,
}

/// Exact optional context populated by a native event adapter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeContext {
    /// Workspace roots explicitly supplied by the native event.
    pub workspace_roots: Vec<Utf8PathBuf>,
    /// Exact native session identifier, when the event carries one.
    pub session_id: Option<SessionId>,
    /// Exact native conversation identifier, when the event carries one.
    pub conversation_id: Option<ConversationId>,
    /// Exact native turn identifier, when the event carries one.
    pub turn_id: Option<TurnId>,
    /// Transcript path supplied by the native event.
    pub transcript_path: Option<Utf8PathBuf>,
    /// Exact native tool-call identifier, when the event carries one.
    pub tool_call_id: Option<ToolCallId>,
    /// Artifact directory supplied by the native event.
    pub artifact_directory: Option<Utf8PathBuf>,
    /// Session-epoch boundary explicitly represented by the event.
    pub session_boundary: Option<SessionBoundaryContext>,
}

/// Exact runtime context. No path or identifier is inferred from arbitrary JSON.
pub struct RuntimeContext<'a> {
    harness: HarnessId,
    snapshot: SnapshotId,
    event: EventId,
    contract: ContractId,
    provenance: ResolutionProvenance,
    raw: &'a RawInvocation,
    native: NativeContext,
    diagnostics: &'a dyn DiagnosticsSink,
}

impl<'a> RuntimeContext<'a> {
    /// Constructs context for an already parsed invocation.
    ///
    /// The event must belong to `harness`; a mismatch is rejected. All native
    /// fields are accepted as exact adapter output and are not inferred or
    /// otherwise revalidated here.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        harness: HarnessId,
        snapshot: SnapshotId,
        event: EventId,
        contract: ContractId,
        provenance: ResolutionProvenance,
        raw: &'a RawInvocation,
        native: NativeContext,
        diagnostics: &'a dyn DiagnosticsSink,
    ) -> crate::Result<Self> {
        if event.harness() != &harness {
            return Err(crate::HookkitError::EventHarnessMismatch { harness, event });
        }
        Ok(Self {
            harness,
            snapshot,
            event,
            contract,
            provenance,
            raw,
            native,
            diagnostics,
        })
    }

    /// Returns the harness selected for this invocation.
    pub fn harness(&self) -> &HarnessId {
        &self.harness
    }

    /// Returns the immutable native-contract snapshot.
    pub fn snapshot(&self) -> SnapshotId {
        self.snapshot
    }

    /// Returns the exact harness-scoped event identity.
    pub fn event(&self) -> &EventId {
        &self.event
    }

    /// Returns the input/output contract identity used for emission.
    pub fn contract(&self) -> ContractId {
        self.contract
    }

    /// Returns how the runtime established the event identity.
    pub fn provenance(&self) -> ResolutionProvenance {
        self.provenance
    }

    /// Returns the lossless raw invocation.
    pub fn raw(&self) -> &RawInvocation {
        self.raw
    }

    /// Returns the workspace roots explicitly supplied by the event.
    pub fn workspace_roots(&self) -> &[Utf8PathBuf] {
        &self.native.workspace_roots
    }

    /// Returns the native session identifier, if present.
    pub fn session_id(&self) -> Option<&SessionId> {
        self.native.session_id.as_ref()
    }

    /// Returns the native conversation identifier, if present.
    pub fn conversation_id(&self) -> Option<&ConversationId> {
        self.native.conversation_id.as_ref()
    }

    /// Returns the native turn identifier, if present.
    pub fn turn_id(&self) -> Option<&TurnId> {
        self.native.turn_id.as_ref()
    }

    /// Returns the native transcript path, if present.
    pub fn transcript_path(&self) -> Option<&crate::Utf8Path> {
        self.native.transcript_path.as_deref()
    }

    /// Returns the native tool-call identifier, if present.
    pub fn tool_call_id(&self) -> Option<&ToolCallId> {
        self.native.tool_call_id.as_ref()
    }

    /// Returns the native artifact directory, if present.
    pub fn artifact_directory(&self) -> Option<&crate::Utf8Path> {
        self.native.artifact_directory.as_deref()
    }

    /// Returns the event's session-boundary observation, if any.
    pub fn session_boundary(&self) -> Option<&SessionBoundaryContext> {
        self.native.session_boundary.as_ref()
    }

    /// Returns the destination for out-of-band runtime diagnostics.
    pub fn diagnostics(&self) -> &dyn DiagnosticsSink {
        self.diagnostics
    }
}
