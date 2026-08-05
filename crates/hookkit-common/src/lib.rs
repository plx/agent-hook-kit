//! Cross-harness wrapper enums and message/output helpers.
#![deny(missing_docs)]
//!
//! This crate provides common wrapper enums that align semantically
//! equivalent events across Claude Code, Codex, and Antigravity while preserving
//! lossless access to the underlying native types.

pub mod aligned;
pub mod message;

pub use aligned::{
    PostToolUseCommandEnvironment, PostToolUseInput, PostToolUseOutput,
    PreToolUseCommandEnvironment, PreToolUseInput, PreToolUseOutput, ToolInputRef,
    TurnCompletionCommandEnvironment, TurnCompletionInput, TurnCompletionOutput,
};
pub use message::{
    AgentFeedback, DiagnosticArtifact, DiagnosticReport, FeedbackSeverity, MessageAudience,
    NoticeLevel, UserNotice,
};
