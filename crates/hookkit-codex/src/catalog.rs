//! Remaining Codex command-hook event implementations.

use hookkit_core::{
    ContractId, EventCategory, EventId, EventSpec, HarnessId, NativeContext, ProcessEmission,
    RawInvocation, SessionBoundaryContext, SessionBoundaryKind, SessionId, ToolCallId, TurnId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{CodexCommandEnvironment, protocol::SNAPSHOT_ID};

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Shared native input envelope for implemented Codex catalog events.
///
/// Event-specific and unknown fields are retained losslessly in the flattened
/// field map and can be accessed through [`Self::field`] or [`Self::fields`].
pub struct CatalogInput {
    /// Native session identifier.
    pub session_id: String,
    /// Transcript path; required by implemented parsers but allowed to be null.
    pub transcript_path: Option<hookkit_core::Utf8PathBuf>,
    /// Current workspace directory.
    pub cwd: hookkit_core::Utf8PathBuf,
    /// Authoritative native event discriminator.
    pub hook_event_name: String,
    /// Model configured for the event's turn; absent from `SessionEnd`.
    #[serde(default)]
    pub model: Option<String>,
    /// Native turn identifier when the event carries one.
    #[serde(default)]
    pub turn_id: Option<String>,
    /// Active permission policy when the event carries one.
    #[serde(default)]
    pub permission_mode: Option<crate::protocol::PermissionMode>,
    #[serde(flatten)]
    fields: BTreeMap<String, serde_json::Value>,
}

impl CatalogInput {
    /// Returns one event-specific or unknown top-level field.
    pub fn field(&self, name: &str) -> Option<&serde_json::Value> {
        self.fields.get(name)
    }

    /// Returns all event-specific and unknown top-level fields.
    pub fn fields(&self) -> &BTreeMap<String, serde_json::Value> {
        &self.fields
    }

    pub(crate) fn event_id(&self) -> EventId {
        catalog_event_id(&self.hook_event_name)
    }

    pub(crate) fn context(&self) -> NativeContext {
        let session_boundary = (self.hook_event_name == "SessionStart").then(|| {
            let kind = match self.field("source").and_then(serde_json::Value::as_str) {
                Some("resume") => SessionBoundaryKind::Resume,
                Some("clear") => SessionBoundaryKind::Clear,
                Some("compact") => SessionBoundaryKind::Compact,
                _ => SessionBoundaryKind::Startup,
            };
            SessionBoundaryContext::observed(kind)
        });
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
            session_boundary,
            ..NativeContext::default()
        }
    }
}

fn catalog_event_id(event: &str) -> EventId {
    let event = match event {
        "PermissionRequest" => "PermissionRequest",
        "PostCompact" => "PostCompact",
        "PreCompact" => "PreCompact",
        "SessionEnd" => "SessionEnd",
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
    Empty,
    Json(serde_json::Value),
    Text(String),
    BlockingError(String),
}

#[derive(Debug, Clone)]
/// Type-erased output used by Codex catalog event wrappers.
///
/// Public event-specific output types are the intended constructors. This type
/// exists so dynamic harness dispatch can retain the exact event arm.
pub struct CatalogOutput {
    event: &'static str,
    outcome: Outcome,
}

impl CatalogOutput {
    fn empty(event: &'static str) -> Self {
        Self {
            event,
            outcome: Outcome::Empty,
        }
    }

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
            "PermissionRequest" => "codex/commit-1e59dc5-r1/PermissionRequest",
            "PostCompact" => "codex/commit-1e59dc5-r1/PostCompact",
            "PreCompact" => "codex/commit-1e59dc5-r1/PreCompact",
            "SessionEnd" => "codex/commit-1e59dc5-r1/SessionEnd",
            "SessionStart" => "codex/commit-1e59dc5-r1/SessionStart",
            "Stop" => "codex/commit-1e59dc5-r1/Stop",
            "SubagentStart" => "codex/commit-1e59dc5-r1/SubagentStart",
            "SubagentStop" => "codex/commit-1e59dc5-r1/SubagentStop",
            "UserPromptSubmit" => "codex/commit-1e59dc5-r1/UserPromptSubmit",
            _ => unreachable!("catalog output constructors fix the event"),
        });
        match self.outcome {
            Outcome::Empty => Ok(ProcessEmission::command_empty(contract)),
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
    if event != "SessionEnd" {
        super::protocol::require_field(invocation, "model", event)?;
    }
    for field in required_fields {
        super::protocol::require_field(invocation, field, event)?;
    }
    if event == "SessionEnd"
        && invocation
            .json()
            .get("reason")
            .and_then(serde_json::Value::as_str)
            != Some("other")
    {
        return Err(hookkit_core::HookkitError::InvalidForHint {
            harness: HarnessId::CODEX,
            event: EventId::builtin(HarnessId::CODEX, event),
            message: "expected reason=other".to_string(),
        });
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
        #[doc = concat!("Native Codex `", $name, "` command contract.")]
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
                ContractId::builtin(concat!("codex/commit-1e59dc5-r1/", $name));

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

macro_rules! no_op_control {
    ($output:ident, $name:literal) => {
        impl $output {
            /// Creates an empty structured response.
            pub fn no_op() -> Self {
                Self(CatalogOutput::json($name, serde_json::json!({})))
            }
        }
    };
}

macro_rules! continuation_controls {
    ($output:ident) => {
        impl $output {
            /// Sets the top-level `continue` control.
            pub fn with_continue(self, continue_session: bool) -> hookkit_core::Result<Self> {
                self.0
                    .with_top_level("continue", continue_session.into())
                    .map(Self)
            }

            /// Sets the top-level stop reason on a structured response.
            pub fn with_stop_reason(self, reason: impl Into<String>) -> hookkit_core::Result<Self> {
                self.0
                    .with_top_level("stopReason", reason.into().into())
                    .map(Self)
            }
        }
    };
}

macro_rules! system_message_control {
    ($output:ident) => {
        impl $output {
            /// Sets a top-level system message on a structured response.
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

macro_rules! common_controls {
    ($output:ident, $name:literal) => {
        no_op_control!($output, $name);
        continuation_controls!($output);
        system_message_control!($output);
    };
}

#[derive(Debug, Clone)]
/// Native response from a Codex permission-request command hook.
pub struct PermissionRequestOutput(CatalogOutput);

impl PermissionRequestOutput {
    /// Creates a structured permission denial with a user-facing message.
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

    /// Creates a structured permission approval.
    pub fn allow() -> Self {
        Self(CatalogOutput::json(
            "PermissionRequest",
            specific(
                "PermissionRequest",
                serde_json::json!({"decision": {"behavior": "allow"}}),
            ),
        ))
    }

    /// Creates a code-2 blocking response with required stderr text.
    pub fn blocking_error(message: impl Into<String>) -> Self {
        Self(CatalogOutput::blocking("PermissionRequest", message))
    }
}
no_op_control!(PermissionRequestOutput, "PermissionRequest");
system_message_control!(PermissionRequestOutput);
event_spec!(
    PermissionRequest,
    PermissionRequestOutput,
    "PermissionRequest",
    Tool,
    ["turn_id", "permission_mode", "tool_input", "tool_name"]
);

#[derive(Debug, Clone)]
/// Native response from a Codex post-compaction command hook.
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
/// Native response from a Codex pre-compaction command hook.
pub struct PreCompactOutput(CatalogOutput);

impl PreCompactOutput {
    /// Stops compaction with a structured reason.
    pub fn stop(reason: impl Into<String>) -> Self {
        Self(CatalogOutput::json(
            "PreCompact",
            serde_json::json!({"continue": false, "stopReason": reason.into()}),
        ))
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
        #[doc = concat!("Native response from a Codex `", $name, "` command hook.")]
        pub struct $output(CatalogOutput);

        impl $output {
            /// Creates a structured response that appends agent context.
            pub fn with_context(additional_context: impl Into<String>) -> Self {
                Self(CatalogOutput::json(
                    $name,
                    context($name, additional_context),
                ))
            }

            /// Creates a successful plain-text context response.
            pub fn text_context(context: impl Into<String>) -> Self {
                Self(CatalogOutput::text($name, context))
            }
        }

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
common_controls!(SessionStartOutput, "SessionStart");
context_text_event!(
    SubagentStart,
    SubagentStartOutput,
    "SubagentStart",
    Agent,
    ["turn_id", "permission_mode", "agent_id", "agent_type"]
);
no_op_control!(SubagentStartOutput, "SubagentStart");
system_message_control!(SubagentStartOutput);

#[derive(Debug, Clone)]
/// Native advisory response from a Codex `SessionEnd` command hook.
pub struct SessionEndOutput(CatalogOutput);

impl SessionEndOutput {
    /// Exits successfully without emitting stdout; `SessionEnd` output is ignored.
    pub fn no_op() -> Self {
        Self(CatalogOutput::empty("SessionEnd"))
    }
}

event_spec!(
    SessionEnd,
    SessionEndOutput,
    "SessionEnd",
    Session,
    ["reason"]
);

macro_rules! blocking_event {
    ($event:ident, $output:ident, $name:literal, $category:ident, [$($required:literal),* $(,)?]) => {
        #[derive(Debug, Clone)]
        #[doc = concat!("Native response from a Codex `", $name, "` command hook.")]
        pub struct $output(CatalogOutput);

        impl $output {
            /// Creates a structured legacy block response.
            pub fn block(reason: impl Into<String>) -> Self {
                Self(CatalogOutput::json($name, block(reason)))
            }

            /// Creates a code-2 blocking response with required stderr text.
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
/// Native response from a Codex user-prompt-submit command hook.
pub struct UserPromptSubmitOutput(CatalogOutput);

impl UserPromptSubmitOutput {
    /// Blocks the prompt while also appending context for the agent.
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

    /// Creates a successful plain-text context response.
    pub fn text_context(context: impl Into<String>) -> Self {
        Self(CatalogOutput::text("UserPromptSubmit", context))
    }

    /// Creates a code-2 blocking response with required stderr text.
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

/// Returns every native command implementation defined in this catalog module.
pub fn events() -> Vec<hookkit_core::NativeEventDescriptor> {
    vec![
        hookkit_core::NativeEventDescriptor::command::<PermissionRequest>(&[
            "structured",
            "exit-2",
        ]),
        hookkit_core::NativeEventDescriptor::command::<PostCompact>(&["structured"]),
        hookkit_core::NativeEventDescriptor::command::<PreCompact>(&["structured"]),
        hookkit_core::NativeEventDescriptor::command::<SessionEnd>(&["no-op"]),
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

/// Returns definitive discriminator metadata for catalog-module events.
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
        hookkit_core::IdentificationDescriptor::definitive::<SessionEnd>(
            "/hook_event_name",
            "SessionEnd",
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

/// Decodes a catalog-module event.
///
/// Returns `Ok(None)` when `event` is not implemented by this module. A known
/// event with malformed native input returns an error.
pub fn decode(event: &EventId, raw: &RawInvocation) -> hookkit_core::Result<Option<CatalogInput>> {
    let input = match event.name() {
        "PermissionRequest" => PermissionRequest::parse(raw)?,
        "PostCompact" => PostCompact::parse(raw)?,
        "PreCompact" => PreCompact::parse(raw)?,
        "SessionEnd" => SessionEnd::parse(raw)?,
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
        assert_eq!(
            SessionStart::context(&input).session_boundary.unwrap().kind,
            SessionBoundaryKind::Startup
        );
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
    fn catalog_parser_keeps_model_required_outside_session_end() {
        let raw = RawInvocation::parse(
            br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"SessionStart","permission_mode":"default","source":"startup"}"#.to_vec(),
        )
        .unwrap();

        assert!(matches!(
            SessionStart::parse(&raw),
            Err(hookkit_core::HookkitError::InvalidForHint { message, .. })
                if message == "missing required field model"
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

    #[test]
    fn session_end_parses_its_advisory_envelope_without_turn_fields() {
        let raw = RawInvocation::parse(
            br#"{"session_id":"s","transcript_path":null,"cwd":"/repo","hook_event_name":"SessionEnd","reason":"other"}"#.to_vec(),
        )
        .unwrap();
        let input = SessionEnd::parse(&raw).unwrap();
        assert_eq!(input.model, None);
        assert_eq!(input.field("reason"), Some(&serde_json::json!("other")));

        let emission = SessionEnd::emit(SessionEndOutput::no_op()).unwrap();
        assert!(emission.stdout().is_empty());
        assert!(emission.stderr().is_empty());
        assert_eq!(emission.exit_code(), 0);

        let invalid = RawInvocation::parse(
            br#"{"session_id":"s","transcript_path":null,"cwd":"/repo","hook_event_name":"SessionEnd","reason":"exit"}"#.to_vec(),
        )
        .unwrap();
        assert!(SessionEnd::parse(&invalid).is_err());
    }
}
