use serde::Serialize;

/// Top-level Claude hook output.
#[derive(Debug, Clone)]
pub enum ClaudeHookOutput {
    /// No output — allow/continue with empty stdout and exit 0.
    Empty,
    /// Structured JSON output on stdout, exit 0.
    Json(OutputEnvelope),
    /// Blocking error — message on stderr, exit 2.
    BlockingError { stderr: String },
}

/// The JSON envelope written to stdout for Claude hooks.
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_specific_output: Option<serde_json::Value>,
}

impl OutputEnvelope {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a "block" output for UserPromptSubmit / PostToolUse / Stop.
    pub fn block(reason: impl Into<String>) -> Self {
        Self {
            decision: Some("block".to_string()),
            reason: Some(reason.into()),
            ..Default::default()
        }
    }

    /// Build an "allow" output.
    pub fn allow() -> Self {
        Self {
            decision: Some("allow".to_string()),
            ..Default::default()
        }
    }

    /// Build output with additional context for the model.
    pub fn with_context(message: impl Into<String>) -> Self {
        Self {
            hook_specific_output: Some(serde_json::json!({
                "additionalContext": message.into()
            })),
            ..Default::default()
        }
    }

    /// Build a PreToolUse deny via hookSpecificOutput.
    pub fn pre_tool_deny(reason: impl Into<String>) -> Self {
        Self {
            hook_specific_output: Some(serde_json::json!({
                "permissionDecision": {
                    "decision": "deny",
                    "reason": reason.into()
                }
            })),
            ..Default::default()
        }
    }

    /// Build a Stop continue response.
    pub fn stop_continue(reason: impl Into<String>) -> Self {
        Self {
            decision: Some("block".to_string()),
            reason: Some(reason.into()),
            ..Default::default()
        }
    }
}
