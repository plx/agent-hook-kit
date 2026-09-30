//! Message intent vocabulary.
//!
//! These value types let hook logic record who a message is for before a
//! harness-specific adapter lowers it:
//! - a [`UserNotice`] is for the user only;
//! - [`AgentContext`] and [`AgentFeedback`] are for the agent only;
//! - a [`DiagnosticReport`] carries user-visible text plus an optional
//!   [`DiagnosticArtifact`] the agent can be pointed at.
//!
//! The types carry no lowering of their own. Each harness has different
//! user and agent channels (for example Claude Code's `systemMessage` versus
//! `additionalContext`), so the adapter that emits the native output chooses
//! the channel.

use serde::Serialize;
use std::path::PathBuf;

/// Intended audience for a message emitted by common hook logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum MessageAudience {
    /// Deliver only through a human-facing channel.
    User,
    /// Deliver only through an agent/model-facing channel.
    Agent,
    /// Deliver through both channels, potentially with different rendering.
    Both,
}

/// A notice intended for the human user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UserNotice {
    /// Human-readable notice text.
    pub text: String,
    /// Notice severity.
    pub level: NoticeLevel,
}

/// Severity level for user-facing notices.
///
/// Levels may be added in later releases, so matches outside this crate need
/// a wildcard arm.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NoticeLevel {
    /// Informational notice.
    Info,
    /// Warning that may require attention.
    Warning,
    /// Error notice describing a failed operation.
    Error,
}

impl UserNotice {
    /// Creates an informational notice.
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            level: NoticeLevel::Info,
        }
    }

    /// Creates a warning notice.
    pub fn warning(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            level: NoticeLevel::Warning,
        }
    }

    /// Creates an error notice.
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            level: NoticeLevel::Error,
        }
    }
}

/// Context injected for the agent/model to see.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AgentContext {
    /// Ordered context lines.
    pub lines: Vec<String>,
}

impl AgentContext {
    /// Creates empty agent context.
    pub fn new() -> Self {
        Self { lines: Vec::new() }
    }

    /// Appends one line and returns the updated context.
    pub fn push(mut self, line: impl Into<String>) -> Self {
        self.lines.push(line.into());
        self
    }

    /// Joins context lines with a single newline and no trailing newline.
    pub fn to_string_block(&self) -> String {
        self.lines.join("\n")
    }
}

/// Severity level for agent-facing feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum FeedbackSeverity {
    /// Informational suggestion.
    Info,
    /// Corrective warning.
    Warning,
    /// Error that prevented the requested operation.
    Error,
}

/// Feedback for the agent: a concise corrective instruction or suggestion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentFeedback {
    /// Concise instruction or suggestion for the agent.
    pub text: String,
    /// Feedback severity.
    pub severity: FeedbackSeverity,
}

impl AgentFeedback {
    /// Creates warning-level feedback.
    ///
    /// This is equivalent to [`Self::warning`] and preserves the original API's
    /// corrective-feedback default.
    pub fn new(text: impl Into<String>) -> Self {
        Self::warning(text)
    }

    /// Creates informational feedback.
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            severity: FeedbackSeverity::Info,
        }
    }

    /// Creates warning-level feedback.
    pub fn warning(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            severity: FeedbackSeverity::Warning,
        }
    }

    /// Creates error-level feedback.
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            severity: FeedbackSeverity::Error,
        }
    }
}

/// A diagnostic artifact produced by a hook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticArtifact {
    /// Filesystem location of the artifact.
    ///
    /// Callers should supply an absolute path so the agent can open the
    /// artifact from any working directory; the value is not validated.
    pub absolute_path: PathBuf,
    /// Optional path relative to the project root for portable display.
    pub project_relative_path: Option<PathBuf>,
    /// Artifact media type, such as `text/plain` or `application/json`.
    pub media_type: String,
    /// Optional short human-readable description.
    pub summary: Option<String>,
}

impl DiagnosticArtifact {
    /// Creates an artifact reference with a path and media type.
    ///
    /// Pass an absolute path: a relative one is kept verbatim and resolves
    /// against whatever directory later reads it.
    pub fn new(absolute_path: impl Into<PathBuf>, media_type: impl Into<String>) -> Self {
        Self {
            absolute_path: absolute_path.into(),
            project_relative_path: None,
            media_type: media_type.into(),
            summary: None,
        }
    }

    /// Adds the equivalent project-relative path.
    pub fn with_project_relative_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.project_relative_path = Some(path.into());
        self
    }

    /// Adds a short human-readable artifact summary.
    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }
}

/// User-visible diagnostic text plus an optional artifact reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticReport {
    /// Short report title.
    pub title: String,
    /// User-visible diagnostic body.
    pub text: String,
    /// Optional artifact containing complete or structured diagnostics.
    pub artifact: Option<DiagnosticArtifact>,
}

impl DiagnosticReport {
    /// Creates a text-only diagnostic report.
    pub fn new(title: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            text: text.into(),
            artifact: None,
        }
    }

    /// Attaches a diagnostic artifact.
    pub fn with_artifact(mut self, artifact: DiagnosticArtifact) -> Self {
        self.artifact = Some(artifact);
        self
    }
}

/// Request to invoke a tool as a tail call after the current hook.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TailToolCall {
    /// Harness-native tool name.
    pub name: String,
    /// Tool arguments in their native JSON shape.
    pub args: serde_json::Value,
}

impl TailToolCall {
    /// Creates a tail-call request.
    pub fn new(name: impl Into<String>, args: serde_json::Value) -> Self {
        Self {
            name: name.into(),
            args,
        }
    }
}
