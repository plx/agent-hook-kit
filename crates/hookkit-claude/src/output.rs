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
///
/// Different events use different subsets of these fields. The builders
/// below help construct valid combinations per event type.
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

    // --- UserPromptSubmit / PostToolUse / Stop: top-level decision ---

    /// Block — used for UserPromptSubmit, PostToolUse, or Stop to reject the action.
    pub fn block(reason: impl Into<String>) -> Self {
        Self {
            decision: Some("block".to_string()),
            reason: Some(reason.into()),
            ..Default::default()
        }
    }

    /// Allow — explicit approval.
    pub fn allow() -> Self {
        Self {
            decision: Some("allow".to_string()),
            ..Default::default()
        }
    }

    // --- PreToolUse: hookSpecificOutput.permissionDecision ---

    /// Deny a PreToolUse via hookSpecificOutput.permissionDecision.
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

    /// Allow a PreToolUse via hookSpecificOutput.permissionDecision.
    pub fn pre_tool_allow() -> Self {
        Self {
            hook_specific_output: Some(serde_json::json!({
                "permissionDecision": {
                    "decision": "allow"
                }
            })),
            ..Default::default()
        }
    }

    /// Ask user permission for a PreToolUse.
    pub fn pre_tool_ask(reason: impl Into<String>) -> Self {
        Self {
            hook_specific_output: Some(serde_json::json!({
                "permissionDecision": {
                    "decision": "ask",
                    "reason": reason.into()
                }
            })),
            ..Default::default()
        }
    }

    /// Deny a PreToolUse and provide rewritten input.
    pub fn pre_tool_deny_with_updated_input(
        reason: impl Into<String>,
        updated_input: serde_json::Value,
    ) -> Self {
        Self {
            hook_specific_output: Some(serde_json::json!({
                "permissionDecision": {
                    "decision": "deny",
                    "reason": reason.into(),
                    "updatedInput": updated_input
                }
            })),
            ..Default::default()
        }
    }

    // --- Context injection ---

    /// Inject additional context visible to the model.
    pub fn with_context(message: impl Into<String>) -> Self {
        Self {
            hook_specific_output: Some(serde_json::json!({
                "additionalContext": message.into()
            })),
            ..Default::default()
        }
    }

    // --- Stop: continue/stop behavior ---

    /// Continue the session when a Stop event fires (block the stop).
    pub fn stop_continue(reason: impl Into<String>) -> Self {
        Self {
            decision: Some("block".to_string()),
            reason: Some(reason.into()),
            ..Default::default()
        }
    }

    // --- PermissionDenied: retry ---

    /// Retry a denied permission.
    pub fn permission_denied_retry() -> Self {
        Self {
            hook_specific_output: Some(serde_json::json!({
                "retry": true
            })),
            ..Default::default()
        }
    }

    // --- Fluent setters ---

    pub fn with_system_message(mut self, msg: impl Into<String>) -> Self {
        self.system_message = Some(msg.into());
        self
    }

    pub fn with_suppress_output(mut self, suppress: bool) -> Self {
        self.suppress_output = Some(suppress);
        self
    }

    pub fn with_continue(mut self, cont: bool) -> Self {
        self.continue_session = Some(cont);
        self
    }

    pub fn with_stop_reason(mut self, reason: impl Into<String>) -> Self {
        self.stop_reason = Some(reason.into());
        self
    }
}
