//! Remaining Gemini CLI command-hook event implementations.

use hookkit_core::{
    ContractId, EventCategory, EventId, EventSpec, HarnessId, NativeContext, ProcessEmission,
    RawInvocation, SessionBoundaryContext, SessionBoundaryKind, SessionId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{GeminiCommandEnvironment, protocol::SNAPSHOT_ID};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogInput {
    pub session_id: String,
    pub transcript_path: hookkit_core::Utf8PathBuf,
    pub cwd: hookkit_core::Utf8PathBuf,
    pub hook_event_name: String,
    pub timestamp: String,
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
        let session_boundary = (self.hook_event_name == "SessionStart").then(|| {
            let kind = match self.field("source").and_then(serde_json::Value::as_str) {
                Some("resume") => SessionBoundaryKind::Resume,
                Some("clear") => SessionBoundaryKind::Clear,
                Some("compact") => SessionBoundaryKind::Compact,
                _ => SessionBoundaryKind::Startup,
            };
            SessionBoundaryContext::observed(kind)
                .with_native_timestamp(&self.timestamp)
                .with_occurrence_key(format!("{}\0{}", self.timestamp, self.session_id))
        });
        NativeContext {
            workspace_roots: vec![self.cwd.clone()],
            session_id: SessionId::new(&self.session_id).ok(),
            transcript_path: Some(self.transcript_path.clone()),
            session_boundary,
            ..NativeContext::default()
        }
    }
}

fn catalog_event_id(event: &str) -> EventId {
    let event = match event {
        "AfterAgent" => "AfterAgent",
        "AfterModel" => "AfterModel",
        "BeforeAgent" => "BeforeAgent",
        "BeforeModel" => "BeforeModel",
        "Notification" => "Notification",
        "PreCompress" => "PreCompress",
        "SessionEnd" => "SessionEnd",
        "SessionStart" => "SessionStart",
        _ => unreachable!("catalog inputs are created only by exact event parsers"),
    };
    EventId::builtin(HarnessId::GEMINI_CLI, event)
}

#[derive(Debug, Clone)]
enum Outcome {
    Json(serde_json::Value),
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
                "structured fields cannot be added to blocking output",
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
            "AfterAgent" => "gemini-cli/commit-f354eeb-r2/AfterAgent",
            "AfterModel" => "gemini-cli/commit-f354eeb-r2/AfterModel",
            "BeforeAgent" => "gemini-cli/commit-f354eeb-r2/BeforeAgent",
            "BeforeModel" => "gemini-cli/commit-f354eeb-r2/BeforeModel",
            "Notification" => "gemini-cli/commit-f354eeb-r2/Notification",
            "PreCompress" => "gemini-cli/commit-f354eeb-r2/PreCompress",
            "SessionEnd" => "gemini-cli/commit-f354eeb-r2/SessionEnd",
            "SessionStart" => "gemini-cli/commit-f354eeb-r2/SessionStart",
            _ => unreachable!("catalog output constructors fix the event"),
        });
        match self.outcome {
            Outcome::Json(value) => ProcessEmission::command_json(contract, &value),
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
                event: EventId::builtin(HarnessId::GEMINI_CLI, event),
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

macro_rules! event_spec {
    ($event:ident, $output:ident, $name:literal, $category:ident, [$($required:literal),* $(,)?]) => {
        pub enum $event {}

        impl EventSpec for $event {
            type Input = CatalogInput;
            type CommandEnvironment = GeminiCommandEnvironment;
            type CommandOutput = $output;
            const HARNESS: HarnessId = HarnessId::GEMINI_CLI;
            const SNAPSHOT: hookkit_core::SnapshotId = SNAPSHOT_ID;
            const EVENT: EventId = EventId::builtin(HarnessId::GEMINI_CLI, $name);
            const CATEGORY: EventCategory = EventCategory::$category;
            const CONTRACT: ContractId =
                ContractId::builtin(concat!("gemini-cli/commit-f354eeb-r2/", $name));

            fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
                parse(invocation, $name, &[$($required),*])
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

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Allow,
    Deny,
    Block,
}

macro_rules! common_controls {
    ($output:ident, $name:literal) => {
        impl $output {
            pub fn no_op() -> Self {
                Self(CatalogOutput::json($name, serde_json::json!({})))
            }

            pub fn with_decision(
                self,
                decision: Decision,
                reason: Option<String>,
            ) -> hookkit_core::Result<Self> {
                let output = self.0.with_top_level(
                    "decision",
                    serde_json::to_value(decision).expect("enum serialization cannot fail"),
                )?;
                match reason {
                    Some(reason) => output.with_top_level("reason", reason.into()).map(Self),
                    None => Ok(Self(output)),
                }
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

system_event!(
    Notification,
    NotificationOutput,
    "Notification",
    Other,
    ["notification_type", "message", "details"]
);
system_event!(
    PreCompress,
    PreCompressOutput,
    "PreCompress",
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

#[derive(Debug, Clone)]
pub struct SessionStartOutput(CatalogOutput);

impl SessionStartOutput {
    pub fn no_op() -> Self {
        Self(CatalogOutput::json("SessionStart", serde_json::json!({})))
    }

    pub fn with_system_message(message: impl Into<String>) -> Self {
        Self(CatalogOutput::json(
            "SessionStart",
            serde_json::json!({"systemMessage": message.into()}),
        ))
    }

    pub fn with_context_and_system_message(
        additional_context: impl Into<String>,
        system_message: impl Into<String>,
    ) -> Self {
        let mut value = specific(
            "SessionStart",
            serde_json::json!({"additionalContext": additional_context.into()}),
        );
        value
            .as_object_mut()
            .expect("object")
            .insert("systemMessage".into(), system_message.into().into());
        Self(CatalogOutput::json("SessionStart", value))
    }

    pub fn with_context(additional_context: impl Into<String>) -> Self {
        Self(CatalogOutput::json(
            "SessionStart",
            specific(
                "SessionStart",
                serde_json::json!({"additionalContext": additional_context.into()}),
            ),
        ))
    }
}
event_spec!(
    SessionStart,
    SessionStartOutput,
    "SessionStart",
    Session,
    ["source"]
);

#[derive(Debug, Clone)]
pub struct BeforeAgentOutput(CatalogOutput);

impl BeforeAgentOutput {
    pub fn with_context(additional_context: impl Into<String>) -> Self {
        Self(CatalogOutput::json(
            "BeforeAgent",
            specific(
                "BeforeAgent",
                serde_json::json!({"additionalContext": additional_context.into()}),
            ),
        ))
    }

    pub fn blocking_error(message: impl Into<String>) -> Self {
        Self(CatalogOutput::blocking("BeforeAgent", message))
    }
}
common_controls!(BeforeAgentOutput, "BeforeAgent");
event_spec!(
    BeforeAgent,
    BeforeAgentOutput,
    "BeforeAgent",
    Agent,
    ["prompt"]
);

#[derive(Debug, Clone)]
pub struct BeforeModelOutput(CatalogOutput);

impl BeforeModelOutput {
    pub fn replace_request(request: serde_json::Value) -> Self {
        Self(CatalogOutput::json(
            "BeforeModel",
            specific("BeforeModel", serde_json::json!({"llm_request": request})),
        ))
    }

    pub fn replace_response(response: serde_json::Value) -> Self {
        Self(CatalogOutput::json(
            "BeforeModel",
            specific("BeforeModel", serde_json::json!({"llm_response": response})),
        ))
    }

    pub fn blocking_error(message: impl Into<String>) -> Self {
        Self(CatalogOutput::blocking("BeforeModel", message))
    }
}
common_controls!(BeforeModelOutput, "BeforeModel");
event_spec!(
    BeforeModel,
    BeforeModelOutput,
    "BeforeModel",
    Model,
    ["llm_request"]
);

#[derive(Debug, Clone)]
pub struct AfterModelOutput(CatalogOutput);

impl AfterModelOutput {
    pub fn replace_response(response: serde_json::Value) -> Self {
        Self(CatalogOutput::json(
            "AfterModel",
            specific("AfterModel", serde_json::json!({"llm_response": response})),
        ))
    }

    pub fn blocking_error(message: impl Into<String>) -> Self {
        Self(CatalogOutput::blocking("AfterModel", message))
    }
}
common_controls!(AfterModelOutput, "AfterModel");
event_spec!(
    AfterModel,
    AfterModelOutput,
    "AfterModel",
    Model,
    ["llm_request", "llm_response"]
);

#[derive(Debug, Clone)]
pub struct AfterAgentOutput(CatalogOutput);

impl AfterAgentOutput {
    pub fn deny(reason: impl Into<String>, clear_context: bool) -> Self {
        let mut value = specific(
            "AfterAgent",
            serde_json::json!({"clearContext": clear_context}),
        );
        let object = value.as_object_mut().expect("object");
        object.insert("decision".into(), "deny".into());
        object.insert("reason".into(), reason.into().into());
        Self(CatalogOutput::json("AfterAgent", value))
    }

    pub fn blocking_error(message: impl Into<String>) -> Self {
        Self(CatalogOutput::blocking("AfterAgent", message))
    }
}
common_controls!(AfterAgentOutput, "AfterAgent");
event_spec!(
    AfterAgent,
    AfterAgentOutput,
    "AfterAgent",
    Agent,
    ["prompt", "prompt_response", "stop_hook_active"]
);

pub fn events() -> Vec<hookkit_core::NativeEventDescriptor> {
    vec![
        hookkit_core::NativeEventDescriptor::command::<AfterAgent>(&["structured", "exit-2"]),
        hookkit_core::NativeEventDescriptor::command::<AfterModel>(&["structured", "exit-2"]),
        hookkit_core::NativeEventDescriptor::command::<BeforeAgent>(&["structured", "exit-2"]),
        hookkit_core::NativeEventDescriptor::command::<BeforeModel>(&["structured", "exit-2"]),
        hookkit_core::NativeEventDescriptor::command::<Notification>(&["structured"]),
        hookkit_core::NativeEventDescriptor::command::<PreCompress>(&["structured"]),
        hookkit_core::NativeEventDescriptor::command::<SessionEnd>(&["structured"]),
        hookkit_core::NativeEventDescriptor::command::<SessionStart>(&["structured"]),
    ]
}

pub fn identification_descriptors() -> Vec<hookkit_core::IdentificationDescriptor> {
    vec![
        hookkit_core::IdentificationDescriptor::definitive::<AfterAgent>(
            "/hook_event_name",
            "AfterAgent",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<AfterModel>(
            "/hook_event_name",
            "AfterModel",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<BeforeAgent>(
            "/hook_event_name",
            "BeforeAgent",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<BeforeModel>(
            "/hook_event_name",
            "BeforeModel",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<Notification>(
            "/hook_event_name",
            "Notification",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<PreCompress>(
            "/hook_event_name",
            "PreCompress",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<SessionEnd>(
            "/hook_event_name",
            "SessionEnd",
        ),
        hookkit_core::IdentificationDescriptor::definitive::<SessionStart>(
            "/hook_event_name",
            "SessionStart",
        ),
    ]
}

pub fn decode(event: &EventId, raw: &RawInvocation) -> hookkit_core::Result<Option<CatalogInput>> {
    let input = match event.name() {
        "AfterAgent" => AfterAgent::parse(raw)?,
        "AfterModel" => AfterModel::parse(raw)?,
        "BeforeAgent" => BeforeAgent::parse(raw)?,
        "BeforeModel" => BeforeModel::parse(raw)?,
        "Notification" => Notification::parse(raw)?,
        "PreCompress" => PreCompress::parse(raw)?,
        "SessionEnd" => SessionEnd::parse(raw)?,
        "SessionStart" => SessionStart::parse(raw)?,
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
            br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"BeforeAgent","timestamp":"2026-07-12T00:00:00Z","prompt":"review","future":true}"#.to_vec(),
        )
        .unwrap();
        let input = BeforeAgent::parse(&raw).unwrap();
        assert_eq!(input.field("prompt"), Some(&serde_json::json!("review")));
        assert_eq!(input.field("future"), Some(&serde_json::json!(true)));
    }

    #[test]
    fn session_start_context_retains_native_timestamp() {
        let raw = RawInvocation::parse(
            br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"SessionStart","timestamp":"2026-07-12T00:00:00Z","source":"startup"}"#.to_vec(),
        )
        .unwrap();
        let context = SessionStart::context(&SessionStart::parse(&raw).unwrap());
        let boundary = context.session_boundary.unwrap();
        assert_eq!(boundary.kind, SessionBoundaryKind::Startup);
        assert_eq!(
            boundary.native_timestamp.as_deref(),
            Some("2026-07-12T00:00:00Z")
        );
        assert!(boundary.occurrence_key.is_some());
    }

    #[test]
    fn catalog_parser_requires_llm_response() {
        let raw = RawInvocation::parse(
            br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"AfterModel","timestamp":"2026-07-12T00:00:00Z","llm_request":{}}"#.to_vec(),
        )
        .unwrap();

        assert!(matches!(
            AfterModel::parse(&raw),
            Err(hookkit_core::HookkitError::InvalidInputForHint { message, .. })
                if message == "missing required field llm_response"
        ));
    }

    #[test]
    fn event_specific_output_stamps_its_discriminator() {
        let emission = BeforeAgent::emit(BeforeAgentOutput::with_context("conventions")).unwrap();
        let output: serde_json::Value = serde_json::from_slice(emission.stdout()).unwrap();
        assert_eq!(output["hookSpecificOutput"]["hookEventName"], "BeforeAgent");
    }

    #[test]
    fn structured_fields_cannot_be_added_after_blocking_transport() {
        assert!(
            BeforeAgentOutput::blocking_error("blocked")
                .with_system_message("notice")
                .is_err()
        );
    }
}
