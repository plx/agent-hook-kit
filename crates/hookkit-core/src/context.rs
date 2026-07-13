use crate::{ContractId, EventId, HarnessId, RawInvocation, SnapshotId, Utf8PathBuf};
use std::fmt;

/// How an invocation's exact event identity was established.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionProvenance {
    TypedStatic,
    DefinitiveDiscriminator,
    SoundShape,
    HintValidated,
}

/// Diagnostic severity for messages sent outside protocol stdout/stderr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Trace,
    Info,
    Warning,
    Error,
}

/// One runtime or handler diagnostic. A sink decides where it is recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub level: DiagnosticLevel,
    pub message: String,
}

impl Diagnostic {
    pub fn new(level: DiagnosticLevel, message: impl Into<String>) -> Self {
        Self {
            level,
            message: message.into(),
        }
    }
}

/// A protocol-independent diagnostic destination.
pub trait DiagnosticsSink: Send + Sync {
    fn record(&self, diagnostic: Diagnostic);
}

/// Default sink used by convenience runners. It intentionally writes nowhere.
#[derive(Debug, Default)]
pub struct DisabledDiagnostics;

impl DiagnosticsSink for DisabledDiagnostics {
    fn record(&self, _diagnostic: Diagnostic) {}
}

pub static DISABLED_DIAGNOSTICS: DisabledDiagnostics = DisabledDiagnostics;

macro_rules! string_identifier {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
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

string_identifier!(SessionId);
string_identifier!(ConversationId);
string_identifier!(TurnId);
string_identifier!(ToolCallId);

/// Exact optional context populated by a native event adapter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeContext {
    pub workspace_roots: Vec<Utf8PathBuf>,
    pub session_id: Option<SessionId>,
    pub conversation_id: Option<ConversationId>,
    pub turn_id: Option<TurnId>,
    pub transcript_path: Option<Utf8PathBuf>,
    pub tool_call_id: Option<ToolCallId>,
    pub artifact_directory: Option<Utf8PathBuf>,
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

    pub fn harness(&self) -> &HarnessId {
        &self.harness
    }

    pub fn snapshot(&self) -> SnapshotId {
        self.snapshot
    }

    pub fn event(&self) -> &EventId {
        &self.event
    }

    pub fn contract(&self) -> ContractId {
        self.contract
    }

    pub fn provenance(&self) -> ResolutionProvenance {
        self.provenance
    }

    pub fn raw(&self) -> &RawInvocation {
        self.raw
    }

    pub fn workspace_roots(&self) -> &[Utf8PathBuf] {
        &self.native.workspace_roots
    }

    pub fn session_id(&self) -> Option<&SessionId> {
        self.native.session_id.as_ref()
    }

    pub fn conversation_id(&self) -> Option<&ConversationId> {
        self.native.conversation_id.as_ref()
    }

    pub fn turn_id(&self) -> Option<&TurnId> {
        self.native.turn_id.as_ref()
    }

    pub fn transcript_path(&self) -> Option<&crate::Utf8Path> {
        self.native.transcript_path.as_deref()
    }

    pub fn tool_call_id(&self) -> Option<&ToolCallId> {
        self.native.tool_call_id.as_ref()
    }

    pub fn artifact_directory(&self) -> Option<&crate::Utf8Path> {
        self.native.artifact_directory.as_deref()
    }

    pub fn diagnostics(&self) -> &dyn DiagnosticsSink {
        self.diagnostics
    }
}
