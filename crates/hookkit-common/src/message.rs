//! Message intent helpers.
//!
//! These types help hook authors answer:
//! - Is this for the user only?
//! - For the agent/model only?
//! - Or for both via different channels?

use serde::Serialize;

/// A notice intended for the human user.
#[derive(Debug, Clone, Serialize)]
pub struct UserNotice {
    pub text: String,
    pub level: NoticeLevel,
}

/// Severity level for user-facing notices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

impl UserNotice {
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            level: NoticeLevel::Info,
        }
    }

    pub fn warning(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            level: NoticeLevel::Warning,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            level: NoticeLevel::Error,
        }
    }
}

/// Context injected for the agent/model to see.
#[derive(Debug, Clone)]
pub struct AgentContext {
    pub lines: Vec<String>,
}

impl AgentContext {
    pub fn new() -> Self {
        Self { lines: Vec::new() }
    }

    pub fn push(mut self, line: impl Into<String>) -> Self {
        self.lines.push(line.into());
        self
    }

    pub fn to_string_block(&self) -> String {
        self.lines.join("\n")
    }
}

impl Default for AgentContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Feedback for the agent — a corrective instruction or suggestion.
#[derive(Debug, Clone)]
pub struct AgentFeedback {
    pub message: String,
}

impl AgentFeedback {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Request to invoke a tool as a tail call after the current hook.
#[derive(Debug, Clone, Serialize)]
pub struct TailToolCall {
    pub name: String,
    pub args: serde_json::Value,
}

impl TailToolCall {
    pub fn new(name: impl Into<String>, args: serde_json::Value) -> Self {
        Self {
            name: name.into(),
            args,
        }
    }
}
