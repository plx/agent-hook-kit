use hookkit_core::RawPayload;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    #[serde(alias = "session_id")]
    pub session_id: String,
    pub cwd: String,
    #[serde(alias = "hook_event_name")]
    pub hook_event_name: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Tool input types — Codex currently centers on Bash
// ---------------------------------------------------------------------------

/// Typed tool inputs for Codex tools.
#[derive(Debug, Clone)]
pub enum CodexToolInput {
    Bash(BashToolInput),
    Unknown {
        tool_name: String,
        raw: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BashToolInput {
    pub command: String,
}

/// Attempt to parse a Codex tool input.
pub fn parse_tool_input(tool_name: &str, value: &serde_json::Value) -> CodexToolInput {
    match tool_name {
        "Bash" => serde_json::from_value(value.clone())
            .map(CodexToolInput::Bash)
            .unwrap_or_else(|_| CodexToolInput::Unknown {
                tool_name: tool_name.to_string(),
                raw: value.clone(),
            }),
        _ => CodexToolInput::Unknown {
            tool_name: tool_name.to_string(),
            raw: value.clone(),
        },
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
    pub fn typed_tool_input(&self) -> Option<CodexToolInput> {
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
    pub fn typed_tool_input(&self) -> Option<CodexToolInput> {
        let name = self.tool_name.as_deref()?;
        let input = self.tool_input.as_ref()?;
        Some(parse_tool_input(name, input))
    }
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

// ---------------------------------------------------------------------------
// Parser dispatch
// ---------------------------------------------------------------------------

/// Parse a Codex hook input from a JSON value.
pub fn parse(value: &serde_json::Value) -> hookkit_core::Result<CodexHookInput> {
    let event_name = value
        .get("hookEventName")
        .or_else(|| value.get("hook_event_name"))
        .and_then(|v| v.as_str())
        .ok_or(hookkit_core::HookkitError::MissingHookEventName)?;

    let mk_err = |e: serde_json::Error| hookkit_core::HookkitError::ParseFailure {
        harness: hookkit_core::Harness::Codex,
        event_name: event_name.to_string(),
        source: e,
    };

    match event_name {
        "SessionStart" => Ok(CodexHookInput::SessionStart(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "PreToolUse" => Ok(CodexHookInput::PreToolUse(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "PostToolUse" => Ok(CodexHookInput::PostToolUse(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "UserPromptSubmit" => Ok(CodexHookInput::UserPromptSubmit(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "Stop" => Ok(CodexHookInput::Stop(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        _ => Ok(CodexHookInput::Unknown {
            event_name: event_name.to_string(),
            raw: RawPayload::from(value.clone()),
        }),
    }
}
