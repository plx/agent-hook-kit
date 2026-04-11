use hookkit_core::RawPayload;
use serde::{Deserialize, Serialize};

/// Top-level parsed Gemini hook input.
#[derive(Debug, Clone)]
pub enum GeminiHookInput {
    SessionStart(SessionStart),
    SessionEnd(SessionEnd),
    BeforeAgent(BeforeAgent),
    AfterAgent(AfterAgent),
    BeforeTool(BeforeTool),
    AfterTool(AfterTool),
    Notification(Notification),
    PreCompress(PreCompress),
    Unknown { event_name: String, raw: RawPayload },
}

/// Common top-level fields present in all Gemini hook inputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommonFields {
    pub session_id: String,
    pub cwd: String,
    pub hook_event_name: String,
    #[serde(flatten)]
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStart {
    #[serde(flatten)]
    pub common: CommonFields,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEnd {
    #[serde(flatten)]
    pub common: CommonFields,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BeforeAgent {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AfterAgent {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_response: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BeforeTool {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_input: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AfterTool {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_input: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_response: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreCompress {
    #[serde(flatten)]
    pub common: CommonFields,
}

/// Parse a Gemini hook input from a JSON value.
pub fn parse(value: &serde_json::Value) -> hookkit_core::Result<GeminiHookInput> {
    let event_name = value
        .get("hookEventName")
        .and_then(|v| v.as_str())
        .ok_or(hookkit_core::HookkitError::MissingHookEventName)?;

    match event_name {
        "SessionStart" => {
            let ev: SessionStart = serde_json::from_value(value.clone()).map_err(|e| {
                hookkit_core::HookkitError::ParseFailure {
                    harness: hookkit_core::Harness::Gemini,
                    event_name: event_name.to_string(),
                    source: e,
                }
            })?;
            Ok(GeminiHookInput::SessionStart(ev))
        }
        "SessionEnd" => {
            let ev: SessionEnd = serde_json::from_value(value.clone()).map_err(|e| {
                hookkit_core::HookkitError::ParseFailure {
                    harness: hookkit_core::Harness::Gemini,
                    event_name: event_name.to_string(),
                    source: e,
                }
            })?;
            Ok(GeminiHookInput::SessionEnd(ev))
        }
        "BeforeAgent" => {
            let ev: BeforeAgent = serde_json::from_value(value.clone()).map_err(|e| {
                hookkit_core::HookkitError::ParseFailure {
                    harness: hookkit_core::Harness::Gemini,
                    event_name: event_name.to_string(),
                    source: e,
                }
            })?;
            Ok(GeminiHookInput::BeforeAgent(ev))
        }
        "AfterAgent" => {
            let ev: AfterAgent = serde_json::from_value(value.clone()).map_err(|e| {
                hookkit_core::HookkitError::ParseFailure {
                    harness: hookkit_core::Harness::Gemini,
                    event_name: event_name.to_string(),
                    source: e,
                }
            })?;
            Ok(GeminiHookInput::AfterAgent(ev))
        }
        "BeforeTool" => {
            let ev: BeforeTool = serde_json::from_value(value.clone()).map_err(|e| {
                hookkit_core::HookkitError::ParseFailure {
                    harness: hookkit_core::Harness::Gemini,
                    event_name: event_name.to_string(),
                    source: e,
                }
            })?;
            Ok(GeminiHookInput::BeforeTool(ev))
        }
        "AfterTool" => {
            let ev: AfterTool = serde_json::from_value(value.clone()).map_err(|e| {
                hookkit_core::HookkitError::ParseFailure {
                    harness: hookkit_core::Harness::Gemini,
                    event_name: event_name.to_string(),
                    source: e,
                }
            })?;
            Ok(GeminiHookInput::AfterTool(ev))
        }
        "Notification" => {
            let ev: Notification = serde_json::from_value(value.clone()).map_err(|e| {
                hookkit_core::HookkitError::ParseFailure {
                    harness: hookkit_core::Harness::Gemini,
                    event_name: event_name.to_string(),
                    source: e,
                }
            })?;
            Ok(GeminiHookInput::Notification(ev))
        }
        "PreCompress" => {
            let ev: PreCompress = serde_json::from_value(value.clone()).map_err(|e| {
                hookkit_core::HookkitError::ParseFailure {
                    harness: hookkit_core::Harness::Gemini,
                    event_name: event_name.to_string(),
                    source: e,
                }
            })?;
            Ok(GeminiHookInput::PreCompress(ev))
        }
        _ => Ok(GeminiHookInput::Unknown {
            event_name: event_name.to_string(),
            raw: RawPayload::from(value.clone()),
        }),
    }
}
