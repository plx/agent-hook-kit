use serde::Serialize;

/// Top-level Gemini hook output.
#[derive(Debug, Clone)]
pub enum GeminiHookOutput {
    /// No output — allow/continue with empty stdout and exit 0.
    Empty,
    /// Structured JSON output on stdout, exit 0.
    Json(OutputEnvelope),
    /// Blocking error — message on stderr, exit 2.
    BlockingError { stderr: String },
}

/// The JSON envelope written to stdout for Gemini hooks.
///
/// Gemini has dedicated output semantics for different event types.
/// The builders below help construct valid combinations.
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
    pub hook_specific_output: Option<serde_json::Value>,
}

impl OutputEnvelope {
    pub fn new() -> Self {
        Self::default()
    }

    // --- BeforeTool ---

    /// Deny a BeforeTool request.
    pub fn deny(reason: impl Into<String>) -> Self {
        Self {
            decision: Some("deny".to_string()),
            reason: Some(reason.into()),
            ..Default::default()
        }
    }

    /// Rewrite the tool input for a BeforeTool event.
    pub fn rewrite_tool_input(new_input: serde_json::Value) -> Self {
        Self {
            hook_specific_output: Some(serde_json::json!({
                "tool_input": new_input
            })),
            ..Default::default()
        }
    }

    // --- AfterTool ---

    /// Replace the tool result for an AfterTool event.
    pub fn replace_tool_result(result: serde_json::Value) -> Self {
        Self {
            hook_specific_output: Some(serde_json::json!({
                "tool_response": result
            })),
            ..Default::default()
        }
    }

    /// Append context after a tool call.
    pub fn with_context(message: impl Into<String>) -> Self {
        Self {
            hook_specific_output: Some(serde_json::json!({
                "additionalContext": message.into()
            })),
            ..Default::default()
        }
    }

    // --- AfterAgent ---

    /// Request a retry from AfterAgent.
    pub fn retry(reason: impl Into<String>) -> Self {
        Self {
            decision: Some("retry".to_string()),
            reason: Some(reason.into()),
            ..Default::default()
        }
    }

    /// Stop the agent from AfterAgent.
    pub fn stop(reason: impl Into<String>) -> Self {
        Self {
            decision: Some("stop".to_string()),
            reason: Some(reason.into()),
            ..Default::default()
        }
    }
}
