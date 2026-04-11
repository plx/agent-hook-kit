//! Common output types with intent-oriented fields.

use crate::message::UserNotice;
use hookkit_claude::output::{ClaudeHookOutput, OutputEnvelope as ClaudeEnvelope};
use hookkit_codex::output::{CodexHookOutput, OutputEnvelope as CodexEnvelope};
use hookkit_core::{Harness, HookEventKey, HookkitError};
use hookkit_gemini::output::{GeminiHookOutput, OutputEnvelope as GeminiEnvelope};

/// Cross-harness hook output.
///
/// Intent-oriented: the author specifies _what_ they want to communicate,
/// and conversion to a harness-native output handles the mapping.
#[derive(Debug, Clone)]
pub enum CommonHookOutput {
    /// No-op: allow/continue.
    Empty,
    /// Structured output for a post-tool event.
    PostToolUse(CommonPostToolUseOutput),
    /// Structured output for a pre-tool event.
    PreToolUse(CommonPreToolUseOutput),
    /// Structured output for a prompt-submit event.
    PromptSubmit(CommonPromptSubmitOutput),
    /// Structured output for a stop event.
    Stop(CommonStopOutput),
}

impl CommonHookOutput {
    pub fn empty() -> Self {
        Self::Empty
    }
}

// ---------------------------------------------------------------------------
// PostToolUse output
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct CommonPostToolUseOutput {
    pub user_notice: Option<UserNotice>,
    pub agent_context: Vec<String>,
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

    /// Convert to a Claude-native output.
    pub fn to_claude(&self) -> ClaudeHookOutput {
        if self.agent_context.is_empty() && self.user_notice.is_none() {
            return ClaudeHookOutput::Empty;
        }

        let mut envelope = ClaudeEnvelope::new();

        if !self.agent_context.is_empty() {
            let ctx = self.agent_context.join("\n");
            envelope.hook_specific_output = Some(serde_json::json!({
                "additionalContext": ctx
            }));
        }

        if let Some(cont) = self.continue_session {
            envelope.continue_session = Some(cont);
        }
        if let Some(ref reason) = self.stop_reason {
            envelope.stop_reason = Some(reason.clone());
        }

        ClaudeHookOutput::Json(envelope)
    }

    /// Convert to a Codex-native output.
    ///
    /// Note: Codex does not support additionalContext, so context
    /// is dropped and a warning is returned if it was present.
    pub fn to_codex(&self) -> Result<CodexHookOutput, HookkitError> {
        if !self.agent_context.is_empty() {
            return Err(HookkitError::UnsupportedCapability {
                harness: Harness::Codex,
                event: HookEventKey::PostToolUse,
                capability: "additionalContext (not supported by Codex)",
            });
        }
        Ok(CodexHookOutput::Empty)
    }

    /// Convert to a Gemini-native output.
    pub fn to_gemini(&self) -> GeminiHookOutput {
        if self.agent_context.is_empty() {
            return GeminiHookOutput::Empty;
        }

        let ctx = self.agent_context.join("\n");
        GeminiHookOutput::Json(GeminiEnvelope::with_context(ctx))
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
            Self::Deny { reason } => Ok(CodexHookOutput::BlockingDeny {
                stderr: reason.clone(),
            }),
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
            Self::Block { reason } => CodexHookOutput::BlockingDeny {
                stderr: reason.clone(),
            },
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
            Self::AllowStop => CodexHookOutput::Empty,
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
