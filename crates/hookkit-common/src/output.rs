//! Common output types with intent-oriented fields.

use crate::message::{TailToolCall, UserNotice};
use hookkit_claude::output::{ClaudeHookOutput, OutputEnvelope as ClaudeEnvelope};
use hookkit_codex::output::{CodexHookOutput, OutputEnvelope as CodexEnvelope};
use hookkit_core::{Harness, HookEventKey, HookkitError};
use hookkit_gemini::output::{GeminiHookOutput, OutputEnvelope as GeminiEnvelope};

fn unsupported(harness: Harness, event: HookEventKey, capability: &'static str) -> HookkitError {
    HookkitError::UnsupportedCapability {
        harness,
        event,
        capability,
    }
}

/// Cross-harness hook output.
///
/// Intent-oriented: the author specifies _what_ they want to communicate,
/// and conversion to a harness-native output handles the mapping.
#[derive(Debug, Clone)]
pub enum CommonHookOutput {
    /// No-op: allow/continue.
    Empty,
    SessionStart(CommonSessionStartOutput),
    /// Structured output for a prompt-submit event.
    PromptSubmit(CommonPromptSubmitOutput),
    /// Structured output for a pre-tool event.
    PreToolUse(CommonPreToolUseOutput),
    /// Structured output for a post-tool event.
    PostToolUse(CommonPostToolUseOutput),
    /// Structured output for a stop event.
    Stop(CommonStopOutput),
    Notification(CommonNotificationOutput),
    SessionEnd(CommonSessionEndOutput),
    PreCompress(CommonPreCompressOutput),
}

impl CommonHookOutput {
    pub fn empty() -> Self {
        Self::Empty
    }
}

// ---------------------------------------------------------------------------
// SessionStart output
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct CommonSessionStartOutput {
    pub user_notice: Option<UserNotice>,
    pub agent_context: Vec<String>,
}

impl CommonSessionStartOutput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_notice(mut self, notice: UserNotice) -> Self {
        self.user_notice = Some(notice);
        self
    }

    pub fn with_agent_context(mut self, line: impl Into<String>) -> Self {
        self.agent_context.push(line.into());
        self
    }

    pub fn to_claude(&self) -> Result<ClaudeHookOutput, HookkitError> {
        if self.user_notice.is_some() {
            return Err(unsupported(
                Harness::Claude,
                HookEventKey::SessionStart,
                "user_notice (no separate user-only channel for Claude SessionStart)",
            ));
        }
        if self.agent_context.is_empty() {
            return Ok(ClaudeHookOutput::Empty);
        }
        Ok(ClaudeHookOutput::Json(ClaudeEnvelope::with_context(
            self.agent_context.join("\n"),
        )))
    }

    pub fn to_codex(&self) -> Result<CodexHookOutput, HookkitError> {
        if self.user_notice.is_some() {
            return Err(unsupported(
                Harness::Codex,
                HookEventKey::SessionStart,
                "user_notice (not supported by Codex SessionStart)",
            ));
        }
        if !self.agent_context.is_empty() {
            return Err(unsupported(
                Harness::Codex,
                HookEventKey::SessionStart,
                "additionalContext (not supported by Codex SessionStart)",
            ));
        }
        Ok(CodexHookOutput::Empty)
    }

    pub fn to_gemini(&self) -> Result<GeminiHookOutput, HookkitError> {
        if self.user_notice.is_some() {
            return Err(unsupported(
                Harness::Gemini,
                HookEventKey::SessionStart,
                "user_notice (not supported by Gemini SessionStart)",
            ));
        }
        if !self.agent_context.is_empty() {
            return Err(unsupported(
                Harness::Gemini,
                HookEventKey::SessionStart,
                "additionalContext (not supported by Gemini SessionStart)",
            ));
        }
        Ok(GeminiHookOutput::Empty)
    }
}

// ---------------------------------------------------------------------------
// PromptSubmit output
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum CommonPromptSubmitOutput {
    Allow,
    Block { reason: String },
}

impl CommonPromptSubmitOutput {
    pub fn allow() -> Self {
        Self::Allow
    }

    pub fn block(reason: impl Into<String>) -> Self {
        Self::Block {
            reason: reason.into(),
        }
    }

    pub fn to_claude(&self) -> ClaudeHookOutput {
        match self {
            Self::Allow => ClaudeHookOutput::Empty,
            Self::Block { reason } => ClaudeHookOutput::Json(ClaudeEnvelope::block(reason)),
        }
    }

    pub fn to_codex(&self) -> CodexHookOutput {
        match self {
            Self::Allow => CodexHookOutput::Empty,
            Self::Block { reason } => CodexHookOutput::Json(CodexEnvelope::deny(reason)),
        }
    }

    pub fn to_gemini(&self) -> GeminiHookOutput {
        match self {
            Self::Allow => GeminiHookOutput::Empty,
            Self::Block { reason } => GeminiHookOutput::Json(GeminiEnvelope::deny(reason)),
        }
    }
}

// ---------------------------------------------------------------------------
// PreToolUse output
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum CommonPreToolUseOutput {
    Allow,
    Deny { reason: String },
}

impl CommonPreToolUseOutput {
    pub fn allow() -> Self {
        Self::Allow
    }

    pub fn deny(reason: impl Into<String>) -> Self {
        Self::Deny {
            reason: reason.into(),
        }
    }

    pub fn to_claude(&self) -> ClaudeHookOutput {
        match self {
            Self::Allow => ClaudeHookOutput::Json(ClaudeEnvelope::pre_tool_allow()),
            Self::Deny { reason } => ClaudeHookOutput::Json(ClaudeEnvelope::pre_tool_deny(reason)),
        }
    }

    pub fn to_codex(&self) -> Result<CodexHookOutput, HookkitError> {
        match self {
            Self::Allow => {
                // Codex does not support allow — it fails open
                Err(hookkit_codex::output::unsupported_allow())
            }
            Self::Deny { reason } => Ok(CodexHookOutput::Json(CodexEnvelope::deny(reason))),
        }
    }

    pub fn to_gemini(&self) -> GeminiHookOutput {
        match self {
            Self::Allow => GeminiHookOutput::Empty,
            Self::Deny { reason } => GeminiHookOutput::Json(GeminiEnvelope::deny(reason)),
        }
    }
}

// ---------------------------------------------------------------------------
// PostToolUse output
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct CommonPostToolUseOutput {
    pub user_notice: Option<UserNotice>,
    pub agent_context: Vec<String>,
    pub agent_feedback_block: Option<String>,
    pub replace_tool_result: Option<serde_json::Value>,
    pub tail_tool_call: Option<TailToolCall>,
    pub continue_session: Option<bool>,
    pub stop_reason: Option<String>,
}

impl CommonPostToolUseOutput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_notice(mut self, notice: UserNotice) -> Self {
        self.user_notice = Some(notice);
        self
    }

    pub fn with_agent_context(mut self, line: impl Into<String>) -> Self {
        self.agent_context.push(line.into());
        self
    }

    pub fn with_agent_feedback(mut self, text: impl Into<String>) -> Self {
        self.agent_feedback_block = Some(text.into());
        self
    }

    pub fn with_replaced_tool_result(mut self, value: serde_json::Value) -> Self {
        self.replace_tool_result = Some(value);
        self
    }

    pub fn with_tail_tool_call(mut self, call: TailToolCall) -> Self {
        self.tail_tool_call = Some(call);
        self
    }

    fn context_lines(&self) -> Vec<String> {
        let mut lines = self.agent_context.clone();
        if let Some(feedback) = &self.agent_feedback_block {
            lines.push(feedback.clone());
        }
        lines
    }

    /// Convert to a Claude-native output.
    pub fn to_claude(&self) -> Result<ClaudeHookOutput, HookkitError> {
        if self.user_notice.is_some() {
            return Err(unsupported(
                Harness::Claude,
                HookEventKey::PostToolUse,
                "user_notice (no separate user-only channel for Claude PostToolUse)",
            ));
        }
        if self.replace_tool_result.is_some() {
            return Err(unsupported(
                Harness::Claude,
                HookEventKey::PostToolUse,
                "replace_tool_result (not supported by Claude PostToolUse)",
            ));
        }
        if self.tail_tool_call.is_some() {
            return Err(unsupported(
                Harness::Claude,
                HookEventKey::PostToolUse,
                "tail_tool_call (not supported by Claude PostToolUse)",
            ));
        }

        let context = self.context_lines();
        if context.is_empty() && self.continue_session.is_none() && self.stop_reason.is_none() {
            return Ok(ClaudeHookOutput::Empty);
        }

        let mut envelope = ClaudeEnvelope::new();
        if !context.is_empty() {
            envelope.hook_specific_output = Some(serde_json::json!({
                "additionalContext": context.join("\n")
            }));
        }

        if let Some(cont) = self.continue_session {
            envelope.continue_session = Some(cont);
        }
        if let Some(ref reason) = self.stop_reason {
            envelope.stop_reason = Some(reason.clone());
        }

        Ok(ClaudeHookOutput::Json(envelope))
    }

    /// Convert to a Codex-native output.
    pub fn to_codex(&self) -> Result<CodexHookOutput, HookkitError> {
        if self.user_notice.is_some() {
            return Err(unsupported(
                Harness::Codex,
                HookEventKey::PostToolUse,
                "user_notice (not supported by Codex PostToolUse)",
            ));
        }
        if !self.context_lines().is_empty() {
            return Err(unsupported(
                Harness::Codex,
                HookEventKey::PostToolUse,
                "additionalContext (not supported by Codex PostToolUse)",
            ));
        }
        if self.replace_tool_result.is_some() {
            return Err(unsupported(
                Harness::Codex,
                HookEventKey::PostToolUse,
                "replace_tool_result (not supported by Codex PostToolUse)",
            ));
        }
        if self.tail_tool_call.is_some() {
            return Err(unsupported(
                Harness::Codex,
                HookEventKey::PostToolUse,
                "tail_tool_call (not supported by Codex PostToolUse)",
            ));
        }
        if self.continue_session.is_some() || self.stop_reason.is_some() {
            return Err(unsupported(
                Harness::Codex,
                HookEventKey::PostToolUse,
                "continue/stop controls (not supported by Codex PostToolUse)",
            ));
        }
        Ok(CodexHookOutput::Empty)
    }

    /// Convert to a Gemini-native output.
    pub fn to_gemini(&self) -> Result<GeminiHookOutput, HookkitError> {
        if self.user_notice.is_some() {
            return Err(unsupported(
                Harness::Gemini,
                HookEventKey::PostToolUse,
                "user_notice (no separate user-only channel for Gemini AfterTool)",
            ));
        }
        if self.tail_tool_call.is_some() {
            return Err(unsupported(
                Harness::Gemini,
                HookEventKey::PostToolUse,
                "tail_tool_call (not supported by Gemini AfterTool)",
            ));
        }
        if self.continue_session.is_some() || self.stop_reason.is_some() {
            return Err(unsupported(
                Harness::Gemini,
                HookEventKey::PostToolUse,
                "continue/stop controls (not supported by Gemini AfterTool)",
            ));
        }

        let context = self.context_lines();
        let has_context = !context.is_empty();
        let has_replace = self.replace_tool_result.is_some();

        if has_context && has_replace {
            return Err(unsupported(
                Harness::Gemini,
                HookEventKey::PostToolUse,
                "simultaneous replace_tool_result and additionalContext",
            ));
        }

        if let Some(replace) = &self.replace_tool_result {
            return Ok(GeminiHookOutput::Json(GeminiEnvelope::replace_tool_result(
                replace.clone(),
            )));
        }

        if has_context {
            return Ok(GeminiHookOutput::Json(GeminiEnvelope::with_context(
                context.join("\n"),
            )));
        }

        Ok(GeminiHookOutput::Empty)
    }
}

// ---------------------------------------------------------------------------
// Stop output
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum CommonStopOutput {
    AllowStop,
    ContinueSession { reason: String },
}

impl CommonStopOutput {
    pub fn allow_stop() -> Self {
        Self::AllowStop
    }

    pub fn continue_session(reason: impl Into<String>) -> Self {
        Self::ContinueSession {
            reason: reason.into(),
        }
    }

    pub fn to_claude(&self) -> ClaudeHookOutput {
        match self {
            Self::AllowStop => ClaudeHookOutput::Empty,
            Self::ContinueSession { reason } => {
                ClaudeHookOutput::Json(ClaudeEnvelope::stop_continue(reason))
            }
        }
    }

    pub fn to_codex(&self) -> CodexHookOutput {
        match self {
            Self::AllowStop => CodexHookOutput::Json(CodexEnvelope::stop_allow()),
            Self::ContinueSession { reason } => {
                CodexHookOutput::Json(CodexEnvelope::stop_continue(reason))
            }
        }
    }

    pub fn to_gemini(&self) -> GeminiHookOutput {
        match self {
            Self::AllowStop => GeminiHookOutput::Empty,
            Self::ContinueSession { reason } => {
                GeminiHookOutput::Json(GeminiEnvelope::retry(reason))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Notification output
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct CommonNotificationOutput {
    pub user_notice: Option<UserNotice>,
}

impl CommonNotificationOutput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_notice(mut self, notice: UserNotice) -> Self {
        self.user_notice = Some(notice);
        self
    }

    pub fn to_claude(&self) -> Result<ClaudeHookOutput, HookkitError> {
        if self.user_notice.is_some() {
            return Err(unsupported(
                Harness::Claude,
                HookEventKey::Notification,
                "user_notice emission (not supported by Claude Notification)",
            ));
        }
        Ok(ClaudeHookOutput::Empty)
    }

    pub fn to_gemini(&self) -> Result<GeminiHookOutput, HookkitError> {
        if self.user_notice.is_some() {
            return Err(unsupported(
                Harness::Gemini,
                HookEventKey::Notification,
                "user_notice emission (not supported by Gemini Notification)",
            ));
        }
        Ok(GeminiHookOutput::Empty)
    }
}

// ---------------------------------------------------------------------------
// SessionEnd output
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct CommonSessionEndOutput {
    pub user_notice: Option<UserNotice>,
}

impl CommonSessionEndOutput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_notice(mut self, notice: UserNotice) -> Self {
        self.user_notice = Some(notice);
        self
    }

    pub fn to_claude(&self) -> Result<ClaudeHookOutput, HookkitError> {
        if self.user_notice.is_some() {
            return Err(unsupported(
                Harness::Claude,
                HookEventKey::SessionEnd,
                "user_notice emission (not supported by Claude SessionEnd)",
            ));
        }
        Ok(ClaudeHookOutput::Empty)
    }

    pub fn to_gemini(&self) -> Result<GeminiHookOutput, HookkitError> {
        if self.user_notice.is_some() {
            return Err(unsupported(
                Harness::Gemini,
                HookEventKey::SessionEnd,
                "user_notice emission (not supported by Gemini SessionEnd)",
            ));
        }
        Ok(GeminiHookOutput::Empty)
    }
}

// ---------------------------------------------------------------------------
// PreCompress output
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct CommonPreCompressOutput {
    pub user_notice: Option<UserNotice>,
    pub agent_context: Vec<String>,
}

impl CommonPreCompressOutput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_notice(mut self, notice: UserNotice) -> Self {
        self.user_notice = Some(notice);
        self
    }

    pub fn with_agent_context(mut self, line: impl Into<String>) -> Self {
        self.agent_context.push(line.into());
        self
    }

    pub fn to_claude(&self) -> Result<ClaudeHookOutput, HookkitError> {
        if self.user_notice.is_some() {
            return Err(unsupported(
                Harness::Claude,
                HookEventKey::PreCompress,
                "user_notice (not supported by Claude PreCompact)",
            ));
        }
        if self.agent_context.is_empty() {
            return Ok(ClaudeHookOutput::Empty);
        }
        Ok(ClaudeHookOutput::Json(ClaudeEnvelope::with_context(
            self.agent_context.join("\n"),
        )))
    }

    pub fn to_gemini(&self) -> Result<GeminiHookOutput, HookkitError> {
        if self.user_notice.is_some() {
            return Err(unsupported(
                Harness::Gemini,
                HookEventKey::PreCompress,
                "user_notice (not supported by Gemini PreCompress)",
            ));
        }
        if self.agent_context.is_empty() {
            return Ok(GeminiHookOutput::Empty);
        }
        Ok(GeminiHookOutput::Json(GeminiEnvelope::with_context(
            self.agent_context.join("\n"),
        )))
    }

    pub fn to_codex(&self) -> Result<CodexHookOutput, HookkitError> {
        if self.user_notice.is_some() {
            return Err(unsupported(
                Harness::Codex,
                HookEventKey::PreCompress,
                "user_notice (not supported by Codex)",
            ));
        }
        if !self.agent_context.is_empty() {
            return Err(unsupported(
                Harness::Codex,
                HookEventKey::PreCompress,
                "additionalContext (not supported by Codex)",
            ));
        }
        Ok(CodexHookOutput::Empty)
    }
}
