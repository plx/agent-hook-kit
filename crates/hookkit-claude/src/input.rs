use hookkit_core::RawPayload;
use serde::{Deserialize, Serialize};

/// Top-level parsed Claude hook input.
#[derive(Debug, Clone)]
pub enum ClaudeHookInput {
    SessionStart(SessionStart),
    UserPromptSubmit(UserPromptSubmit),
    PreToolUse(PreToolUse),
    PostToolUse(PostToolUse),
    Stop(Stop),
    Unknown { event_name: String, raw: RawPayload },
}

/// Common top-level fields present in all Claude hook inputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommonFields {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<String>,
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
pub struct UserPromptSubmit {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreToolUse {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_input: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostToolUse {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_input: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stop {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_assistant_message: Option<String>,
}

/// Parse a Claude hook input from a JSON value.
pub fn parse(value: &serde_json::Value) -> hookkit_core::Result<ClaudeHookInput> {
    let event_name = value
        .get("hookEventName")
        .and_then(|v| v.as_str())
        .ok_or(hookkit_core::HookkitError::MissingHookEventName)?;

    match event_name {
        "SessionStart" => {
            let ev: SessionStart = serde_json::from_value(value.clone()).map_err(|e| {
                hookkit_core::HookkitError::ParseFailure {
                    harness: hookkit_core::Harness::Claude,
                    event_name: event_name.to_string(),
                    source: e,
                }
            })?;
            Ok(ClaudeHookInput::SessionStart(ev))
        }
        "UserPromptSubmit" => {
            let ev: UserPromptSubmit = serde_json::from_value(value.clone()).map_err(|e| {
                hookkit_core::HookkitError::ParseFailure {
                    harness: hookkit_core::Harness::Claude,
                    event_name: event_name.to_string(),
                    source: e,
                }
            })?;
            Ok(ClaudeHookInput::UserPromptSubmit(ev))
        }
        "PreToolUse" => {
            let ev: PreToolUse = serde_json::from_value(value.clone()).map_err(|e| {
                hookkit_core::HookkitError::ParseFailure {
                    harness: hookkit_core::Harness::Claude,
                    event_name: event_name.to_string(),
                    source: e,
                }
            })?;
            Ok(ClaudeHookInput::PreToolUse(ev))
        }
        "PostToolUse" => {
            let ev: PostToolUse = serde_json::from_value(value.clone()).map_err(|e| {
                hookkit_core::HookkitError::ParseFailure {
                    harness: hookkit_core::Harness::Claude,
                    event_name: event_name.to_string(),
                    source: e,
                }
            })?;
            Ok(ClaudeHookInput::PostToolUse(ev))
        }
        "Stop" => {
            let ev: Stop = serde_json::from_value(value.clone()).map_err(|e| {
                hookkit_core::HookkitError::ParseFailure {
                    harness: hookkit_core::Harness::Claude,
                    event_name: event_name.to_string(),
                    source: e,
                }
            })?;
            Ok(ClaudeHookInput::Stop(ev))
        }
        _ => Ok(ClaudeHookInput::Unknown {
            event_name: event_name.to_string(),
            raw: RawPayload::from(value.clone()),
        }),
    }
}
