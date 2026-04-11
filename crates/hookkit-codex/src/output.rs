use serde::Serialize;

/// Top-level Codex hook output.
#[derive(Debug, Clone)]
pub enum CodexHookOutput {
    /// No output — allow/continue with empty stdout and exit 0.
    Empty,
    /// Structured JSON output on stdout, exit 0.
    Json(OutputEnvelope),
    /// Blocking deny — message on stderr, exit 2.
    BlockingDeny { stderr: String },
}

/// The JSON envelope written to stdout for Codex hooks.
///
/// Codex currently supports a narrower output surface than Claude.
/// The builders make the supported path ergonomic.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputEnvelope {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(rename = "continue", skip_serializing_if = "Option::is_none")]
    pub continue_session: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppress_output: Option<bool>,
}

impl OutputEnvelope {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a deny response for PreToolUse.
    pub fn deny(reason: impl Into<String>) -> Self {
        Self {
            decision: Some("deny".to_string()),
            reason: Some(reason.into()),
            ..Default::default()
        }
    }

    /// Build a Stop continue response (keep the session going).
    pub fn stop_continue(reason: impl Into<String>) -> Self {
        Self {
            decision: Some("block".to_string()),
            reason: Some(reason.into()),
            ..Default::default()
        }
    }
}
