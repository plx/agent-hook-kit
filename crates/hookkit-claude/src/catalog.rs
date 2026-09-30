//! Command-runtime implementations for the Claude events that share the
//! standard hook envelope.
//!
//! Every input here is a [`CatalogInput`]; every output type follows the
//! crate's output builder conventions (see the [crate] documentation).

use hookkit_core::{
    ContractId, EventCategory, EventId, EventSpec, HarnessId, NativeContext, ProcessEmission,
    RawInvocation, SessionId, ToolCallId, Utf8PathBuf,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use crate::ClaudeCommandEnvironment;
use crate::protocol::{SNAPSHOT_ID, contract_id, deserialize_input, require_event};
use crate::values::{
    CompactTrigger, Effort, McpServer, NotificationType, PermissionMode, SessionEndReason,
    StopFailureError,
};

/// Shared input envelope for the command events in this module.
///
/// The fields common to every Claude command hook are strongly typed. Every
/// other top-level field is retained verbatim in a map that
/// [`Self::field`] and [`Self::fields`] expose, and the typed accessors below
/// read from that map, so nothing an event sends is dropped. Re-serializing
/// the input omits absent optional fields; numbers keep their JSON value but
/// not necessarily their original spelling.
///
/// The exact parser that produced an input fixes its [`Self::event_id`].
/// Parsers check that every required event field is present with its
/// documented JSON type. Typed accessors return `None` when the field is
/// absent or has a different type; accessors documented for particular
/// events also return `None` for every other event, so a field such as
/// `reason` is never read with the wrong meaning. Prefer them to
/// [`Self::field`], where a misspelled name silently yields `None`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogInput {
    /// Native session identifier.
    pub session_id: String,
    /// Path to the native conversation transcript.
    pub transcript_path: Utf8PathBuf,
    /// Current workspace directory.
    pub cwd: Utf8PathBuf,
    /// Authoritative native event discriminator as sent on the wire.
    pub hook_event_name: String,
    /// Optional nested effort setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,
    /// Permission policy active for the event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
    /// Prompt identifier associated with the current turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_id: Option<String>,
    /// Session scratchpad directory; absent when the session has none
    /// (Claude Code v2.1.257 or later).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scratchpad_dir: Option<Utf8PathBuf>,
    #[serde(flatten)]
    fields: BTreeMap<String, Value>,
    #[serde(skip)]
    event: Option<&'static str>,
}

impl CatalogInput {
    /// Returns one event-specific or unknown top-level field.
    pub fn field(&self, name: &str) -> Option<&Value> {
        self.fields.get(name)
    }

    /// Returns all event-specific and unknown top-level fields.
    pub fn fields(&self) -> &BTreeMap<String, Value> {
        &self.fields
    }

    /// Returns the exact event this input was parsed for.
    ///
    /// For an input produced by an exact parser this is that parser's event,
    /// even if [`Self::hook_event_name`] is later modified. For an input
    /// deserialized directly, the identity is derived from
    /// `hook_event_name`; a name outside the Claude catalog yields a dynamic
    /// event id carrying that name.
    pub fn event_id(&self) -> EventId {
        match self
            .event
            .or_else(|| catalog_event_name(&self.hook_event_name))
        {
            Some(name) => EventId::builtin(HarnessId::CLAUDE_CODE, name),
            None => EventId::new(HarnessId::CLAUDE_CODE, self.hook_event_name.clone())
                .unwrap_or_else(|_| EventId::builtin(HarnessId::CLAUDE_CODE, "unnamed")),
        }
    }

    fn event_name(&self) -> &str {
        self.event.unwrap_or(&self.hook_event_name)
    }

    fn string(&self, name: &str) -> Option<&str> {
        self.fields.get(name).and_then(Value::as_str)
    }

    fn scoped_string(&self, events: &[&str], name: &str) -> Option<&str> {
        events
            .contains(&self.event_name())
            .then(|| self.string(name))
            .flatten()
    }

    /// Subagent identifier, when the event ran on behalf of a subagent.
    /// Always present on `SubagentStart` and `SubagentStop`.
    pub fn agent_id(&self) -> Option<&str> {
        self.string("agent_id")
    }

    /// Subagent type. Always present on `SubagentStart` and `SubagentStop`;
    /// may be empty on `SubagentStop` for internal agents.
    pub fn agent_type(&self) -> Option<&str> {
        self.string("agent_type")
    }

    /// Harness-native tool name. Present on `PreToolUse`,
    /// `PermissionRequest`, `PostToolUseFailure`, and `PermissionDenied`.
    pub fn tool_name(&self) -> Option<&str> {
        self.string("tool_name")
    }

    /// Tool arguments in their native JSON shape. Present on the same events
    /// as [`Self::tool_name`].
    pub fn tool_input(&self) -> Option<&Value> {
        self.fields.get("tool_input")
    }

    /// Native tool-call identifier. Present on `PreToolUse`,
    /// `PostToolUseFailure`, and `PermissionDenied`; `PermissionRequest`
    /// does not carry one.
    pub fn tool_use_id(&self) -> Option<&str> {
        self.string("tool_use_id")
    }

    /// MCP server that owns the tool, for MCP tools on Claude Code v2.1.274
    /// or later. `None` when absent or malformed.
    pub fn mcp_server(&self) -> Option<McpServer> {
        self.fields
            .get("mcp_server")
            .and_then(|value| McpServer::deserialize(value).ok())
    }

    /// Prompt text for `UserPromptSubmit` and `UserPromptExpansion`.
    pub fn prompt(&self) -> Option<&str> {
        self.scoped_string(&["UserPromptSubmit", "UserPromptExpansion"], "prompt")
    }

    /// Whether a Stop hook already continued this turn, for `Stop` and
    /// `SubagentStop`.
    pub fn stop_hook_active(&self) -> Option<bool> {
        ["Stop", "SubagentStop"]
            .contains(&self.event_name())
            .then(|| self.fields.get("stop_hook_active").and_then(Value::as_bool))
            .flatten()
    }

    /// Final assistant text for `Stop`, `SubagentStop`, and `StopFailure`.
    ///
    /// Optional: the Agent SDK types it optional, and a subagent that hands
    /// back through `SubagentHandback` sends only closing text, if any.
    pub fn last_assistant_message(&self) -> Option<&str> {
        self.scoped_string(
            &["Stop", "SubagentStop", "StopFailure"],
            "last_assistant_message",
        )
    }

    /// Path to the subagent transcript, for `SubagentStop`.
    pub fn agent_transcript_path(&self) -> Option<&str> {
        self.scoped_string(&["SubagentStop"], "agent_transcript_path")
    }

    /// Reason the session ended, for `SessionEnd`.
    pub fn session_end_reason(&self) -> Option<SessionEndReason> {
        self.scoped_string(&["SessionEnd"], "reason")
            .map(SessionEndReason::from)
    }

    /// Notification text, for `Notification`.
    pub fn notification_message(&self) -> Option<&str> {
        self.scoped_string(&["Notification"], "message")
    }

    /// Notification kind, for `Notification`.
    pub fn notification_type(&self) -> Option<NotificationType> {
        self.scoped_string(&["Notification"], "notification_type")
            .map(NotificationType::from)
    }

    /// Optional notification title, for `Notification`.
    pub fn notification_title(&self) -> Option<&str> {
        self.scoped_string(&["Notification"], "title")
    }

    /// API failure that ended the turn, for `StopFailure`.
    pub fn stop_failure_error(&self) -> Option<StopFailureError> {
        self.scoped_string(&["StopFailure"], "error")
            .map(StopFailureError::from)
    }

    /// Optional failure detail string, for `StopFailure`.
    pub fn stop_failure_details(&self) -> Option<&str> {
        self.scoped_string(&["StopFailure"], "error_details")
    }

    /// What triggered compaction, for `PreCompact` and `PostCompact`.
    pub fn compact_trigger(&self) -> Option<CompactTrigger> {
        self.scoped_string(&["PreCompact", "PostCompact"], "trigger")
            .map(CompactTrigger::from)
    }

    /// Instructions passed to a manual `/compact`, for `PreCompact`.
    ///
    /// `None` for automatic compaction and for a bare `/compact`, where
    /// Claude Code sends `null`.
    pub fn custom_instructions(&self) -> Option<&str> {
        self.scoped_string(&["PreCompact"], "custom_instructions")
    }

    /// Generated conversation summary, for `PostCompact`.
    pub fn compact_summary(&self) -> Option<&str> {
        self.scoped_string(&["PostCompact"], "compact_summary")
    }

    /// Error text reported by the failed tool, for `PostToolUseFailure`.
    pub fn tool_error(&self) -> Option<&str> {
        self.scoped_string(&["PostToolUseFailure"], "error")
    }

    /// Whether the failure reached Claude Code as an abort, for
    /// `PostToolUseFailure`.
    pub fn is_interrupt(&self) -> Option<bool> {
        (self.event_name() == "PostToolUseFailure")
            .then(|| self.fields.get("is_interrupt").and_then(Value::as_bool))
            .flatten()
    }

    /// Denial reason, for `PermissionDenied`.
    pub fn permission_denied_reason(&self) -> Option<&str> {
        self.scoped_string(&["PermissionDenied"], "reason")
    }

    /// Tool calls in the completed batch, for `PostToolBatch`.
    pub fn tool_calls(&self) -> Option<&[Value]> {
        (self.event_name() == "PostToolBatch")
            .then(|| {
                self.fields
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .map(Vec::as_slice)
            })
            .flatten()
    }

    pub(crate) fn context(&self) -> NativeContext {
        NativeContext {
            workspace_roots: vec![self.cwd.clone()],
            session_id: SessionId::new(&self.session_id).ok(),
            transcript_path: Some(self.transcript_path.clone()),
            tool_call_id: self.tool_use_id().and_then(|id| ToolCallId::new(id).ok()),
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

const CATALOG_EVENTS: &[&str] = &[
    "ConfigChange",
    "CwdChanged",
    "DirectoryAdded",
    "Elicitation",
    "ElicitationResult",
    "FileChanged",
    "InstructionsLoaded",
    "MessageDisplay",
    "Notification",
    "PermissionDenied",
    "PermissionRequest",
    "PostCompact",
    "PostToolBatch",
    "PostToolUseFailure",
    "PreCompact",
    "PreToolUse",
    "SessionEnd",
    "Setup",
    "Stop",
    "StopFailure",
    "SubagentStart",
    "SubagentStop",
    "TaskCompleted",
    "TaskCreated",
    "TeammateIdle",
    "UserPromptExpansion",
    "UserPromptSubmit",
    "WorktreeRemove",
];

fn catalog_event_name(event: &str) -> Option<&'static str> {
    CATALOG_EVENTS.iter().copied().find(|name| *name == event)
}

/// JSON type a required input field must have.
#[derive(Debug, Clone, Copy)]
pub(crate) enum FieldKind {
    Any,
    String,
    Bool,
    Array,
    NullableString,
}

impl FieldKind {
    fn accepts(self, value: &Value) -> bool {
        match self {
            Self::Any => true,
            Self::String => value.is_string(),
            Self::Bool => value.is_boolean(),
            Self::Array => value.is_array(),
            Self::NullableString => value.is_string() || value.is_null(),
        }
    }

    fn describe(self) -> &'static str {
        match self {
            Self::Any => "any JSON value",
            Self::String => "a string",
            Self::Bool => "a boolean",
            Self::Array => "an array",
            Self::NullableString => "a string or null",
        }
    }
}

pub(crate) fn require_fields(
    invocation: &RawInvocation,
    event: &'static str,
    required: &[(&str, FieldKind)],
) -> hookkit_core::Result<()> {
    for (field, kind) in required {
        let invalid = |message: String| hookkit_core::HookkitError::InvalidInputForHint {
            event: EventId::builtin(HarnessId::CLAUDE_CODE, event),
            message,
        };
        match invocation.json().get(field) {
            None => return Err(invalid(format!("missing required field {field}"))),
            Some(value) if !kind.accepts(value) => {
                return Err(invalid(format!(
                    "field {field} must be {}",
                    kind.describe()
                )));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

fn parse(
    invocation: &RawInvocation,
    event: &'static str,
    required: &[(&str, FieldKind)],
) -> hookkit_core::Result<CatalogInput> {
    require_event(invocation, event)?;
    require_fields(invocation, event, required)?;
    let mut input: CatalogInput = deserialize_input(invocation, event)?;
    input.event = Some(event);
    Ok(input)
}

/// Reports whether Claude Code parses `text` as JSON rather than plain text.
///
/// Since Claude Code v2.1.248, stdout whose trimmed form starts with `{` and
/// ends with `}` is parsed as a JSON response; when that fails the text is
/// dropped and a hook error is reported instead of adding context.
///
/// Claude Code trims as JavaScript's `String.prototype.trim` does. Rust's
/// `char::is_whitespace` plus U+FEFF covers that set and also NEXT LINE
/// (U+0085), so the predicate errs toward structured context, which still
/// reaches Claude.
pub(crate) fn parsed_as_json(text: &str) -> bool {
    let trimmed = text.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    trimmed.starts_with('{') && trimmed.ends_with('}')
}

/// Reports whether a JSON response makes a blocking decision that Claude
/// Code applies with exit 2: a top-level `decision: "block"`, a
/// `permissionDecision: "deny"`, or a `PermissionRequest` decision with
/// `behavior: "deny"`.
///
/// Claude Code takes the exit-2 blocking message from such a decision's
/// reason and falls back to stderr only when the JSON makes none.
pub(crate) fn makes_blocking_decision(value: &Map<String, Value>) -> bool {
    let specific = value.get("hookSpecificOutput");
    value.get("decision").and_then(Value::as_str) == Some("block")
        || specific
            .and_then(|specific| specific.get("permissionDecision"))
            .and_then(Value::as_str)
            == Some("deny")
        || specific
            .and_then(|specific| specific.get("decision"))
            .and_then(|decision| decision.get("behavior"))
            .and_then(Value::as_str)
            == Some("deny")
}

/// Checks the stderr of an exit-2 response that also prints `value`.
///
/// The snapshot lets such a response leave stderr empty. HookKit requires a
/// message only when the JSON makes no blocking decision, because stderr is
/// then the only message Claude Code can show.
pub(crate) fn require_exit_2_message(
    value: &Map<String, Value>,
    stderr: &str,
) -> hookkit_core::Result<()> {
    if stderr.is_empty() && !makes_blocking_decision(value) {
        return Err(hookkit_core::HookkitError::InvalidProcessEmission(
            "an exit-2 response needs a non-empty message unless its JSON makes a blocking decision",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
enum Outcome {
    /// Exit 0 with a JSON object on stdout.
    Json(Map<String, Value>),
    /// Exit 0 with plain-text stdout.
    Text(String),
    /// Exit 0 with empty stdout.
    Empty,
    /// Exit 2 with a JSON object on stdout and stderr, which may be empty
    /// only when the JSON makes a blocking decision.
    BlockingJson {
        value: Map<String, Value>,
        stderr: String,
    },
    /// Nonzero exit with stderr and empty stdout.
    Stderr {
        message: String,
        exit_code: u8,
        required: bool,
    },
}

/// Builds `{"hookSpecificOutput": {"hookEventName": <event>, ..fields}}`.
fn hook_specific_object<E: EventSpec>(
    fields: impl IntoIterator<Item = (&'static str, Value)>,
) -> Map<String, Value> {
    let mut specific = Map::new();
    specific.insert("hookEventName".into(), E::EVENT.name().into());
    specific.extend(
        fields
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value)),
    );
    let mut object = Map::new();
    object.insert("hookSpecificOutput".into(), Value::Object(specific));
    object
}

#[derive(Debug, Clone)]
/// Type-erased output used by Claude Code catalog event wrappers.
///
/// Public event-specific output types are the intended constructors. This type
/// exists so dynamic harness dispatch can retain the exact event arm.
pub struct CatalogOutput {
    event: EventId,
    contract: ContractId,
    outcome: Outcome,
}

impl CatalogOutput {
    fn new<E: EventSpec>(outcome: Outcome) -> Self {
        Self {
            event: E::EVENT,
            contract: E::CONTRACT,
            outcome,
        }
    }

    pub(crate) fn json<E: EventSpec>(value: Map<String, Value>) -> Self {
        Self::new::<E>(Outcome::Json(value))
    }

    pub(crate) fn empty_object<E: EventSpec>() -> Self {
        Self::json::<E>(Map::new())
    }

    pub(crate) fn empty<E: EventSpec>() -> Self {
        Self::new::<E>(Outcome::Empty)
    }

    /// A JSON object holding exactly `fields` at the top level.
    pub(crate) fn top_level<E: EventSpec>(
        fields: impl IntoIterator<Item = (&'static str, Value)>,
    ) -> Self {
        Self::json::<E>(
            fields
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
        )
    }

    /// A JSON object whose `hookSpecificOutput` holds the event's
    /// discriminator and `fields`.
    pub(crate) fn hook_specific<E: EventSpec>(
        fields: impl IntoIterator<Item = (&'static str, Value)>,
    ) -> Self {
        Self::json::<E>(hook_specific_object::<E>(fields))
    }

    /// [`Self::hook_specific`] printed while exiting 2 with `stderr`.
    pub(crate) fn hook_specific_with_stderr<E: EventSpec>(
        fields: impl IntoIterator<Item = (&'static str, Value)>,
        stderr: String,
    ) -> Self {
        Self::new::<E>(Outcome::BlockingJson {
            value: hook_specific_object::<E>(fields),
            stderr,
        })
    }

    /// A JSON object with a top-level `decision: "block"` and `reason`.
    pub(crate) fn block<E: EventSpec>(reason: String) -> Self {
        Self::top_level::<E>([("decision", "block".into()), ("reason", reason.into())])
    }

    /// Plain-text context, re-routed to `additionalContext` JSON when Claude
    /// Code would parse the text as JSON and drop it.
    pub(crate) fn text_context<E: EventSpec>(text: String) -> Self {
        if parsed_as_json(&text) {
            Self::hook_specific::<E>([("additionalContext", text.into())])
        } else {
            Self::new::<E>(Outcome::Text(text))
        }
    }

    pub(crate) fn blocking<E: EventSpec>(message: impl Into<String>) -> Self {
        Self::new::<E>(Outcome::Stderr {
            message: message.into(),
            exit_code: 2,
            required: true,
        })
    }

    pub(crate) fn nonblocking<E: EventSpec>(message: impl Into<String>) -> Self {
        Self::new::<E>(Outcome::Stderr {
            message: message.into(),
            exit_code: 1,
            required: true,
        })
    }

    pub(crate) fn failure<E: EventSpec>(
        message: impl Into<String>,
        exit_code: u8,
    ) -> hookkit_core::Result<Self> {
        if exit_code == 0 {
            return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "failure exit code must be nonzero",
            ));
        }
        Ok(Self::new::<E>(Outcome::Stderr {
            message: message.into(),
            exit_code,
            required: false,
        }))
    }

    fn object_mut(&mut self) -> hookkit_core::Result<&mut Map<String, Value>> {
        match &mut self.outcome {
            Outcome::Json(value) => Ok(value),
            _ => Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "structured fields cannot be added to text, empty, or exit-code output",
            )),
        }
    }

    fn object(&self) -> Option<&Map<String, Value>> {
        match &self.outcome {
            Outcome::Json(value) | Outcome::BlockingJson { value, .. } => Some(value),
            _ => None,
        }
    }

    pub(crate) fn with_top_level(
        mut self,
        name: &'static str,
        value: Value,
    ) -> hookkit_core::Result<Self> {
        self.object_mut()?.insert(name.into(), value);
        Ok(self)
    }

    pub(crate) fn specific_mut(&mut self) -> hookkit_core::Result<&mut Map<String, Value>> {
        let event = self.event.name().to_owned();
        self.object_mut()?
            .entry("hookSpecificOutput")
            .or_insert_with(|| serde_json::json!({ "hookEventName": event }))
            .as_object_mut()
            .ok_or(hookkit_core::HookkitError::InvalidProcessEmission(
                "hookSpecificOutput must be a JSON object",
            ))
    }

    pub(crate) fn with_specific(
        mut self,
        name: &'static str,
        value: Value,
    ) -> hookkit_core::Result<Self> {
        self.specific_mut()?.insert(name.into(), value);
        Ok(self)
    }

    pub(crate) fn specific_field(&self, name: &str) -> Option<&Value> {
        self.object()?.get("hookSpecificOutput")?.get(name)
    }

    pub(crate) fn with_block(mut self, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        self.object_mut()?;
        self.insert_block(reason.into());
        Ok(self)
    }

    /// Adds `decision: "block"` and `reason` to a JSON outcome; a no-op for
    /// any other outcome.
    pub(crate) fn insert_block(&mut self, reason: String) {
        if let Outcome::Json(value) = &mut self.outcome {
            value.insert("decision".into(), "block".into());
            value.insert("reason".into(), reason.into());
        }
    }

    /// Turns a JSON outcome into exit 2 with the same stdout and `stderr`.
    ///
    /// `stderr` may be empty only when the JSON makes a blocking decision,
    /// whose reason Claude Code then uses as the message. Otherwise an empty
    /// `stderr` is rejected here, while the caller can still choose another
    /// response, rather than when the output is emitted.
    pub(crate) fn into_blocking(self, stderr: impl Into<String>) -> hookkit_core::Result<Self> {
        let Outcome::Json(value) = self.outcome else {
            return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "only a structured JSON response can exit 2 with stdout",
            ));
        };
        let stderr = stderr.into();
        require_exit_2_message(&value, &stderr)?;
        Ok(Self {
            outcome: Outcome::BlockingJson { value, stderr },
            ..self
        })
    }

    pub(crate) fn event_id(&self) -> EventId {
        self.event.clone()
    }

    pub(crate) fn emit(self) -> hookkit_core::Result<ProcessEmission> {
        let contract = self.contract;
        match self.outcome {
            Outcome::Json(value) => ProcessEmission::command_json(contract, &value),
            Outcome::Text(value) => Ok(ProcessEmission::command_text(contract, value)),
            Outcome::Empty => Ok(ProcessEmission::command_empty(contract)),
            Outcome::BlockingJson { value, stderr } => {
                require_exit_2_message(&value, &stderr)?;
                Ok(ProcessEmission::command_unchecked(
                    contract,
                    serde_json::to_vec(&value)?,
                    stderr.into_bytes(),
                    2,
                ))
            }
            Outcome::Stderr {
                message,
                exit_code,
                required: true,
            } => ProcessEmission::command_required_stderr(contract, message, exit_code),
            Outcome::Stderr {
                message,
                exit_code,
                required: false,
            } => ProcessEmission::command_stderr(contract, message, exit_code),
        }
    }
}

macro_rules! output_type {
    ($output:ident, $name:literal) => {
        #[derive(Debug, Clone)]
        #[doc = concat!("Native response from a Claude Code `", $name, "` command hook.")]
        pub struct $output(CatalogOutput);
    };
}

macro_rules! event_spec {
    (
        $event:ident, $output:ident, $name:literal, $category:ident,
        [$($required:literal : $kind:ident),* $(,)?]
    ) => {
        #[doc = concat!("Native Claude Code `", $name, "` command contract.")]
        pub enum $event {}

        impl EventSpec for $event {
            type Input = CatalogInput;
            type CommandEnvironment = ClaudeCommandEnvironment;
            type CommandOutput = $output;
            const HARNESS: HarnessId = HarnessId::CLAUDE_CODE;
            const SNAPSHOT: hookkit_core::SnapshotId = SNAPSHOT_ID;
            const EVENT: EventId = EventId::builtin(HarnessId::CLAUDE_CODE, $name);
            const CATEGORY: EventCategory = EventCategory::$category;
            const CONTRACT: ContractId = contract_id!($name);

            fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
                parse(invocation, $name, &[$(($required, FieldKind::$kind)),*])
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

/// Generates the always-available `no_op` constructor.
macro_rules! no_op {
    ($event:ident, $output:ident) => {
        impl $output {
            /// Creates an empty structured response (`{}`).
            pub fn no_op() -> Self {
                Self($crate::catalog::CatalogOutput::empty_object::<$event>())
            }
        }
    };
}
pub(crate) use no_op;

/// Generates universal-field builders Claude Code honors for the event.
macro_rules! universal_builders {
    ($output:ident: $($field:ident),* $(,)?) => {
        impl $output {
            $(universal_builders!(@active $field);)*
        }
    };
    (@active continue_session) => {
        /// Sets Claude's universal top-level `continue` control. `false`
        /// stops Claude after the hook runs, taking precedence over
        /// event-specific decisions.
        pub fn with_continue(self, continue_session: bool) -> hookkit_core::Result<Self> {
            self.0
                .with_top_level("continue", continue_session.into())
                .map(Self)
        }
    };
    (@active stop_reason) => {
        /// Sets the universal top-level `stopReason`, shown to the user when
        /// `continue` is `false`. It stays in the conversation, so Claude sees
        /// it if the conversation continues.
        pub fn with_stop_reason(self, reason: impl Into<String>) -> hookkit_core::Result<Self> {
            self.0
                .with_top_level("stopReason", reason.into().into())
                .map(Self)
        }
    };
    (@active system_message) => {
        /// Sets the universal top-level `systemMessage`, a warning shown to
        /// the user.
        pub fn with_system_message(self, message: impl Into<String>) -> hookkit_core::Result<Self> {
            self.0
                .with_top_level("systemMessage", message.into().into())
                .map(Self)
        }
    };
    (@active terminal_sequence) => {
        /// Requests emission of an allowlisted terminal notification
        /// sequence (OSC 0/1/2/9/99/777 or BEL). Claude Code writes it only in
        /// an interactive session with its interface on screen.
        pub fn with_terminal_sequence(
            self,
            sequence: impl Into<String>,
        ) -> hookkit_core::Result<Self> {
            self.0
                .with_top_level("terminalSequence", sequence.into().into())
                .map(Self)
        }
    };
}
pub(crate) use universal_builders;

/// Generates deprecated builders for universal fields Claude Code discards
/// on the event. They still emit the field, which the output schema accepts.
macro_rules! discarded_builders {
    ($event:ident, $output:ident: $($field:ident),* $(,)?) => {
        impl $output {
            $(discarded_builders!(@discarded $event, $field);)*
        }
    };
    (@discarded $event:ident, continue_session) => {
        /// Sets the universal `continue` field, which Claude Code discards
        /// for this event.
        #[deprecated(note = "Claude Code discards `continue` for this event (claude-code/docs-2026-09-29-r1)")]
        pub fn with_continue(self, continue_session: bool) -> hookkit_core::Result<Self> {
            self.0
                .with_top_level("continue", continue_session.into())
                .map(Self)
        }
    };
    (@discarded $event:ident, stop_reason) => {
        /// Sets the universal `stopReason` field, which has no effect because
        /// Claude Code discards `continue` for this event.
        #[deprecated(note = "Claude Code discards `continue` and `stopReason` for this event (claude-code/docs-2026-09-29-r1)")]
        pub fn with_stop_reason(self, reason: impl Into<String>) -> hookkit_core::Result<Self> {
            self.0
                .with_top_level("stopReason", reason.into().into())
                .map(Self)
        }
    };
    (@discarded $event:ident, system_message) => {
        /// Sets the universal `systemMessage` field, which Claude Code
        /// discards for this event.
        #[deprecated(note = "Claude Code discards `systemMessage` for this event (claude-code/docs-2026-09-29-r1)")]
        pub fn with_system_message(self, message: impl Into<String>) -> hookkit_core::Result<Self> {
            self.0
                .with_top_level("systemMessage", message.into().into())
                .map(Self)
        }
    };
    (@discarded $event:ident, terminal_sequence) => {
        /// Sets the universal `terminalSequence` field, which Claude Code
        /// ignores for this event.
        #[deprecated(note = "Claude Code ignores this event's JSON output, including `terminalSequence` (claude-code/docs-2026-09-29-r1)")]
        pub fn with_terminal_sequence(
            self,
            sequence: impl Into<String>,
        ) -> hookkit_core::Result<Self> {
            self.0
                .with_top_level("terminalSequence", sequence.into().into())
                .map(Self)
        }
    };
    (@discarded $event:ident, suppress_output) => {
        /// Sets the universal `suppressOutput` field. Claude Code accepts it
        /// but ignores it on every event: a successful hook's stdout is never
        /// shown in the transcript.
        #[deprecated(note = "Claude Code accepts `suppressOutput` but ignores it on every event (claude-code/docs-2026-09-29-r1)")]
        pub fn with_suppress_output(self, suppress: bool) -> hookkit_core::Result<Self> {
            self.0
                .with_top_level("suppressOutput", suppress.into())
                .map(Self)
        }
    };
    (@discarded $event:ident, system_message_constructor) => {
        /// Creates a response carrying only a `systemMessage`, which Claude
        /// Code discards for this event.
        #[deprecated(note = "Claude Code discards `systemMessage` for this event (claude-code/docs-2026-09-29-r1); use `no_op`")]
        pub fn with_system_message(message: impl Into<String>) -> Self {
            Self($crate::catalog::CatalogOutput::top_level::<$event>([(
                "systemMessage",
                message.into().into(),
            )]))
        }
    };
}

/// Generates the `with_context` constructor and the chainable
/// `with_additional_context` builder.
macro_rules! context_builders {
    ($event:ident, $output:ident, $where_:literal) => {
        impl $output {
            #[doc = concat!(
                        "Creates a structured response that adds `additionalContext` for Claude ",
                        $where_, "."
                    )]
            pub fn with_context(additional_context: impl Into<String>) -> Self {
                Self($crate::catalog::CatalogOutput::hook_specific::<$event>([(
                    "additionalContext",
                    additional_context.into().into(),
                )]))
            }

            /// Sets `hookSpecificOutput.additionalContext` on a structured
            /// response, replacing any earlier value.
            pub fn with_additional_context(
                self,
                additional_context: impl Into<String>,
            ) -> hookkit_core::Result<Self> {
                self.0
                    .with_specific("additionalContext", additional_context.into().into())
                    .map(Self)
            }
        }
    };
}
pub(crate) use context_builders;

/// Generates the plain-text context constructor with Claude's JSON guard.
macro_rules! text_context {
    ($event:ident, $output:ident) => {
        impl $output {
            /// Creates a successful plain-text context response.
            ///
            /// Claude Code v2.1.248 and later parse stdout whose trimmed text
            /// starts with `{` and ends with `}` as JSON and drop it when it is
            /// not a valid response. Such text is therefore emitted as
            /// structured `additionalContext` instead, so the context always
            /// reaches Claude. Other text is written verbatim, without a
            /// trailing newline.
            pub fn text_context(context: impl Into<String>) -> Self {
                Self($crate::catalog::CatalogOutput::text_context::<$event>(
                    context.into(),
                ))
            }
        }
    };
}
pub(crate) use text_context;

/// Generates top-level `decision: "block"` constructors and builders.
macro_rules! block_builders {
    ($event:ident, $output:ident, $effect:literal) => {
        impl $output {
            #[doc = concat!("Creates a top-level `decision: \"block\"` response. ", $effect)]
            pub fn block(reason: impl Into<String>) -> Self {
                Self($crate::catalog::CatalogOutput::block::<$event>(
                    reason.into(),
                ))
            }

            /// Adds a top-level `decision: "block"` and `reason` to a
            /// structured response, replacing any earlier reason.
            pub fn with_block(self, reason: impl Into<String>) -> hookkit_core::Result<Self> {
                self.0.with_block(reason).map(Self)
            }
        }
    };
}
pub(crate) use block_builders;

/// Generates the combined block-and-context constructor.
macro_rules! block_with_context {
    ($event:ident, $output:ident) => {
        impl $output {
            /// Blocks with `reason` while also adding `additionalContext` for
            /// Claude. Equivalent to `with_context(additional_context)`
            /// followed by `with_block(reason)`.
            pub fn block_with_context(
                reason: impl Into<String>,
                additional_context: impl Into<String>,
            ) -> Self {
                let mut output = $crate::catalog::CatalogOutput::hook_specific::<$event>([(
                    "additionalContext",
                    additional_context.into().into(),
                )]);
                output.insert_block(reason.into());
                Self(output)
            }
        }
    };
}

/// Generates the code-2 constructor and the structured exit-2 builder.
macro_rules! blocking_error {
    ($event:ident, $output:ident, $effect:literal) => {
        impl $output {
            #[doc = concat!(
                        "Creates a code-2 blocking response with required, non-empty stderr. ",
                        $effect
                    )]
            pub fn blocking_error(message: impl Into<String>) -> Self {
                Self($crate::catalog::CatalogOutput::blocking::<$event>(message))
            }
        }
    };
}
pub(crate) use blocking_error;

/// Generates `into_blocking_error` for events whose exit 2 blocks while
/// Claude Code still reads JSON stdout.
macro_rules! into_blocking_error {
    ($output:ident) => {
        impl $output {
            /// Emits this structured response on stdout while exiting 2 with
            /// `message` on stderr.
            ///
            /// Claude Code still blocks, reads the JSON fields, and uses a JSON
            /// blocking `reason` as the message when the JSON makes one. If
            /// the JSON fails validation the block stands and `message` is the
            /// reason, so this is the fail-closed form of a structured block.
            ///
            /// `message` may be empty when the JSON makes a blocking decision
            /// (a `block` decision or a `deny` permission decision); the
            /// process then exits 2 with empty stderr, which the snapshot
            /// allows. Without such a decision an empty `message` is rejected
            /// here, so the handler can still choose another response. No
            /// field can be added afterwards.
            pub fn into_blocking_error(
                self,
                message: impl Into<String>,
            ) -> hookkit_core::Result<Self> {
                self.0.into_blocking(message).map(Self)
            }
        }
    };
}
pub(crate) use into_blocking_error;

/// Generates `into_feedback_error` for events whose exit 2 only shows stderr
/// to Claude while Claude Code still reads JSON stdout.
macro_rules! into_feedback_error {
    ($output:ident) => {
        impl $output {
            /// Emits this structured response on stdout while exiting 2 with
            /// `message` on stderr.
            ///
            /// Claude Code reads JSON on every exit code, so the structured
            /// fields still apply, and it shows `message` to Claude as
            /// feedback. Nothing is blocked. `message` may be empty only when
            /// the JSON makes a `block` decision, whose `reason` is then the
            /// feedback; otherwise an empty `message` is rejected here. No
            /// field can be added afterwards.
            pub fn into_feedback_error(
                self,
                message: impl Into<String>,
            ) -> hookkit_core::Result<Self> {
                self.0.into_blocking(message).map(Self)
            }
        }
    };
}

/// Generates the exit-1 user-notice constructor.
macro_rules! nonblocking_error {
    ($event:ident, $output:ident) => {
        impl $output {
            /// Reports a non-blocking hook error: exits 1 with required stderr,
            /// which Claude Code shows the user as a `<hook> hook error`
            /// notice. The action proceeds and Claude does not see the text.
            pub fn nonblocking_error(message: impl Into<String>) -> Self {
                Self($crate::catalog::CatalogOutput::nonblocking::<$event>(
                    message,
                ))
            }
        }
    };
}
pub(crate) use nonblocking_error;

/// Generates the terminal-sequence constructor for output-discarding events.
macro_rules! terminal_sequence_constructor {
    ($event:ident, $output:ident) => {
        impl $output {
            /// Creates a response carrying only `terminalSequence`, the one
            /// output field this event honors.
            pub fn terminal_sequence(sequence: impl Into<String>) -> Self {
                Self($crate::catalog::CatalogOutput::top_level::<$event>([(
                    "terminalSequence",
                    sequence.into().into(),
                )]))
            }
        }
    };
}

/// Generates the `system_message` constructor for events that deliver it.
macro_rules! system_message_constructor {
    ($event:ident, $output:ident, $delivery:literal) => {
        impl $output {
            #[doc = concat!("Creates a response carrying a top-level `systemMessage`. ", $delivery)]
            pub fn system_message(message: impl Into<String>) -> Self {
                Self($crate::catalog::CatalogOutput::top_level::<$event>([(
                    "systemMessage",
                    message.into().into(),
                )]))
            }

            /// Creates a response carrying a top-level `systemMessage`.
            ///
            /// This associated function predates the builder convention; a
            /// later release turns `with_system_message` into a `self`
            /// builder like the other events'.
            #[deprecated(note = "renamed to `system_message`; the `with_` prefix is reserved for builders that take `self`")]
            pub fn with_system_message(message: impl Into<String>) -> Self {
                Self::system_message(message)
            }
        }
    };
}

fn watch_paths_value(
    paths: Vec<Utf8PathBuf>,
    message: &'static str,
) -> hookkit_core::Result<Value> {
    if paths.iter().any(|path| !path.is_absolute()) {
        return Err(hookkit_core::HookkitError::InvalidProcessEmission(message));
    }
    Ok(Value::Array(
        paths
            .into_iter()
            .map(|path| Value::String(path.into_string()))
            .collect(),
    ))
}

// ---------------------------------------------------------------------------
// Output-discarding events: Setup, InstructionsLoaded, Notification,
// StopFailure, SessionEnd, PostCompact.
// ---------------------------------------------------------------------------

output_type!(SetupOutput, "Setup");
event_spec!(Setup, SetupOutput, "Setup", Session, ["trigger": String]);
no_op!(Setup, SetupOutput);
discarded_builders!(Setup, SetupOutput: continue_session, stop_reason, system_message, terminal_sequence, suppress_output);

impl SetupOutput {
    /// Creates an empty response. Claude Code discards every Setup output
    /// field, including `additionalContext`, so this emits `{}`.
    #[deprecated(
        note = "Claude Code discards Setup output, including `additionalContext` (claude-code/docs-2026-09-29-r1); this now emits `{}`, use `no_op`"
    )]
    pub fn with_context(additional_context: impl Into<String>) -> Self {
        let _ = additional_context.into();
        Self::no_op()
    }
}

output_type!(InstructionsLoadedOutput, "InstructionsLoaded");
event_spec!(
    InstructionsLoaded,
    InstructionsLoadedOutput,
    "InstructionsLoaded",
    Context,
    ["file_path": String, "memory_type": String, "load_reason": String]
);
no_op!(InstructionsLoaded, InstructionsLoadedOutput);
terminal_sequence_constructor!(InstructionsLoaded, InstructionsLoadedOutput);
universal_builders!(InstructionsLoadedOutput: terminal_sequence);
discarded_builders!(InstructionsLoaded, InstructionsLoadedOutput: system_message_constructor, continue_session, stop_reason, suppress_output);

output_type!(NotificationOutput, "Notification");
event_spec!(
    Notification,
    NotificationOutput,
    "Notification",
    Other,
    ["message": String, "notification_type": String]
);
no_op!(Notification, NotificationOutput);
terminal_sequence_constructor!(Notification, NotificationOutput);
universal_builders!(NotificationOutput: terminal_sequence);
discarded_builders!(Notification, NotificationOutput: system_message_constructor, continue_session, stop_reason, suppress_output);

output_type!(StopFailureOutput, "StopFailure");
event_spec!(StopFailure, StopFailureOutput, "StopFailure", Agent, ["error": String]);
no_op!(StopFailure, StopFailureOutput);
terminal_sequence_constructor!(StopFailure, StopFailureOutput);
universal_builders!(StopFailureOutput: terminal_sequence);
discarded_builders!(StopFailure, StopFailureOutput: system_message_constructor, continue_session, stop_reason, suppress_output);

output_type!(SessionEndOutput, "SessionEnd");
event_spec!(SessionEnd, SessionEndOutput, "SessionEnd", Session, ["reason": String]);
no_op!(SessionEnd, SessionEndOutput);
terminal_sequence_constructor!(SessionEnd, SessionEndOutput);
universal_builders!(SessionEndOutput: terminal_sequence);
nonblocking_error!(SessionEnd, SessionEndOutput);
discarded_builders!(SessionEnd, SessionEndOutput: system_message_constructor, continue_session, stop_reason, suppress_output);

output_type!(PostCompactOutput, "PostCompact");
event_spec!(
    PostCompact,
    PostCompactOutput,
    "PostCompact",
    Context,
    ["trigger": String, "compact_summary": String]
);
no_op!(PostCompact, PostCompactOutput);
terminal_sequence_constructor!(PostCompact, PostCompactOutput);
universal_builders!(PostCompactOutput: terminal_sequence);
nonblocking_error!(PostCompact, PostCompactOutput);
discarded_builders!(PostCompact, PostCompactOutput: system_message_constructor, continue_session, stop_reason, suppress_output);

// ---------------------------------------------------------------------------
// Workspace events: CwdChanged, FileChanged, DirectoryAdded, WorktreeRemove.
// ---------------------------------------------------------------------------

output_type!(CwdChangedOutput, "CwdChanged");
event_spec!(
    CwdChanged,
    CwdChangedOutput,
    "CwdChanged",
    Context,
    ["old_cwd": String, "new_cwd": String]
);
no_op!(CwdChanged, CwdChangedOutput);
system_message_constructor!(
    CwdChanged,
    CwdChangedOutput,
    "Interactive sessions show it as a brief terminal notification; it does not reach the SDK message stream."
);
universal_builders!(CwdChangedOutput: terminal_sequence);
nonblocking_error!(CwdChanged, CwdChangedOutput);
discarded_builders!(CwdChanged, CwdChangedOutput: continue_session, stop_reason, suppress_output);

impl CwdChangedOutput {
    /// Replaces the dynamic watched-path list after the directory change.
    ///
    /// Every path must be absolute. An empty list clears dynamically
    /// registered paths; matcher-configured paths remain active.
    pub fn with_watch_paths(self, paths: Vec<Utf8PathBuf>) -> hookkit_core::Result<Self> {
        let paths = watch_paths_value(paths, "CwdChanged watch paths must be absolute")?;
        self.0.with_specific("watchPaths", paths).map(Self)
    }
}

output_type!(FileChangedOutput, "FileChanged");
event_spec!(
    FileChanged,
    FileChangedOutput,
    "FileChanged",
    Context,
    ["file_path": String, "event": String]
);
no_op!(FileChanged, FileChangedOutput);
system_message_constructor!(
    FileChanged,
    FileChangedOutput,
    "Interactive sessions show it as a brief terminal notification; it does not reach the SDK message stream."
);
universal_builders!(FileChangedOutput: terminal_sequence);
nonblocking_error!(FileChanged, FileChangedOutput);
discarded_builders!(FileChanged, FileChangedOutput: continue_session, stop_reason, suppress_output);

impl FileChangedOutput {
    /// Replaces the dynamic watched-path list after the file change.
    ///
    /// Every path must be absolute. An empty list clears dynamically
    /// registered paths; matcher-configured paths remain active.
    pub fn with_watch_paths(self, paths: Vec<Utf8PathBuf>) -> hookkit_core::Result<Self> {
        let paths = watch_paths_value(paths, "FileChanged watch paths must be absolute")?;
        self.0.with_specific("watchPaths", paths).map(Self)
    }
}

output_type!(DirectoryAddedOutput, "DirectoryAdded");
event_spec!(
    DirectoryAdded,
    DirectoryAddedOutput,
    "DirectoryAdded",
    Context,
    ["directory": String, "source": String]
);
no_op!(DirectoryAdded, DirectoryAddedOutput);
system_message_constructor!(
    DirectoryAdded,
    DirectoryAddedOutput,
    "Claude Code delivers it to Claude as context for `slash_command` additions and writes it only to the debug log for `register_repo_root`."
);
universal_builders!(DirectoryAddedOutput: terminal_sequence);
discarded_builders!(DirectoryAdded, DirectoryAddedOutput: continue_session, stop_reason, suppress_output);

output_type!(WorktreeRemoveOutput, "WorktreeRemove");
event_spec!(
    WorktreeRemove,
    WorktreeRemoveOutput,
    "WorktreeRemove",
    Worktree,
    ["worktree_path": String]
);
discarded_builders!(WorktreeRemove, WorktreeRemoveOutput: system_message_constructor, continue_session, stop_reason, terminal_sequence, suppress_output);

impl WorktreeRemoveOutput {
    /// Reports that the worktree was removed: exits 0 with empty stdout.
    ///
    /// Claude Code reads only the exit code and nothing else from the hook,
    /// so the hook itself must have deleted the directory.
    pub fn removed() -> Self {
        Self(CatalogOutput::empty::<WorktreeRemove>())
    }

    /// Reports a failed removal: exits with nonzero `exit_code` and writes
    /// `message`, which may be empty, to stderr.
    ///
    /// Claude Code fails the removal when `worktree_path` still exists
    /// afterwards, leaving the worktree on disk with no git fallback, and
    /// writes the stderr to its debug log. Any nonzero exit, including the
    /// HookKit runtime's exit-1 error path, has this effect.
    pub fn failed(message: impl Into<String>, exit_code: u8) -> hookkit_core::Result<Self> {
        CatalogOutput::failure::<WorktreeRemove>(message, exit_code).map(Self)
    }

    /// Creates an exit-0 response with `{}` on stdout, which Claude Code
    /// ignores; the worktree counts as removed.
    #[deprecated(
        note = "WorktreeRemove has no JSON output (claude-code/docs-2026-09-29-r1); use `removed`"
    )]
    pub fn no_op() -> Self {
        Self(CatalogOutput::empty_object::<WorktreeRemove>())
    }
}

// ---------------------------------------------------------------------------
// Context events: SubagentStart, MessageDisplay, PermissionDenied.
// ---------------------------------------------------------------------------

output_type!(SubagentStartOutput, "SubagentStart");
event_spec!(
    SubagentStart,
    SubagentStartOutput,
    "SubagentStart",
    Agent,
    ["agent_id": String, "agent_type": String]
);
no_op!(SubagentStart, SubagentStartOutput);
context_builders!(
    SubagentStart,
    SubagentStartOutput,
    "at the start of the subagent's conversation"
);
nonblocking_error!(SubagentStart, SubagentStartOutput);
universal_builders!(SubagentStartOutput: continue_session, stop_reason, system_message, terminal_sequence);
discarded_builders!(SubagentStart, SubagentStartOutput: suppress_output);

output_type!(MessageDisplayOutput, "MessageDisplay");
event_spec!(
    MessageDisplay,
    MessageDisplayOutput,
    "MessageDisplay",
    Other,
    [
        "turn_id": String,
        "message_id": String,
        "index": Any,
        "final": Bool,
        "delta": String,
    ]
);
no_op!(MessageDisplay, MessageDisplayOutput);
universal_builders!(MessageDisplayOutput: terminal_sequence);
discarded_builders!(MessageDisplay, MessageDisplayOutput: continue_session, stop_reason, system_message, suppress_output);

impl MessageDisplayOutput {
    /// Replaces the text rendered on screen for this batch of the streamed
    /// message. The transcript and what Claude sees keep the original text.
    pub fn display(content: impl Into<String>) -> Self {
        Self(CatalogOutput::hook_specific::<MessageDisplay>([(
            "displayContent",
            content.into().into(),
        )]))
    }
}

output_type!(PermissionDeniedOutput, "PermissionDenied");
event_spec!(
    PermissionDenied,
    PermissionDeniedOutput,
    "PermissionDenied",
    Tool,
    [
        "tool_name": String,
        "tool_input": Any,
        "tool_use_id": String,
        "reason": String,
    ]
);
no_op!(PermissionDenied, PermissionDeniedOutput);
universal_builders!(PermissionDeniedOutput: continue_session, stop_reason, system_message, terminal_sequence);
discarded_builders!(PermissionDenied, PermissionDeniedOutput: suppress_output);

impl PermissionDeniedOutput {
    /// Chooses whether to tell the model it may retry the denied call.
    /// Claude Code ignores `retry: true` for no-verdict denials.
    pub fn retry(retry: bool) -> Self {
        Self(CatalogOutput::hook_specific::<PermissionDenied>([(
            "retry",
            retry.into(),
        )]))
    }
}

// ---------------------------------------------------------------------------
// Blocking decision events.
// ---------------------------------------------------------------------------

output_type!(ConfigChangeOutput, "ConfigChange");
event_spec!(ConfigChange, ConfigChangeOutput, "ConfigChange", Context, ["source": String]);
no_op!(ConfigChange, ConfigChangeOutput);
block_builders!(
    ConfigChange,
    ConfigChangeOutput,
    "The change does not take effect (except `policy_settings`); Claude Code never shows `reason`."
);
blocking_error!(
    ConfigChange,
    ConfigChangeOutput,
    "Blocks the change (except `policy_settings`); stderr goes only to the debug log."
);
into_blocking_error!(ConfigChangeOutput);
nonblocking_error!(ConfigChange, ConfigChangeOutput);
universal_builders!(ConfigChangeOutput: terminal_sequence);
discarded_builders!(ConfigChange, ConfigChangeOutput: continue_session, stop_reason, system_message, suppress_output);

output_type!(PreCompactOutput, "PreCompact");
event_spec!(
    PreCompact,
    PreCompactOutput,
    "PreCompact",
    Context,
    ["trigger": String, "custom_instructions": NullableString]
);
no_op!(PreCompact, PreCompactOutput);
block_builders!(
    PreCompact,
    PreCompactOutput,
    "Compaction is skipped or, when recovering from a context-limit error, the request fails."
);
blocking_error!(
    PreCompact,
    PreCompactOutput,
    "Blocks compaction; for a manual `/compact` the stderr is shown to the user."
);
into_blocking_error!(PreCompactOutput);
nonblocking_error!(PreCompact, PreCompactOutput);
universal_builders!(PreCompactOutput: terminal_sequence);
discarded_builders!(PreCompact, PreCompactOutput: continue_session, stop_reason, system_message, suppress_output);

output_type!(PostToolBatchOutput, "PostToolBatch");
event_spec!(PostToolBatch, PostToolBatchOutput, "PostToolBatch", Tool, ["tool_calls": Array]);
no_op!(PostToolBatch, PostToolBatchOutput);
context_builders!(
    PostToolBatch,
    PostToolBatchOutput,
    "next to the batch's tool results"
);
block_builders!(
    PostToolBatch,
    PostToolBatchOutput,
    "Stops the agentic loop before the next model call; the reason is shown as a transcript warning that Claude sees if the conversation continues."
);
block_with_context!(PostToolBatch, PostToolBatchOutput);
blocking_error!(
    PostToolBatch,
    PostToolBatchOutput,
    "Stops the agentic loop; stderr is shown to the user and stays in the conversation."
);
into_blocking_error!(PostToolBatchOutput);
nonblocking_error!(PostToolBatch, PostToolBatchOutput);
universal_builders!(PostToolBatchOutput: continue_session, stop_reason, system_message, terminal_sequence);
discarded_builders!(PostToolBatch, PostToolBatchOutput: suppress_output);

output_type!(PostToolUseFailureOutput, "PostToolUseFailure");
event_spec!(
    PostToolUseFailure,
    PostToolUseFailureOutput,
    "PostToolUseFailure",
    Tool,
    [
        "tool_name": String,
        "tool_input": Any,
        "tool_use_id": String,
        "error": String,
    ]
);
no_op!(PostToolUseFailure, PostToolUseFailureOutput);
context_builders!(
    PostToolUseFailure,
    PostToolUseFailureOutput,
    "next to the failed tool result"
);
block_builders!(
    PostToolUseFailure,
    PostToolUseFailureOutput,
    "The tool already failed, so this adds `reason` as feedback for Claude."
);
block_with_context!(PostToolUseFailure, PostToolUseFailureOutput);
nonblocking_error!(PostToolUseFailure, PostToolUseFailureOutput);
universal_builders!(PostToolUseFailureOutput: continue_session, stop_reason, system_message, terminal_sequence);
discarded_builders!(PostToolUseFailure, PostToolUseFailureOutput: suppress_output);

into_feedback_error!(PostToolUseFailureOutput);

impl PostToolUseFailureOutput {
    /// Creates a code-2 feedback response with required stderr text.
    ///
    /// The tool has already failed, so Claude Code shows the feedback to
    /// Claude without blocking or rolling back an action.
    pub fn feedback_error(message: impl Into<String>) -> Self {
        Self(CatalogOutput::blocking::<PostToolUseFailure>(message))
    }
}

output_type!(StopOutput, "Stop");
event_spec!(Stop, StopOutput, "Stop", Agent, ["stop_hook_active": Bool]);
no_op!(Stop, StopOutput);
context_builders!(
    Stop,
    StopOutput,
    "at the end of the turn as non-error feedback.\n\nLike [`Self::block`], this keeps the conversation going so Claude can act on the feedback, bounded by the same loop protections (`stop_hook_active` and Claude Code's consecutive-continuation cap); the transcript labels it `Stop hook feedback` instead of a hook error. A hook that only wants to inform should not return it on every stop: use `no_op().with_system_message(..)` for a user notice, or check [`CatalogInput::stop_hook_active`] first"
);
block_builders!(
    Stop,
    StopOutput,
    "Prevents Claude from stopping; `reason` tells Claude why the conversation continues."
);
block_with_context!(Stop, StopOutput);
blocking_error!(
    Stop,
    StopOutput,
    "Prevents Claude from stopping; stderr is shown to Claude."
);
into_blocking_error!(StopOutput);
nonblocking_error!(Stop, StopOutput);
universal_builders!(StopOutput: continue_session, stop_reason, system_message, terminal_sequence);
discarded_builders!(Stop, StopOutput: suppress_output);

output_type!(SubagentStopOutput, "SubagentStop");
event_spec!(
    SubagentStop,
    SubagentStopOutput,
    "SubagentStop",
    Agent,
    [
        "stop_hook_active": Bool,
        "agent_id": String,
        "agent_type": String,
        "agent_transcript_path": String,
    ]
);
no_op!(SubagentStop, SubagentStopOutput);
context_builders!(
    SubagentStop,
    SubagentStopOutput,
    "at the end of the subagent's turn as non-error feedback.\n\nLike [`Self::block`], this keeps the subagent running so it can act on the feedback, bounded by the same loop protections (`stop_hook_active` and Claude Code's consecutive-continuation cap); it is shown as hook feedback instead of a hook error. A hook that only wants to inform should not return it on every stop: use `no_op().with_system_message(..)` for a user notice, or check [`CatalogInput::stop_hook_active`] first"
);
block_builders!(
    SubagentStop,
    SubagentStopOutput,
    "Prevents the subagent from stopping; `reason` tells it why."
);
block_with_context!(SubagentStop, SubagentStopOutput);
blocking_error!(
    SubagentStop,
    SubagentStopOutput,
    "Prevents the subagent from stopping; stderr is shown to the subagent."
);
into_blocking_error!(SubagentStopOutput);
nonblocking_error!(SubagentStop, SubagentStopOutput);
universal_builders!(SubagentStopOutput: continue_session, stop_reason, system_message, terminal_sequence);
discarded_builders!(SubagentStop, SubagentStopOutput: suppress_output);

output_type!(TaskCreatedOutput, "TaskCreated");
event_spec!(
    TaskCreated,
    TaskCreatedOutput,
    "TaskCreated",
    Agent,
    ["task_id": String, "task_subject": String]
);
no_op!(TaskCreated, TaskCreatedOutput);
block_builders!(
    TaskCreated,
    TaskCreatedOutput,
    "Deletes the task and returns `reason` to Claude as the TaskCreate tool error."
);
blocking_error!(
    TaskCreated,
    TaskCreatedOutput,
    "Rolls back the task creation; stderr is returned to Claude."
);
into_blocking_error!(TaskCreatedOutput);
nonblocking_error!(TaskCreated, TaskCreatedOutput);
universal_builders!(TaskCreatedOutput: system_message, terminal_sequence);
discarded_builders!(TaskCreated, TaskCreatedOutput: continue_session, stop_reason, suppress_output);

output_type!(TaskCompletedOutput, "TaskCompleted");
event_spec!(
    TaskCompleted,
    TaskCompletedOutput,
    "TaskCompleted",
    Agent,
    ["task_id": String, "task_subject": String]
);
no_op!(TaskCompleted, TaskCompletedOutput);
blocking_error!(
    TaskCompleted,
    TaskCompletedOutput,
    "Prevents the task from being marked completed; stderr is shown to Claude."
);
into_blocking_error!(TaskCompletedOutput);
nonblocking_error!(TaskCompleted, TaskCompletedOutput);
universal_builders!(TaskCompletedOutput: continue_session, stop_reason, system_message, terminal_sequence);
discarded_builders!(TaskCompleted, TaskCompletedOutput: suppress_output);

output_type!(TeammateIdleOutput, "TeammateIdle");
// `team_name` is not required: Claude Code deprecated it and announced its
// removal, and a parse failure would let the teammate go idle (fail open).
// The field is still retained in `CatalogInput::fields` when sent.
event_spec!(
    TeammateIdle,
    TeammateIdleOutput,
    "TeammateIdle",
    Agent,
    ["teammate_name": String]
);
no_op!(TeammateIdle, TeammateIdleOutput);
blocking_error!(
    TeammateIdle,
    TeammateIdleOutput,
    "Prevents the teammate from going idle; stderr is shown to the teammate."
);
into_blocking_error!(TeammateIdleOutput);
nonblocking_error!(TeammateIdle, TeammateIdleOutput);
universal_builders!(TeammateIdleOutput: continue_session, stop_reason, system_message, terminal_sequence);
discarded_builders!(TeammateIdle, TeammateIdleOutput: suppress_output);

// ---------------------------------------------------------------------------
// Elicitation events.
// ---------------------------------------------------------------------------

macro_rules! elicitation_actions {
    ($event:ident, $output:ident, $subject:literal) => {
        impl $output {
            #[doc = concat!(
                                "Accepts ", $subject, " with form field values in `content`."
                            )]
            pub fn accept(content: Map<String, Value>) -> Self {
                Self($crate::catalog::CatalogOutput::hook_specific::<$event>([
                    ("action", "accept".into()),
                    ("content", content.into()),
                ]))
            }

            #[doc = concat!(
                                "Accepts ", $subject, " without `content`, as for a URL-mode ",
                                "elicitation, which has no form fields."
                            )]
            pub fn accept_without_content() -> Self {
                Self($crate::catalog::CatalogOutput::hook_specific::<$event>([(
                    "action",
                    "accept".into(),
                )]))
            }

            #[doc = concat!("Declines ", $subject, ".")]
            pub fn decline() -> Self {
                Self($crate::catalog::CatalogOutput::hook_specific::<$event>([(
                    "action",
                    "decline".into(),
                )]))
            }

            #[doc = concat!("Cancels ", $subject, ".")]
            pub fn cancel() -> Self {
                Self($crate::catalog::CatalogOutput::hook_specific::<$event>([(
                    "action",
                    "cancel".into(),
                )]))
            }
        }
    };
}

output_type!(ElicitationOutput, "Elicitation");
event_spec!(
    Elicitation,
    ElicitationOutput,
    "Elicitation",
    Other,
    ["mcp_server_name": String, "message": String]
);
no_op!(Elicitation, ElicitationOutput);
elicitation_actions!(Elicitation, ElicitationOutput, "the elicitation");
block_builders!(
    Elicitation,
    ElicitationOutput,
    "Declines the elicitation (Claude Code v2.1.284 changelog; the hooks reference does not yet document it, and where `reason` is shown is unknown)."
);
blocking_error!(
    Elicitation,
    ElicitationOutput,
    "Denies the elicitation; stderr is shown nowhere."
);
nonblocking_error!(Elicitation, ElicitationOutput);
universal_builders!(ElicitationOutput: terminal_sequence);
discarded_builders!(Elicitation, ElicitationOutput: continue_session, stop_reason, system_message, suppress_output);

output_type!(ElicitationResultOutput, "ElicitationResult");
event_spec!(
    ElicitationResult,
    ElicitationResultOutput,
    "ElicitationResult",
    Other,
    ["mcp_server_name": String, "action": String]
);
no_op!(ElicitationResult, ElicitationResultOutput);
elicitation_actions!(
    ElicitationResult,
    ElicitationResultOutput,
    "the response in place of the user's result"
);
block_builders!(
    ElicitationResult,
    ElicitationResultOutput,
    "Declines the response (Claude Code v2.1.284 changelog; the hooks reference does not yet document it, and where `reason` is shown is unknown)."
);
blocking_error!(
    ElicitationResult,
    ElicitationResultOutput,
    "Changes the effective action to decline; stderr is shown nowhere."
);
nonblocking_error!(ElicitationResult, ElicitationResultOutput);
universal_builders!(ElicitationResultOutput: terminal_sequence);
discarded_builders!(ElicitationResult, ElicitationResultOutput: continue_session, stop_reason, system_message, suppress_output);

// ---------------------------------------------------------------------------
// Permission events.
// ---------------------------------------------------------------------------

/// Behavior returned for a Claude permission request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum PermissionRequestBehavior {
    /// Allow the requested operation.
    Allow,
    /// Deny the requested operation.
    Deny,
}

impl PermissionRequestBehavior {
    /// Returns the native wire spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }
}

output_type!(PermissionRequestOutput, "PermissionRequest");
event_spec!(
    PermissionRequest,
    PermissionRequestOutput,
    "PermissionRequest",
    Tool,
    ["tool_name": String, "tool_input": Any]
);
no_op!(PermissionRequest, PermissionRequestOutput);
nonblocking_error!(PermissionRequest, PermissionRequestOutput);
universal_builders!(PermissionRequestOutput: continue_session, stop_reason, system_message, terminal_sequence);
discarded_builders!(PermissionRequest, PermissionRequestOutput: suppress_output);

impl PermissionRequestOutput {
    fn with_decision(
        behavior: PermissionRequestBehavior,
        fields: impl IntoIterator<Item = (&'static str, Value)>,
    ) -> Map<String, Value> {
        let mut decision = Map::new();
        decision.insert("behavior".into(), behavior.as_str().into());
        decision.extend(
            fields
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value)),
        );
        decision
    }

    fn from_decision(decision: Map<String, Value>) -> Self {
        Self(CatalogOutput::hook_specific::<PermissionRequest>([(
            "decision",
            decision.into(),
        )]))
    }

    /// Grants the permission. Deny and ask rules are still evaluated, so
    /// this does not override a matching deny rule.
    ///
    /// Refine it with [`Self::with_updated_input`] and
    /// [`Self::with_updated_permissions`].
    pub fn allow() -> Self {
        Self::from_decision(Self::with_decision(PermissionRequestBehavior::Allow, []))
    }

    /// Denies the permission; `message` tells Claude why.
    ///
    /// Claude Code applies a decision object on every exit code, so this is
    /// the only way to deny: exit 2 alone leaves the permission flow
    /// unchanged. Refine it with [`Self::with_interrupt`].
    pub fn deny(message: impl Into<String>) -> Self {
        Self::from_decision(Self::with_decision(
            PermissionRequestBehavior::Deny,
            [("message", message.into().into())],
        ))
    }

    /// Creates a permission decision with optional associated fields.
    ///
    /// `message` and `interrupt` are emitted only when supplied, whatever the
    /// behavior.
    #[deprecated(
        note = "use `allow()` or `deny(message)` with `with_interrupt`; Claude Code ignores `message` and `interrupt` on allow"
    )]
    pub fn decide(
        behavior: PermissionRequestBehavior,
        message: Option<String>,
        interrupt: Option<bool>,
    ) -> Self {
        let message = message.map(|message| ("message", Value::from(message)));
        let interrupt = interrupt.map(|interrupt| ("interrupt", Value::from(interrupt)));
        Self::from_decision(Self::with_decision(
            behavior,
            message.into_iter().chain(interrupt),
        ))
    }

    /// Denies the permission with `message` while exiting 2 with `message`
    /// on stderr.
    ///
    /// Claude Code no longer honors exit 2 for this event: without a
    /// decision object the permission flow proceeds unchanged and stderr is
    /// discarded. This shim therefore also prints the equivalent
    /// [`Self::deny`] decision, which Claude Code applies on every exit code,
    /// so the request is denied on releases with either behavior. An empty
    /// `message` still denies: stderr is then empty, which the snapshot
    /// allows next to a decision.
    #[deprecated(
        note = "Claude Code ignores exit 2 on PermissionRequest and discards its stderr (claude-code/docs-2026-09-29-r1); use `deny(message)`"
    )]
    pub fn blocking_error(message: impl Into<String>) -> Self {
        let message = message.into();
        let decision = Self::with_decision(
            PermissionRequestBehavior::Deny,
            [("message", message.clone().into())],
        );
        Self(
            CatalogOutput::hook_specific_with_stderr::<PermissionRequest>(
                [("decision", decision.into())],
                message,
            ),
        )
    }

    fn behavior(&self) -> Option<&str> {
        self.0
            .specific_field("decision")
            .and_then(|decision| decision.get("behavior"))
            .and_then(Value::as_str)
    }

    fn with_decision_field(
        mut self,
        name: &'static str,
        value: Value,
    ) -> hookkit_core::Result<Self> {
        self.0
            .specific_mut()?
            .get_mut("decision")
            .and_then(Value::as_object_mut)
            .ok_or(hookkit_core::HookkitError::InvalidProcessEmission(
                "permission decision fields require an existing decision",
            ))?
            .insert(name.into(), value);
        Ok(self)
    }

    fn require_behavior(
        &self,
        expected: PermissionRequestBehavior,
        message: &'static str,
    ) -> hookkit_core::Result<()> {
        if self.behavior() == Some(expected.as_str()) {
            Ok(())
        } else {
            Err(hookkit_core::HookkitError::InvalidProcessEmission(message))
        }
    }

    /// Replaces the tool input on an `allow` decision. The object replaces
    /// the whole input and is re-evaluated against deny and ask rules.
    pub fn with_updated_input(self, input: Map<String, Value>) -> hookkit_core::Result<Self> {
        self.require_behavior(
            PermissionRequestBehavior::Allow,
            "updatedInput applies only to an allow decision",
        )?;
        self.with_decision_field("updatedInput", input.into())
    }

    /// Applies permission update entries on an `allow` decision.
    pub fn with_updated_permissions(self, permissions: Vec<Value>) -> hookkit_core::Result<Self> {
        self.require_behavior(
            PermissionRequestBehavior::Allow,
            "updatedPermissions applies only to an allow decision",
        )?;
        self.with_decision_field("updatedPermissions", permissions.into())
    }

    /// Sets whether a `deny` decision also stops Claude.
    pub fn with_interrupt(self, interrupt: bool) -> hookkit_core::Result<Self> {
        self.require_behavior(
            PermissionRequestBehavior::Deny,
            "interrupt applies only to a deny decision",
        )?;
        self.with_decision_field("interrupt", interrupt.into())
    }
}

/// Permission decision returned by a Claude pre-tool hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum PreToolPermissionDecision {
    /// Skip the permission prompt. Claude Code auto-approves the call,
    /// except for actions no mode auto-approves.
    Allow,
    /// Deny the pending tool call.
    Deny,
    /// Ask the user for permission.
    Ask,
    /// Pause a non-interactive tool call so an integration can resume it later.
    Defer,
}

impl PreToolPermissionDecision {
    /// Returns the native wire spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Ask => "ask",
            Self::Defer => "defer",
        }
    }
}

output_type!(PreToolUseOutput, "PreToolUse");
event_spec!(
    PreToolUse,
    PreToolUseOutput,
    "PreToolUse",
    Tool,
    ["tool_name": String, "tool_input": Any, "tool_use_id": String]
);
no_op!(PreToolUse, PreToolUseOutput);
blocking_error!(
    PreToolUse,
    PreToolUseOutput,
    "Blocks the tool call; stderr is shown to Claude. No JSON decision can override it."
);
into_blocking_error!(PreToolUseOutput);
nonblocking_error!(PreToolUse, PreToolUseOutput);
universal_builders!(PreToolUseOutput: continue_session, stop_reason, system_message, terminal_sequence);
discarded_builders!(PreToolUse, PreToolUseOutput: suppress_output);

impl PreToolUseOutput {
    fn with_decision(decision: PreToolPermissionDecision, reason: Option<String>) -> Self {
        let reason = reason.map(|reason| ("permissionDecisionReason", Value::from(reason)));
        Self(CatalogOutput::hook_specific::<PreToolUse>(
            std::iter::once(("permissionDecision", decision.as_str().into())).chain(reason),
        ))
    }

    /// Adds `additionalContext` for Claude without making a permission
    /// decision, so the normal permission flow applies.
    ///
    /// Prefer this over [`Self::allow`] for context-only hooks: `allow`
    /// skips the permission prompt and auto-approves the call. To rewrite the
    /// call without auto-approving it, use [`Self::ask`] with
    /// [`Self::with_updated_input`]; see that builder for why a rewrite
    /// without a decision is not documented.
    pub fn with_context(additional_context: impl Into<String>) -> Self {
        Self(CatalogOutput::hook_specific::<PreToolUse>([(
            "additionalContext",
            additional_context.into().into(),
        )]))
    }

    /// Skips the permission prompt and auto-approves the call, except for
    /// actions no permission mode auto-approves and for `AskUserQuestion` and
    /// `ExitPlanMode`, which also need [`Self::with_updated_input`]. Deny and
    /// ask rules are still evaluated.
    ///
    /// A hook that only wants to add context should not choose `allow`; use
    /// [`Self::with_context`] or [`Self::no_op`] instead. A rewrite that
    /// should not auto-approve belongs on [`Self::ask`].
    pub fn allow() -> Self {
        Self::with_decision(PreToolPermissionDecision::Allow, None)
    }

    /// Denies the tool call; `reason` is shown to Claude.
    pub fn deny(reason: impl Into<String>) -> Self {
        Self::with_decision(PreToolPermissionDecision::Deny, Some(reason.into()))
    }

    /// Asks the user to confirm the call; `reason` is shown to the user but
    /// not to Claude. In auto mode this still forces a permission prompt.
    pub fn ask(reason: impl Into<String>) -> Self {
        Self::with_decision(PreToolPermissionDecision::Ask, Some(reason.into()))
    }

    /// Pauses a non-interactive tool call so an integration can resume it
    /// later. Claude Code ignores `updatedInput` and `additionalContext` with
    /// this decision, so the corresponding builders reject it.
    pub fn defer() -> Self {
        Self::with_decision(PreToolPermissionDecision::Defer, None)
    }

    /// Creates a pre-tool permission decision with optional associated fields.
    ///
    /// No relationship between the decision and the optional fields is
    /// enforced.
    #[deprecated(
        note = "use `allow`, `deny`, `ask`, `defer`, or `with_context`, then `with_updated_input` and `with_additional_context`"
    )]
    pub fn decide(
        decision: PreToolPermissionDecision,
        reason: Option<String>,
        updated_input: Option<Map<String, Value>>,
        additional_context: Option<String>,
    ) -> Self {
        let fields = [
            Some(("permissionDecision", Value::from(decision.as_str()))),
            reason.map(|reason| ("permissionDecisionReason", Value::from(reason))),
            updated_input.map(|input| ("updatedInput", Value::from(input))),
            additional_context.map(|context| ("additionalContext", Value::from(context))),
        ];
        Self(CatalogOutput::hook_specific::<PreToolUse>(
            fields.into_iter().flatten(),
        ))
    }

    fn decision(&self) -> Option<&str> {
        self.0
            .specific_field("permissionDecision")
            .and_then(Value::as_str)
    }

    /// Sets `permissionDecisionReason` on an existing decision. For `allow`
    /// and `defer` Claude Code writes it only to the debug log.
    pub fn with_decision_reason(self, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        if self.decision().is_none() {
            return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "permissionDecisionReason requires a permission decision",
            ));
        }
        self.0
            .with_specific("permissionDecisionReason", reason.into().into())
            .map(Self)
    }

    /// Replaces the whole tool input before execution. Permission rules are
    /// evaluated against the returned input. Rejected for `deny` and `defer`,
    /// where Claude Code ignores it.
    ///
    /// The pinned reference documents `updatedInput` only with a decision:
    /// combine it with [`Self::allow`] to auto-approve the rewritten call, or
    /// with [`Self::ask`] to show it to the user, which is the documented way
    /// to rewrite without auto-approving. HookKit still emits it on a
    /// response without a decision, but the reference does not say whether
    /// Claude Code applies the rewrite and keeps its normal permission flow
    /// there or drops it and runs the original input, so do not rely on that
    /// form for a sanitizing rewrite.
    pub fn with_updated_input(self, input: Map<String, Value>) -> hookkit_core::Result<Self> {
        if matches!(self.decision(), Some("deny" | "defer")) {
            return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "updatedInput is ignored for deny and defer decisions",
            ));
        }
        self.0.with_specific("updatedInput", input.into()).map(Self)
    }

    /// Adds `additionalContext` for Claude next to the tool result. Rejected
    /// for `defer`, where Claude Code ignores it.
    pub fn with_additional_context(
        self,
        additional_context: impl Into<String>,
    ) -> hookkit_core::Result<Self> {
        if self.decision() == Some("defer") {
            return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "additionalContext is ignored for a defer decision",
            ));
        }
        self.0
            .with_specific("additionalContext", additional_context.into().into())
            .map(Self)
    }
}

// ---------------------------------------------------------------------------
// Prompt events.
// ---------------------------------------------------------------------------

output_type!(UserPromptExpansionOutput, "UserPromptExpansion");
// `command_source` is not required: the Agent SDK types it optional
// (0.3.223 and 0.3.285), and a parse failure would let a blocking hook's
// expansion through (fail open). It is still retained when sent.
event_spec!(
    UserPromptExpansion,
    UserPromptExpansionOutput,
    "UserPromptExpansion",
    Prompt,
    [
        "expansion_type": String,
        "command_name": String,
        "command_args": String,
        "prompt": String,
    ]
);
no_op!(UserPromptExpansion, UserPromptExpansionOutput);
context_builders!(
    UserPromptExpansion,
    UserPromptExpansionOutput,
    "alongside the expanded prompt"
);
text_context!(UserPromptExpansion, UserPromptExpansionOutput);
block_builders!(
    UserPromptExpansion,
    UserPromptExpansionOutput,
    "Prevents the command from expanding; `reason` is shown to the user."
);
block_with_context!(UserPromptExpansion, UserPromptExpansionOutput);
blocking_error!(
    UserPromptExpansion,
    UserPromptExpansionOutput,
    "Blocks the expansion; stderr is shown to the user."
);
into_blocking_error!(UserPromptExpansionOutput);
nonblocking_error!(UserPromptExpansion, UserPromptExpansionOutput);
universal_builders!(UserPromptExpansionOutput: continue_session, stop_reason, system_message, terminal_sequence);
discarded_builders!(UserPromptExpansion, UserPromptExpansionOutput: suppress_output);

impl UserPromptExpansionOutput {
    /// Sets `hookSpecificOutput.suppressOriginalPrompt`, which omits the
    /// original prompt from the block message when the expansion is blocked.
    ///
    /// Agent SDK 0.3.285 types this field for `UserPromptExpansion` ("When
    /// decision is \"block\", omit the original prompt from the block
    /// message"). The pinned hooks reference documents it only for
    /// `UserPromptSubmit`, and the claude-code/docs-2026-09-29-r1 output
    /// schema does not list it for this event, so this is the one builder
    /// whose JSON that schema rejects, and whether a given Claude Code
    /// release honors the field here is unverified.
    pub fn with_suppress_original_prompt(self, suppress: bool) -> hookkit_core::Result<Self> {
        self.0
            .with_specific("suppressOriginalPrompt", suppress.into())
            .map(Self)
    }
}

output_type!(UserPromptSubmitOutput, "UserPromptSubmit");
event_spec!(UserPromptSubmit, UserPromptSubmitOutput, "UserPromptSubmit", Prompt, ["prompt": String]);
no_op!(UserPromptSubmit, UserPromptSubmitOutput);
context_builders!(
    UserPromptSubmit,
    UserPromptSubmitOutput,
    "alongside the submitted prompt"
);
text_context!(UserPromptSubmit, UserPromptSubmitOutput);
block_builders!(
    UserPromptSubmit,
    UserPromptSubmitOutput,
    "Prevents the prompt from being processed and erases it; `reason` is shown to the user and not added to context."
);
blocking_error!(
    UserPromptSubmit,
    UserPromptSubmitOutput,
    "Blocks and erases the prompt; stderr is shown to the user and not added to Claude's context."
);
into_blocking_error!(UserPromptSubmitOutput);
nonblocking_error!(UserPromptSubmit, UserPromptSubmitOutput);
universal_builders!(UserPromptSubmitOutput: continue_session, stop_reason, system_message, terminal_sequence);
discarded_builders!(UserPromptSubmit, UserPromptSubmitOutput: suppress_output);

impl UserPromptSubmitOutput {
    /// Sets `hookSpecificOutput.sessionTitle`, with the same effect as
    /// `/rename`. It does not block the prompt.
    pub fn with_session_title(self, title: impl Into<String>) -> hookkit_core::Result<Self> {
        self.0
            .with_specific("sessionTitle", title.into().into())
            .map(Self)
    }

    /// Sets `suppressOriginalPrompt`, which omits the original prompt text
    /// from the block message shown to the user. The pinned reference
    /// describes it only together with `decision: "block"`; the 2026-09-30
    /// reference adds that it also applies when the hook blocks by exiting 2
    /// with this JSON on stdout (see `into_blocking_error`). The prompt text
    /// can still reach local files such as the transcript.
    pub fn with_suppress_original_prompt(self, suppress: bool) -> hookkit_core::Result<Self> {
        self.0
            .with_specific("suppressOriginalPrompt", suppress.into())
            .map(Self)
    }

    /// Blocks the submitted prompt while appending context and optional
    /// prompt presentation controls.
    #[deprecated(
        note = "use `block(reason)` with `with_additional_context`, `with_session_title`, and `with_suppress_original_prompt`"
    )]
    pub fn block_with_context(
        reason: impl Into<String>,
        additional_context: impl Into<String>,
        session_title: Option<String>,
        suppress_original_prompt: Option<bool>,
    ) -> Self {
        let fields = [
            Some(("additionalContext", Value::from(additional_context.into()))),
            session_title.map(|title| ("sessionTitle", Value::from(title))),
            suppress_original_prompt
                .map(|suppress| ("suppressOriginalPrompt", Value::from(suppress))),
        ];
        let mut output =
            CatalogOutput::hook_specific::<UserPromptSubmit>(fields.into_iter().flatten());
        output.insert_block(reason.into());
        Self(output)
    }
}

/// Returns every native command implementation defined in this catalog module.
///
/// Each descriptor names the snapshot process fixtures a typed constructor
/// reproduces exactly: the exit code and stderr byte for byte, and stdout
/// byte for byte or, for JSON, as an equal value with the same framing.
/// Fixtures that show malformed or discouraged output (invalid JSON, JSON
/// with a failing exit other than 2, exit 2 with invalid stdout, output that
/// Claude Code ignores) are not declared.
pub fn events() -> Vec<hookkit_core::NativeEventDescriptor> {
    use hookkit_core::NativeEventDescriptor as D;
    const EXIT_2: &[&str] = &[
        "command-structured",
        "command-exit-2",
        "command-exit-2-structured",
        "command-nonzero-unstructured",
    ];
    const TEXT_CONTEXT: &[&str] = &[
        "command-structured",
        "command-text",
        "command-text-open-brace",
        "command-exit-2",
        "command-exit-2-structured",
        "command-nonzero-unstructured",
    ];
    const WATCH: &[&str] = &["command-structured", "command-nonzero-unstructured"];
    const ELICITATION: &[&str] = &[
        "command-structured",
        "command-decision-block",
        "command-exit-2",
        "command-nonzero-unstructured",
    ];
    const DISCARDED_WITH_NOTICE: &[&str] = &["command-structured", "command-nonzero"];
    vec![
        D::command::<ConfigChange>(EXIT_2),
        D::command::<CwdChanged>(WATCH),
        D::command::<DirectoryAdded>(&["command-structured"]),
        D::command::<Elicitation>(ELICITATION),
        D::command::<ElicitationResult>(ELICITATION),
        D::command::<FileChanged>(WATCH),
        D::command::<InstructionsLoaded>(&["command-structured"]),
        D::command::<MessageDisplay>(&["command-structured"]),
        D::command::<Notification>(&["command-structured"]),
        D::command::<PermissionDenied>(&["command-structured"]),
        D::command::<PermissionRequest>(&["command-structured", "command-nonzero-unstructured"]),
        D::command::<PostCompact>(DISCARDED_WITH_NOTICE),
        D::command::<PostToolBatch>(EXIT_2),
        D::command::<PostToolUseFailure>(EXIT_2),
        D::command::<PreCompact>(EXIT_2),
        D::command::<PreToolUse>(EXIT_2),
        D::command::<SessionEnd>(DISCARDED_WITH_NOTICE),
        D::command::<Setup>(&["command-structured"]),
        D::command::<Stop>(EXIT_2),
        D::command::<StopFailure>(&["command-structured", "command-terminal-sequence"]),
        D::command::<SubagentStart>(&["command-structured", "command-nonzero-unstructured"]),
        D::command::<SubagentStop>(EXIT_2),
        D::command::<TaskCompleted>(EXIT_2),
        D::command::<TaskCreated>(EXIT_2),
        D::command::<TeammateIdle>(EXIT_2),
        D::command::<UserPromptExpansion>(TEXT_CONTEXT),
        D::command::<UserPromptSubmit>(TEXT_CONTEXT),
        D::command::<WorktreeRemove>(&["command-removed", "command-failed", "command-exit-2"]),
    ]
}

/// Returns definitive discriminator metadata for catalog-module events.
pub fn identification_descriptors() -> Vec<hookkit_core::IdentificationDescriptor> {
    use hookkit_core::IdentificationDescriptor as D;
    const POINTER: &str = "/hook_event_name";
    vec![
        D::definitive::<ConfigChange>(POINTER, "ConfigChange"),
        D::definitive::<CwdChanged>(POINTER, "CwdChanged"),
        D::definitive::<DirectoryAdded>(POINTER, "DirectoryAdded"),
        D::definitive::<Elicitation>(POINTER, "Elicitation"),
        D::definitive::<ElicitationResult>(POINTER, "ElicitationResult"),
        D::definitive::<FileChanged>(POINTER, "FileChanged"),
        D::definitive::<InstructionsLoaded>(POINTER, "InstructionsLoaded"),
        D::definitive::<MessageDisplay>(POINTER, "MessageDisplay"),
        D::definitive::<Notification>(POINTER, "Notification"),
        D::definitive::<PermissionDenied>(POINTER, "PermissionDenied"),
        D::definitive::<PermissionRequest>(POINTER, "PermissionRequest"),
        D::definitive::<PostCompact>(POINTER, "PostCompact"),
        D::definitive::<PostToolBatch>(POINTER, "PostToolBatch"),
        D::definitive::<PostToolUseFailure>(POINTER, "PostToolUseFailure"),
        D::definitive::<PreCompact>(POINTER, "PreCompact"),
        D::definitive::<PreToolUse>(POINTER, "PreToolUse"),
        D::definitive::<SessionEnd>(POINTER, "SessionEnd"),
        D::definitive::<Setup>(POINTER, "Setup"),
        D::definitive::<Stop>(POINTER, "Stop"),
        D::definitive::<StopFailure>(POINTER, "StopFailure"),
        D::definitive::<SubagentStart>(POINTER, "SubagentStart"),
        D::definitive::<SubagentStop>(POINTER, "SubagentStop"),
        D::definitive::<TaskCompleted>(POINTER, "TaskCompleted"),
        D::definitive::<TaskCreated>(POINTER, "TaskCreated"),
        D::definitive::<TeammateIdle>(POINTER, "TeammateIdle"),
        D::definitive::<UserPromptExpansion>(POINTER, "UserPromptExpansion"),
        D::definitive::<UserPromptSubmit>(POINTER, "UserPromptSubmit"),
        D::definitive::<WorktreeRemove>(POINTER, "WorktreeRemove"),
    ]
}

/// Decodes a catalog-module event.
///
/// Returns `Ok(None)` when `event` is not implemented by this module. A known
/// event with malformed native input returns an error.
pub fn decode(event: &EventId, raw: &RawInvocation) -> hookkit_core::Result<Option<CatalogInput>> {
    let input = match event.name() {
        "ConfigChange" => ConfigChange::parse(raw)?,
        "CwdChanged" => CwdChanged::parse(raw)?,
        "DirectoryAdded" => DirectoryAdded::parse(raw)?,
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

    fn raw(value: Value) -> RawInvocation {
        RawInvocation::parse(serde_json::to_vec(&value).unwrap()).unwrap()
    }

    fn envelope(event: &str, fields: Value) -> Value {
        let mut value = serde_json::json!({
            "session_id": "s",
            "transcript_path": "/tmp/t",
            "cwd": "/repo",
            "hook_event_name": event,
        });
        value
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        value
    }

    fn stdout_json(emission: &ProcessEmission) -> Value {
        serde_json::from_slice(emission.stdout()).unwrap()
    }

    fn pre_tool_use() -> Value {
        envelope(
            "PreToolUse",
            serde_json::json!({
                "tool_name": "Bash",
                "tool_input": {"command": "cargo test"},
                "tool_use_id": "toolu_1",
            }),
        )
    }

    #[test]
    fn catalog_input_retains_event_specific_and_future_fields() {
        let value = envelope(
            "ConfigChange",
            serde_json::json!({"source": "skills", "file_path": "/repo/CLAUDE.md", "future": true}),
        );
        let input = ConfigChange::parse(&raw(value.clone())).unwrap();
        assert_eq!(input.field("source"), Some(&serde_json::json!("skills")));
        assert_eq!(input.field("future"), Some(&serde_json::json!(true)));
        assert_eq!(serde_json::to_value(&input).unwrap(), value);
    }

    #[test]
    fn catalog_context_retains_tool_use_id() {
        let input = PreToolUse::parse(&raw(pre_tool_use())).unwrap();
        assert_eq!(
            input
                .context()
                .tool_call_id
                .as_ref()
                .map(ToolCallId::as_str),
            Some("toolu_1")
        );
        assert_eq!(input.tool_name(), Some("Bash"));
        assert_eq!(input.tool_input().unwrap()["command"], "cargo test");
        assert_eq!(input.tool_use_id(), Some("toolu_1"));
        assert!(input.mcp_server().is_none());
    }

    #[test]
    fn catalog_parser_reports_missing_and_mistyped_required_fields() {
        let missing = envelope("CwdChanged", serde_json::json!({"old_cwd": "/repo"}));
        assert!(matches!(
            CwdChanged::parse(&raw(missing)),
            Err(hookkit_core::HookkitError::InvalidInputForHint { message, .. })
                if message == "missing required field new_cwd"
        ));
        let mistyped = envelope("Stop", serde_json::json!({"stop_hook_active": "yes"}));
        assert!(matches!(
            Stop::parse(&raw(mistyped)),
            Err(hookkit_core::HookkitError::InvalidInputForHint { message, .. })
                if message == "field stop_hook_active must be a boolean"
        ));
    }

    #[test]
    fn directory_added_is_decoded_with_current_fields() {
        let value = envelope(
            "DirectoryAdded",
            serde_json::json!({"directory": "/other", "source": "slash_command"}),
        );
        let input = DirectoryAdded::parse(&raw(value)).unwrap();
        assert_eq!(input.field("directory"), Some(&serde_json::json!("/other")));
    }

    #[test]
    fn optional_and_nullable_catalog_fields_may_be_absent_or_null() {
        let permission = envelope(
            "PermissionRequest",
            serde_json::json!({"tool_name": "Bash", "tool_input": {"command": "ls"}}),
        );
        let input = PermissionRequest::parse(&raw(permission)).unwrap();
        assert_eq!(input.tool_use_id(), None);

        let task = envelope(
            "TaskCreated",
            serde_json::json!({"task_id": "1", "task_subject": "Test"}),
        );
        TaskCreated::parse(&raw(task)).unwrap();

        let elicitation = envelope(
            "Elicitation",
            serde_json::json!({"mcp_server_name": "forms", "message": "Choose"}),
        );
        Elicitation::parse(&raw(elicitation)).unwrap();

        let stop = envelope("Stop", serde_json::json!({"stop_hook_active": false}));
        let input = Stop::parse(&raw(stop)).unwrap();
        assert_eq!(input.stop_hook_active(), Some(false));
        assert_eq!(input.last_assistant_message(), None);

        let compact = envelope(
            "PreCompact",
            serde_json::json!({"trigger": "auto", "custom_instructions": null}),
        );
        let input = PreCompact::parse(&raw(compact)).unwrap();
        assert_eq!(input.custom_instructions(), None);
        assert_eq!(input.compact_trigger(), Some(CompactTrigger::Auto));

        let missing = envelope("PreCompact", serde_json::json!({"trigger": "auto"}));
        assert!(PreCompact::parse(&raw(missing)).is_err());

        let subagent = envelope(
            "SubagentStop",
            serde_json::json!({
                "stop_hook_active": false,
                "agent_id": "a",
                "agent_type": "",
                "agent_transcript_path": "/tmp/a.jsonl",
            }),
        );
        let input = SubagentStop::parse(&raw(subagent)).unwrap();
        assert_eq!(input.agent_type(), Some(""));
        assert_eq!(input.agent_transcript_path(), Some("/tmp/a.jsonl"));
    }

    #[test]
    fn sdk_optional_and_deprecated_fields_may_be_absent() {
        // Regression: a missing `command_source` (optional in the Agent SDK)
        // or `team_name` (deprecated, announced for removal) failed the parse,
        // which exits 1 and lets a blocking hook's action through.
        let expansion = envelope(
            "UserPromptExpansion",
            serde_json::json!({
                "expansion_type": "mcp_prompt",
                "command_name": "deploy",
                "command_args": "",
                "prompt": "/deploy",
            }),
        );
        let input = UserPromptExpansion::parse(&raw(expansion.clone())).unwrap();
        assert_eq!(input.prompt(), Some("/deploy"));
        assert_eq!(input.field("command_source"), None);
        assert_eq!(serde_json::to_value(&input).unwrap(), expansion);

        let idle = envelope(
            "TeammateIdle",
            serde_json::json!({"teammate_name": "reviewer"}),
        );
        let input = TeammateIdle::parse(&raw(idle)).unwrap();
        assert_eq!(input.field("team_name"), None);
        let idle = envelope(
            "TeammateIdle",
            serde_json::json!({"teammate_name": "reviewer", "team_name": "session-a1"}),
        );
        let input = TeammateIdle::parse(&raw(idle)).unwrap();
        assert_eq!(
            input.field("team_name"),
            Some(&serde_json::json!("session-a1"))
        );

        // The fields that remain required still are.
        let missing = envelope(
            "TeammateIdle",
            serde_json::json!({"team_name": "session-a1"}),
        );
        assert!(TeammateIdle::parse(&raw(missing)).is_err());
    }

    #[test]
    fn user_prompt_expansion_can_suppress_the_original_prompt() {
        let value = stdout_json(
            &UserPromptExpansion::emit(
                UserPromptExpansionOutput::block("not available")
                    .with_suppress_original_prompt(true)
                    .unwrap(),
            )
            .unwrap(),
        );
        assert_eq!(
            value,
            serde_json::json!({
                "decision": "block",
                "reason": "not available",
                "hookSpecificOutput": {
                    "hookEventName": "UserPromptExpansion",
                    "suppressOriginalPrompt": true,
                },
            })
        );
        assert!(
            UserPromptExpansionOutput::text_context("plain")
                .with_suppress_original_prompt(true)
                .is_err()
        );
    }

    #[test]
    fn harness_sent_enum_values_outside_the_snapshot_still_parse() {
        let mut value = pre_tool_use();
        value["permission_mode"] = "newMode".into();
        value["effort"] = serde_json::json!({"level": "ultra"});
        value["mcp_server"] = serde_json::json!({"name": "db", "source": "registry"});
        let input = PreToolUse::parse(&raw(value)).unwrap();
        assert_eq!(input.permission_mode.as_ref().unwrap().as_str(), "newMode");
        assert!(!input.effort.as_ref().unwrap().level.is_documented());
        let server = input.mcp_server().unwrap();
        assert_eq!(server.name, "db");
        assert!(!server.source.is_sdk());

        let end = envelope("SessionEnd", serde_json::json!({"reason": "shutdown"}));
        let input = SessionEnd::parse(&raw(end)).unwrap();
        assert_eq!(
            input.session_end_reason(),
            Some(SessionEndReason::Unknown("shutdown".into()))
        );

        let failure = envelope("StopFailure", serde_json::json!({"error": "brand_new"}));
        let input = StopFailure::parse(&raw(failure)).unwrap();
        assert_eq!(input.stop_failure_error().unwrap().as_str(), "brand_new");
    }

    #[test]
    fn typed_accessors_are_scoped_to_their_events() {
        let prompt = envelope("UserPromptSubmit", serde_json::json!({"prompt": "hi"}));
        let input = UserPromptSubmit::parse(&raw(prompt)).unwrap();
        assert_eq!(input.prompt(), Some("hi"));
        assert_eq!(input.stop_hook_active(), None);

        let denied = envelope(
            "PermissionDenied",
            serde_json::json!({
                "tool_name": "Bash",
                "tool_input": {},
                "tool_use_id": "t",
                "reason": "auto mode",
            }),
        );
        let input = PermissionDenied::parse(&raw(denied)).unwrap();
        assert_eq!(input.permission_denied_reason(), Some("auto mode"));
        assert_eq!(input.session_end_reason(), None);
        assert_eq!(input.prompt(), None);
    }

    #[test]
    fn the_parsed_event_identity_survives_edits_to_the_wire_name() {
        let mut input = PreToolUse::parse(&raw(pre_tool_use())).unwrap();
        input.hook_event_name = "SessionStart".into();
        assert_eq!(input.event_id(), PreToolUse::EVENT);

        let mut value = pre_tool_use();
        value["hook_event_name"] = "Bogus".into();
        let direct: CatalogInput = serde_json::from_value(value).unwrap();
        assert_eq!(direct.event_id().name(), "Bogus");
        let environment = ClaudeCommandEnvironment::from_map(
            &Stop::EVENT,
            &hookkit_core::EnvironmentVariables::from_pairs([
                ("CLAUDECODE", "1"),
                ("CLAUDE_CODE_CHILD_SESSION", "1"),
                ("CLAUDE_CODE_SESSION_ID", "s"),
                ("CLAUDE_PROJECT_DIR", "/repo"),
            ]),
        )
        .unwrap();
        <Stop as EventSpec>::validate_command_environment(&direct, &environment).unwrap();
    }

    #[test]
    fn rewrites_documented_without_auto_approval_use_ask() {
        // The reference documents `updatedInput` only with `allow` or `ask`;
        // `ask` is the documented rewrite that does not auto-approve.
        let rewrite = PreToolUseOutput::ask("stripped --force")
            .with_updated_input(
                serde_json::json!({"command": "git push"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        let value = stdout_json(&PreToolUse::emit(rewrite).unwrap());
        assert_eq!(value["hookSpecificOutput"]["permissionDecision"], "ask");
        assert_eq!(
            value["hookSpecificOutput"]["updatedInput"]["command"],
            "git push"
        );
    }

    #[test]
    fn pre_tool_use_context_only_output_makes_no_permission_decision() {
        let value = stdout_json(
            &PreToolUse::emit(PreToolUseOutput::with_context("This file is generated.")).unwrap(),
        );
        assert_eq!(
            value,
            serde_json::json!({"hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "additionalContext": "This file is generated.",
            }})
        );

        let rewrite = PreToolUseOutput::no_op()
            .with_updated_input(
                serde_json::json!({"command": "ls"})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
        let value = stdout_json(&PreToolUse::emit(rewrite).unwrap());
        assert!(
            value["hookSpecificOutput"]
                .get("permissionDecision")
                .is_none()
        );
        assert_eq!(value["hookSpecificOutput"]["updatedInput"]["command"], "ls");
    }

    #[test]
    fn pre_tool_use_decisions_carry_their_reasons_in_the_right_field() {
        let value = stdout_json(&PreToolUse::emit(PreToolUseOutput::deny("no rm")).unwrap());
        assert_eq!(value["hookSpecificOutput"]["permissionDecision"], "deny");
        assert_eq!(
            value["hookSpecificOutput"]["permissionDecisionReason"],
            "no rm"
        );
        assert!(
            value["hookSpecificOutput"]
                .get("additionalContext")
                .is_none()
        );

        let value = stdout_json(&PreToolUse::emit(PreToolUseOutput::defer()).unwrap());
        assert_eq!(value["hookSpecificOutput"]["permissionDecision"], "defer");
    }

    #[test]
    fn pre_tool_use_builders_reject_fields_claude_code_ignores() {
        let input = || {
            serde_json::json!({"command": "ls"})
                .as_object()
                .unwrap()
                .clone()
        };
        assert!(
            PreToolUseOutput::deny("no")
                .with_updated_input(input())
                .is_err()
        );
        assert!(
            PreToolUseOutput::defer()
                .with_updated_input(input())
                .is_err()
        );
        assert!(
            PreToolUseOutput::defer()
                .with_additional_context("c")
                .is_err()
        );
        assert!(PreToolUseOutput::no_op().with_decision_reason("r").is_err());
        assert!(
            PreToolUseOutput::with_context("c")
                .with_decision_reason("r")
                .is_err()
        );
        assert!(
            PreToolUseOutput::ask("sure?")
                .with_updated_input(input())
                .is_ok()
        );
        assert!(
            PreToolUseOutput::blocking_error("x")
                .with_additional_context("c")
                .is_err()
        );
    }

    #[test]
    fn permission_request_decisions_keep_allow_and_deny_fields_apart() {
        let input = serde_json::json!({"command": "npm run lint"})
            .as_object()
            .unwrap()
            .clone();
        assert!(
            PermissionRequestOutput::deny("no")
                .with_updated_input(input.clone())
                .is_err()
        );
        assert!(
            PermissionRequestOutput::deny("no")
                .with_updated_permissions(Vec::new())
                .is_err()
        );
        assert!(
            PermissionRequestOutput::allow()
                .with_interrupt(true)
                .is_err()
        );
        assert!(
            PermissionRequestOutput::no_op()
                .with_updated_input(input.clone())
                .is_err()
        );

        let value = stdout_json(
            &PermissionRequest::emit(
                PermissionRequestOutput::allow()
                    .with_updated_input(input)
                    .unwrap(),
            )
            .unwrap(),
        );
        assert_eq!(
            value["hookSpecificOutput"]["decision"],
            serde_json::json!({"behavior": "allow", "updatedInput": {"command": "npm run lint"}})
        );
    }

    #[test]
    #[allow(deprecated)]
    fn permission_request_blocking_error_also_prints_a_deny_decision() {
        // Claude Code ignores exit 2 on PermissionRequest; only the decision
        // object denies.
        let emission =
            PermissionRequest::emit(PermissionRequestOutput::blocking_error("no rm")).unwrap();
        assert_eq!(emission.exit_code(), 2);
        assert_eq!(emission.stderr(), b"no rm");
        assert_eq!(
            stdout_json(&emission)["hookSpecificOutput"]["decision"],
            serde_json::json!({"behavior": "deny", "message": "no rm"})
        );

        // Regression: an empty message used to fail at emission, which
        // dropped the deny decision and let the request through.
        let emission =
            PermissionRequest::emit(PermissionRequestOutput::blocking_error("")).unwrap();
        assert_eq!((emission.exit_code(), emission.stderr()), (2, &b""[..]));
        assert_eq!(
            stdout_json(&emission)["hookSpecificOutput"]["decision"]["behavior"],
            "deny"
        );
    }

    #[test]
    fn structured_exit_2_needs_stderr_only_without_a_blocking_decision() {
        // Regression: an empty message used to pass `into_blocking_error`
        // and fail only at emission, which exits 1 under the default runner
        // policy and so dropped a structured block (fail-open).
        for (emission, reason) in [
            (
                PreToolUse::emit(
                    PreToolUseOutput::deny("no rm")
                        .into_blocking_error("")
                        .unwrap(),
                )
                .unwrap(),
                "no rm",
            ),
            (
                Stop::emit(StopOutput::block("again").into_blocking_error("").unwrap()).unwrap(),
                "again",
            ),
            (
                PostToolUseFailure::emit(
                    PostToolUseFailureOutput::block("retry")
                        .into_feedback_error("")
                        .unwrap(),
                )
                .unwrap(),
                "retry",
            ),
        ] {
            assert_eq!(emission.exit_code(), 2);
            assert!(emission.stderr().is_empty());
            let value = stdout_json(&emission);
            let json_reason = value
                .get("reason")
                .or_else(|| value["hookSpecificOutput"].get("permissionDecisionReason"));
            assert_eq!(json_reason, Some(&Value::from(reason)));
        }
        use crate::model_switch::{PreModelSwitch, PreModelSwitchOutput};
        let emission = PreModelSwitch::emit(
            PreModelSwitchOutput::deny("retired")
                .into_blocking_error("")
                .unwrap(),
        )
        .unwrap();
        assert_eq!((emission.exit_code(), emission.stderr()), (2, &b""[..]));

        // Without a blocking decision stderr is the only message, so an empty
        // one is rejected while the handler can still fall back.
        assert!(PreToolUseOutput::allow().into_blocking_error("").is_err());
        assert!(
            PreToolUseOutput::ask("sure?")
                .into_blocking_error("")
                .is_err()
        );
        assert!(
            StopOutput::with_context("c")
                .into_blocking_error("")
                .is_err()
        );
        assert!(TeammateIdleOutput::no_op().into_blocking_error("").is_err());
        assert!(
            PostToolUseFailureOutput::with_context("c")
                .into_feedback_error("")
                .is_err()
        );
    }

    #[test]
    fn blocking_decisions_are_recognized_in_every_json_form() {
        let object = |value: Value| value.as_object().unwrap().clone();
        assert!(makes_blocking_decision(&object(
            serde_json::json!({"decision": "block", "reason": "r"})
        )));
        assert!(makes_blocking_decision(&object(serde_json::json!({
            "hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny"}
        }))));
        assert!(makes_blocking_decision(&object(serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": {"behavior": "deny"}
            }
        }))));
        for value in [
            serde_json::json!({}),
            serde_json::json!({"decision": "approve"}),
            serde_json::json!({"hookSpecificOutput": {"permissionDecision": "ask"}}),
            serde_json::json!({"hookSpecificOutput": {"decision": {"behavior": "allow"}}}),
            serde_json::json!({"continue": false}),
        ] {
            assert!(!makes_blocking_decision(&object(value)));
        }
    }

    #[test]
    fn user_prompt_submit_can_title_a_session_without_blocking() {
        let value = stdout_json(
            &UserPromptSubmit::emit(
                UserPromptSubmitOutput::no_op()
                    .with_session_title("auth-refactor")
                    .unwrap(),
            )
            .unwrap(),
        );
        assert_eq!(
            value,
            serde_json::json!({"hookSpecificOutput": {
                "hookEventName": "UserPromptSubmit",
                "sessionTitle": "auth-refactor",
            }})
        );

        let value =
            stdout_json(&UserPromptSubmit::emit(UserPromptSubmitOutput::block("no")).unwrap());
        assert_eq!(
            value,
            serde_json::json!({"decision": "block", "reason": "no"})
        );
    }

    #[test]
    fn prompt_text_context_is_verbatim_unless_claude_would_parse_it_as_json() {
        let emission =
            UserPromptSubmit::emit(UserPromptSubmitOutput::text_context("plain\n")).unwrap();
        assert_eq!(emission.stdout(), b"plain\n");

        let emission =
            UserPromptExpansion::emit(UserPromptExpansionOutput::text_context(" {\"a\":1} \n"))
                .unwrap();
        let value = stdout_json(&emission);
        assert_eq!(
            value["hookSpecificOutput"]["hookEventName"],
            "UserPromptExpansion"
        );
        assert_eq!(
            value["hookSpecificOutput"]["additionalContext"],
            " {\"a\":1} \n"
        );
    }

    #[test]
    fn structured_blocks_can_keep_json_while_exiting_2() {
        let emission = Stop::emit(
            StopOutput::block_with_context("again", "ctx")
                .into_blocking_error("stderr")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(emission.exit_code(), 2);
        assert_eq!(emission.stderr(), b"stderr");
        let value = stdout_json(&emission);
        assert_eq!(value["decision"], "block");
        assert_eq!(value["hookSpecificOutput"]["additionalContext"], "ctx");
    }

    #[test]
    fn structured_fields_cannot_be_added_after_an_exit_code_outcome() {
        assert!(
            ConfigChangeOutput::blocking_error("blocked")
                .with_block("again")
                .is_err()
        );
        assert!(
            StopOutput::block("x")
                .into_blocking_error("y")
                .unwrap()
                .with_system_message("late")
                .is_err()
        );
        assert!(
            StopOutput::nonblocking_error("x")
                .into_blocking_error("y")
                .is_err()
        );
        assert!(
            UserPromptSubmitOutput::text_context("plain")
                .with_session_title("t")
                .is_err()
        );
    }

    #[test]
    #[allow(deprecated)]
    fn discarded_fields_stay_schema_valid_but_setup_context_is_dropped() {
        let value = stdout_json(&Setup::emit(SetupOutput::with_context("ignored")).unwrap());
        assert_eq!(value, serde_json::json!({}));

        let value = stdout_json(
            &Notification::emit(
                NotificationOutput::terminal_sequence("\u{7}")
                    .with_continue(true)
                    .unwrap(),
            )
            .unwrap(),
        );
        assert_eq!(
            value,
            serde_json::json!({"terminalSequence": "\u{7}", "continue": true})
        );
    }

    #[test]
    fn watch_paths_must_be_absolute_and_worktree_removal_uses_the_exit_code() {
        let value = stdout_json(
            &CwdChanged::emit(
                CwdChangedOutput::no_op()
                    .with_watch_paths(vec!["/repo/.env".into()])
                    .unwrap()
                    .with_terminal_sequence("\u{7}")
                    .unwrap(),
            )
            .unwrap(),
        );
        assert_eq!(
            value["hookSpecificOutput"]["watchPaths"],
            serde_json::json!(["/repo/.env"])
        );
        assert!(
            FileChangedOutput::no_op()
                .with_watch_paths(vec!["relative/.env".into()])
                .is_err()
        );

        let emission = WorktreeRemove::emit(WorktreeRemoveOutput::removed()).unwrap();
        assert!(emission.stdout().is_empty());
        let emission =
            WorktreeRemove::emit(WorktreeRemoveOutput::failed("in use", 1).unwrap()).unwrap();
        assert_eq!(
            (emission.exit_code(), emission.stderr()),
            (1, &b"in use"[..])
        );
    }

    #[test]
    fn elicitation_actions_emit_their_documented_shapes() {
        let value =
            stdout_json(&Elicitation::emit(ElicitationOutput::accept_without_content()).unwrap());
        assert_eq!(
            value,
            serde_json::json!({"hookSpecificOutput": {
                "hookEventName": "Elicitation",
                "action": "accept",
            }})
        );
        let value = stdout_json(
            &ElicitationResult::emit(ElicitationResultOutput::block("declined")).unwrap(),
        );
        assert_eq!(
            value,
            serde_json::json!({"decision": "block", "reason": "declined"})
        );
    }

    #[test]
    fn parsed_as_json_follows_claude_codes_brace_rule() {
        assert!(parsed_as_json("{}"));
        assert!(parsed_as_json(" \n{\"a\":1}\n\t"));
        assert!(parsed_as_json("{\"a\":1}\n{\"b\":2}"));
        assert!(!parsed_as_json("{ unclosed"));
        assert!(!parsed_as_json("[1]"));
        assert!(!parsed_as_json("\"{}\""));
        assert!(!parsed_as_json("text {}"));
        // JavaScript's trim also strips these, so Claude Code parses the text.
        for text in ["\u{2028}{}\u{2029}", "\u{3000}{}", "\u{feff}{}", "{}\u{a0}"] {
            assert!(parsed_as_json(text), "{text:?}");
        }
        // Neither JavaScript nor this predicate trims these.
        for text in ["\u{200b}{}", "{}\u{180e}"] {
            assert!(!parsed_as_json(text), "{text:?}");
        }
    }
}
