use hookkit_core::RawPayload;
use serde::{Deserialize, Serialize};

/// Top-level parsed Codex hook input.
#[derive(Debug, Clone)]
pub enum CodexHookInput {
    SessionStart(SessionStart),
    PreToolUse(PreToolUse),
    PostToolUse(PostToolUse),
    UserPromptSubmit(UserPromptSubmit),
    Stop(Stop),
    Unknown { event_name: String, raw: RawPayload },
}

/// Common top-level fields present in all Codex hook inputs.
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
pub struct UserPromptSubmit {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_prompt: Option<String>,
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

/// Parse a Codex hook input from a JSON value.
pub fn parse(value: &serde_json::Value) -> hookkit_core::Result<CodexHookInput> {
    let event_name = value
        .get("hookEventName")
        .and_then(|v| v.as_str())
        .ok_or(hookkit_core::HookkitError::MissingHookEventName)?;

    match event_name {
        "SessionStart" => {
            let ev: SessionStart = serde_json::from_value(value.clone()).map_err(|e| {
                hookkit_core::HookkitError::ParseFailure {
                    harness: hookkit_core::Harness::Codex,
                    event_name: event_name.to_string(),
                    source: e,
                }
            })?;
            Ok(CodexHookInput::SessionStart(ev))
        }
        "PreToolUse" => {
            let ev: PreToolUse = serde_json::from_value(value.clone()).map_err(|e| {
                hookkit_core::HookkitError::ParseFailure {
                    harness: hookkit_core::Harness::Codex,
                    event_name: event_name.to_string(),
                    source: e,
                }
            })?;
            Ok(CodexHookInput::PreToolUse(ev))
        }
        "PostToolUse" => {
            let ev: PostToolUse = serde_json::from_value(value.clone()).map_err(|e| {
                hookkit_core::HookkitError::ParseFailure {
                    harness: hookkit_core::Harness::Codex,
                    event_name: event_name.to_string(),
                    source: e,
                }
            })?;
            Ok(CodexHookInput::PostToolUse(ev))
        }
        "UserPromptSubmit" => {
            let ev: UserPromptSubmit = serde_json::from_value(value.clone()).map_err(|e| {
                hookkit_core::HookkitError::ParseFailure {
                    harness: hookkit_core::Harness::Codex,
                    event_name: event_name.to_string(),
                    source: e,
                }
            })?;
            Ok(CodexHookInput::UserPromptSubmit(ev))
        }
        "Stop" => {
            let ev: Stop = serde_json::from_value(value.clone()).map_err(|e| {
                hookkit_core::HookkitError::ParseFailure {
                    harness: hookkit_core::Harness::Codex,
                    event_name: event_name.to_string(),
                    source: e,
                }
            })?;
            Ok(CodexHookInput::Stop(ev))
        }
        _ => Ok(CodexHookInput::Unknown {
            event_name: event_name.to_string(),
            raw: RawPayload::from(value.clone()),
        }),
    }
}
