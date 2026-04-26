use hookkit_core::RawPayload;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Top-level parsed Claude hook input.
#[derive(Debug, Clone)]
pub enum ClaudeHookInput {
    SessionStart(SessionStart),
    UserPromptSubmit(UserPromptSubmit),
    UserPromptExpansion(UserPromptExpansion),
    PreToolUse(PreToolUse),
    PermissionRequest(PermissionRequest),
    PermissionDenied(PermissionDenied),
    PostToolUse(PostToolUse),
    PostToolUseFailure(PostToolUseFailure),
    PostToolBatch(PostToolBatch),
    Notification(Notification),
    SubagentStart(SubagentStart),
    SubagentStop(SubagentStop),
    TaskCreated(TaskCreated),
    TaskCompleted(TaskCompleted),
    Stop(Stop),
    StopFailure(StopFailure),
    TeammateIdle(TeammateIdle),
    InstructionsLoaded(InstructionsLoaded),
    ConfigChange(ConfigChange),
    CwdChanged(CwdChanged),
    FileChanged(FileChanged),
    WorktreeCreate(WorktreeCreate),
    WorktreeRemove(WorktreeRemove),
    PreCompact(PreCompact),
    PostCompact(PostCompact),
    Elicitation(Elicitation),
    ElicitationResult(ElicitationResult),
    SessionEnd(SessionEnd),
    Unknown { event_name: String, raw: RawPayload },
}

/// Common top-level fields present in all Claude hook inputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommonFields {
    #[serde(alias = "sessionId")]
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "transcriptPath")]
    pub transcript_path: Option<String>,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "permissionMode")]
    pub permission_mode: Option<String>,
    #[serde(alias = "hookEventName")]
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
pub struct SessionStart {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "agentType")]
    pub agent_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPromptSubmit {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "userPrompt")]
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPromptExpansion {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "expansionType")]
    pub expansion_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "commandName")]
    pub command_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "commandArgs")]
    pub command_args: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "commandSource")]
    pub command_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "userPrompt")]
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreToolUse {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "toolName")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "toolInput")]
    pub tool_input: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "toolUseId")]
    pub tool_use_id: Option<String>,
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
pub struct PermissionRequest {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "toolName")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "toolInput")]
    pub tool_input: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "permissionSuggestions")]
    pub permission_suggestions: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionDenied {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "toolName")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "toolInput")]
    pub tool_input: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "toolUseId")]
    pub tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostToolUse {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "toolName")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "toolInput")]
    pub tool_input: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "toolResponse")]
    pub tool_response: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "toolUseId")]
    pub tool_use_id: Option<String>,
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
pub struct PostToolUseFailure {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "toolName")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "toolInput")]
    pub tool_input: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "toolUseId")]
    pub tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "isInterrupt")]
    pub is_interrupt: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostToolBatchCall {
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "toolName")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "toolInput")]
    pub tool_input: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "toolUseId")]
    pub tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "toolResponse")]
    pub tool_response: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostToolBatch {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "toolCalls")]
    pub tool_calls: Option<Vec<PostToolBatchCall>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "notificationType")]
    pub notification_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentStart {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "agentId",
        alias = "subagentId"
    )]
    pub agent_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "agentType",
        alias = "subagentType"
    )]
    pub agent_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentStop {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "stopHookActive")]
    pub stop_hook_active: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "agentId",
        alias = "subagentId"
    )]
    pub agent_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "agentType",
        alias = "subagentType"
    )]
    pub agent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "agentTranscriptPath")]
    pub agent_transcript_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "lastAssistantMessage")]
    pub last_assistant_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCreated {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "taskSubject")]
    pub task_subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "taskDescription")]
    pub task_description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "teammateName")]
    pub teammate_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "teamName")]
    pub team_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCompleted {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "taskSubject")]
    pub task_subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "taskDescription")]
    pub task_description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "teammateName")]
    pub teammate_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "teamName")]
    pub team_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stop {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "stopHookActive")]
    pub stop_hook_active: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "lastAssistantMessage")]
    pub last_assistant_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopFailure {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "errorDetails")]
    pub error_details: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "lastAssistantMessage")]
    pub last_assistant_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeammateIdle {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "teammateName")]
    pub teammate_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "teamName")]
    pub team_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionsLoaded {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "filePath")]
    pub file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "memoryType")]
    pub memory_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "loadReason")]
    pub load_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub globs: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "triggerFilePath")]
    pub trigger_file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "parentFilePath")]
    pub parent_file_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigChange {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "filePath")]
    pub file_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CwdChanged {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "oldCwd")]
    pub old_cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "newCwd")]
    pub new_cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChanged {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "filePath")]
    pub file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "changeType")]
    pub event: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeCreate {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeRemove {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "worktreePath")]
    pub worktree_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreCompact {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "customInstructions")]
    pub custom_instructions: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostCompact {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "compactSummary")]
    pub compact_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Elicitation {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "mcpServerName")]
    pub mcp_server_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "elicitationId")]
    pub elicitation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "requestedSchema")]
    pub requested_schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElicitationResult {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "mcpServerName")]
    pub mcp_server_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "elicitationId")]
    pub elicitation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEnd {
    #[serde(flatten)]
    pub common: CommonFields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

// ---------------------------------------------------------------------------
// Parser dispatch
// ---------------------------------------------------------------------------

/// Parse a Claude hook input from a JSON value.
pub fn parse(value: &serde_json::Value) -> hookkit_core::Result<ClaudeHookInput> {
    let event_name = value
        .get("hookEventName")
        .or_else(|| value.get("hook_event_name"))
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
        "UserPromptExpansion" => Ok(ClaudeHookInput::UserPromptExpansion(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "PreToolUse" => Ok(ClaudeHookInput::PreToolUse(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "PermissionRequest" => Ok(ClaudeHookInput::PermissionRequest(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "PermissionDenied" => Ok(ClaudeHookInput::PermissionDenied(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "PostToolUse" => Ok(ClaudeHookInput::PostToolUse(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "PostToolUseFailure" => Ok(ClaudeHookInput::PostToolUseFailure(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "PostToolBatch" => Ok(ClaudeHookInput::PostToolBatch(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "Notification" => Ok(ClaudeHookInput::Notification(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "SubagentStart" => Ok(ClaudeHookInput::SubagentStart(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "SubagentStop" => Ok(ClaudeHookInput::SubagentStop(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "TaskCreated" => Ok(ClaudeHookInput::TaskCreated(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "TaskCompleted" => Ok(ClaudeHookInput::TaskCompleted(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "Stop" => Ok(ClaudeHookInput::Stop(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "StopFailure" => Ok(ClaudeHookInput::StopFailure(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "TeammateIdle" => Ok(ClaudeHookInput::TeammateIdle(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "InstructionsLoaded" => Ok(ClaudeHookInput::InstructionsLoaded(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "ConfigChange" => Ok(ClaudeHookInput::ConfigChange(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "CwdChanged" => Ok(ClaudeHookInput::CwdChanged(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "FileChanged" => Ok(ClaudeHookInput::FileChanged(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "WorktreeCreate" => Ok(ClaudeHookInput::WorktreeCreate(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "WorktreeRemove" => Ok(ClaudeHookInput::WorktreeRemove(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "PreCompact" => Ok(ClaudeHookInput::PreCompact(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "PostCompact" => Ok(ClaudeHookInput::PostCompact(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "Elicitation" => Ok(ClaudeHookInput::Elicitation(
            serde_json::from_value(value.clone()).map_err(mk_err)?,
        )),
        "ElicitationResult" => Ok(ClaudeHookInput::ElicitationResult(
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
