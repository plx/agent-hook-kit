use serde::Serialize;

/// Top-level Claude hook output.
#[derive(Debug, Clone)]
pub enum ClaudeHookOutput {
    /// No output — allow/continue with empty stdout and exit 0.
    Empty,
    /// Structured JSON output on stdout, exit 0.
    Json(OutputEnvelope),
    /// Blocking error — message on stderr, exit 2.
    BlockingError { stderr: String },
}

/// The JSON envelope written to stdout for Claude hooks.
///
/// Different events use different subsets of these fields. The builders
/// below help construct valid combinations per event type.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputEnvelope {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(rename = "continue", skip_serializing_if = "Option::is_none")]
    pub continue_session: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppress_output: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_specific_output: Option<serde_json::Value>,
}

/// Event-scoped Claude output model.
///
/// This keeps event-level output intent explicit and avoids forcing callers
/// through one universal envelope shape.
#[derive(Debug, Clone)]
pub enum ClaudeEventOutput {
    SessionStart(ClaudeSessionStartOutput),
    PromptSubmit(ClaudePromptSubmitOutput),
    PreToolUse(ClaudePreToolUseOutput),
    PostToolUse(ClaudePostToolUseOutput),
    PermissionDenied(ClaudePermissionDeniedOutput),
    Stop(ClaudeStopOutput),
    WorktreeCreate(ClaudeWorktreeCreateOutput),
    FileChanged(ClaudeFileChangedOutput),
}

impl From<ClaudeEventOutput> for OutputEnvelope {
    fn from(value: ClaudeEventOutput) -> Self {
        match value {
            ClaudeEventOutput::SessionStart(v) => v.into(),
            ClaudeEventOutput::PromptSubmit(v) => v.into(),
            ClaudeEventOutput::PreToolUse(v) => v.into(),
            ClaudeEventOutput::PostToolUse(v) => v.into(),
            ClaudeEventOutput::PermissionDenied(v) => v.into(),
            ClaudeEventOutput::Stop(v) => v.into(),
            ClaudeEventOutput::WorktreeCreate(v) => v.into(),
            ClaudeEventOutput::FileChanged(v) => v.into(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ClaudeSessionStartOutput {
    pub additional_context: Option<String>,
    pub system_message: Option<String>,
    pub suppress_output: Option<bool>,
}

impl ClaudeSessionStartOutput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_context(mut self, message: impl Into<String>) -> Self {
        self.additional_context = Some(message.into());
        self
    }

    pub fn with_system_message(mut self, message: impl Into<String>) -> Self {
        self.system_message = Some(message.into());
        self
    }

    pub fn with_suppress_output(mut self, suppress: bool) -> Self {
        self.suppress_output = Some(suppress);
        self
    }
}

impl From<ClaudeSessionStartOutput> for OutputEnvelope {
    fn from(value: ClaudeSessionStartOutput) -> Self {
        let mut env = OutputEnvelope::new();
        if let Some(ctx) = value.additional_context {
            env.hook_specific_output = Some(serde_json::json!({ "additionalContext": ctx }));
        }
        env.system_message = value.system_message;
        env.suppress_output = value.suppress_output;
        env
    }
}

#[derive(Debug, Clone)]
pub enum ClaudePromptSubmitOutput {
    Allow,
    Block { reason: String },
}

impl ClaudePromptSubmitOutput {
    pub fn allow() -> Self {
        Self::Allow
    }

    pub fn block(reason: impl Into<String>) -> Self {
        Self::Block {
            reason: reason.into(),
        }
    }
}

impl From<ClaudePromptSubmitOutput> for OutputEnvelope {
    fn from(value: ClaudePromptSubmitOutput) -> Self {
        match value {
            ClaudePromptSubmitOutput::Allow => OutputEnvelope {
                decision: Some("allow".to_string()),
                ..OutputEnvelope::new()
            },
            ClaudePromptSubmitOutput::Block { reason } => OutputEnvelope {
                decision: Some("block".to_string()),
                reason: Some(reason),
                ..OutputEnvelope::new()
            },
        }
    }
}

#[derive(Debug, Clone)]
pub enum ClaudePreToolUseOutput {
    Allow,
    Deny {
        reason: String,
        updated_input: Option<serde_json::Value>,
    },
    Ask {
        reason: String,
    },
}

impl ClaudePreToolUseOutput {
    pub fn allow() -> Self {
        Self::Allow
    }

    pub fn deny(reason: impl Into<String>) -> Self {
        Self::Deny {
            reason: reason.into(),
            updated_input: None,
        }
    }

    pub fn deny_with_updated_input(
        reason: impl Into<String>,
        updated_input: serde_json::Value,
    ) -> Self {
        Self::Deny {
            reason: reason.into(),
            updated_input: Some(updated_input),
        }
    }

    pub fn ask(reason: impl Into<String>) -> Self {
        Self::Ask {
            reason: reason.into(),
        }
    }
}

impl From<ClaudePreToolUseOutput> for OutputEnvelope {
    fn from(value: ClaudePreToolUseOutput) -> Self {
        match value {
            ClaudePreToolUseOutput::Allow => OutputEnvelope {
                hook_specific_output: Some(serde_json::json!({
                    "permissionDecision": { "decision": "allow" }
                })),
                ..OutputEnvelope::new()
            },
            ClaudePreToolUseOutput::Deny {
                reason,
                updated_input,
            } => {
                let mut decision = serde_json::json!({
                    "decision": "deny",
                    "reason": reason
                });
                if let Some(updated) = updated_input {
                    decision["updatedInput"] = updated;
                }
                OutputEnvelope {
                    hook_specific_output: Some(serde_json::json!({
                        "permissionDecision": decision
                    })),
                    ..OutputEnvelope::new()
                }
            }
            ClaudePreToolUseOutput::Ask { reason } => OutputEnvelope {
                hook_specific_output: Some(serde_json::json!({
                    "permissionDecision": {
                        "decision": "ask",
                        "reason": reason
                    }
                })),
                ..OutputEnvelope::new()
            },
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ClaudePostToolUseOutput {
    pub additional_context: Option<String>,
}

impl ClaudePostToolUseOutput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_context(mut self, message: impl Into<String>) -> Self {
        self.additional_context = Some(message.into());
        self
    }
}

impl From<ClaudePostToolUseOutput> for OutputEnvelope {
    fn from(value: ClaudePostToolUseOutput) -> Self {
        match value.additional_context {
            Some(ctx) => OutputEnvelope {
                hook_specific_output: Some(serde_json::json!({
                    "additionalContext": ctx
                })),
                ..OutputEnvelope::new()
            },
            None => OutputEnvelope::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ClaudePermissionDeniedOutput {
    pub retry: bool,
}

impl ClaudePermissionDeniedOutput {
    pub fn retry() -> Self {
        Self { retry: true }
    }
}

impl From<ClaudePermissionDeniedOutput> for OutputEnvelope {
    fn from(value: ClaudePermissionDeniedOutput) -> Self {
        if value.retry {
            OutputEnvelope {
                hook_specific_output: Some(serde_json::json!({ "retry": true })),
                ..OutputEnvelope::new()
            }
        } else {
            OutputEnvelope::new()
        }
    }
}

#[derive(Debug, Clone)]
pub enum ClaudeStopOutput {
    AllowStop,
    ContinueSession { reason: String },
}

impl ClaudeStopOutput {
    pub fn allow_stop() -> Self {
        Self::AllowStop
    }

    pub fn continue_session(reason: impl Into<String>) -> Self {
        Self::ContinueSession {
            reason: reason.into(),
        }
    }
}

impl From<ClaudeStopOutput> for OutputEnvelope {
    fn from(value: ClaudeStopOutput) -> Self {
        match value {
            ClaudeStopOutput::AllowStop => OutputEnvelope::new(),
            ClaudeStopOutput::ContinueSession { reason } => OutputEnvelope {
                decision: Some("block".to_string()),
                reason: Some(reason),
                ..OutputEnvelope::new()
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClaudeWorktreeCreateOutput {
    pub worktree_path: String,
}

impl ClaudeWorktreeCreateOutput {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            worktree_path: path.into(),
        }
    }
}

impl From<ClaudeWorktreeCreateOutput> for OutputEnvelope {
    fn from(value: ClaudeWorktreeCreateOutput) -> Self {
        OutputEnvelope {
            hook_specific_output: Some(serde_json::json!({
                "worktreePath": value.worktree_path
            })),
            ..OutputEnvelope::new()
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ClaudeFileChangedOutput {
    pub watch_paths: Vec<String>,
}

impl ClaudeFileChangedOutput {
    pub fn new(paths: Vec<String>) -> Self {
        Self { watch_paths: paths }
    }
}

impl From<ClaudeFileChangedOutput> for OutputEnvelope {
    fn from(value: ClaudeFileChangedOutput) -> Self {
        OutputEnvelope {
            hook_specific_output: Some(serde_json::json!({
                "watchPaths": value.watch_paths
            })),
            ..OutputEnvelope::new()
        }
    }
}

impl OutputEnvelope {
    pub fn new() -> Self {
        Self::default()
    }

    // --- UserPromptSubmit / PostToolUse / Stop: top-level decision ---

    /// Block — used for UserPromptSubmit, PostToolUse, or Stop to reject the action.
    pub fn block(reason: impl Into<String>) -> Self {
        ClaudePromptSubmitOutput::block(reason).into()
    }

    /// Allow — explicit approval.
    pub fn allow() -> Self {
        ClaudePromptSubmitOutput::allow().into()
    }

    // --- PreToolUse: hookSpecificOutput.permissionDecision ---

    /// Deny a PreToolUse via hookSpecificOutput.permissionDecision.
    pub fn pre_tool_deny(reason: impl Into<String>) -> Self {
        ClaudePreToolUseOutput::deny(reason).into()
    }

    /// Allow a PreToolUse via hookSpecificOutput.permissionDecision.
    pub fn pre_tool_allow() -> Self {
        ClaudePreToolUseOutput::allow().into()
    }

    /// Ask user permission for a PreToolUse.
    pub fn pre_tool_ask(reason: impl Into<String>) -> Self {
        ClaudePreToolUseOutput::ask(reason).into()
    }

    /// Deny a PreToolUse and provide rewritten input.
    pub fn pre_tool_deny_with_updated_input(
        reason: impl Into<String>,
        updated_input: serde_json::Value,
    ) -> Self {
        ClaudePreToolUseOutput::deny_with_updated_input(reason, updated_input).into()
    }

    // --- Context injection ---

    /// Inject additional context visible to the model.
    pub fn with_context(message: impl Into<String>) -> Self {
        ClaudePostToolUseOutput::new().with_context(message).into()
    }

    // --- Stop: continue/stop behavior ---

    /// Continue the session when a Stop event fires (block the stop).
    pub fn stop_continue(reason: impl Into<String>) -> Self {
        ClaudeStopOutput::continue_session(reason).into()
    }

    // --- PermissionDenied: retry ---

    /// Retry a denied permission.
    pub fn permission_denied_retry() -> Self {
        ClaudePermissionDeniedOutput::retry().into()
    }

    // --- PermissionRequest: decision ---

    /// Approve a permission request.
    pub fn permission_approve() -> Self {
        Self {
            hook_specific_output: Some(serde_json::json!({
                "permissionDecision": {
                    "decision": "allow"
                }
            })),
            ..Default::default()
        }
    }

    /// Deny a permission request.
    pub fn permission_deny(reason: impl Into<String>) -> Self {
        Self {
            hook_specific_output: Some(serde_json::json!({
                "permissionDecision": {
                    "decision": "deny",
                    "reason": reason.into()
                }
            })),
            ..Default::default()
        }
    }

    // --- WorktreeCreate: path return ---

    /// Return a worktree path for WorktreeCreate.
    pub fn worktree_path(path: impl Into<String>) -> Self {
        ClaudeWorktreeCreateOutput::new(path).into()
    }

    // --- FileChanged: watch paths ---

    /// Set dynamic watch paths for FileChanged.
    pub fn watch_paths(paths: Vec<String>) -> Self {
        ClaudeFileChangedOutput::new(paths).into()
    }

    // --- Fluent setters ---

    pub fn with_system_message(mut self, msg: impl Into<String>) -> Self {
        self.system_message = Some(msg.into());
        self
    }

    pub fn with_suppress_output(mut self, suppress: bool) -> Self {
        self.suppress_output = Some(suppress);
        self
    }

    pub fn with_continue(mut self, cont: bool) -> Self {
        self.continue_session = Some(cont);
        self
    }

    pub fn with_stop_reason(mut self, reason: impl Into<String>) -> Self {
        self.stop_reason = Some(reason.into());
        self
    }
}
