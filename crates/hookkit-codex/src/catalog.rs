//! Remaining Codex command-hook event implementations.
//!
//! Every event here shares [`CatalogInput`] and a typed output wrapper. Two
//! Codex rules shape the outputs:
//!
//! - Empty stdout at exit 0 is each event's `no-op` outcome, so every
//!   `no_op()` emits no bytes; builders turn it into structured JSON.
//! - Codex ignores every control effect (block decisions, `continue: false`,
//!   exit 2, permission decisions) from a handler configured with
//!   `async: true`. Such handlers only deliver additional context and system
//!   messages, and the stdin payload does not say which mode is in use.

use hookkit_core::{
    ContractId, EventCategory, EventId, EventSpec, HarnessId, NativeContext, ProcessEmission,
    RawInvocation, SessionBoundaryContext, SessionBoundaryKind, SessionId, ToolCallId, TurnId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::CodexCommandEnvironment;
use crate::protocol::{PermissionMode, SNAPSHOT_ID};
use crate::wire::{self, contract_id, open_string_enum};

open_string_enum! {
    /// Why Codex started a session, from `SessionStart.source`.
    pub enum SessionStartSource {
        /// A new thread started.
        Startup => "startup",
        /// A persisted thread resumed, including `thread/resume` with supplied
        /// history (Codex >= 0.155.0).
        Resume => "resume",
        /// The conversation was cleared.
        Clear => "clear",
        /// The conversation was compacted.
        Compact => "compact",
        /// The thread was forked from a parent thread (Codex >= 0.155.0).
        Fork => "fork",
    }
}

open_string_enum! {
    /// What started a compaction, from `PreCompact`/`PostCompact` `trigger`.
    pub enum CompactTrigger {
        /// The user requested compaction.
        Manual => "manual",
        /// Codex compacted automatically.
        Auto => "auto",
    }
}

open_string_enum! {
    /// Why a Codex session ended, from `SessionEnd.reason`.
    pub enum SessionEndReason {
        /// The only reason the implemented snapshot reports.
        Other => "other",
    }
}

/// Event identity validated when a [`CatalogInput`] is created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum CatalogEvent {
    Interrupt,
    PermissionRequest,
    PostCompact,
    PreCompact,
    SessionEnd,
    SessionStart,
    Stop,
    SubagentStart,
    SubagentStop,
    UserPromptSubmit,
}

#[derive(Debug, Clone, Copy)]
enum FieldKind {
    String,
    NullableString,
    Bool,
    Any,
}

#[derive(Debug, Clone, Copy)]
struct FieldRule {
    name: &'static str,
    required: bool,
    kind: FieldKind,
}

const fn required(name: &'static str, kind: FieldKind) -> FieldRule {
    FieldRule {
        name,
        required: true,
        kind,
    }
}

const fn optional(name: &'static str, kind: FieldKind) -> FieldRule {
    FieldRule {
        name,
        required: false,
        kind,
    }
}

const TRANSCRIPT_PATH: FieldRule = required("transcript_path", FieldKind::NullableString);
const MODEL: FieldRule = required("model", FieldKind::String);
const TURN_ID: FieldRule = required("turn_id", FieldKind::String);
const PERMISSION_MODE: FieldRule = required("permission_mode", FieldKind::String);
const AGENT_ID: FieldRule = required("agent_id", FieldKind::String);
const AGENT_TYPE: FieldRule = required("agent_type", FieldKind::String);
const OPTIONAL_AGENT_ID: FieldRule = optional("agent_id", FieldKind::String);
const OPTIONAL_AGENT_TYPE: FieldRule = optional("agent_type", FieldKind::String);
const LAST_ASSISTANT_MESSAGE: FieldRule =
    required("last_assistant_message", FieldKind::NullableString);
const STOP_HOOK_ACTIVE: FieldRule = required("stop_hook_active", FieldKind::Bool);
const TRIGGER: FieldRule = required("trigger", FieldKind::String);
const TOOL_NAME: FieldRule = required("tool_name", FieldKind::String);
const TOOL_INPUT: FieldRule = required("tool_input", FieldKind::Any);
const END_REASON: FieldRule = required("reason", FieldKind::String);
const SOURCE: FieldRule = required("source", FieldKind::String);
const AGENT_TRANSCRIPT_PATH: FieldRule =
    required("agent_transcript_path", FieldKind::NullableString);
const PROMPT: FieldRule = required("prompt", FieldKind::String);

impl CatalogEvent {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "Interrupt" => Self::Interrupt,
            "PermissionRequest" => Self::PermissionRequest,
            "PostCompact" => Self::PostCompact,
            "PreCompact" => Self::PreCompact,
            "SessionEnd" => Self::SessionEnd,
            "SessionStart" => Self::SessionStart,
            "Stop" => Self::Stop,
            "SubagentStart" => Self::SubagentStart,
            "SubagentStop" => Self::SubagentStop,
            "UserPromptSubmit" => Self::UserPromptSubmit,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::Interrupt => "Interrupt",
            Self::PermissionRequest => "PermissionRequest",
            Self::PostCompact => "PostCompact",
            Self::PreCompact => "PreCompact",
            Self::SessionEnd => "SessionEnd",
            Self::SessionStart => "SessionStart",
            Self::Stop => "Stop",
            Self::SubagentStart => "SubagentStart",
            Self::SubagentStop => "SubagentStop",
            Self::UserPromptSubmit => "UserPromptSubmit",
        }
    }

    fn event_id(self) -> EventId {
        EventId::builtin(HarnessId::CODEX, self.name())
    }

    fn contract(self) -> ContractId {
        match self {
            Self::Interrupt => contract_id!("Interrupt"),
            Self::PermissionRequest => contract_id!("PermissionRequest"),
            Self::PostCompact => contract_id!("PostCompact"),
            Self::PreCompact => contract_id!("PreCompact"),
            Self::SessionEnd => contract_id!("SessionEnd"),
            Self::SessionStart => contract_id!("SessionStart"),
            Self::Stop => contract_id!("Stop"),
            Self::SubagentStart => contract_id!("SubagentStart"),
            Self::SubagentStop => contract_id!("SubagentStop"),
            Self::UserPromptSubmit => contract_id!("UserPromptSubmit"),
        }
    }

    /// Fields the native parser checks beyond the shared envelope types:
    /// presence of every schema-required key and the JSON type of each
    /// event-specific field. Enum-valued fields are only required to be
    /// strings so undocumented values stay parseable.
    fn fields(self) -> &'static [FieldRule] {
        match self {
            Self::Interrupt => &[TRANSCRIPT_PATH, MODEL, TURN_ID, PERMISSION_MODE],
            Self::PermissionRequest => &[
                TRANSCRIPT_PATH,
                MODEL,
                TURN_ID,
                PERMISSION_MODE,
                TOOL_NAME,
                TOOL_INPUT,
                OPTIONAL_AGENT_ID,
                OPTIONAL_AGENT_TYPE,
            ],
            Self::PostCompact | Self::PreCompact => &[
                TRANSCRIPT_PATH,
                MODEL,
                TURN_ID,
                TRIGGER,
                OPTIONAL_AGENT_ID,
                OPTIONAL_AGENT_TYPE,
            ],
            Self::SessionEnd => &[TRANSCRIPT_PATH, END_REASON],
            Self::SessionStart => &[TRANSCRIPT_PATH, MODEL, PERMISSION_MODE, SOURCE],
            Self::Stop => &[
                TRANSCRIPT_PATH,
                MODEL,
                TURN_ID,
                PERMISSION_MODE,
                LAST_ASSISTANT_MESSAGE,
                STOP_HOOK_ACTIVE,
            ],
            Self::SubagentStart => &[
                TRANSCRIPT_PATH,
                MODEL,
                TURN_ID,
                PERMISSION_MODE,
                AGENT_ID,
                AGENT_TYPE,
            ],
            Self::SubagentStop => &[
                TRANSCRIPT_PATH,
                MODEL,
                TURN_ID,
                PERMISSION_MODE,
                AGENT_ID,
                AGENT_TYPE,
                AGENT_TRANSCRIPT_PATH,
                LAST_ASSISTANT_MESSAGE,
                STOP_HOOK_ACTIVE,
            ],
            Self::UserPromptSubmit => &[
                TRANSCRIPT_PATH,
                MODEL,
                TURN_ID,
                PERMISSION_MODE,
                PROMPT,
                OPTIONAL_AGENT_ID,
                OPTIONAL_AGENT_TYPE,
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "CatalogInputWire")]
/// Shared native input envelope for implemented Codex catalog events.
///
/// Event-specific and unknown fields are retained losslessly in the flattened
/// field map. Typed accessors such as [`Self::source`] or
/// [`Self::stop_hook_active`] read the documented ones; [`Self::field`] and
/// [`Self::fields`] expose everything. Serializing reproduces the native keys
/// without adding `null` members Codex did not send.
///
/// Deserializing accepts only catalog event names, and routing through
/// [`Self::event_id`] uses the event validated at that point, not the public
/// `hook_event_name` string.
pub struct CatalogInput {
    /// Native session identifier.
    pub session_id: String,
    /// Transcript path; required by implemented parsers but allowed to be null.
    pub transcript_path: Option<hookkit_core::Utf8PathBuf>,
    /// Working directory reported by the event: the turn directory, except
    /// for `PermissionRequest`, which reports the step-local directory of the
    /// tool call (see [`crate::protocol::PreToolUseInput`]).
    pub cwd: hookkit_core::Utf8PathBuf,
    /// Authoritative native event discriminator.
    pub hook_event_name: String,
    /// Model configured for the event's turn; absent from `SessionEnd`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Native turn identifier when the event carries one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// Active permission policy when the event carries one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
    #[serde(skip)]
    event: CatalogEvent,
    #[serde(flatten)]
    fields: BTreeMap<String, serde_json::Value>,
}

#[derive(Deserialize)]
struct CatalogInputWire {
    session_id: String,
    transcript_path: Option<hookkit_core::Utf8PathBuf>,
    cwd: hookkit_core::Utf8PathBuf,
    hook_event_name: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    turn_id: Option<String>,
    #[serde(default)]
    permission_mode: Option<PermissionMode>,
    #[serde(flatten)]
    fields: BTreeMap<String, serde_json::Value>,
}

impl TryFrom<CatalogInputWire> for CatalogInput {
    type Error = String;

    fn try_from(wire: CatalogInputWire) -> Result<Self, Self::Error> {
        let event = CatalogEvent::from_name(&wire.hook_event_name).ok_or_else(|| {
            format!(
                "hook_event_name `{}` is not a Codex catalog event",
                wire.hook_event_name
            )
        })?;
        Ok(Self {
            session_id: wire.session_id,
            transcript_path: wire.transcript_path,
            cwd: wire.cwd,
            hook_event_name: wire.hook_event_name,
            model: wire.model,
            turn_id: wire.turn_id,
            permission_mode: wire.permission_mode,
            event,
            fields: wire.fields,
        })
    }
}

impl CatalogInput {
    /// Returns the exact catalog event this input was parsed as.
    pub fn event_id(&self) -> EventId {
        self.event.event_id()
    }

    /// Returns one event-specific or unknown top-level field.
    pub fn field(&self, name: &str) -> Option<&serde_json::Value> {
        self.fields.get(name)
    }

    /// Returns all event-specific and unknown top-level fields.
    pub fn fields(&self) -> &BTreeMap<String, serde_json::Value> {
        &self.fields
    }

    fn str_field(&self, name: &str) -> Option<&str> {
        self.field(name).and_then(serde_json::Value::as_str)
    }

    /// Returns `SessionStart.source`, including undocumented values.
    pub fn source(&self) -> Option<SessionStartSource> {
        self.str_field("source").map(SessionStartSource::from)
    }

    /// Returns the `PreCompact`/`PostCompact` `trigger`, including
    /// undocumented values.
    pub fn trigger(&self) -> Option<CompactTrigger> {
        self.str_field("trigger").map(CompactTrigger::from)
    }

    /// Returns `SessionEnd.reason`, including undocumented values.
    pub fn session_end_reason(&self) -> Option<SessionEndReason> {
        self.str_field("reason").map(SessionEndReason::from)
    }

    /// Returns `Stop`/`SubagentStop` `stop_hook_active`: `true` when Codex is
    /// already continuing because of an earlier stop-hook block.
    pub fn stop_hook_active(&self) -> Option<bool> {
        self.field("stop_hook_active")
            .and_then(serde_json::Value::as_bool)
    }

    /// Returns `Stop`/`SubagentStop` `last_assistant_message`; `None` when
    /// the key is absent or JSON `null`.
    pub fn last_assistant_message(&self) -> Option<&str> {
        self.str_field("last_assistant_message")
    }

    /// Returns the submitted `UserPromptSubmit` prompt.
    pub fn prompt(&self) -> Option<&str> {
        self.str_field("prompt")
    }

    /// Returns the subagent identifier: required on subagent events and
    /// present on other events that ran on behalf of a subagent.
    pub fn agent_id(&self) -> Option<&str> {
        self.str_field("agent_id")
    }

    /// Returns the subagent type: required on subagent events and present on
    /// other events that ran on behalf of a subagent.
    pub fn agent_type(&self) -> Option<&str> {
        self.str_field("agent_type")
    }

    /// Returns the `SubagentStop` subagent transcript path; `None` when the
    /// key is absent or JSON `null`.
    pub fn agent_transcript_path(&self) -> Option<&hookkit_core::Utf8Path> {
        self.str_field("agent_transcript_path")
            .map(hookkit_core::Utf8Path::new)
    }

    /// Returns the `PermissionRequest` tool name.
    pub fn tool_name(&self) -> Option<&str> {
        self.str_field("tool_name")
    }

    /// Returns the `PermissionRequest` tool arguments in their native shape.
    pub fn tool_input(&self) -> Option<&serde_json::Value> {
        self.field("tool_input")
    }

    pub(crate) fn context(&self) -> NativeContext {
        let session_boundary = (self.event == CatalogEvent::SessionStart)
            .then(|| self.source())
            .flatten()
            .and_then(|source| {
                Some(match source {
                    SessionStartSource::Startup => SessionBoundaryKind::Startup,
                    SessionStartSource::Resume => SessionBoundaryKind::Resume,
                    SessionStartSource::Clear => SessionBoundaryKind::Clear,
                    SessionStartSource::Compact => SessionBoundaryKind::Compact,
                    SessionStartSource::Fork => SessionBoundaryKind::Fork,
                    // An undocumented source claims no boundary kind.
                    SessionStartSource::Unknown(_) => return None,
                })
            })
            .map(SessionBoundaryContext::observed);
        NativeContext {
            workspace_roots: vec![self.cwd.clone()],
            session_id: SessionId::new(&self.session_id).ok(),
            turn_id: self.turn_id.as_deref().and_then(|id| TurnId::new(id).ok()),
            transcript_path: self.transcript_path.clone(),
            tool_call_id: self
                .str_field("tool_use_id")
                .and_then(|id| ToolCallId::new(id).ok()),
            session_boundary,
            ..NativeContext::default()
        }
    }
}

fn parse(invocation: &RawInvocation, event: CatalogEvent) -> hookkit_core::Result<CatalogInput> {
    let name = event.name();
    super::protocol::require_event(invocation, name)?;
    for rule in event.fields() {
        let Some(value) = invocation.json().get(rule.name) else {
            if rule.required {
                super::protocol::require_field(invocation, rule.name, name)?;
            }
            continue;
        };
        let (matches, expected) = match rule.kind {
            FieldKind::String => (value.is_string(), "a string"),
            FieldKind::NullableString => (value.is_string() || value.is_null(), "a string or null"),
            FieldKind::Bool => (value.is_boolean(), "a boolean"),
            FieldKind::Any => (true, "any value"),
        };
        if !matches {
            return Err(super::protocol::invalid_input(
                name,
                format!("field {} must be {expected}", rule.name),
            ));
        }
    }
    super::protocol::deserialize_input(invocation, name)
}

#[derive(Debug, Clone, PartialEq)]
enum Outcome {
    /// Structured JSON; an object with no members emits empty stdout.
    Json(serde_json::Map<String, serde_json::Value>),
    /// Plain-text additional context.
    Text(String),
    /// Exit 2 with a stderr reason Codex reads as the block.
    BlockingError(String),
    /// Exit 1 with stderr Codex shows as the failure message.
    Failure(String),
}

#[derive(Debug, Clone, PartialEq)]
/// Type-erased output used by Codex catalog event wrappers.
///
/// Public event-specific output types are the intended constructors. This type
/// exists so dynamic harness dispatch can retain the exact event arm.
pub struct CatalogOutput {
    event: CatalogEvent,
    outcome: Outcome,
}

impl CatalogOutput {
    fn json(event: CatalogEvent, value: serde_json::Map<String, serde_json::Value>) -> Self {
        Self {
            event,
            outcome: Outcome::Json(value),
        }
    }

    fn empty(event: CatalogEvent) -> Self {
        Self {
            event,
            outcome: Outcome::Json(serde_json::Map::new()),
        }
    }

    fn text(event: CatalogEvent, value: impl Into<String>) -> Self {
        Self {
            event,
            outcome: Outcome::Text(value.into()),
        }
    }

    fn blocking(event: CatalogEvent, message: impl Into<String>) -> Self {
        Self {
            event,
            outcome: Outcome::BlockingError(message.into()),
        }
    }

    fn failure(event: CatalogEvent, message: impl Into<String>) -> Self {
        Self {
            event,
            outcome: Outcome::Failure(message.into()),
        }
    }

    fn with_top_level(
        mut self,
        name: &'static str,
        value: serde_json::Value,
    ) -> hookkit_core::Result<Self> {
        let Outcome::Json(output) = &mut self.outcome else {
            return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "structured fields cannot be added to text, blocking, or failure output",
            ));
        };
        output.insert(name.into(), value);
        Ok(self)
    }

    /// Returns the exact catalog event this output answers.
    pub fn event_id(&self) -> EventId {
        self.event.event_id()
    }

    pub(crate) fn emit(self) -> hookkit_core::Result<ProcessEmission> {
        let contract = self.event.contract();
        match self.outcome {
            Outcome::Json(value) => wire::structured(contract, &value),
            // Codex trims plain-text stdout, so blank text is its no-op.
            Outcome::Text(value) if value.trim().is_empty() => {
                Ok(ProcessEmission::command_empty(contract))
            }
            Outcome::Text(value) if wire::looks_like_json(&value) => {
                // Codex parses such stdout as JSON and would drop (or
                // misinterpret) the context, so use the structured form.
                wire::structured(contract, &context(self.event, value))
            }
            Outcome::Text(value) => Ok(ProcessEmission::command_text(contract, value)),
            Outcome::BlockingError(message) => wire::blocking_stderr(contract, message),
            Outcome::Failure(message) => ProcessEmission::command_stderr(contract, message, 1),
        }
    }
}

fn specific(
    event: CatalogEvent,
    fields: serde_json::Value,
) -> serde_json::Map<String, serde_json::Value> {
    let mut fields = object(fields);
    fields.insert("hookEventName".into(), event.name().into());
    let mut output = serde_json::Map::new();
    output.insert("hookSpecificOutput".into(), fields.into());
    output
}

fn context(
    event: CatalogEvent,
    value: impl Into<String>,
) -> serde_json::Map<String, serde_json::Value> {
    specific(
        event,
        serde_json::json!({"additionalContext": value.into()}),
    )
}

fn block(reason: impl Into<String>) -> serde_json::Map<String, serde_json::Value> {
    let mut output = serde_json::Map::new();
    output.insert("decision".into(), "block".into());
    output.insert("reason".into(), reason.into().into());
    output
}

fn object(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    let serde_json::Value::Object(value) = value else {
        unreachable!("catalog constructors build JSON objects")
    };
    value
}

macro_rules! event_spec {
    ($event:ident, $output:ident, $name:literal, $category:ident) => {
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
            const CONTRACT: ContractId = contract_id!($name);

            fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
                parse(invocation, CatalogEvent::$event)
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

macro_rules! output_type {
    ($output:ident, $name:literal) => {
        #[derive(Debug, Clone, PartialEq)]
        #[doc = concat!("Native response from a Codex `", $name, "` command hook.")]
        pub struct $output(CatalogOutput);
    };
}

macro_rules! no_op_control {
    ($output:ident, $event:ident) => {
        impl $output {
            /// Creates an empty response: exit 0 with no stdout, Codex's
            /// `no-op` outcome. Builders turn it into a structured response.
            pub fn no_op() -> Self {
                Self(CatalogOutput::empty(CatalogEvent::$event))
            }
        }
    };
}

macro_rules! continuation_controls {
    ($output:ident) => {
        impl $output {
            /// Sets the top-level `continue` control; `false` stops the turn.
            pub fn with_continue(self, continue_session: bool) -> hookkit_core::Result<Self> {
                self.0
                    .with_top_level("continue", continue_session.into())
                    .map(Self)
            }

            /// Sets the top-level stop reason shown when `continue` is `false`.
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
            /// Sets a user-facing top-level `systemMessage`, shown as a
            /// warning.
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

macro_rules! failure_control {
    ($output:ident, $event:ident) => {
        impl $output {
            /// Fails the run with exit code 1; Codex shows the stderr message
            /// (or `hook exited with code 1` when it is blank) and continues.
            pub fn failure(message: impl Into<String>) -> Self {
                Self(CatalogOutput::failure(CatalogEvent::$event, message))
            }
        }
    };
}

macro_rules! common_controls {
    ($output:ident, $event:ident) => {
        no_op_control!($output, $event);
        continuation_controls!($output);
        system_message_control!($output);
    };
}

output_type!(InterruptOutput, "Interrupt");

impl InterruptOutput {
    /// Creates a response whose only member is a user-facing `systemMessage`.
    ///
    /// Codex accepts nothing else from `Interrupt`: output cannot prevent the
    /// interruption, and every non-zero exit, including 2, fails the run.
    pub fn system_message(message: impl Into<String>) -> Self {
        Self(CatalogOutput::json(
            CatalogEvent::Interrupt,
            object(serde_json::json!({"systemMessage": message.into()})),
        ))
    }
}
no_op_control!(InterruptOutput, Interrupt);
system_message_control!(InterruptOutput);
event_spec!(Interrupt, InterruptOutput, "Interrupt", Agent);

output_type!(PermissionRequestOutput, "PermissionRequest");

impl PermissionRequestOutput {
    /// Creates a structured permission denial with a user-facing message.
    ///
    /// A blank message is still a denial; Codex substitutes
    /// `PermissionRequest hook denied approval`.
    pub fn deny(message: impl Into<String>) -> Self {
        Self(CatalogOutput::json(
            CatalogEvent::PermissionRequest,
            specific(
                CatalogEvent::PermissionRequest,
                serde_json::json!({
                    "decision": {"behavior": "deny", "message": message.into()}
                }),
            ),
        ))
    }

    /// Creates a structured permission approval.
    pub fn allow() -> Self {
        Self(CatalogOutput::json(
            CatalogEvent::PermissionRequest,
            specific(
                CatalogEvent::PermissionRequest,
                serde_json::json!({"decision": {"behavior": "allow"}}),
            ),
        ))
    }

    /// Creates a code-2 denial with required stderr text.
    ///
    /// The message must be non-empty after trimming; this is checked during
    /// emission.
    pub fn blocking_error(message: impl Into<String>) -> Self {
        Self(CatalogOutput::blocking(
            CatalogEvent::PermissionRequest,
            message,
        ))
    }
}
no_op_control!(PermissionRequestOutput, PermissionRequest);
system_message_control!(PermissionRequestOutput);
event_spec!(
    PermissionRequest,
    PermissionRequestOutput,
    "PermissionRequest",
    Tool
);

output_type!(PostCompactOutput, "PostCompact");
common_controls!(PostCompactOutput, PostCompact);
failure_control!(PostCompactOutput, PostCompact);
event_spec!(PostCompact, PostCompactOutput, "PostCompact", Context);

output_type!(PreCompactOutput, "PreCompact");

impl PreCompactOutput {
    /// Stops compaction with a structured reason.
    ///
    /// On a manual compact this aborts the turn as interrupted, which also
    /// dispatches `Interrupt`.
    pub fn stop(reason: impl Into<String>) -> Self {
        Self(CatalogOutput::json(
            CatalogEvent::PreCompact,
            object(serde_json::json!({"continue": false, "stopReason": reason.into()})),
        ))
    }
}
common_controls!(PreCompactOutput, PreCompact);
failure_control!(PreCompactOutput, PreCompact);
event_spec!(PreCompact, PreCompactOutput, "PreCompact", Context);

macro_rules! context_text_event {
    ($event:ident, $output:ident, $name:literal) => {
        output_type!($output, $name);

        impl $output {
            /// Creates a structured response that appends agent context.
            pub fn with_context(additional_context: impl Into<String>) -> Self {
                Self(CatalogOutput::json(
                    CatalogEvent::$event,
                    context(CatalogEvent::$event, additional_context),
                ))
            }

            /// Creates a successful plain-text context response.
            ///
            /// Codex trims plain-text stdout and parses stdout whose first
            /// non-whitespace character is `{` or `[` as JSON, which would
            /// drop the context. Such text is therefore emitted in the
            /// structured form of [`Self::with_context`] instead. Blank text
            /// emits nothing, which is what Codex makes of it.
            pub fn text_context(context: impl Into<String>) -> Self {
                Self(CatalogOutput::text(CatalogEvent::$event, context))
            }
        }
    };
}

context_text_event!(SessionStart, SessionStartOutput, "SessionStart");
common_controls!(SessionStartOutput, SessionStart);
event_spec!(SessionStart, SessionStartOutput, "SessionStart", Session);

context_text_event!(SubagentStart, SubagentStartOutput, "SubagentStart");
no_op_control!(SubagentStartOutput, SubagentStart);
system_message_control!(SubagentStartOutput);
event_spec!(SubagentStart, SubagentStartOutput, "SubagentStart", Agent);

output_type!(SessionEndOutput, "SessionEnd");

impl SessionEndOutput {
    /// Exits successfully without emitting stdout; Codex ignores `SessionEnd`
    /// stdout.
    pub fn no_op() -> Self {
        Self(CatalogOutput::empty(CatalogEvent::SessionEnd))
    }
}
failure_control!(SessionEndOutput, SessionEnd);
event_spec!(SessionEnd, SessionEndOutput, "SessionEnd", Session);

macro_rules! blocking_event {
    ($event:ident, $output:ident, $name:literal, $category:ident) => {
        output_type!($output, $name);

        impl $output {
            /// Creates a structured `decision: "block"` response: Codex keeps
            /// the agent working and uses the reason as its next prompt.
            ///
            /// The reason must be non-empty after trimming unless
            /// `continue: false` is also set; this is checked during emission.
            /// Under `continue: false` a blank block is left out of the JSON,
            /// since the stop already applies.
            pub fn block(reason: impl Into<String>) -> Self {
                Self(CatalogOutput::json(CatalogEvent::$event, block(reason)))
            }

            /// Creates a code-2 response whose stderr becomes the continuation
            /// prompt.
            ///
            /// The message must be non-empty after trimming; this is checked
            /// during emission.
            pub fn blocking_error(message: impl Into<String>) -> Self {
                Self(CatalogOutput::blocking(CatalogEvent::$event, message))
            }
        }

        common_controls!($output, $event);

        event_spec!($event, $output, $name, $category);
    };
}

blocking_event!(Stop, StopOutput, "Stop", Agent);
blocking_event!(SubagentStop, SubagentStopOutput, "SubagentStop", Agent);

context_text_event!(UserPromptSubmit, UserPromptSubmitOutput, "UserPromptSubmit");

impl UserPromptSubmitOutput {
    /// Creates a structured `decision: "block"` response that rejects the
    /// prompt.
    ///
    /// The reason must be non-empty after trimming unless `continue: false`
    /// is also set; this is checked during emission. Under `continue: false`
    /// a blank block is left out of the JSON, since the stop already applies.
    pub fn block(reason: impl Into<String>) -> Self {
        Self(CatalogOutput::json(
            CatalogEvent::UserPromptSubmit,
            block(reason),
        ))
    }

    /// Blocks the prompt while also appending context for the agent.
    ///
    /// Codex drops the context together with a block whose reason is blank,
    /// so the reason must be non-empty after trimming; this is checked during
    /// emission. Under `continue: false`, which stops the turn by itself, a
    /// blank block is left out of the JSON instead, so the context survives.
    pub fn block_with_context(
        reason: impl Into<String>,
        additional_context: impl Into<String>,
    ) -> Self {
        let mut value = block(reason);
        value.extend(context(CatalogEvent::UserPromptSubmit, additional_context));
        Self(CatalogOutput::json(CatalogEvent::UserPromptSubmit, value))
    }

    /// Creates a code-2 response that rejects the prompt with stderr text.
    ///
    /// The message must be non-empty after trimming; this is checked during
    /// emission.
    pub fn blocking_error(message: impl Into<String>) -> Self {
        Self(CatalogOutput::blocking(
            CatalogEvent::UserPromptSubmit,
            message,
        ))
    }
}
common_controls!(UserPromptSubmitOutput, UserPromptSubmit);
event_spec!(
    UserPromptSubmit,
    UserPromptSubmitOutput,
    "UserPromptSubmit",
    Prompt
);

/// Returns every native command implementation defined in this catalog module.
pub fn events() -> Vec<hookkit_core::NativeEventDescriptor> {
    vec![
        hookkit_core::NativeEventDescriptor::command::<Interrupt>(&["structured", "no-op"]),
        hookkit_core::NativeEventDescriptor::command::<PermissionRequest>(&[
            "structured",
            "structured-allow",
            "no-op",
            "exit-2",
        ]),
        hookkit_core::NativeEventDescriptor::command::<PostCompact>(&[
            "structured",
            "no-op",
            "failure",
        ]),
        hookkit_core::NativeEventDescriptor::command::<PreCompact>(&[
            "structured",
            "no-op",
            "failure",
        ]),
        hookkit_core::NativeEventDescriptor::command::<SessionEnd>(&["no-op", "failure"]),
        hookkit_core::NativeEventDescriptor::command::<SessionStart>(&[
            "structured",
            "no-op",
            "text-context",
        ]),
        hookkit_core::NativeEventDescriptor::command::<Stop>(&["structured", "no-op", "exit-2"]),
        hookkit_core::NativeEventDescriptor::command::<SubagentStart>(&[
            "structured",
            "no-op",
            "text-context",
        ]),
        hookkit_core::NativeEventDescriptor::command::<SubagentStop>(&[
            "structured",
            "no-op",
            "exit-2",
        ]),
        hookkit_core::NativeEventDescriptor::command::<UserPromptSubmit>(&[
            "structured",
            "structured-context",
            "structured-block",
            "no-op",
            "text-context",
            "exit-2",
        ]),
    ]
}

/// Returns definitive discriminator metadata for catalog-module events.
pub fn identification_descriptors() -> Vec<hookkit_core::IdentificationDescriptor> {
    vec![
        hookkit_core::IdentificationDescriptor::definitive::<Interrupt>(
            "/hook_event_name",
            "Interrupt",
        ),
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
    match CatalogEvent::from_name(event.name()) {
        Some(event) => parse(raw, event).map(Some),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(value: serde_json::Value) -> RawInvocation {
        RawInvocation::parse(serde_json::to_vec(&value).unwrap()).unwrap()
    }

    fn stdout_json(emission: &ProcessEmission) -> serde_json::Value {
        serde_json::from_slice(emission.stdout()).unwrap()
    }

    fn session_start(source: &str) -> serde_json::Value {
        serde_json::json!({
            "session_id": "s",
            "transcript_path": "/tmp/t",
            "cwd": "/repo",
            "hook_event_name": "SessionStart",
            "model": "gpt-test",
            "permission_mode": "default",
            "source": source,
        })
    }

    #[test]
    fn catalog_input_retains_event_specific_and_future_fields() {
        let mut value = session_start("startup");
        value["future"] = true.into();
        let input = SessionStart::parse(&raw(value)).unwrap();
        assert_eq!(input.field("source"), Some(&serde_json::json!("startup")));
        assert_eq!(input.source(), Some(SessionStartSource::Startup));
        assert_eq!(input.field("future"), Some(&serde_json::json!(true)));
        assert_eq!(
            SessionStart::context(&input).session_boundary.unwrap().kind,
            SessionBoundaryKind::Startup
        );
    }

    #[test]
    fn session_start_fork_maps_to_a_fork_boundary() {
        let input = SessionStart::parse(&raw(session_start("fork"))).unwrap();
        assert_eq!(input.source(), Some(SessionStartSource::Fork));
        assert_eq!(
            SessionStart::context(&input).session_boundary.unwrap().kind,
            SessionBoundaryKind::Fork
        );
    }

    #[test]
    fn undocumented_enum_values_parse_without_claiming_a_boundary() {
        let input = SessionStart::parse(&raw(session_start("restore"))).unwrap();
        assert_eq!(
            input.source(),
            Some(SessionStartSource::Unknown("restore".into()))
        );
        assert!(SessionStart::context(&input).session_boundary.is_none());

        let mut value = session_start("startup");
        value["permission_mode"] = "autoReview".into();
        let input = SessionStart::parse(&raw(value)).unwrap();
        assert_eq!(
            input.permission_mode,
            Some(PermissionMode::Unknown("autoReview".into()))
        );

        let end = SessionEnd::parse(&raw(serde_json::json!({
            "session_id": "s", "transcript_path": null, "cwd": "/repo",
            "hook_event_name": "SessionEnd", "reason": "logout"
        })))
        .unwrap();
        assert_eq!(
            end.session_end_reason(),
            Some(SessionEndReason::Unknown("logout".into()))
        );
    }

    #[test]
    fn catalog_parser_requires_schema_required_optional_envelope_fields() {
        let mut value = session_start("startup");
        value.as_object_mut().unwrap().remove("permission_mode");
        assert!(matches!(
            SessionStart::parse(&raw(value)),
            Err(hookkit_core::HookkitError::InvalidInputForHint { message, .. })
                if message == "missing required field permission_mode"
        ));
    }

    #[test]
    fn catalog_parser_keeps_model_required_outside_session_end() {
        let mut value = session_start("startup");
        value.as_object_mut().unwrap().remove("model");
        assert!(matches!(
            SessionStart::parse(&raw(value)),
            Err(hookkit_core::HookkitError::InvalidInputForHint { message, .. })
                if message == "missing required field model"
        ));
    }

    #[test]
    fn catalog_parser_checks_event_specific_field_types() {
        let stop = serde_json::json!({
            "session_id": "s", "transcript_path": null, "cwd": "/repo",
            "hook_event_name": "Stop", "model": "gpt-test", "turn_id": "t",
            "permission_mode": "default", "last_assistant_message": null,
            "stop_hook_active": "true"
        });
        assert!(matches!(
            Stop::parse(&raw(stop)),
            Err(hookkit_core::HookkitError::InvalidInputForHint { message, .. })
                if message == "field stop_hook_active must be a boolean"
        ));
    }

    #[test]
    fn typed_accessors_read_documented_fields() {
        let input = Stop::parse(&raw(serde_json::json!({
            "session_id": "s", "transcript_path": null, "cwd": "/repo",
            "hook_event_name": "Stop", "model": "gpt-test", "turn_id": "t",
            "permission_mode": "default", "last_assistant_message": "done",
            "stop_hook_active": true
        })))
        .unwrap();
        assert_eq!(input.stop_hook_active(), Some(true));
        assert_eq!(input.last_assistant_message(), Some("done"));
        assert_eq!(input.event_id(), Stop::EVENT);

        let input = SubagentStop::parse(&raw(serde_json::json!({
            "session_id": "s", "transcript_path": null, "cwd": "/repo",
            "hook_event_name": "SubagentStop", "model": "gpt-test", "turn_id": "t",
            "permission_mode": "default", "agent_id": "a", "agent_type": "reviewer",
            "agent_transcript_path": "/tmp/a.jsonl", "last_assistant_message": null,
            "stop_hook_active": false
        })))
        .unwrap();
        assert_eq!(input.agent_id(), Some("a"));
        assert_eq!(input.agent_type(), Some("reviewer"));
        assert_eq!(
            input.agent_transcript_path().map(|path| path.as_str()),
            Some("/tmp/a.jsonl")
        );
        assert_eq!(input.last_assistant_message(), None);

        let input = PreCompact::parse(&raw(serde_json::json!({
            "session_id": "s", "transcript_path": null, "cwd": "/repo",
            "hook_event_name": "PreCompact", "model": "gpt-test", "turn_id": "t",
            "trigger": "auto"
        })))
        .unwrap();
        assert_eq!(input.trigger(), Some(CompactTrigger::Auto));
    }

    #[test]
    fn deserialized_catalog_input_rejects_non_catalog_events_instead_of_panicking() {
        let pre_tool = serde_json::json!({
            "session_id": "s", "transcript_path": null, "cwd": "/repo",
            "hook_event_name": "PreToolUse", "model": "m", "turn_id": "t",
            "permission_mode": "default", "tool_name": "Bash", "tool_use_id": "u",
            "tool_input": {}
        });
        assert!(serde_json::from_value::<CatalogInput>(pre_tool).is_err());

        let mut input = SessionStart::parse(&raw(session_start("startup"))).unwrap();
        input.hook_event_name = "PreToolUse".into();
        let routed = <crate::protocol::Codex as hookkit_core::HarnessSpec>::input_event(
            &crate::protocol::AnyInput::Catalog(input),
        );
        assert_eq!(routed, SessionStart::EVENT);
    }

    #[test]
    fn serialization_is_lossless_and_omits_absent_optional_fields() {
        let value = serde_json::json!({
            "session_id": "s", "transcript_path": null, "cwd": "/repo",
            "hook_event_name": "SessionEnd", "reason": "other"
        });
        let input = SessionEnd::parse(&raw(value.clone())).unwrap();
        assert_eq!(serde_json::to_value(&input).unwrap(), value);
        let round_trip: CatalogInput = serde_json::from_value(value).unwrap();
        assert_eq!(round_trip, input);
    }

    #[test]
    fn event_specific_output_stamps_its_discriminator() {
        let emission = PermissionRequest::emit(PermissionRequestOutput::deny("policy")).unwrap();
        let output = stdout_json(&emission);
        assert_eq!(
            output["hookSpecificOutput"]["hookEventName"],
            "PermissionRequest"
        );
        assert_eq!(
            emission.contract().as_str(),
            "codex/commit-ff6aec9-r1/PermissionRequest"
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
    fn no_op_outputs_emit_empty_stdout_and_builders_emit_json() {
        for emission in [
            Interrupt::emit(InterruptOutput::no_op()).unwrap(),
            Stop::emit(StopOutput::no_op()).unwrap(),
            PermissionRequest::emit(PermissionRequestOutput::no_op()).unwrap(),
            SessionStart::emit(SessionStartOutput::no_op()).unwrap(),
            UserPromptSubmit::emit(UserPromptSubmitOutput::no_op()).unwrap(),
            PostCompact::emit(PostCompactOutput::no_op()).unwrap(),
        ] {
            assert!(emission.stdout().is_empty());
            assert!(emission.stderr().is_empty());
            assert_eq!(emission.exit_code(), 0);
        }
        let emission = Stop::emit(StopOutput::no_op().with_system_message("hi").unwrap()).unwrap();
        assert_eq!(
            stdout_json(&emission),
            serde_json::json!({"systemMessage": "hi"})
        );
    }

    #[test]
    fn interrupt_output_is_empty_or_system_message_only() {
        let emission = Interrupt::emit(InterruptOutput::system_message("saved")).unwrap();
        assert_eq!(
            stdout_json(&emission),
            serde_json::json!({"systemMessage": "saved"})
        );
        assert_eq!(
            emission.contract().as_str(),
            "codex/commit-ff6aec9-r1/Interrupt"
        );
        let built = InterruptOutput::no_op()
            .with_system_message("saved")
            .unwrap();
        assert_eq!(built, InterruptOutput::system_message("saved"));
    }

    #[test]
    fn blank_block_reasons_and_exit_two_messages_are_rejected() {
        assert!(Stop::emit(StopOutput::block("")).is_err());
        assert!(Stop::emit(StopOutput::block("  \n")).is_err());
        assert!(SubagentStop::emit(SubagentStopOutput::block("")).is_err());
        assert!(UserPromptSubmit::emit(UserPromptSubmitOutput::block("\t")).is_err());
        assert!(
            UserPromptSubmit::emit(UserPromptSubmitOutput::block_with_context("", "ctx")).is_err()
        );
        assert!(Stop::emit(StopOutput::blocking_error("\n")).is_err());
        assert!(SubagentStop::emit(SubagentStopOutput::blocking_error(" ")).is_err());
        assert!(PermissionRequest::emit(PermissionRequestOutput::blocking_error(" \n")).is_err());
        assert!(UserPromptSubmit::emit(UserPromptSubmitOutput::blocking_error("")).is_err());

        // `continue: false` takes precedence over the block in Codex.
        let stopping = StopOutput::block("")
            .with_continue(false)
            .unwrap()
            .with_stop_reason("halt")
            .unwrap();
        assert_eq!(
            stdout_json(&Stop::emit(stopping).unwrap()),
            serde_json::json!({"continue": false, "stopReason": "halt"})
        );

        // Regression: Codex discards UserPromptSubmit context whose block
        // reason is blank, even under `continue: false`; the blank block is
        // left out so the context survives while the stop still applies.
        let stopping = UserPromptSubmitOutput::block_with_context("", "clarify first")
            .with_continue(false)
            .unwrap();
        assert_eq!(
            stdout_json(&UserPromptSubmit::emit(stopping).unwrap()),
            serde_json::json!({
                "continue": false,
                "hookSpecificOutput": {
                    "hookEventName": "UserPromptSubmit",
                    "additionalContext": "clarify first"
                }
            })
        );

        // A blank PermissionRequest deny message is still a denial.
        assert!(PermissionRequest::emit(PermissionRequestOutput::deny("")).is_ok());
    }

    #[test]
    fn json_lookalike_text_context_uses_the_structured_form() {
        for text in ["[repo-policy] never push to main", " {\"continue\":false}"] {
            let emission =
                UserPromptSubmit::emit(UserPromptSubmitOutput::text_context(text)).unwrap();
            assert_eq!(
                stdout_json(&emission),
                serde_json::json!({
                    "hookSpecificOutput": {
                        "hookEventName": "UserPromptSubmit",
                        "additionalContext": text
                    }
                })
            );
            let emission = SessionStart::emit(SessionStartOutput::text_context(text)).unwrap();
            assert_eq!(
                stdout_json(&emission)["hookSpecificOutput"]["hookEventName"],
                "SessionStart"
            );
        }
        let emission =
            SubagentStart::emit(SubagentStartOutput::text_context("plain context")).unwrap();
        assert_eq!(emission.stdout(), b"plain context");
        let emission = SessionStart::emit(SessionStartOutput::text_context(" \n")).unwrap();
        assert!(emission.stdout().is_empty());
    }

    #[test]
    fn user_prompt_submit_has_structured_context_and_plain_block() {
        let emission =
            UserPromptSubmit::emit(UserPromptSubmitOutput::with_context("clarify")).unwrap();
        assert_eq!(
            stdout_json(&emission),
            serde_json::json!({
                "hookSpecificOutput": {"hookEventName": "UserPromptSubmit", "additionalContext": "clarify"}
            })
        );
        let emission = UserPromptSubmit::emit(UserPromptSubmitOutput::block("confirm")).unwrap();
        assert_eq!(
            stdout_json(&emission),
            serde_json::json!({"decision": "block", "reason": "confirm"})
        );
    }

    #[test]
    fn compaction_and_session_end_failures_surface_stderr() {
        for emission in [
            PreCompact::emit(PreCompactOutput::failure("hook failed\n")).unwrap(),
            PostCompact::emit(PostCompactOutput::failure("hook failed\n")).unwrap(),
            SessionEnd::emit(SessionEndOutput::failure("hook failed\n")).unwrap(),
        ] {
            assert!(emission.stdout().is_empty());
            assert_eq!(emission.stderr(), b"hook failed\n");
            assert_eq!(emission.exit_code(), 1);
        }
        assert!(PreCompactOutput::failure("x").with_continue(false).is_err());
    }

    #[test]
    fn session_end_parses_its_advisory_envelope_without_turn_fields() {
        let input = SessionEnd::parse(&raw(serde_json::json!({
            "session_id": "s", "transcript_path": null, "cwd": "/repo",
            "hook_event_name": "SessionEnd", "reason": "other"
        })))
        .unwrap();
        assert_eq!(input.model, None);
        assert_eq!(input.session_end_reason(), Some(SessionEndReason::Other));

        let emission = SessionEnd::emit(SessionEndOutput::no_op()).unwrap();
        assert!(emission.stdout().is_empty());
        assert!(emission.stderr().is_empty());
        assert_eq!(emission.exit_code(), 0);

        let missing = raw(serde_json::json!({
            "session_id": "s", "transcript_path": null, "cwd": "/repo",
            "hook_event_name": "SessionEnd"
        }));
        assert!(SessionEnd::parse(&missing).is_err());
    }
}
