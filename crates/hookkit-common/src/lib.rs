//! Cross-harness wrapper enums and message/output helpers.
//!
//! This crate provides common wrapper enums that align semantically
//! equivalent events across Claude, Codex, and Gemini, while preserving
//! lossless access to the underlying native types.

pub mod aligned;
pub mod message;

pub use aligned::{PostToolUseInput, PostToolUseOutput};
pub use message::{
    AgentFeedback, DiagnosticArtifact, DiagnosticReport, FeedbackSeverity, MessageAudience,
    NoticeLevel, UserNotice,
};
