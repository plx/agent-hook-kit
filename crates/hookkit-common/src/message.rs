//! Message intent helpers.
//!
//! These types help hook authors answer:
//! - Is this for the user only?
//! - For the agent/model only?
//! - Or for both via different channels?

use serde::Serialize;
use std::path::PathBuf;

/// Intended audience for a message emitted by common hook logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageAudience {
    User,
    Agent,
    Both,
}

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

/// Severity level for agent-facing feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackSeverity {
    Info,
    Warning,
    Error,
}

/// Feedback for the agent: a concise corrective instruction or suggestion.
#[derive(Debug, Clone)]
pub struct AgentFeedback {
    pub text: String,
    pub severity: FeedbackSeverity,
}

impl AgentFeedback {
    pub fn new(text: impl Into<String>) -> Self {
        Self::warning(text)
    }

    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            severity: FeedbackSeverity::Info,
        }
    }

    pub fn warning(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            severity: FeedbackSeverity::Warning,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            severity: FeedbackSeverity::Error,
        }
    }
}

/// A diagnostic artifact produced by a hook.
#[derive(Debug, Clone)]
pub struct DiagnosticArtifact {
    pub absolute_path: PathBuf,
    pub project_relative_path: Option<PathBuf>,
    pub media_type: String,
    pub summary: Option<String>,
}

impl DiagnosticArtifact {
    pub fn new(absolute_path: impl Into<PathBuf>, media_type: impl Into<String>) -> Self {
        Self {
            absolute_path: absolute_path.into(),
            project_relative_path: None,
            media_type: media_type.into(),
            summary: None,
        }
    }

    pub fn with_project_relative_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.project_relative_path = Some(path.into());
        self
    }

    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }
}

/// User-visible diagnostic text plus an optional artifact reference.
#[derive(Debug, Clone)]
pub struct DiagnosticReport {
    pub title: String,
    pub text: String,
    pub artifact: Option<DiagnosticArtifact>,
}

impl DiagnosticReport {
    pub fn new(title: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            text: text.into(),
            artifact: None,
        }
    }

    pub fn with_artifact(mut self, artifact: DiagnosticArtifact) -> Self {
        self.artifact = Some(artifact);
        self
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
