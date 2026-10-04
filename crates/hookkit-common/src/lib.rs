//! Cross-harness wrapper enums and message/output helpers.
#![deny(missing_docs)]
//!
//! This crate provides common wrapper enums that align semantically
//! equivalent events across Claude Code, Codex, and Antigravity while preserving
//! lossless access to the underlying native types.

pub mod aligned;
pub mod message;

pub use aligned::{
    PermissionRequestCommandEnvironment, PermissionRequestInput, PermissionRequestOutput,
    PostCompactCommandEnvironment, PostCompactInput, PostCompactOutput,
    PostToolUseCommandEnvironment, PostToolUseInput, PostToolUseOutput,
    PreCompactCommandEnvironment, PreCompactInput, PreCompactOutput, PreToolUseCommandEnvironment,
    PreToolUseInput, PreToolUseOutput, RewriteApproval, SessionEndCommandEnvironment,
    SessionEndInput, SessionEndOutput, SessionStartCommandEnvironment, SessionStartInput,
    SessionStartOutput, SubagentStartCommandEnvironment, SubagentStartInput, SubagentStartOutput,
    SubagentStopCommandEnvironment, SubagentStopInput, SubagentStopOutput, ToolInputRef,
    TurnCompletionCommandEnvironment, TurnCompletionInput, TurnCompletionOutput,
    UserPromptSubmitCommandEnvironment, UserPromptSubmitInput, UserPromptSubmitOutput,
};
pub use message::{
    AgentContext, AgentFeedback, DiagnosticArtifact, DiagnosticReport, FeedbackSeverity,
    MessageAudience, NoticeLevel, TailToolCall, UserNotice,
};
