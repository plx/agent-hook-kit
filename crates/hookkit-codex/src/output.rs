use serde::Serialize;

/// Top-level Codex hook output.
#[derive(Debug, Clone)]
pub enum CodexHookOutput {
    /// No output — allow/continue with empty stdout and exit 0.
    Empty,
    /// Structured JSON output on stdout, exit 0.
    /// Codex Stop requires JSON on stdout when exiting 0.
    Json(OutputEnvelope),
    /// Blocking deny — message on stderr, exit 2.
    BlockingDeny { stderr: String },
}

/// The JSON envelope written to stdout for Codex hooks.
///
/// Codex currently supports a narrower output surface than Claude.
/// The builders make the supported path ergonomic and the unsupported
/// path explicit via `UnsupportedCapability` errors.
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

    /// Deny a PreToolUse — the documented supported path.
    pub fn deny(reason: impl Into<String>) -> Self {
        Self {
            decision: Some("deny".to_string()),
            reason: Some(reason.into()),
            ..Default::default()
        }
    }

    /// Continue the session when a Stop event fires (block the stop).
    pub fn stop_continue(reason: impl Into<String>) -> Self {
        Self {
            decision: Some("block".to_string()),
            reason: Some(reason.into()),
            ..Default::default()
        }
    }
}

/// Attempt to build a Codex output that uses an unsupported capability.
///
/// Returns an `UnsupportedCapability` error with context about what is
/// not supported and why.
pub fn unsupported_allow() -> hookkit_core::HookkitError {
    hookkit_core::HookkitError::UnsupportedCapability {
        harness: hookkit_core::Harness::Codex,
        event: hookkit_core::HookEventKey::PreToolUse,
        capability: "allow (PreToolUse allow is parsed but not supported by Codex — fails open)",
    }
}

pub fn unsupported_updated_input() -> hookkit_core::HookkitError {
    hookkit_core::HookkitError::UnsupportedCapability {
        harness: hookkit_core::Harness::Codex,
        event: hookkit_core::HookEventKey::PreToolUse,
        capability: "updatedInput (not supported by Codex)",
    }
}

pub fn unsupported_additional_context() -> hookkit_core::HookkitError {
    hookkit_core::HookkitError::UnsupportedCapability {
        harness: hookkit_core::Harness::Codex,
        event: hookkit_core::HookEventKey::PostToolUse,
        capability: "additionalContext (not supported by Codex)",
    }
}
