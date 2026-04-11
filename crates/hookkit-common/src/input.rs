//! Common input wrapper enums.

use hookkit_claude::input as claude;
use hookkit_codex::input as codex;
use hookkit_gemini::input as gemini;

/// Cross-harness input event.
///
/// Each variant wraps a semantically aligned group of native events.
/// Access the underlying native type via the `as_*` methods or
/// by matching on the inner harness-tagged enum.
#[derive(Debug, Clone)]
pub enum CommonHookInput {
    SessionStart(CommonSessionStartInput),
    PromptSubmit(CommonPromptSubmitInput),
    PreToolUse(CommonPreToolUseInput),
    PostToolUse(CommonPostToolUseInput),
    Stop(CommonStopInput),
    Notification(CommonNotificationInput),
    SessionEnd(CommonSessionEndInput),
    PreCompress(CommonPreCompressInput),
}

// ---------------------------------------------------------------------------
// SessionStart
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum CommonSessionStartInput {
    Claude(claude::SessionStart),
    Codex(codex::SessionStart),
    Gemini(gemini::SessionStart),
}

impl CommonSessionStartInput {
    pub fn session_id(&self) -> &str {
        match self {
            Self::Claude(ev) => &ev.common.session_id,
            Self::Codex(ev) => &ev.common.session_id,
            Self::Gemini(ev) => &ev.common.session_id,
        }
    }

    pub fn cwd(&self) -> &str {
        match self {
            Self::Claude(ev) => &ev.common.cwd,
            Self::Codex(ev) => &ev.common.cwd,
            Self::Gemini(ev) => &ev.common.cwd,
        }
    }

    pub fn as_claude(&self) -> Option<&claude::SessionStart> {
        match self {
            Self::Claude(ev) => Some(ev),
            _ => None,
        }
    }

    pub fn as_codex(&self) -> Option<&codex::SessionStart> {
        match self {
            Self::Codex(ev) => Some(ev),
            _ => None,
        }
    }

    pub fn as_gemini(&self) -> Option<&gemini::SessionStart> {
        match self {
            Self::Gemini(ev) => Some(ev),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// PromptSubmit (Claude UserPromptSubmit / Codex UserPromptSubmit / Gemini BeforeAgent)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum CommonPromptSubmitInput {
    Claude(claude::UserPromptSubmit),
    Codex(codex::UserPromptSubmit),
    Gemini(gemini::BeforeAgent),
}

impl CommonPromptSubmitInput {
    pub fn session_id(&self) -> &str {
        match self {
            Self::Claude(ev) => &ev.common.session_id,
            Self::Codex(ev) => &ev.common.session_id,
            Self::Gemini(ev) => &ev.common.session_id,
        }
    }

    pub fn cwd(&self) -> &str {
        match self {
            Self::Claude(ev) => &ev.common.cwd,
            Self::Codex(ev) => &ev.common.cwd,
            Self::Gemini(ev) => &ev.common.cwd,
        }
    }

    pub fn user_prompt(&self) -> Option<&str> {
        match self {
            Self::Claude(ev) => ev.user_prompt.as_deref(),
            Self::Codex(ev) => ev.user_prompt.as_deref(),
            Self::Gemini(ev) => ev.user_prompt.as_deref(),
        }
    }

    pub fn as_claude(&self) -> Option<&claude::UserPromptSubmit> {
        match self {
            Self::Claude(ev) => Some(ev),
            _ => None,
        }
    }

    pub fn as_codex(&self) -> Option<&codex::UserPromptSubmit> {
        match self {
            Self::Codex(ev) => Some(ev),
            _ => None,
        }
    }

    pub fn as_gemini(&self) -> Option<&gemini::BeforeAgent> {
        match self {
            Self::Gemini(ev) => Some(ev),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// PreToolUse (Claude PreToolUse / Codex PreToolUse / Gemini BeforeTool)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum CommonPreToolUseInput {
    Claude(claude::PreToolUse),
    Codex(codex::PreToolUse),
    Gemini(gemini::BeforeTool),
}

impl CommonPreToolUseInput {
    pub fn session_id(&self) -> &str {
        match self {
            Self::Claude(ev) => &ev.common.session_id,
            Self::Codex(ev) => &ev.common.session_id,
            Self::Gemini(ev) => &ev.common.session_id,
        }
    }

    pub fn cwd(&self) -> &str {
        match self {
            Self::Claude(ev) => &ev.common.cwd,
            Self::Codex(ev) => &ev.common.cwd,
            Self::Gemini(ev) => &ev.common.cwd,
        }
    }

    pub fn tool_name(&self) -> Option<&str> {
        match self {
            Self::Claude(ev) => ev.tool_name.as_deref(),
            Self::Codex(ev) => ev.tool_name.as_deref(),
            Self::Gemini(ev) => ev.tool_name.as_deref(),
        }
    }

    pub fn raw_tool_input(&self) -> Option<&serde_json::Value> {
        match self {
            Self::Claude(ev) => ev.tool_input.as_ref(),
            Self::Codex(ev) => ev.tool_input.as_ref(),
            Self::Gemini(ev) => ev.tool_input.as_ref(),
        }
    }

    pub fn as_claude(&self) -> Option<&claude::PreToolUse> {
        match self {
            Self::Claude(ev) => Some(ev),
            _ => None,
        }
    }

    pub fn as_codex(&self) -> Option<&codex::PreToolUse> {
        match self {
            Self::Codex(ev) => Some(ev),
            _ => None,
        }
    }

    pub fn as_gemini(&self) -> Option<&gemini::BeforeTool> {
        match self {
            Self::Gemini(ev) => Some(ev),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// PostToolUse (Claude PostToolUse / Codex PostToolUse / Gemini AfterTool)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum CommonPostToolUseInput {
    Claude(claude::PostToolUse),
    Codex(codex::PostToolUse),
    Gemini(gemini::AfterTool),
}

impl CommonPostToolUseInput {
    pub fn session_id(&self) -> &str {
        match self {
            Self::Claude(ev) => &ev.common.session_id,
            Self::Codex(ev) => &ev.common.session_id,
            Self::Gemini(ev) => &ev.common.session_id,
        }
    }

    pub fn cwd(&self) -> &str {
        match self {
            Self::Claude(ev) => &ev.common.cwd,
            Self::Codex(ev) => &ev.common.cwd,
            Self::Gemini(ev) => &ev.common.cwd,
        }
    }

    pub fn tool_name(&self) -> Option<&str> {
        match self {
            Self::Claude(ev) => ev.tool_name.as_deref(),
            Self::Codex(ev) => ev.tool_name.as_deref(),
            Self::Gemini(ev) => ev.tool_name.as_deref(),
        }
    }

    pub fn raw_tool_input(&self) -> Option<&serde_json::Value> {
        match self {
            Self::Claude(ev) => ev.tool_input.as_ref(),
            Self::Codex(ev) => ev.tool_input.as_ref(),
            Self::Gemini(ev) => ev.tool_input.as_ref(),
        }
    }

    pub fn as_claude(&self) -> Option<&claude::PostToolUse> {
        match self {
            Self::Claude(ev) => Some(ev),
            _ => None,
        }
    }

    pub fn as_codex(&self) -> Option<&codex::PostToolUse> {
        match self {
            Self::Codex(ev) => Some(ev),
            _ => None,
        }
    }

    pub fn as_gemini(&self) -> Option<&gemini::AfterTool> {
        match self {
            Self::Gemini(ev) => Some(ev),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Stop (Claude Stop / Codex Stop / Gemini AfterAgent)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum CommonStopInput {
    Claude(claude::Stop),
    Codex(codex::Stop),
    Gemini(gemini::AfterAgent),
}

impl CommonStopInput {
    pub fn session_id(&self) -> &str {
        match self {
            Self::Claude(ev) => &ev.common.session_id,
            Self::Codex(ev) => &ev.common.session_id,
            Self::Gemini(ev) => &ev.common.session_id,
        }
    }

    pub fn cwd(&self) -> &str {
        match self {
            Self::Claude(ev) => &ev.common.cwd,
            Self::Codex(ev) => &ev.common.cwd,
            Self::Gemini(ev) => &ev.common.cwd,
        }
    }

    pub fn last_assistant_message(&self) -> Option<&str> {
        match self {
            Self::Claude(ev) => ev.last_assistant_message.as_deref(),
            Self::Codex(ev) => ev.last_assistant_message.as_deref(),
            Self::Gemini(ev) => ev.agent_response.as_deref(),
        }
    }

    pub fn as_claude(&self) -> Option<&claude::Stop> {
        match self {
            Self::Claude(ev) => Some(ev),
            _ => None,
        }
    }

    pub fn as_codex(&self) -> Option<&codex::Stop> {
        match self {
            Self::Codex(ev) => Some(ev),
            _ => None,
        }
    }

    pub fn as_gemini(&self) -> Option<&gemini::AfterAgent> {
        match self {
            Self::Gemini(ev) => Some(ev),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Notification (Claude Notification / Gemini Notification — Codex has none)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum CommonNotificationInput {
    Claude(claude::Notification),
    Gemini(gemini::Notification),
}

impl CommonNotificationInput {
    pub fn session_id(&self) -> &str {
        match self {
            Self::Claude(ev) => &ev.common.session_id,
            Self::Gemini(ev) => &ev.common.session_id,
        }
    }

    pub fn message(&self) -> Option<&str> {
        match self {
            Self::Claude(ev) => ev.message.as_deref(),
            Self::Gemini(ev) => ev.message.as_deref(),
        }
    }
}

// ---------------------------------------------------------------------------
// SessionEnd (Claude SessionEnd / Gemini SessionEnd — Codex has none yet)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum CommonSessionEndInput {
    Claude(claude::SessionEnd),
    Gemini(gemini::SessionEnd),
}

impl CommonSessionEndInput {
    pub fn session_id(&self) -> &str {
        match self {
            Self::Claude(ev) => &ev.common.session_id,
            Self::Gemini(ev) => &ev.common.session_id,
        }
    }
}

// ---------------------------------------------------------------------------
// PreCompress (Claude PreCompact / Gemini PreCompress)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum CommonPreCompressInput {
    Gemini(gemini::PreCompress),
}

impl CommonPreCompressInput {
    pub fn session_id(&self) -> &str {
        match self {
            Self::Gemini(ev) => &ev.common.session_id,
        }
    }
}

// ---------------------------------------------------------------------------
// Conversion from native inputs
// ---------------------------------------------------------------------------

impl From<claude::SessionStart> for CommonHookInput {
    fn from(ev: claude::SessionStart) -> Self {
        CommonHookInput::SessionStart(CommonSessionStartInput::Claude(ev))
    }
}

impl From<codex::SessionStart> for CommonHookInput {
    fn from(ev: codex::SessionStart) -> Self {
        CommonHookInput::SessionStart(CommonSessionStartInput::Codex(ev))
    }
}

impl From<gemini::SessionStart> for CommonHookInput {
    fn from(ev: gemini::SessionStart) -> Self {
        CommonHookInput::SessionStart(CommonSessionStartInput::Gemini(ev))
    }
}

impl From<claude::UserPromptSubmit> for CommonHookInput {
    fn from(ev: claude::UserPromptSubmit) -> Self {
        CommonHookInput::PromptSubmit(CommonPromptSubmitInput::Claude(ev))
    }
}

impl From<codex::UserPromptSubmit> for CommonHookInput {
    fn from(ev: codex::UserPromptSubmit) -> Self {
        CommonHookInput::PromptSubmit(CommonPromptSubmitInput::Codex(ev))
    }
}

impl From<gemini::BeforeAgent> for CommonHookInput {
    fn from(ev: gemini::BeforeAgent) -> Self {
        CommonHookInput::PromptSubmit(CommonPromptSubmitInput::Gemini(ev))
    }
}

impl From<claude::PreToolUse> for CommonHookInput {
    fn from(ev: claude::PreToolUse) -> Self {
        CommonHookInput::PreToolUse(CommonPreToolUseInput::Claude(ev))
    }
}

impl From<codex::PreToolUse> for CommonHookInput {
    fn from(ev: codex::PreToolUse) -> Self {
        CommonHookInput::PreToolUse(CommonPreToolUseInput::Codex(ev))
    }
}

impl From<gemini::BeforeTool> for CommonHookInput {
    fn from(ev: gemini::BeforeTool) -> Self {
        CommonHookInput::PreToolUse(CommonPreToolUseInput::Gemini(ev))
    }
}

impl From<claude::PostToolUse> for CommonHookInput {
    fn from(ev: claude::PostToolUse) -> Self {
        CommonHookInput::PostToolUse(CommonPostToolUseInput::Claude(ev))
    }
}

impl From<codex::PostToolUse> for CommonHookInput {
    fn from(ev: codex::PostToolUse) -> Self {
        CommonHookInput::PostToolUse(CommonPostToolUseInput::Codex(ev))
    }
}

impl From<gemini::AfterTool> for CommonHookInput {
    fn from(ev: gemini::AfterTool) -> Self {
        CommonHookInput::PostToolUse(CommonPostToolUseInput::Gemini(ev))
    }
}

impl From<claude::Stop> for CommonHookInput {
    fn from(ev: claude::Stop) -> Self {
        CommonHookInput::Stop(CommonStopInput::Claude(ev))
    }
}

impl From<codex::Stop> for CommonHookInput {
    fn from(ev: codex::Stop) -> Self {
        CommonHookInput::Stop(CommonStopInput::Codex(ev))
    }
}

impl From<gemini::AfterAgent> for CommonHookInput {
    fn from(ev: gemini::AfterAgent) -> Self {
        CommonHookInput::Stop(CommonStopInput::Gemini(ev))
    }
}
