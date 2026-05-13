//! Cross-harness wrapper enums and semantic helpers.
//!
//! This crate provides common wrapper enums that align semantically
//! equivalent events across Claude, Codex, and Gemini, while preserving
//! lossless access to the underlying native types.

pub mod input;
pub mod message;
pub mod output;
pub mod semantic;

pub use input::CommonHookInput;
pub use message::{
    AgentFeedback, DiagnosticArtifact, DiagnosticReport, FeedbackSeverity, MessageAudience,
    NoticeLevel, UserNotice,
};
pub use output::{
    CommonHookOutput, IntentRequirement, LoweredPostToolUseOutput, LoweringAction, LoweringPolicy,
    LoweringWarning, SessionControl,
};
pub use semantic::{
    CommonEventMeta, CommonPostToolUseNative, CommonPostToolUseView, CommonToolResultView,
    CommonToolUseView, DerivedConfidence, DerivedSource, PathCandidate, PathRole,
    ToolExecutionStatus,
};

#[cfg(test)]
mod tests;
