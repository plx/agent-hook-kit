//! Core types and error model for hookkit.

mod error;
mod harness;
pub mod json_helpers;
mod raw;

pub use error::HookkitError;
pub use harness::Harness;
pub use raw::RawPayload;

pub use camino::{Utf8Path, Utf8PathBuf};

/// Convenience result alias.
pub type Result<T> = std::result::Result<T, HookkitError>;

/// Hook event key for internal categorization.
///
/// This is only for capability/validation lookups. Native event enums
/// carry more precise information.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HookEventKey {
    SessionStart,
    SessionEnd,
    PromptSubmit,
    PromptExpansion,
    PreToolUse,
    PermissionRequest,
    PermissionDenied,
    PostToolUse,
    PostToolUseFailure,
    PostToolBatch,
    Stop,
    StopFailure,
    Notification,
    SubagentStart,
    SubagentStop,
    TaskCreated,
    TaskCompleted,
    TeammateIdle,
    InstructionsLoaded,
    ConfigChange,
    CwdChanged,
    FileChanged,
    WorktreeCreate,
    WorktreeRemove,
    PreCompact,
    PostCompact,
    Elicitation,
    ElicitationResult,
    BeforeModel,
    AfterModel,
    BeforeToolSelection,
    PreCompress,
    Other(String),
}
