//! Core types and error model for hookkit.
mod context;
mod environment;
mod error;
mod event;
mod identity;
pub mod json_helpers;
mod raw;

pub use context::{
    ConversationId, DISABLED_DIAGNOSTICS, Diagnostic, DiagnosticLevel, DiagnosticsSink,
    DisabledDiagnostics, NativeContext, ResolutionProvenance, RuntimeContext,
    SessionBoundaryContext, SessionBoundaryKind, SessionId, ToolCallId, TurnId,
};
pub use environment::{CommandEnvironmentSpec, EnvironmentVariables, NoCommandEnvironment};
pub use error::HookkitError;
pub use event::{
    EventCategory, EventSelector, EventSpec, HandlerKind, HarnessSpec, IdentificationDescriptor,
    IdentificationStrength, NativeEventDescriptor, ProcessEmission,
};
pub use identity::{
    AlignedEventKind, BuiltinHarness, ContractId, DialectLineage, EventId, HarnessId, SnapshotId,
};
pub use raw::RawInvocation;

pub use camino::{Utf8Path, Utf8PathBuf};

/// Convenience result alias.
pub type Result<T> = std::result::Result<T, HookkitError>;
