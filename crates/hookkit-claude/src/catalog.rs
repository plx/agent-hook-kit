//! Command-runtime implementations for the Claude events that share the
//! standard hook envelope.

use hookkit_core::{
    ContractId, EventCategory, EventId, EventSpec, HarnessId, NativeContext, ProcessEmission,
    RawInvocation, SessionId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{ClaudeCommandEnvironment, protocol::SNAPSHOT_ID};

/// Lossless shared input envelope for the command events in this module.
///
/// Event-specific fields remain available through [`Self::field`] while the
/// fields common to every Claude command hook are strongly typed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogInput {
    pub session_id: String,
    pub transcript_path: hookkit_core::Utf8PathBuf,
    pub cwd: hookkit_core::Utf8PathBuf,
    pub hook_event_name: String,
    #[serde(default)]
    pub effort: Option<crate::protocol::Effort>,
    #[serde(default)]
    pub permission_mode: Option<crate::protocol::PermissionMode>,
    #[serde(default)]
    pub prompt_id: Option<String>,
    #[serde(flatten)]
    fields: BTreeMap<String, serde_json::Value>,
}

impl CatalogInput {
    pub fn field(&self, name: &str) -> Option<&serde_json::Value> {
        self.fields.get(name)
    }

    pub fn fields(&self) -> &BTreeMap<String, serde_json::Value> {
        &self.fields
    }

    pub(crate) fn event_id(&self) -> EventId {
        catalog_event_id(&self.hook_event_name)
    }

    pub(crate) fn context(&self) -> NativeContext {
        NativeContext {
            workspace_roots: vec![self.cwd.clone()],
            session_id: SessionId::new(&self.session_id).ok(),
            transcript_path: Some(self.transcript_path.clone()),
            ..NativeContext::default()
        }
    }

    pub(crate) fn validate_environment(
        &self,
        environment: &ClaudeCommandEnvironment,
    ) -> hookkit_core::Result<()> {
        environment.validate_input(
            &self.event_id(),
            &self.session_id,
            self.effort.as_ref().map(|effort| effort.level.as_str()),
        )
    }
}

fn catalog_event_id(event: &str) -> EventId {
    let event = match event {
        "ConfigChange" => "ConfigChange",
        "CwdChanged" => "CwdChanged",
        "Elicitation" => "Elicitation",
        "ElicitationResult" => "ElicitationResult",
        "FileChanged" => "FileChanged",
        "InstructionsLoaded" => "InstructionsLoaded",
        "MessageDisplay" => "MessageDisplay",
        "Notification" => "Notification",
        "PermissionDenied" => "PermissionDenied",
        "PermissionRequest" => "PermissionRequest",
        "PostCompact" => "PostCompact",
        "PostToolBatch" => "PostToolBatch",
        "PostToolUseFailure" => "PostToolUseFailure",
        "PreCompact" => "PreCompact",
        "PreToolUse" => "PreToolUse",
        "SessionEnd" => "SessionEnd",
        "Setup" => "Setup",
        "Stop" => "Stop",
        "StopFailure" => "StopFailure",
        "SubagentStart" => "SubagentStart",
        "SubagentStop" => "SubagentStop",
        "TaskCompleted" => "TaskCompleted",
        "TaskCreated" => "TaskCreated",
        "TeammateIdle" => "TeammateIdle",
        "UserPromptExpansion" => "UserPromptExpansion",
        "UserPromptSubmit" => "UserPromptSubmit",
        "WorktreeRemove" => "WorktreeRemove",
        _ => unreachable!("catalog inputs are created only by exact event parsers"),
    };
    EventId::builtin(HarnessId::CLAUDE_CODE, event)
}

#[derive(Debug, Clone)]
enum Outcome {
    Json(serde_json::Value),
    Text(String),
    BlockingError(String),
}

#[derive(Debug, Clone)]
pub struct CatalogOutput {
    event: &'static str,
    outcome: Outcome,
}

impl CatalogOutput {
    fn json(event: &'static str, value: serde_json::Value) -> Self {
        Self {
            event,
            outcome: Outcome::Json(value),
        }
    }

    fn text(event: &'static str, value: impl Into<String>) -> Self {
        Self {
            event,
            outcome: Outcome::Text(value.into()),
        }
    }

    fn blocking(event: &'static str, message: impl Into<String>) -> Self {
        Self {
            event,
            outcome: Outcome::BlockingError(message.into()),
        }
    }

    fn with_top_level(
        mut self,
        name: &'static str,
        value: serde_json::Value,
    ) -> hookkit_core::Result<Self> {
        let Outcome::Json(output) = &mut self.outcome else {
            return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "structured fields cannot be added to text or blocking output",
            ));
        };
        output
            .as_object_mut()
            .ok_or(hookkit_core::HookkitError::InvalidProcessEmission(
                "structured command output must be a JSON object",
            ))?
            .insert(name.into(), value);
        Ok(self)
    }

    fn with_permission_decision_field(
        mut self,
        name: &'static str,
        value: serde_json::Value,
    ) -> hookkit_core::Result<Self> {
        let Outcome::Json(output) = &mut self.outcome else {
            return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "permission decision fields require structured output",
            ));
        };
        output
            .get_mut("hookSpecificOutput")
            .and_then(|specific| specific.get_mut("decision"))
            .and_then(serde_json::Value::as_object_mut)
            .ok_or(hookkit_core::HookkitError::InvalidProcessEmission(
                "permission decision fields require an existing decision",
            ))?
            .insert(name.into(), value);
        Ok(self)
    }

    pub(crate) fn event_id(&self) -> EventId {
        EventId::builtin(HarnessId::CLAUDE_CODE, self.event)
    }

    pub(crate) fn emit(self) -> hookkit_core::Result<ProcessEmission> {
        let contract = ContractId::builtin(match self.event {
            "ConfigChange" => "claude-code/docs-2026-07-12-r2/ConfigChange",
            "CwdChanged" => "claude-code/docs-2026-07-12-r2/CwdChanged",
            "Elicitation" => "claude-code/docs-2026-07-12-r2/Elicitation",
            "ElicitationResult" => "claude-code/docs-2026-07-12-r2/ElicitationResult",
            "FileChanged" => "claude-code/docs-2026-07-12-r2/FileChanged",
            "InstructionsLoaded" => "claude-code/docs-2026-07-12-r2/InstructionsLoaded",
            "MessageDisplay" => "claude-code/docs-2026-07-12-r2/MessageDisplay",
            "Notification" => "claude-code/docs-2026-07-12-r2/Notification",
            "PermissionDenied" => "claude-code/docs-2026-07-12-r2/PermissionDenied",
            "PermissionRequest" => "claude-code/docs-2026-07-12-r2/PermissionRequest",
            "PostCompact" => "claude-code/docs-2026-07-12-r2/PostCompact",
            "PostToolBatch" => "claude-code/docs-2026-07-12-r2/PostToolBatch",
            "PostToolUseFailure" => "claude-code/docs-2026-07-12-r2/PostToolUseFailure",
            "PreCompact" => "claude-code/docs-2026-07-12-r2/PreCompact",
            "PreToolUse" => "claude-code/docs-2026-07-12-r2/PreToolUse",
            "SessionEnd" => "claude-code/docs-2026-07-12-r2/SessionEnd",
            "Setup" => "claude-code/docs-2026-07-12-r2/Setup",
            "Stop" => "claude-code/docs-2026-07-12-r2/Stop",
            "StopFailure" => "claude-code/docs-2026-07-12-r2/StopFailure",
            "SubagentStart" => "claude-code/docs-2026-07-12-r2/SubagentStart",
            "SubagentStop" => "claude-code/docs-2026-07-12-r2/SubagentStop",
            "TaskCompleted" => "claude-code/docs-2026-07-12-r2/TaskCompleted",
            "TaskCreated" => "claude-code/docs-2026-07-12-r2/TaskCreated",
            "TeammateIdle" => "claude-code/docs-2026-07-12-r2/TeammateIdle",
            "UserPromptExpansion" => "claude-code/docs-2026-07-12-r2/UserPromptExpansion",
            "UserPromptSubmit" => "claude-code/docs-2026-07-12-r2/UserPromptSubmit",
            "WorktreeRemove" => "claude-code/docs-2026-07-12-r2/WorktreeRemove",
            _ => unreachable!("catalog output constructors fix the event"),
        });
        match self.outcome {
            Outcome::Json(value) => ProcessEmission::command_json(contract, &value),
            Outcome::Text(value) => Ok(ProcessEmission::command_text(contract, value)),
            Outcome::BlockingError(message) => {
                ProcessEmission::command_required_stderr(contract, message, 2)
            }
        }
    }
}

fn parse(
    invocation: &RawInvocation,
    event: &'static str,
    required_fields: &[&str],
) -> hookkit_core::Result<CatalogInput> {
    super::protocol::require_event(invocation, event)?;
    for field in required_fields {
        if invocation.json().get(field).is_none() {
            return Err(hookkit_core::HookkitError::InvalidInputForHint {
                event: EventId::builtin(HarnessId::CLAUDE_CODE, event),
                message: format!("missing required field {field}"),
            });
        }
    }
    serde_json::from_value(invocation.json().clone()).map_err(Into::into)
}

fn specific(event: &'static str, fields: serde_json::Value) -> serde_json::Value {
    let mut fields = fields.as_object().cloned().unwrap_or_default();
    fields.insert("hookEventName".into(), event.into());
    serde_json::json!({"hookSpecificOutput": fields})
}

fn context(event: &'static str, value: impl Into<String>) -> serde_json::Value {
    specific(
        event,
        serde_json::json!({"additionalContext": value.into()}),
    )
}

fn block(reason: impl Into<String>) -> serde_json::Value {
    serde_json::json!({"decision": "block", "reason": reason.into()})
}

fn block_with_context(
    event: &'static str,
    reason: impl Into<String>,
    additional_context: impl Into<String>,
) -> serde_json::Value {
    let mut value = block(reason);
    value.as_object_mut().expect("object").insert(
        "hookSpecificOutput".into(),
        serde_json::json!({
            "hookEventName": event,
            "additionalContext": additional_context.into(),
        }),
    );
    value
}

macro_rules! event_spec {
    ($event:ident, $output:ident, $name:literal, $category:ident, [$($required:literal),* $(,)?]) => {
        pub enum $event {}

        impl EventSpec for $event {
            type Input = CatalogInput;
            type CommandEnvironment = ClaudeCommandEnvironment;
            type CommandOutput = $output;
            const HARNESS: HarnessId = HarnessId::CLAUDE_CODE;
            const SNAPSHOT: hookkit_core::SnapshotId = SNAPSHOT_ID;
            const EVENT: EventId = EventId::builtin(HarnessId::CLAUDE_CODE, $name);
            const CATEGORY: EventCategory = EventCategory::$category;
            const CONTRACT: ContractId = ContractId::builtin(concat!(
                "claude-code/docs-2026-07-12-r2/",
                $name
            ));

            fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
                parse(invocation, $name, &[$($required),*])
            }

            fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
                output.0.emit()
            }

            fn validate_command_environment(
                input: &Self::Input,
                environment: &Self::CommandEnvironment,
            ) -> hookkit_core::Result<()> {
                input.validate_environment(environment)
            }

            fn context(input: &Self::Input) -> NativeContext {
                input.context()
            }
        }

        impl From<$output> for CatalogOutput {
            fn from(output: $output) -> Self {
                output.0
            }
        }

        impl From<$output> for crate::protocol::AnyCommandOutput {
            fn from(output: $output) -> Self {
                Self::Catalog(output.0)
            }
        }
    };
}

macro_rules! system_event {
    ($event:ident, $output:ident, $name:literal, $category:ident, [$($required:literal),* $(,)?]) => {
        #[derive(Debug, Clone)]
        pub struct $output(CatalogOutput);

        impl $output {
            pub fn no_op() -> Self {
                Self(CatalogOutput::json($name, serde_json::json!({})))
            }

            pub fn with_system_message(message: impl Into<String>) -> Self {
                Self(CatalogOutput::json(
                    $name,
                    serde_json::json!({"systemMessage": message.into()}),
                ))
            }
        }

        event_spec!($event, $output, $name, $category, [$($required),*]);
    };
}

macro_rules! context_event {
    ($event:ident, $output:ident, $name:literal, $category:ident, [$($required:literal),* $(,)?]) => {
        #[derive(Debug, Clone)]
        pub struct $output(CatalogOutput);

        impl $output {
            pub fn no_op() -> Self {
                Self(CatalogOutput::json($name, serde_json::json!({})))
            }

            pub fn with_context(additional_context: impl Into<String>) -> Self {
                Self(CatalogOutput::json($name, context($name, additional_context)))
            }

            pub fn block(reason: impl Into<String>) -> Self {
                Self(CatalogOutput::json($name, block(reason)))
            }

            pub fn block_with_context(
                reason: impl Into<String>,
                additional_context: impl Into<String>,
            ) -> Self {
                Self(CatalogOutput::json(
                    $name,
                    block_with_context($name, reason, additional_context),
                ))
            }

            pub fn with_continue(self, continue_session: bool) -> hookkit_core::Result<Self> {
                self.0
                    .with_top_level("continue", continue_session.into())
                    .map(Self)
            }

            pub fn with_stop_reason(
                self,
                reason: impl Into<String>,
            ) -> hookkit_core::Result<Self> {
                self.0
                    .with_top_level("stopReason", reason.into().into())
                    .map(Self)
            }

            pub fn with_suppress_output(self, suppress: bool) -> hookkit_core::Result<Self> {
                self.0
                    .with_top_level("suppressOutput", suppress.into())
                    .map(Self)
            }

            pub fn with_system_message(
                self,
                message: impl Into<String>,
            ) -> hookkit_core::Result<Self> {
                self.0
                    .with_top_level("systemMessage", message.into().into())
                    .map(Self)
            }
        }

        event_spec!($event, $output, $name, $category, [$($required),*]);
    };
}

macro_rules! blocking_context_event {
    ($event:ident, $output:ident, $name:literal, $category:ident, [$($required:literal),* $(,)?]) => {
        context_event!($event, $output, $name, $category, [$($required),*]);

        impl $output {
            pub fn blocking_error(message: impl Into<String>) -> Self {
                Self(CatalogOutput::blocking($name, message))
            }
        }
    };
}

system_event!(
    CwdChanged,
    CwdChangedOutput,
    "CwdChanged",
    Context,
    ["old_cwd"]
);
system_event!(
    FileChanged,
    FileChangedOutput,
    "FileChanged",
    Context,
    ["file_path"]
);
system_event!(
    InstructionsLoaded,
    InstructionsLoadedOutput,
    "InstructionsLoaded",
    Context,
    ["file_path"]
);
system_event!(
    Notification,
    NotificationOutput,
    "Notification",
    Other,
    ["message"]
);
system_event!(
    PostCompact,
    PostCompactOutput,
    "PostCompact",
    Context,
    ["trigger"]
);
system_event!(
    SessionEnd,
    SessionEndOutput,
    "SessionEnd",
    Session,
    ["reason"]
);
system_event!(
    StopFailure,
    StopFailureOutput,
    "StopFailure",
    Agent,
    ["error"]
);
system_event!(
    WorktreeRemove,
    WorktreeRemoveOutput,
    "WorktreeRemove",
    Worktree,
    ["worktree_path"]
);

blocking_context_event!(
    ConfigChange,
    ConfigChangeOutput,
    "ConfigChange",
    Context,
    ["source"]
);
blocking_context_event!(
    PostToolBatch,
    PostToolBatchOutput,
    "PostToolBatch",
    Tool,
    ["tool_calls"]
);
context_event!(
    PostToolUseFailure,
    PostToolUseFailureOutput,
    "PostToolUseFailure",
    Tool,
    ["tool_name"]
);
blocking_context_event!(
    PreCompact,
    PreCompactOutput,
    "PreCompact",
    Context,
    ["trigger"]
);
context_event!(Setup, SetupOutput, "Setup", Session, ["trigger"]);
blocking_context_event!(Stop, StopOutput, "Stop", Agent, ["stop_hook_active"]);
context_event!(
    SubagentStart,
    SubagentStartOutput,
    "SubagentStart",
    Agent,
    ["agent_id"]
);
blocking_context_event!(
    SubagentStop,
    SubagentStopOutput,
    "SubagentStop",
    Agent,
    ["stop_hook_active"]
);
blocking_context_event!(
    TaskCompleted,
    TaskCompletedOutput,
    "TaskCompleted",
    Agent,
    ["task_id"]
);
blocking_context_event!(
    TaskCreated,
    TaskCreatedOutput,
    "TaskCreated",
    Agent,
    ["task_id"]
);
blocking_context_event!(
    TeammateIdle,
    TeammateIdleOutput,
    "TeammateIdle",
    Agent,
    ["teammate_name"]
);

#[derive(Debug, Clone)]
pub struct ElicitationOutput(CatalogOutput);

impl ElicitationOutput {
    pub fn no_op() -> Self {
        Self(CatalogOutput::json("Elicitation", serde_json::json!({})))
    }

    pub fn accept(content: serde_json::Value) -> Self {
        Self(CatalogOutput::json(
            "Elicitation",
            specific(
                "Elicitation",
                serde_json::json!({"action": "accept", "content": content}),
            ),
        ))
    }

    pub fn decline() -> Self {
        Self(CatalogOutput::json(
            "Elicitation",
            specific("Elicitation", serde_json::json!({"action": "decline"})),
        ))
    }

    pub fn cancel() -> Self {
        Self(CatalogOutput::json(
            "Elicitation",
            specific("Elicitation", serde_json::json!({"action": "cancel"})),
        ))
    }

    pub fn blocking_error(message: impl Into<String>) -> Self {
        Self(CatalogOutput::blocking("Elicitation", message))
    }
}
event_spec!(
    Elicitation,
    ElicitationOutput,
    "Elicitation",
    Other,
    ["mcp_server_name"]
);

#[derive(Debug, Clone)]
pub struct ElicitationResultOutput(CatalogOutput);

impl ElicitationResultOutput {
    pub fn no_op() -> Self {
        Self(CatalogOutput::json(
            "ElicitationResult",
            serde_json::json!({}),
        ))
    }

    pub fn accept(content: serde_json::Value) -> Self {
        Self(CatalogOutput::json(
            "ElicitationResult",
            specific(
                "ElicitationResult",
                serde_json::json!({"action": "accept", "content": content}),
            ),
        ))
    }

    pub fn decline() -> Self {
        Self(CatalogOutput::json(
            "ElicitationResult",
            specific(
                "ElicitationResult",
                serde_json::json!({"action": "decline"}),
            ),
        ))
    }

    pub fn cancel() -> Self {
        Self(CatalogOutput::json(
            "ElicitationResult",
            specific("ElicitationResult", serde_json::json!({"action": "cancel"})),
        ))
    }

    pub fn blocking_error(message: impl Into<String>) -> Self {
        Self(CatalogOutput::blocking("ElicitationResult", message))
    }
}
event_spec!(
    ElicitationResult,
    ElicitationResultOutput,
    "ElicitationResult",
    Other,
    ["mcp_server_name"]
);

#[derive(Debug, Clone)]
pub struct MessageDisplayOutput(CatalogOutput);

impl MessageDisplayOutput {
    pub fn no_op() -> Self {
        Self(CatalogOutput::json("MessageDisplay", serde_json::json!({})))
    }

    pub fn display(content: impl Into<String>) -> Self {
        Self(CatalogOutput::json(
            "MessageDisplay",
            specific(
                "MessageDisplay",
                serde_json::json!({"displayContent": content.into()}),
            ),
        ))
    }
}
event_spec!(
    MessageDisplay,
    MessageDisplayOutput,
    "MessageDisplay",
    Other,
    ["turn_id"]
);

#[derive(Debug, Clone)]
pub struct PermissionDeniedOutput(CatalogOutput);

impl PermissionDeniedOutput {
    pub fn no_op() -> Self {
        Self(CatalogOutput::json(
            "PermissionDenied",
            serde_json::json!({}),
        ))
    }

    pub fn retry(retry: bool) -> Self {
        Self(CatalogOutput::json(
            "PermissionDenied",
            specific("PermissionDenied", serde_json::json!({"retry": retry})),
        ))
    }
}
event_spec!(
    PermissionDenied,
    PermissionDeniedOutput,
    "PermissionDenied",
    Tool,
    ["tool_name"]
);

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionRequestBehavior {
    Allow,
    Deny,
}

#[derive(Debug, Clone)]
pub struct PermissionRequestOutput(CatalogOutput);

impl PermissionRequestOutput {
    pub fn no_op() -> Self {
        Self(CatalogOutput::json(
            "PermissionRequest",
            serde_json::json!({}),
        ))
    }

    pub fn decide(
        behavior: PermissionRequestBehavior,
        message: Option<String>,
        interrupt: Option<bool>,
    ) -> Self {
        let mut decision = serde_json::Map::new();
        decision.insert(
            "behavior".into(),
            serde_json::to_value(behavior).expect("enum serialization cannot fail"),
        );
        if let Some(message) = message {
            decision.insert("message".into(), message.into());
        }
        if let Some(interrupt) = interrupt {
            decision.insert("interrupt".into(), interrupt.into());
        }
        Self(CatalogOutput::json(
            "PermissionRequest",
            specific(
                "PermissionRequest",
                serde_json::json!({"decision": decision}),
            ),
        ))
    }

    pub fn blocking_error(message: impl Into<String>) -> Self {
        Self(CatalogOutput::blocking("PermissionRequest", message))
    }

    pub fn with_continue(self, continue_session: bool) -> hookkit_core::Result<Self> {
        self.0
            .with_top_level("continue", continue_session.into())
            .map(Self)
    }

    pub fn with_stop_reason(self, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        self.0
            .with_top_level("stopReason", reason.into().into())
            .map(Self)
    }

    pub fn with_suppress_output(self, suppress: bool) -> hookkit_core::Result<Self> {
        self.0
            .with_top_level("suppressOutput", suppress.into())
            .map(Self)
    }

    pub fn with_system_message(self, message: impl Into<String>) -> hookkit_core::Result<Self> {
        self.0
            .with_top_level("systemMessage", message.into().into())
            .map(Self)
    }

    pub fn with_updated_input(self, input: serde_json::Value) -> hookkit_core::Result<Self> {
        self.0
            .with_permission_decision_field("updatedInput", input)
            .map(Self)
    }

    pub fn with_updated_permissions(
        self,
        permissions: Vec<serde_json::Value>,
    ) -> hookkit_core::Result<Self> {
        self.0
            .with_permission_decision_field("updatedPermissions", permissions.into())
            .map(Self)
    }
}
event_spec!(
    PermissionRequest,
    PermissionRequestOutput,
    "PermissionRequest",
    Tool,
    ["tool_name"]
);

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PreToolPermissionDecision {
    Allow,
    Deny,
    Ask,
}

#[derive(Debug, Clone)]
pub struct PreToolUseOutput(CatalogOutput);

impl PreToolUseOutput {
    pub fn no_op() -> Self {
        Self(CatalogOutput::json("PreToolUse", serde_json::json!({})))
    }

    pub fn decide(
        decision: PreToolPermissionDecision,
        reason: Option<String>,
        updated_input: Option<serde_json::Map<String, serde_json::Value>>,
        additional_context: Option<String>,
    ) -> Self {
        let mut fields = serde_json::Map::new();
        fields.insert(
            "permissionDecision".into(),
            serde_json::to_value(decision).expect("enum serialization cannot fail"),
        );
        if let Some(reason) = reason {
            fields.insert("permissionDecisionReason".into(), reason.into());
        }
        if let Some(updated_input) = updated_input {
            fields.insert("updatedInput".into(), updated_input.into());
        }
        if let Some(context) = additional_context {
            fields.insert("additionalContext".into(), context.into());
        }
        Self(CatalogOutput::json(
            "PreToolUse",
            specific("PreToolUse", fields.into()),
        ))
    }

    pub fn blocking_error(message: impl Into<String>) -> Self {
        Self(CatalogOutput::blocking("PreToolUse", message))
    }

    pub fn with_continue(self, continue_session: bool) -> hookkit_core::Result<Self> {
        self.0
            .with_top_level("continue", continue_session.into())
            .map(Self)
    }

    pub fn with_stop_reason(self, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        self.0
            .with_top_level("stopReason", reason.into().into())
            .map(Self)
    }

    pub fn with_suppress_output(self, suppress: bool) -> hookkit_core::Result<Self> {
        self.0
            .with_top_level("suppressOutput", suppress.into())
            .map(Self)
    }

    pub fn with_system_message(self, message: impl Into<String>) -> hookkit_core::Result<Self> {
        self.0
            .with_top_level("systemMessage", message.into().into())
            .map(Self)
    }
}
event_spec!(
    PreToolUse,
    PreToolUseOutput,
    "PreToolUse",
    Tool,
    ["tool_name"]
);

macro_rules! prompt_event {
    ($event:ident, $output:ident, $name:literal, $required:literal) => {
        #[derive(Debug, Clone)]
        pub struct $output(CatalogOutput);

        impl $output {
            pub fn with_context(additional_context: impl Into<String>) -> Self {
                Self(CatalogOutput::json(
                    $name,
                    context($name, additional_context),
                ))
            }

            pub fn block_with_context(
                reason: impl Into<String>,
                additional_context: impl Into<String>,
                session_title: Option<String>,
                suppress_original_prompt: Option<bool>,
            ) -> Self {
                let mut value = block_with_context($name, reason, additional_context);
                let fields = value["hookSpecificOutput"]
                    .as_object_mut()
                    .expect("specific output is an object");
                if let Some(title) = session_title {
                    fields.insert("sessionTitle".into(), title.into());
                }
                if let Some(suppress) = suppress_original_prompt {
                    fields.insert("suppressOriginalPrompt".into(), suppress.into());
                }
                Self(CatalogOutput::json($name, value))
            }

            pub fn text_context(context: impl Into<String>) -> Self {
                Self(CatalogOutput::text($name, context))
            }

            pub fn blocking_error(message: impl Into<String>) -> Self {
                Self(CatalogOutput::blocking($name, message))
            }
        }

        event_spec!($event, $output, $name, Prompt, [$required]);
    };
}

prompt_event!(
    UserPromptExpansion,
    UserPromptExpansionOutput,
    "UserPromptExpansion",
    "expansion_type"
);
prompt_event!(
    UserPromptSubmit,
    UserPromptSubmitOutput,
    "UserPromptSubmit",
    "prompt"
);

pub fn events() -> Vec<hookkit_core::NativeEventDescriptor> {
    vec![
        hookkit_core::NativeEventDescriptor::command::<ConfigChange>(&[
            "command-structured",
            "command-exit-2",
        ]),
        hookkit_core::NativeEventDescriptor::command::<CwdChanged>(&["command-structured"]),
        hookkit_core::NativeEventDescriptor::command::<Elicitation>(&[
            "command-structured",
            "command-exit-2",
        ]),
        hookkit_core::NativeEventDescriptor::command::<ElicitationResult>(&[
            "command-structured",
            "command-exit-2",
        ]),
        hookkit_core::NativeEventDescriptor::command::<FileChanged>(&["command-structured"]),
        hookkit_core::NativeEventDescriptor::command::<InstructionsLoaded>(&["command-structured"]),
        hookkit_core::NativeEventDescriptor::command::<MessageDisplay>(&["command-structured"]),
        hookkit_core::NativeEventDescriptor::command::<Notification>(&["command-structured"]),
        hookkit_core::NativeEventDescriptor::command::<PermissionDenied>(&["command-structured"]),
        hookkit_core::NativeEventDescriptor::command::<PermissionRequest>(&[
            "command-structured",
            "command-exit-2",
        ]),
        hookkit_core::NativeEventDescriptor::command::<PostCompact>(&["command-structured"]),
        hookkit_core::NativeEventDescriptor::command::<PostToolBatch>(&[
            "command-structured",
            "command-exit-2",
        ]),
        hookkit_core::NativeEventDescriptor::command::<PostToolUseFailure>(&["command-structured"]),
        hookkit_core::NativeEventDescriptor::command::<PreCompact>(&[
            "command-structured",
            "command-exit-2",
        ]),
        hookkit_core::NativeEventDescriptor::command::<PreToolUse>(&[
            "command-structured",
            "command-exit-2",
        ]),
        hookkit_core::NativeEventDescriptor::command::<SessionEnd>(&["command-structured"]),
        hookkit_core::NativeEventDescriptor::command::<Setup>(&["command-structured"]),
        hookkit_core::NativeEventDescriptor::command::<Stop>(&[
            "command-structured",
            "command-exit-2",
        ]),
        hookkit_core::NativeEventDescriptor::command::<StopFailure>(&["command-structured"]),
        hookkit_core::NativeEventDescriptor::command::<SubagentStart>(&["command-structured"]),
        hookkit_core::NativeEventDescriptor::command::<SubagentStop>(&[
            "command-structured",
            "command-exit-2",
        ]),
        hookkit_core::NativeEventDescriptor::command::<TaskCompleted>(&[
            "command-structured",
            "command-exit-2",
        ]),
        hookkit_core::NativeEventDescriptor::command::<TaskCreated>(&[
            "command-structured",
            "command-exit-2",
        ]),
        hookkit_core::NativeEventDescriptor::command::<TeammateIdle>(&[
            "command-structured",
            "command-exit-2",
        ]),
        hookkit_core::NativeEventDescriptor::command::<UserPromptExpansion>(&[
            "command-structured",
            "command-text",
            "command-exit-2",
        ]),
        hookkit_core::NativeEventDescriptor::command::<UserPromptSubmit>(&[
            "command-structured",
            "command-text",
            "command-exit-2",
        ]),
        hookkit_core::NativeEventDescriptor::command::<WorktreeRemove>(&["command-structured"]),
    ]
}

pub fn identification_descriptors() -> Vec<hookkit_core::IdentificationDescriptor> {
    vec![
        hookkit_core::IdentificationDescriptor::definitive::<ConfigChange>(
            "/hook_event_name",
            "ConfigChange",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<CwdChanged>(
            "/hook_event_name",
            "CwdChanged",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<Elicitation>(
            "/hook_event_name",
            "Elicitation",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<ElicitationResult>(
            "/hook_event_name",
            "ElicitationResult",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<FileChanged>(
            "/hook_event_name",
            "FileChanged",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<InstructionsLoaded>(
            "/hook_event_name",
            "InstructionsLoaded",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<MessageDisplay>(
            "/hook_event_name",
            "MessageDisplay",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<Notification>(
            "/hook_event_name",
            "Notification",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<PermissionDenied>(
            "/hook_event_name",
            "PermissionDenied",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<PermissionRequest>(
            "/hook_event_name",
            "PermissionRequest",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<PostCompact>(
            "/hook_event_name",
            "PostCompact",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<PostToolBatch>(
            "/hook_event_name",
            "PostToolBatch",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<PostToolUseFailure>(
            "/hook_event_name",
            "PostToolUseFailure",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<PreCompact>(
            "/hook_event_name",
            "PreCompact",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<PreToolUse>(
            "/hook_event_name",
            "PreToolUse",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<SessionEnd>(
            "/hook_event_name",
            "SessionEnd",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<Setup>("/hook_event_name", "Setup"),
        hookkit_core::IdentificationDescriptor::definitive::<Stop>("/hook_event_name", "Stop"),
        hookkit_core::IdentificationDescriptor::definitive::<StopFailure>(
            "/hook_event_name",
            "StopFailure",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<SubagentStart>(
            "/hook_event_name",
            "SubagentStart",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<SubagentStop>(
            "/hook_event_name",
            "SubagentStop",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<TaskCompleted>(
            "/hook_event_name",
            "TaskCompleted",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<TaskCreated>(
            "/hook_event_name",
            "TaskCreated",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<TeammateIdle>(
            "/hook_event_name",
            "TeammateIdle",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<UserPromptExpansion>(
            "/hook_event_name",
            "UserPromptExpansion",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<UserPromptSubmit>(
            "/hook_event_name",
            "UserPromptSubmit",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<WorktreeRemove>(
            "/hook_event_name",
            "WorktreeRemove",
        ),
    ]
}

pub fn decode(event: &EventId, raw: &RawInvocation) -> hookkit_core::Result<Option<CatalogInput>> {
    let input = match event.name() {
        "ConfigChange" => ConfigChange::parse(raw)?,
        "CwdChanged" => CwdChanged::parse(raw)?,
        "Elicitation" => Elicitation::parse(raw)?,
        "ElicitationResult" => ElicitationResult::parse(raw)?,
        "FileChanged" => FileChanged::parse(raw)?,
        "InstructionsLoaded" => InstructionsLoaded::parse(raw)?,
        "MessageDisplay" => MessageDisplay::parse(raw)?,
        "Notification" => Notification::parse(raw)?,
        "PermissionDenied" => PermissionDenied::parse(raw)?,
        "PermissionRequest" => PermissionRequest::parse(raw)?,
        "PostCompact" => PostCompact::parse(raw)?,
        "PostToolBatch" => PostToolBatch::parse(raw)?,
        "PostToolUseFailure" => PostToolUseFailure::parse(raw)?,
        "PreCompact" => PreCompact::parse(raw)?,
        "PreToolUse" => PreToolUse::parse(raw)?,
        "SessionEnd" => SessionEnd::parse(raw)?,
        "Setup" => Setup::parse(raw)?,
        "Stop" => Stop::parse(raw)?,
        "StopFailure" => StopFailure::parse(raw)?,
        "SubagentStart" => SubagentStart::parse(raw)?,
        "SubagentStop" => SubagentStop::parse(raw)?,
        "TaskCompleted" => TaskCompleted::parse(raw)?,
        "TaskCreated" => TaskCreated::parse(raw)?,
        "TeammateIdle" => TeammateIdle::parse(raw)?,
        "UserPromptExpansion" => UserPromptExpansion::parse(raw)?,
        "UserPromptSubmit" => UserPromptSubmit::parse(raw)?,
        "WorktreeRemove" => WorktreeRemove::parse(raw)?,
        _ => return Ok(None),
    };
    Ok(Some(input))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_input_retains_event_specific_and_future_fields() {
        let raw = RawInvocation::parse(
            br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"ConfigChange","source":"skills","file_path":"/repo/CLAUDE.md","future":true}"#.to_vec(),
        )
        .unwrap();
        let input = ConfigChange::parse(&raw).unwrap();
        assert_eq!(input.field("source"), Some(&serde_json::json!("skills")));
        assert_eq!(input.field("future"), Some(&serde_json::json!(true)));
    }

    #[test]
    fn event_specific_output_stamps_its_discriminator() {
        let emission = PreToolUse::emit(PreToolUseOutput::decide(
            PreToolPermissionDecision::Ask,
            Some("confirm".into()),
            None,
            Some("production".into()),
        ))
        .unwrap();
        let output: serde_json::Value = serde_json::from_slice(emission.stdout()).unwrap();
        assert_eq!(output["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    }

    #[test]
    fn structured_fields_cannot_be_added_after_blocking_transport() {
        assert!(
            ConfigChangeOutput::blocking_error("blocked")
                .with_system_message("notice")
                .is_err()
        );
    }
}
