use hookkit_core::RawPayload;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Top-level parsed Gemini hook input.
#[derive(Debug, Clone)]
pub enum GeminiHookInput {
    // Phase 1 events
    SessionStart(SessionStart),
    SessionEnd(SessionEnd),
    BeforeAgent(BeforeAgent),
    AfterAgent(AfterAgent),
    BeforeTool(BeforeTool),
    AfterTool(AfterTool),
    Notification(Notification),
    PreCompress(PreCompress),
    // Phase 5: Model-layer events
    BeforeModel(BeforeModel),
    AfterModel(AfterModel),
    BeforeToolSelection(BeforeToolSelection),
    Unknown { event_name: String, raw: RawPayload },
}

/// Common top-level fields present in all Gemini hook inputs.
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
// Tool-related types
// ---------------------------------------------------------------------------

/// Typed tool inputs for well-known Gemini tools.
#[derive(Debug, Clone)]
pub enum GeminiToolInput {
    Shell(ShellToolInput),
    Unknown {
        tool_name: String,
        raw: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellToolInput {
    pub command: Vec<String>,
}

/// Attempt to parse a Gemini tool input.
pub fn parse_tool_input(tool_name: &str, value: &serde_json::Value) -> GeminiToolInput {
    match tool_name {
        "shell" => serde_json::from_value(value.clone())
            .map(GeminiToolInput::Shell)
            .unwrap_or_else(|_| GeminiToolInput::Unknown {
                tool_name: tool_name.to_string(),
                raw: value.clone(),
            }),
        _ => GeminiToolInput::Unknown {
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

impl BeforeTool {
    /// Parse the tool input into a typed variant if possible.
    pub fn typed_tool_input(&self) -> Option<GeminiToolInput> {
        let name = self.tool_name.as_deref()?;
        let input = self.tool_input.as_ref()?;
        Some(parse_tool_input(name, input))
    }
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

impl AfterTool {
    /// Parse the tool input into a typed variant if possible.
    pub fn typed_tool_input(&self) -> Option<GeminiToolInput> {
        let name = self.tool_name.as_deref()?;
        let input = self.tool_input.as_ref()?;
        Some(parse_tool_input(name, input))
    }
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

// ---------------------------------------------------------------------------
// Phase 5: Model-layer types
// ---------------------------------------------------------------------------

/// Typed LLM request for model-layer hooks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messages: Option<Vec<serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Typed LLM response for model-layer hooks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<serde_json::Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BeforeModel {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_request: Option<serde_json::Value>,
}

impl BeforeModel {
    /// Parse the LLM request into a typed struct.
    pub fn typed_llm_request(&self) -> Option<Result<LlmRequest, serde_json::Error>> {
        self.llm_request
            .as_ref()
            .map(|v| serde_json::from_value(v.clone()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AfterModel {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_request: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_response: Option<serde_json::Value>,
}

impl AfterModel {
    /// Parse the LLM response into a typed struct.
    pub fn typed_llm_response(&self) -> Option<Result<LlmResponse, serde_json::Error>> {
        self.llm_response
            .as_ref()
            .map(|v| serde_json::from_value(v.clone()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BeforeToolSelection {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_tools: Option<Vec<serde_json::Value>>,
}

// ---------------------------------------------------------------------------
// Parser dispatch
// ---------------------------------------------------------------------------

/// Parse a Gemini hook input from a JSON value.
pub fn parse(value: &serde_json::Value) -> hookkit_core::Result<GeminiHookInput> {
    let event_name = value
        .get("hookEventName")
        .or_else(|| value.get("hook_event_name"))
        .and_then(|v| v.as_str())
        .ok_or(hookkit_core::HookkitError::MissingHookEventName)?;

    let mk_err = |e: serde_json::Error| hookkit_core::HookkitError::ParseFailure {
        harness: hookkit_core::Harness::Gemini,
        event_name: event_name.to_string(),
        source: e,
    };

    match event_name {
        "SessionStart" => Ok(GeminiHookInput::SessionStart(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "SessionEnd" => Ok(GeminiHookInput::SessionEnd(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "BeforeAgent" => Ok(GeminiHookInput::BeforeAgent(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "AfterAgent" => Ok(GeminiHookInput::AfterAgent(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "BeforeTool" => Ok(GeminiHookInput::BeforeTool(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "AfterTool" => Ok(GeminiHookInput::AfterTool(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "Notification" => Ok(GeminiHookInput::Notification(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "PreCompress" => Ok(GeminiHookInput::PreCompress(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        // Phase 5: Model-layer events
        "BeforeModel" => Ok(GeminiHookInput::BeforeModel(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "AfterModel" => Ok(GeminiHookInput::AfterModel(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "BeforeToolSelection" => Ok(GeminiHookInput::BeforeToolSelection(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        _ => Ok(GeminiHookInput::Unknown {
            event_name: event_name.to_string(),
            raw: RawPayload::from(value.clone()),
        }),
    }
}
