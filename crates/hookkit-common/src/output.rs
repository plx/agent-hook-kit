//! Common output types with intent-oriented fields.

use crate::message::{
    AgentFeedback, DiagnosticArtifact, DiagnosticReport, NoticeLevel, TailToolCall, UserNotice,
};
use hookkit_claude::output::{
    ClaudeHookOutput, ClaudeSessionStartOutput, OutputEnvelope as ClaudeEnvelope,
};
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

/// Policy for intents that the selected harness/event cannot represent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LoweringPolicy {
    #[default]
    Strict,
    BestEffort,
    BestEffortWithWarnings,
}

/// Whether an intent must be preserved during lowering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentRequirement {
    Required,
    Optional,
}

/// Action taken for an unsupported or redirected intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoweringAction {
    Dropped,
    RedirectedToStderr,
    Converted,
}

/// Warning emitted when best-effort lowering changes an intent.
#[derive(Debug, Clone)]
pub struct LoweringWarning {
    pub harness: Harness,
    pub event: HookEventKey,
    pub intent: &'static str,
    pub action: LoweringAction,
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
        Ok(ClaudeHookOutput::Json(
            ClaudeSessionStartOutput::new()
                .with_context(self.agent_context.join("\n"))
                .into(),
        ))
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
    pub notices: Vec<UserNotice>,
    pub agent_feedback: Vec<AgentFeedback>,
    pub diagnostics: Vec<DiagnosticReport>,
    pub replace_tool_result: Option<serde_json::Value>,
    pub tail_tool_call: Option<TailToolCall>,
    pub session_control: Option<SessionControl>,
    pub lowering: LoweringPolicy,
}

/// Session control intent for post-tool events.
#[derive(Debug, Clone, Default)]
pub struct SessionControl {
    pub continue_session: Option<bool>,
    pub stop_reason: Option<String>,
}

/// Lowered post-tool output plus side-channel messages for the runtime.
#[derive(Debug, Clone)]
pub struct LoweredPostToolUseOutput<T> {
    pub native: T,
    pub stderr_messages: Vec<String>,
    pub warnings: Vec<LoweringWarning>,
}

impl CommonPostToolUseOutput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_user_notice(mut self, notice: UserNotice) -> Self {
        self.notices.push(notice);
        self
    }

    pub fn with_notice(mut self, notice: UserNotice) -> Self {
        self.notices.push(notice);
        self
    }

    pub fn with_agent_context(mut self, line: impl Into<String>) -> Self {
        self.agent_feedback.push(AgentFeedback::info(line));
        self
    }

    pub fn with_agent_feedback(mut self, text: impl Into<String>) -> Self {
        self.agent_feedback.push(AgentFeedback::new(text));
        self
    }

    pub fn with_diagnostic_report(mut self, report: DiagnosticReport) -> Self {
        self.diagnostics.push(report);
        self
    }

    pub fn with_diagnostic_artifact(mut self, artifact: DiagnosticArtifact) -> Self {
        let title = artifact
            .summary
            .clone()
            .unwrap_or_else(|| "diagnostic artifact".to_string());
        let text = format!(
            "Diagnostics written to {}",
            artifact.absolute_path.display()
        );
        self.diagnostics
            .push(DiagnosticReport::new(title, text).with_artifact(artifact));
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

    pub fn with_continue_session(mut self, continue_session: bool) -> Self {
        self.session_control
            .get_or_insert_with(SessionControl::default)
            .continue_session = Some(continue_session);
        self
    }

    pub fn with_stop_reason(mut self, reason: impl Into<String>) -> Self {
        self.session_control
            .get_or_insert_with(SessionControl::default)
            .stop_reason = Some(reason.into());
        self
    }

    pub fn with_lowering_policy(mut self, policy: LoweringPolicy) -> Self {
        self.lowering = policy;
        self
    }

    fn context_lines(&self) -> Vec<String> {
        self.agent_feedback
            .iter()
            .map(|feedback| feedback.text.clone())
            .collect()
    }

    fn session_continue(&self) -> Option<bool> {
        self.session_control
            .as_ref()
            .and_then(|control| control.continue_session)
    }

    fn session_stop_reason(&self) -> Option<&str> {
        self.session_control
            .as_ref()
            .and_then(|control| control.stop_reason.as_deref())
    }

    fn has_session_control(&self) -> bool {
        self.session_continue().is_some() || self.session_stop_reason().is_some()
    }

    /// Convert to a Claude-native output.
    pub fn to_claude(&self) -> Result<ClaudeHookOutput, HookkitError> {
        Ok(self.to_claude_lowered()?.native)
    }

    pub fn to_claude_lowered(
        &self,
    ) -> Result<LoweredPostToolUseOutput<ClaudeHookOutput>, HookkitError> {
        let mut lowered = PostToolUseLowering::new(Harness::Claude, self.lowering);
        lowered.fallback_user_messages(&self.notices, &self.diagnostics)?;

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
        if context.is_empty() && !self.has_session_control() {
            return Ok(lowered.finish(ClaudeHookOutput::Empty));
        }

        let mut envelope = ClaudeEnvelope::new();
        if !context.is_empty() {
            envelope.hook_specific_output = Some(serde_json::json!({
                "hookEventName": "PostToolUse",
                "additionalContext": context.join("\n")
            }));
        }

        if let Some(cont) = self.session_continue() {
            envelope.continue_session = Some(cont);
        }
        if let Some(reason) = self.session_stop_reason() {
            envelope.stop_reason = Some(reason.to_string());
        }

        Ok(lowered.finish(ClaudeHookOutput::Json(envelope)))
    }

    /// Convert to a Codex-native output.
    pub fn to_codex(&self) -> Result<CodexHookOutput, HookkitError> {
        Ok(self.to_codex_lowered()?.native)
    }

    pub fn to_codex_lowered(
        &self,
    ) -> Result<LoweredPostToolUseOutput<CodexHookOutput>, HookkitError> {
        let mut lowered = PostToolUseLowering::new(Harness::Codex, self.lowering);
        lowered.fallback_user_messages(&self.notices, &self.diagnostics)?;

        if !self.context_lines().is_empty() {
            lowered.optional_unsupported("additionalContext")?;
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
        if self.has_session_control() {
            return Err(unsupported(
                Harness::Codex,
                HookEventKey::PostToolUse,
                "continue/stop controls (not supported by Codex PostToolUse)",
            ));
        }
        Ok(lowered.finish(CodexHookOutput::Empty))
    }

    /// Convert to a Gemini-native output.
    pub fn to_gemini(&self) -> Result<GeminiHookOutput, HookkitError> {
        Ok(self.to_gemini_lowered()?.native)
    }

    pub fn to_gemini_lowered(
        &self,
    ) -> Result<LoweredPostToolUseOutput<GeminiHookOutput>, HookkitError> {
        let mut lowered = PostToolUseLowering::new(Harness::Gemini, self.lowering);
        lowered.fallback_user_messages(&self.notices, &self.diagnostics)?;

        if self.tail_tool_call.is_some() {
            return Err(unsupported(
                Harness::Gemini,
                HookEventKey::PostToolUse,
                "tail_tool_call (not supported by Gemini AfterTool)",
            ));
        }
        if self.has_session_control() {
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
            lowered.optional_unsupported("additionalContext")?;
        }

        if let Some(replace) = &self.replace_tool_result {
            return Ok(lowered.finish(GeminiHookOutput::Json(
                GeminiEnvelope::replace_tool_result(replace.clone()),
            )));
        }

        if has_context {
            return Ok(
                lowered.finish(GeminiHookOutput::Json(GeminiEnvelope::with_context(
                    context.join("\n"),
                ))),
            );
        }

        Ok(lowered.finish(GeminiHookOutput::Empty))
    }
}

struct PostToolUseLowering {
    harness: Harness,
    policy: LoweringPolicy,
    stderr_messages: Vec<String>,
    warnings: Vec<LoweringWarning>,
}

impl PostToolUseLowering {
    fn new(harness: Harness, policy: LoweringPolicy) -> Self {
        Self {
            harness,
            policy,
            stderr_messages: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn finish<T>(self, native: T) -> LoweredPostToolUseOutput<T> {
        LoweredPostToolUseOutput {
            native,
            stderr_messages: self.stderr_messages,
            warnings: self.warnings,
        }
    }

    fn fallback_user_messages(
        &mut self,
        notices: &[UserNotice],
        diagnostics: &[DiagnosticReport],
    ) -> Result<(), HookkitError> {
        if notices.is_empty() && diagnostics.is_empty() {
            return Ok(());
        }

        if self.policy == LoweringPolicy::Strict {
            return Err(unsupported(
                self.harness,
                HookEventKey::PostToolUse,
                "user_notice/diagnostics (no structured user-only channel for PostToolUse)",
            ));
        }

        for notice in notices {
            self.stderr_messages.push(format_notice(notice));
        }
        for diagnostic in diagnostics {
            self.stderr_messages.push(format_diagnostic(diagnostic));
        }
        self.warn(
            "user_notice/diagnostics",
            LoweringAction::RedirectedToStderr,
        );
        Ok(())
    }

    fn optional_unsupported(&mut self, intent: &'static str) -> Result<(), HookkitError> {
        if self.policy == LoweringPolicy::Strict {
            return Err(unsupported(self.harness, HookEventKey::PostToolUse, intent));
        }
        self.warn(intent, LoweringAction::Dropped);
        Ok(())
    }

    fn warn(&mut self, intent: &'static str, action: LoweringAction) {
        if self.policy == LoweringPolicy::BestEffortWithWarnings {
            self.warnings.push(LoweringWarning {
                harness: self.harness,
                event: HookEventKey::PostToolUse,
                intent,
                action,
            });
        }
    }
}

fn format_notice(notice: &UserNotice) -> String {
    match notice.level {
        NoticeLevel::Info => notice.text.clone(),
        NoticeLevel::Warning => format!("warning: {}", notice.text),
        NoticeLevel::Error => format!("error: {}", notice.text),
    }
}

fn format_diagnostic(diagnostic: &DiagnosticReport) -> String {
    let mut out = format!("{}:\n{}", diagnostic.title, diagnostic.text.trim());
    if let Some(artifact) = &diagnostic.artifact {
        out.push_str(&format!(
            "\nartifact: {} ({})",
            artifact.absolute_path.display(),
            artifact.media_type
        ));
        if let Some(project_relative_path) = &artifact.project_relative_path {
            out.push_str(&format!(
                "\nproject-relative artifact: {}",
                project_relative_path.display()
            ));
        }
        if let Some(summary) = &artifact.summary {
            out.push_str(&format!("\nsummary: {summary}"));
        }
    }
    out
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
        if !self.agent_context.is_empty() {
            return Err(unsupported(
                Harness::Claude,
                HookEventKey::PreCompress,
                "additionalContext (not supported by Claude PreCompact)",
            ));
        }
        Ok(ClaudeHookOutput::Empty)
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
