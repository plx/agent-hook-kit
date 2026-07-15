//! Remaining Codex command-hook event implementations.

use hookkit_core::{
    ContractId, EventCategory, EventId, EventSpec, HarnessId, NativeContext, ProcessEmission,
    RawInvocation, SessionId, ToolCallId, TurnId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{CodexCommandEnvironment, protocol::SNAPSHOT_ID};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogInput {
    pub session_id: String,
    pub transcript_path: Option<hookkit_core::Utf8PathBuf>,
    pub cwd: hookkit_core::Utf8PathBuf,
    pub hook_event_name: String,
    pub model: String,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<crate::protocol::PermissionMode>,
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
            turn_id: self.turn_id.as_deref().and_then(|id| TurnId::new(id).ok()),
            transcript_path: self.transcript_path.clone(),
            tool_call_id: self
                .fields
                .get("tool_use_id")
                .and_then(serde_json::Value::as_str)
                .and_then(|id| ToolCallId::new(id).ok()),
            ..NativeContext::default()
        }
    }
}

fn catalog_event_id(event: &str) -> EventId {
    let event = match event {
        "PermissionRequest" => "PermissionRequest",
        "PostCompact" => "PostCompact",
        "PreCompact" => "PreCompact",
        "SessionStart" => "SessionStart",
        "Stop" => "Stop",
        "SubagentStart" => "SubagentStart",
        "SubagentStop" => "SubagentStop",
        "UserPromptSubmit" => "UserPromptSubmit",
        _ => unreachable!("catalog inputs are created only by exact event parsers"),
    };
    EventId::builtin(HarnessId::CODEX, event)
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

    pub(crate) fn event_id(&self) -> EventId {
        catalog_event_id(self.event)
    }

    pub(crate) fn emit(self) -> hookkit_core::Result<ProcessEmission> {
        let contract = ContractId::builtin(match self.event {
            "PermissionRequest" => "codex/commit-9e552e9-r2/PermissionRequest",
            "PostCompact" => "codex/commit-9e552e9-r2/PostCompact",
            "PreCompact" => "codex/commit-9e552e9-r2/PreCompact",
            "SessionStart" => "codex/commit-9e552e9-r2/SessionStart",
            "Stop" => "codex/commit-9e552e9-r2/Stop",
            "SubagentStart" => "codex/commit-9e552e9-r2/SubagentStart",
            "SubagentStop" => "codex/commit-9e552e9-r2/SubagentStop",
            "UserPromptSubmit" => "codex/commit-9e552e9-r2/UserPromptSubmit",
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
        super::protocol::require_field(invocation, field, event)?;
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

macro_rules! event_spec {
    ($event:ident, $output:ident, $name:literal, $category:ident, [$($required:literal),* $(,)?]) => {
        pub enum $event {}

        impl EventSpec for $event {
            type Input = CatalogInput;
            type CommandEnvironment = CodexCommandEnvironment;
            type CommandOutput = $output;
            const HARNESS: HarnessId = HarnessId::CODEX;
            const SNAPSHOT: hookkit_core::SnapshotId = SNAPSHOT_ID;
            const EVENT: EventId = EventId::builtin(HarnessId::CODEX, $name);
            const CATEGORY: EventCategory = EventCategory::$category;
            const CONTRACT: ContractId =
                ContractId::builtin(concat!("codex/commit-9e552e9-r2/", $name));

            fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
                parse(invocation, $name, &["transcript_path", $($required),*])
            }

            fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
                output.0.emit()
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

macro_rules! common_controls {
    ($output:ident, $name:literal) => {
        impl $output {
            pub fn no_op() -> Self {
                Self(CatalogOutput::json($name, serde_json::json!({})))
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

            pub fn with_system_message(
                self,
                message: impl Into<String>,
            ) -> hookkit_core::Result<Self> {
                self.0
                    .with_top_level("systemMessage", message.into().into())
                    .map(Self)
            }
        }
    };
}

#[derive(Debug, Clone)]
pub struct PermissionRequestOutput(CatalogOutput);

impl PermissionRequestOutput {
    pub fn deny(message: impl Into<String>) -> Self {
        Self(CatalogOutput::json(
            "PermissionRequest",
            specific(
                "PermissionRequest",
                serde_json::json!({
                    "decision": {"behavior": "deny", "message": message.into()}
                }),
            ),
        ))
    }

    pub fn allow() -> Self {
        Self(CatalogOutput::json(
            "PermissionRequest",
            specific(
                "PermissionRequest",
                serde_json::json!({"decision": {"behavior": "allow"}}),
            ),
        ))
    }

    pub fn blocking_error(message: impl Into<String>) -> Self {
        Self(CatalogOutput::blocking("PermissionRequest", message))
    }
}
common_controls!(PermissionRequestOutput, "PermissionRequest");
event_spec!(
    PermissionRequest,
    PermissionRequestOutput,
    "PermissionRequest",
    Tool,
    ["turn_id", "permission_mode", "tool_input", "tool_name"]
);

#[derive(Debug, Clone)]
pub struct PostCompactOutput(CatalogOutput);

common_controls!(PostCompactOutput, "PostCompact");
event_spec!(
    PostCompact,
    PostCompactOutput,
    "PostCompact",
    Context,
    ["turn_id", "trigger"]
);

#[derive(Debug, Clone)]
pub struct PreCompactOutput(CatalogOutput);

impl PreCompactOutput {
    pub fn stop(reason: impl Into<String>) -> Self {
        Self(CatalogOutput::json(
            "PreCompact",
            serde_json::json!({"continue": false, "stopReason": reason.into()}),
        ))
    }

    pub fn blocking_error(message: impl Into<String>) -> Self {
        Self(CatalogOutput::blocking("PreCompact", message))
    }
}
common_controls!(PreCompactOutput, "PreCompact");
event_spec!(
    PreCompact,
    PreCompactOutput,
    "PreCompact",
    Context,
    ["turn_id", "trigger"]
);

macro_rules! context_text_event {
    ($event:ident, $output:ident, $name:literal, $category:ident, [$($required:literal),* $(,)?]) => {
        #[derive(Debug, Clone)]
        pub struct $output(CatalogOutput);

        impl $output {
            pub fn with_context(additional_context: impl Into<String>) -> Self {
                Self(CatalogOutput::json(
                    $name,
                    context($name, additional_context),
                ))
            }

            pub fn text_context(context: impl Into<String>) -> Self {
                Self(CatalogOutput::text($name, context))
            }
        }

        common_controls!($output, $name);

        event_spec!($event, $output, $name, $category, [$($required),*]);
    };
}

context_text_event!(
    SessionStart,
    SessionStartOutput,
    "SessionStart",
    Session,
    ["permission_mode", "source"]
);
context_text_event!(
    SubagentStart,
    SubagentStartOutput,
    "SubagentStart",
    Agent,
    ["turn_id", "permission_mode", "agent_id", "agent_type"]
);

macro_rules! blocking_event {
    ($event:ident, $output:ident, $name:literal, $category:ident, [$($required:literal),* $(,)?]) => {
        #[derive(Debug, Clone)]
        pub struct $output(CatalogOutput);

        impl $output {
            pub fn block(reason: impl Into<String>) -> Self {
                Self(CatalogOutput::json($name, block(reason)))
            }

            pub fn blocking_error(message: impl Into<String>) -> Self {
                Self(CatalogOutput::blocking($name, message))
            }
        }

        common_controls!($output, $name);

        event_spec!($event, $output, $name, $category, [$($required),*]);
    };
}

blocking_event!(
    Stop,
    StopOutput,
    "Stop",
    Agent,
    [
        "turn_id",
        "permission_mode",
        "last_assistant_message",
        "stop_hook_active"
    ]
);
blocking_event!(
    SubagentStop,
    SubagentStopOutput,
    "SubagentStop",
    Agent,
    [
        "turn_id",
        "permission_mode",
        "agent_id",
        "agent_type",
        "agent_transcript_path",
        "last_assistant_message",
        "stop_hook_active"
    ]
);

#[derive(Debug, Clone)]
pub struct UserPromptSubmitOutput(CatalogOutput);

impl UserPromptSubmitOutput {
    pub fn block_with_context(
        reason: impl Into<String>,
        additional_context: impl Into<String>,
    ) -> Self {
        let mut value = block(reason);
        value.as_object_mut().expect("object").insert(
            "hookSpecificOutput".into(),
            serde_json::json!({
                "hookEventName": "UserPromptSubmit",
                "additionalContext": additional_context.into(),
            }),
        );
        Self(CatalogOutput::json("UserPromptSubmit", value))
    }

    pub fn text_context(context: impl Into<String>) -> Self {
        Self(CatalogOutput::text("UserPromptSubmit", context))
    }

    pub fn blocking_error(message: impl Into<String>) -> Self {
        Self(CatalogOutput::blocking("UserPromptSubmit", message))
    }
}
common_controls!(UserPromptSubmitOutput, "UserPromptSubmit");
event_spec!(
    UserPromptSubmit,
    UserPromptSubmitOutput,
    "UserPromptSubmit",
    Prompt,
    ["turn_id", "permission_mode", "prompt"]
);

pub fn events() -> Vec<hookkit_core::NativeEventDescriptor> {
    vec![
        hookkit_core::NativeEventDescriptor::command::<PermissionRequest>(&[
            "structured",
            "exit-2",
        ]),
        hookkit_core::NativeEventDescriptor::command::<PostCompact>(&["structured"]),
        hookkit_core::NativeEventDescriptor::command::<PreCompact>(&["structured", "exit-2"]),
        hookkit_core::NativeEventDescriptor::command::<SessionStart>(&[
            "structured",
            "text-context",
        ]),
        hookkit_core::NativeEventDescriptor::command::<Stop>(&["structured", "exit-2"]),
        hookkit_core::NativeEventDescriptor::command::<SubagentStart>(&[
            "structured",
            "text-context",
        ]),
        hookkit_core::NativeEventDescriptor::command::<SubagentStop>(&["structured", "exit-2"]),
        hookkit_core::NativeEventDescriptor::command::<UserPromptSubmit>(&[
            "structured",
            "text-context",
            "exit-2",
        ]),
    ]
}

pub fn identification_descriptors() -> Vec<hookkit_core::IdentificationDescriptor> {
    vec![
        hookkit_core::IdentificationDescriptor::definitive::<PermissionRequest>(
            "/hook_event_name",
            "PermissionRequest",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<PostCompact>(
            "/hook_event_name",
            "PostCompact",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<PreCompact>(
            "/hook_event_name",
            "PreCompact",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<SessionStart>(
            "/hook_event_name",
            "SessionStart",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<Stop>("/hook_event_name", "Stop"),
        hookkit_core::IdentificationDescriptor::definitive::<SubagentStart>(
            "/hook_event_name",
            "SubagentStart",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<SubagentStop>(
            "/hook_event_name",
            "SubagentStop",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<UserPromptSubmit>(
            "/hook_event_name",
            "UserPromptSubmit",
        ),
    ]
}

pub fn decode(event: &EventId, raw: &RawInvocation) -> hookkit_core::Result<Option<CatalogInput>> {
    let input = match event.name() {
        "PermissionRequest" => PermissionRequest::parse(raw)?,
        "PostCompact" => PostCompact::parse(raw)?,
        "PreCompact" => PreCompact::parse(raw)?,
        "SessionStart" => SessionStart::parse(raw)?,
        "Stop" => Stop::parse(raw)?,
        "SubagentStart" => SubagentStart::parse(raw)?,
        "SubagentStop" => SubagentStop::parse(raw)?,
        "UserPromptSubmit" => UserPromptSubmit::parse(raw)?,
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
            br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"SessionStart","model":"gpt-test","permission_mode":"default","source":"startup","future":true}"#.to_vec(),
        )
        .unwrap();
        let input = SessionStart::parse(&raw).unwrap();
        assert_eq!(input.field("source"), Some(&serde_json::json!("startup")));
        assert_eq!(input.field("future"), Some(&serde_json::json!(true)));
    }

    #[test]
    fn catalog_parser_requires_schema_required_optional_envelope_fields() {
        let raw = RawInvocation::parse(
            br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"SessionStart","model":"gpt-test","source":"startup"}"#.to_vec(),
        )
        .unwrap();

        assert!(matches!(
            SessionStart::parse(&raw),
            Err(hookkit_core::HookkitError::InvalidForHint { message, .. })
                if message == "missing required field permission_mode"
        ));
    }

    #[test]
    fn event_specific_output_stamps_its_discriminator() {
        let emission = PermissionRequest::emit(PermissionRequestOutput::deny("policy")).unwrap();
        let output: serde_json::Value = serde_json::from_slice(emission.stdout()).unwrap();
        assert_eq!(
            output["hookSpecificOutput"]["hookEventName"],
            "PermissionRequest"
        );
    }

    #[test]
    fn structured_fields_cannot_be_added_after_text_transport() {
        assert!(
            SessionStartOutput::text_context("context")
                .with_system_message("notice")
                .is_err()
        );
    }
}
