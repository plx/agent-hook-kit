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

fn hook_specific_output(
    event_name: &str,
    mut fields: serde_json::Map<String, serde_json::Value>,
) -> serde_json::Value {
    fields.insert(
        "hookEventName".to_string(),
        serde_json::Value::String(event_name.to_string()),
    );
    serde_json::Value::Object(fields)
}

fn block_envelope(reason: impl Into<String>) -> OutputEnvelope {
    OutputEnvelope {
        decision: Some("block".to_string()),
        reason: Some(reason.into()),
        ..OutputEnvelope::new()
    }
}

fn maybe_block(reason: Option<String>) -> OutputEnvelope {
    match reason {
        Some(reason) => block_envelope(reason),
        None => OutputEnvelope::new(),
    }
}

/// Event-scoped Claude output model.
///
/// This keeps event-level output intent explicit and avoids forcing callers
/// through one universal envelope shape.
#[derive(Debug, Clone)]
pub enum ClaudeEventOutput {
    SessionStart(ClaudeSessionStartOutput),
    PromptSubmit(ClaudePromptSubmitOutput),
    PromptExpansion(ClaudePromptExpansionOutput),
    PreToolUse(ClaudePreToolUseOutput),
    PermissionRequest(ClaudePermissionRequestOutput),
    PostToolUse(ClaudePostToolUseOutput),
    PostToolUseFailure(ClaudePostToolUseFailureOutput),
    PostToolBatch(ClaudePostToolBatchOutput),
    PermissionDenied(ClaudePermissionDeniedOutput),
    Stop(ClaudeStopOutput),
    Notification(ClaudeNotificationOutput),
    SubagentStart(ClaudeSubagentStartOutput),
    WorktreeCreate(ClaudeWorktreeCreateOutput),
    CwdChanged(ClaudeWatchPathsOutput),
    FileChanged(ClaudeWatchPathsOutput),
    Elicitation(ClaudeElicitationOutput),
    ElicitationResult(ClaudeElicitationResultOutput),
}

impl From<ClaudeEventOutput> for OutputEnvelope {
    fn from(value: ClaudeEventOutput) -> Self {
        match value {
            ClaudeEventOutput::SessionStart(v) => v.into(),
            ClaudeEventOutput::PromptSubmit(v) => v.into(),
            ClaudeEventOutput::PromptExpansion(v) => v.into(),
            ClaudeEventOutput::PreToolUse(v) => v.into(),
            ClaudeEventOutput::PermissionRequest(v) => v.into(),
            ClaudeEventOutput::PostToolUse(v) => v.into(),
            ClaudeEventOutput::PostToolUseFailure(v) => v.into(),
            ClaudeEventOutput::PostToolBatch(v) => v.into(),
            ClaudeEventOutput::PermissionDenied(v) => v.into(),
            ClaudeEventOutput::Stop(v) => v.into(),
            ClaudeEventOutput::Notification(v) => v.into(),
            ClaudeEventOutput::SubagentStart(v) => v.into(),
            ClaudeEventOutput::WorktreeCreate(v) => v.into(),
            ClaudeEventOutput::CwdChanged(v) => v.into(),
            ClaudeEventOutput::FileChanged(v) => v.into(),
            ClaudeEventOutput::Elicitation(v) => v.into(),
            ClaudeEventOutput::ElicitationResult(v) => v.into(),
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
            let mut fields = serde_json::Map::new();
            fields.insert(
                "additionalContext".to_string(),
                serde_json::Value::String(ctx),
            );
            env.hook_specific_output = Some(hook_specific_output("SessionStart", fields));
        }
        env.system_message = value.system_message;
        env.suppress_output = value.suppress_output;
        env
    }
}

#[derive(Debug, Clone, Default)]
pub struct ClaudePromptSubmitOutput {
    pub block_reason: Option<String>,
    pub additional_context: Option<String>,
    pub session_title: Option<String>,
    pub system_message: Option<String>,
    pub suppress_output: Option<bool>,
}

impl ClaudePromptSubmitOutput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allow() -> Self {
        Self::default()
    }

    pub fn block(reason: impl Into<String>) -> Self {
        Self {
            block_reason: Some(reason.into()),
            ..Self::default()
        }
    }

    pub fn with_context(mut self, message: impl Into<String>) -> Self {
        self.additional_context = Some(message.into());
        self
    }

    pub fn with_session_title(mut self, title: impl Into<String>) -> Self {
        self.session_title = Some(title.into());
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

impl From<ClaudePromptSubmitOutput> for OutputEnvelope {
    fn from(value: ClaudePromptSubmitOutput) -> Self {
        let mut env = maybe_block(value.block_reason);

        let mut fields = serde_json::Map::new();
        if let Some(ctx) = value.additional_context {
            fields.insert(
                "additionalContext".to_string(),
                serde_json::Value::String(ctx),
            );
        }
        if let Some(title) = value.session_title {
            fields.insert("sessionTitle".to_string(), serde_json::Value::String(title));
        }
        if !fields.is_empty() {
            env.hook_specific_output = Some(hook_specific_output("UserPromptSubmit", fields));
        }
        env.system_message = value.system_message;
        env.suppress_output = value.suppress_output;
        env
    }
}

#[derive(Debug, Clone, Default)]
pub struct ClaudePromptExpansionOutput {
    pub block_reason: Option<String>,
    pub additional_context: Option<String>,
    pub system_message: Option<String>,
    pub suppress_output: Option<bool>,
}

impl ClaudePromptExpansionOutput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn allow() -> Self {
        Self::default()
    }

    pub fn block(reason: impl Into<String>) -> Self {
        Self {
            block_reason: Some(reason.into()),
            ..Self::default()
        }
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

impl From<ClaudePromptExpansionOutput> for OutputEnvelope {
    fn from(value: ClaudePromptExpansionOutput) -> Self {
        let mut env = maybe_block(value.block_reason);
        if let Some(ctx) = value.additional_context {
            let mut fields = serde_json::Map::new();
            fields.insert(
                "additionalContext".to_string(),
                serde_json::Value::String(ctx),
            );
            env.hook_specific_output = Some(hook_specific_output("UserPromptExpansion", fields));
        }
        env.system_message = value.system_message;
        env.suppress_output = value.suppress_output;
        env
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ClaudePreToolDecision {
    Allow,
    Deny,
    Ask,
    Defer,
}

impl ClaudePreToolDecision {
    fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Ask => "ask",
            Self::Defer => "defer",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClaudePreToolUseOutput {
    pub decision: ClaudePreToolDecision,
    pub reason: Option<String>,
    pub updated_input: Option<serde_json::Value>,
    pub additional_context: Option<String>,
}

impl ClaudePreToolUseOutput {
    pub fn allow() -> Self {
        Self {
            decision: ClaudePreToolDecision::Allow,
            reason: None,
            updated_input: None,
            additional_context: None,
        }
    }

    pub fn deny(reason: impl Into<String>) -> Self {
        Self {
            decision: ClaudePreToolDecision::Deny,
            reason: Some(reason.into()),
            updated_input: None,
            additional_context: None,
        }
    }

    pub fn ask(reason: impl Into<String>) -> Self {
        Self {
            decision: ClaudePreToolDecision::Ask,
            reason: Some(reason.into()),
            updated_input: None,
            additional_context: None,
        }
    }

    pub fn defer() -> Self {
        Self {
            decision: ClaudePreToolDecision::Defer,
            reason: None,
            updated_input: None,
            additional_context: None,
        }
    }

    pub fn with_updated_input(mut self, updated_input: serde_json::Value) -> Self {
        self.updated_input = Some(updated_input);
        self
    }

    pub fn with_context(mut self, message: impl Into<String>) -> Self {
        self.additional_context = Some(message.into());
        self
    }
}

impl From<ClaudePreToolUseOutput> for OutputEnvelope {
    fn from(value: ClaudePreToolUseOutput) -> Self {
        let mut fields = serde_json::Map::new();
        fields.insert(
            "permissionDecision".to_string(),
            serde_json::Value::String(value.decision.as_str().to_string()),
        );
        if let Some(reason) = value.reason {
            fields.insert(
                "permissionDecisionReason".to_string(),
                serde_json::Value::String(reason),
            );
        }
        if let Some(updated_input) = value.updated_input {
            fields.insert("updatedInput".to_string(), updated_input);
        }
        if let Some(ctx) = value.additional_context {
            fields.insert(
                "additionalContext".to_string(),
                serde_json::Value::String(ctx),
            );
        }
        OutputEnvelope {
            hook_specific_output: Some(hook_specific_output("PreToolUse", fields)),
            ..OutputEnvelope::new()
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ClaudePermissionRequestBehavior {
    Allow,
    Deny,
}

impl ClaudePermissionRequestBehavior {
    fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClaudePermissionRequestOutput {
    pub behavior: ClaudePermissionRequestBehavior,
    pub updated_input: Option<serde_json::Value>,
    pub updated_permissions: Option<Vec<serde_json::Value>>,
    pub message: Option<String>,
    pub interrupt: Option<bool>,
}

impl ClaudePermissionRequestOutput {
    pub fn allow() -> Self {
        Self {
            behavior: ClaudePermissionRequestBehavior::Allow,
            updated_input: None,
            updated_permissions: None,
            message: None,
            interrupt: None,
        }
    }

    pub fn deny(message: impl Into<String>) -> Self {
        Self {
            behavior: ClaudePermissionRequestBehavior::Deny,
            updated_input: None,
            updated_permissions: None,
            message: Some(message.into()),
            interrupt: None,
        }
    }

    pub fn with_updated_input(mut self, updated_input: serde_json::Value) -> Self {
        self.updated_input = Some(updated_input);
        self
    }

    pub fn with_updated_permissions(mut self, entries: Vec<serde_json::Value>) -> Self {
        self.updated_permissions = Some(entries);
        self
    }

    pub fn with_interrupt(mut self, interrupt: bool) -> Self {
        self.interrupt = Some(interrupt);
        self
    }
}

impl From<ClaudePermissionRequestOutput> for OutputEnvelope {
    fn from(value: ClaudePermissionRequestOutput) -> Self {
        let mut decision = serde_json::Map::new();
        decision.insert(
            "behavior".to_string(),
            serde_json::Value::String(value.behavior.as_str().to_string()),
        );
        if let Some(updated_input) = value.updated_input {
            decision.insert("updatedInput".to_string(), updated_input);
        }
        if let Some(updated_permissions) = value.updated_permissions {
            decision.insert(
                "updatedPermissions".to_string(),
                serde_json::Value::Array(updated_permissions),
            );
        }
        if let Some(message) = value.message {
            decision.insert("message".to_string(), serde_json::Value::String(message));
        }
        if let Some(interrupt) = value.interrupt {
            decision.insert("interrupt".to_string(), serde_json::Value::Bool(interrupt));
        }

        let mut fields = serde_json::Map::new();
        fields.insert("decision".to_string(), serde_json::Value::Object(decision));
        OutputEnvelope {
            hook_specific_output: Some(hook_specific_output("PermissionRequest", fields)),
            ..OutputEnvelope::new()
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ClaudePostToolUseOutput {
    pub block_reason: Option<String>,
    pub additional_context: Option<String>,
    pub updated_mcp_tool_output: Option<serde_json::Value>,
}

impl ClaudePostToolUseOutput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn block(reason: impl Into<String>) -> Self {
        Self {
            block_reason: Some(reason.into()),
            ..Self::default()
        }
    }

    pub fn with_context(mut self, message: impl Into<String>) -> Self {
        self.additional_context = Some(message.into());
        self
    }

    pub fn with_updated_mcp_tool_output(mut self, output: serde_json::Value) -> Self {
        self.updated_mcp_tool_output = Some(output);
        self
    }
}

impl From<ClaudePostToolUseOutput> for OutputEnvelope {
    fn from(value: ClaudePostToolUseOutput) -> Self {
        let mut env = maybe_block(value.block_reason);
        let mut fields = serde_json::Map::new();
        if let Some(ctx) = value.additional_context {
            fields.insert(
                "additionalContext".to_string(),
                serde_json::Value::String(ctx),
            );
        }
        if let Some(output) = value.updated_mcp_tool_output {
            fields.insert("updatedMCPToolOutput".to_string(), output);
        }
        if !fields.is_empty() {
            env.hook_specific_output = Some(hook_specific_output("PostToolUse", fields));
        }
        env
    }
}

#[derive(Debug, Clone, Default)]
pub struct ClaudePostToolUseFailureOutput {
    pub block_reason: Option<String>,
    pub additional_context: Option<String>,
}

impl ClaudePostToolUseFailureOutput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn block(reason: impl Into<String>) -> Self {
        Self {
            block_reason: Some(reason.into()),
            ..Self::default()
        }
    }

    pub fn with_context(mut self, message: impl Into<String>) -> Self {
        self.additional_context = Some(message.into());
        self
    }
}

impl From<ClaudePostToolUseFailureOutput> for OutputEnvelope {
    fn from(value: ClaudePostToolUseFailureOutput) -> Self {
        let mut env = maybe_block(value.block_reason);
        if let Some(ctx) = value.additional_context {
            let mut fields = serde_json::Map::new();
            fields.insert(
                "additionalContext".to_string(),
                serde_json::Value::String(ctx),
            );
            env.hook_specific_output = Some(hook_specific_output("PostToolUseFailure", fields));
        }
        env
    }
}

#[derive(Debug, Clone, Default)]
pub struct ClaudePostToolBatchOutput {
    pub block_reason: Option<String>,
    pub additional_context: Option<String>,
}

impl ClaudePostToolBatchOutput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn block(reason: impl Into<String>) -> Self {
        Self {
            block_reason: Some(reason.into()),
            ..Self::default()
        }
    }

    pub fn with_context(mut self, message: impl Into<String>) -> Self {
        self.additional_context = Some(message.into());
        self
    }
}

impl From<ClaudePostToolBatchOutput> for OutputEnvelope {
    fn from(value: ClaudePostToolBatchOutput) -> Self {
        let mut env = maybe_block(value.block_reason);
        if let Some(ctx) = value.additional_context {
            let mut fields = serde_json::Map::new();
            fields.insert(
                "additionalContext".to_string(),
                serde_json::Value::String(ctx),
            );
            env.hook_specific_output = Some(hook_specific_output("PostToolBatch", fields));
        }
        env
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
            let mut fields = serde_json::Map::new();
            fields.insert("retry".to_string(), serde_json::Value::Bool(true));
            OutputEnvelope {
                hook_specific_output: Some(hook_specific_output("PermissionDenied", fields)),
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
            ClaudeStopOutput::ContinueSession { reason } => block_envelope(reason),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ClaudeNotificationOutput {
    pub additional_context: Option<String>,
}

impl ClaudeNotificationOutput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_context(mut self, message: impl Into<String>) -> Self {
        self.additional_context = Some(message.into());
        self
    }
}

impl From<ClaudeNotificationOutput> for OutputEnvelope {
    fn from(value: ClaudeNotificationOutput) -> Self {
        if let Some(ctx) = value.additional_context {
            let mut fields = serde_json::Map::new();
            fields.insert(
                "additionalContext".to_string(),
                serde_json::Value::String(ctx),
            );
            OutputEnvelope {
                hook_specific_output: Some(hook_specific_output("Notification", fields)),
                ..OutputEnvelope::new()
            }
        } else {
            OutputEnvelope::new()
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ClaudeSubagentStartOutput {
    pub additional_context: Option<String>,
}

impl ClaudeSubagentStartOutput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_context(mut self, message: impl Into<String>) -> Self {
        self.additional_context = Some(message.into());
        self
    }
}

impl From<ClaudeSubagentStartOutput> for OutputEnvelope {
    fn from(value: ClaudeSubagentStartOutput) -> Self {
        if let Some(ctx) = value.additional_context {
            let mut fields = serde_json::Map::new();
            fields.insert(
                "additionalContext".to_string(),
                serde_json::Value::String(ctx),
            );
            OutputEnvelope {
                hook_specific_output: Some(hook_specific_output("SubagentStart", fields)),
                ..OutputEnvelope::new()
            }
        } else {
            OutputEnvelope::new()
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
        let mut fields = serde_json::Map::new();
        fields.insert(
            "worktreePath".to_string(),
            serde_json::Value::String(value.worktree_path),
        );
        OutputEnvelope {
            hook_specific_output: Some(hook_specific_output("WorktreeCreate", fields)),
            ..OutputEnvelope::new()
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ClaudeWatchPathsOutput {
    pub watch_paths: Vec<String>,
}

impl ClaudeWatchPathsOutput {
    pub fn new(paths: Vec<String>) -> Self {
        Self { watch_paths: paths }
    }
}

impl From<ClaudeWatchPathsOutput> for OutputEnvelope {
    fn from(value: ClaudeWatchPathsOutput) -> Self {
        OutputEnvelope {
            hook_specific_output: Some(serde_json::json!({
                "watchPaths": value.watch_paths
            })),
            ..OutputEnvelope::new()
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClaudeElicitationOutput {
    pub action: String,
    pub content: Option<serde_json::Value>,
}

impl ClaudeElicitationOutput {
    pub fn accept(content: serde_json::Value) -> Self {
        Self {
            action: "accept".to_string(),
            content: Some(content),
        }
    }

    pub fn decline() -> Self {
        Self {
            action: "decline".to_string(),
            content: None,
        }
    }

    pub fn cancel() -> Self {
        Self {
            action: "cancel".to_string(),
            content: None,
        }
    }
}

impl From<ClaudeElicitationOutput> for OutputEnvelope {
    fn from(value: ClaudeElicitationOutput) -> Self {
        let mut fields = serde_json::Map::new();
        fields.insert(
            "action".to_string(),
            serde_json::Value::String(value.action),
        );
        if let Some(content) = value.content {
            fields.insert("content".to_string(), content);
        }
        OutputEnvelope {
            hook_specific_output: Some(hook_specific_output("Elicitation", fields)),
            ..OutputEnvelope::new()
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClaudeElicitationResultOutput {
    pub action: String,
    pub content: Option<serde_json::Value>,
}

impl ClaudeElicitationResultOutput {
    pub fn accept(content: serde_json::Value) -> Self {
        Self {
            action: "accept".to_string(),
            content: Some(content),
        }
    }

    pub fn decline() -> Self {
        Self {
            action: "decline".to_string(),
            content: None,
        }
    }

    pub fn cancel() -> Self {
        Self {
            action: "cancel".to_string(),
            content: None,
        }
    }
}

impl From<ClaudeElicitationResultOutput> for OutputEnvelope {
    fn from(value: ClaudeElicitationResultOutput) -> Self {
        let mut fields = serde_json::Map::new();
        fields.insert(
            "action".to_string(),
            serde_json::Value::String(value.action),
        );
        if let Some(content) = value.content {
            fields.insert("content".to_string(), content);
        }
        OutputEnvelope {
            hook_specific_output: Some(hook_specific_output("ElicitationResult", fields)),
            ..OutputEnvelope::new()
        }
    }
}

impl OutputEnvelope {
    pub fn new() -> Self {
        Self::default()
    }

    // --- Block-only decision helpers ---

    /// Block the current action with a user/model-visible reason.
    pub fn block(reason: impl Into<String>) -> Self {
        block_envelope(reason)
    }

    /// Allow the action to proceed by emitting no decision fields.
    pub fn allow() -> Self {
        OutputEnvelope::new()
    }

    // --- PreToolUse helpers ---

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

    /// Defer a PreToolUse so it can be resumed later.
    pub fn pre_tool_defer() -> Self {
        ClaudePreToolUseOutput::defer().into()
    }

    /// Return a PreToolUse permission decision with rewritten input.
    pub fn pre_tool_deny_with_updated_input(
        reason: impl Into<String>,
        updated_input: serde_json::Value,
    ) -> Self {
        ClaudePreToolUseOutput::deny(reason)
            .with_updated_input(updated_input)
            .into()
    }

    // --- Context injection ---

    /// Inject additional context for a PostToolUse hook.
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
        ClaudePermissionRequestOutput::allow().into()
    }

    /// Deny a permission request.
    pub fn permission_deny(reason: impl Into<String>) -> Self {
        ClaudePermissionRequestOutput::deny(reason).into()
    }

    // --- WorktreeCreate: path return ---

    /// Return a worktree path for WorktreeCreate.
    pub fn worktree_path(path: impl Into<String>) -> Self {
        ClaudeWorktreeCreateOutput::new(path).into()
    }

    // --- FileChanged / CwdChanged: watch paths ---

    /// Set dynamic watch paths.
    pub fn watch_paths(paths: Vec<String>) -> Self {
        ClaudeWatchPathsOutput::new(paths).into()
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
