use hookkit_core::RawPayload;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Top-level parsed Claude hook input.
#[derive(Debug, Clone)]
pub enum ClaudeHookInput {
    SessionStart(SessionStart),
    UserPromptSubmit(UserPromptSubmit),
    PreToolUse(PreToolUse),
    PostToolUse(PostToolUse),
    PostToolUseFailure(PostToolUseFailure),
    PermissionDenied(PermissionDenied),
    Stop(Stop),
    Notification(Notification),
    SessionEnd(SessionEnd),
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
    pub extra: BTreeMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Tool input types
// ---------------------------------------------------------------------------

/// Typed tool inputs for well-known Claude tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ClaudeToolInput {
    Bash(BashToolInput),
    Write(WriteToolInput),
    Edit(EditToolInput),
    Unknown(serde_json::Value),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BashToolInput {
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteToolInput {
    pub file_path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditToolInput {
    pub file_path: String,
    pub old_string: String,
    pub new_string: String,
}

/// Attempt to parse a tool input value into a typed variant based on tool name.
pub fn parse_tool_input(tool_name: &str, value: &serde_json::Value) -> ClaudeToolInput {
    match tool_name {
        "Bash" => serde_json::from_value(value.clone())
            .map(ClaudeToolInput::Bash)
            .unwrap_or_else(|_| ClaudeToolInput::Unknown(value.clone())),
        "Write" => serde_json::from_value(value.clone())
            .map(ClaudeToolInput::Write)
            .unwrap_or_else(|_| ClaudeToolInput::Unknown(value.clone())),
        "Edit" => serde_json::from_value(value.clone())
            .map(ClaudeToolInput::Edit)
            .unwrap_or_else(|_| ClaudeToolInput::Unknown(value.clone())),
        _ => ClaudeToolInput::Unknown(value.clone()),
    }
}

// ---------------------------------------------------------------------------
// Event types
// ---------------------------------------------------------------------------

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

impl PreToolUse {
    /// Parse the tool input into a typed variant if possible.
    pub fn typed_tool_input(&self) -> Option<ClaudeToolInput> {
        let name = self.tool_name.as_deref()?;
        let input = self.tool_input.as_ref()?;
        Some(parse_tool_input(name, input))
    }
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

impl PostToolUse {
    /// Parse the tool input into a typed variant if possible.
    pub fn typed_tool_input(&self) -> Option<ClaudeToolInput> {
        let name = self.tool_name.as_deref()?;
        let input = self.tool_input.as_ref()?;
        Some(parse_tool_input(name, input))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostToolUseFailure {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_input: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionDenied {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_input: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_decision: Option<serde_json::Value>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEnd {
    #[serde(flatten)]
    pub common: CommonFields,
}

// ---------------------------------------------------------------------------
// Parser dispatch
// ---------------------------------------------------------------------------

/// Parse a Claude hook input from a JSON value.
pub fn parse(value: &serde_json::Value) -> hookkit_core::Result<ClaudeHookInput> {
    let event_name = value
        .get("hookEventName")
        .and_then(|v| v.as_str())
        .ok_or(hookkit_core::HookkitError::MissingHookEventName)?;

    let mk_err = |e: serde_json::Error| hookkit_core::HookkitError::ParseFailure {
        harness: hookkit_core::Harness::Claude,
        event_name: event_name.to_string(),
        source: e,
    };

    match event_name {
        "SessionStart" => Ok(ClaudeHookInput::SessionStart(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "UserPromptSubmit" => Ok(ClaudeHookInput::UserPromptSubmit(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "PreToolUse" => Ok(ClaudeHookInput::PreToolUse(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "PostToolUse" => Ok(ClaudeHookInput::PostToolUse(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "PostToolUseFailure" => Ok(ClaudeHookInput::PostToolUseFailure(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "PermissionDenied" => Ok(ClaudeHookInput::PermissionDenied(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "Stop" => Ok(ClaudeHookInput::Stop(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "Notification" => Ok(ClaudeHookInput::Notification(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "SessionEnd" => Ok(ClaudeHookInput::SessionEnd(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        _ => Ok(ClaudeHookInput::Unknown {
            event_name: event_name.to_string(),
            raw: RawPayload::from(value.clone()),
        }),
    }
}
