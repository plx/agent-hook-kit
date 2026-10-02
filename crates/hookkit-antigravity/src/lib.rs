//! Native Antigravity hook contracts.
//!
//! This crate implements the five command events of the frozen
//! `antigravity/docs-2026-09-30-r1` snapshot, which records the unified hook
//! reference shared by Antigravity 2.0, the Antigravity CLI, and the
//! Antigravity IDE.
//!
//! # Wire model
//!
//! * Every hook receives one JSON document on stdin with camelCase fields.
//!   The common fields are `conversationId`, `workspacePaths`,
//!   `transcriptPath`, `artifactDirectoryPath`, and the optional `modelName`.
//! * Every hook answers with exit code 0 and one JSON document on stdout.
//!   Exit-zero stderr is treated only as diagnostics.
//! * The reference documents no nonzero-exit, timeout, or invalid-stdout
//!   behavior. Antigravity 2.0 v2.12.0 and later report a failing hook as an
//!   error and continue the conversation. Earlier 2.0 builds ended the
//!   session, and the CLI and IDE notes say nothing. Do not assume that a
//!   failing `PreToolUse` hook lets the pending tool run.
//!
//! # Forward compatibility
//!
//! Inputs are read tolerantly. Unknown top-level and `toolCall` fields are
//! kept in `extra` maps. Open vocabularies such as [`TerminationReason`] keep
//! values this snapshot does not list in an `Unknown` arm, verbatim, and a
//! missing or `null` `toolCall.args` reads as an empty object. Input structs
//! and the [`Event`], [`AnyInput`], and [`AnyCommandOutput`] enums are
//! `#[non_exhaustive]`, so they can gain documented fields and events without
//! a breaking change. Strict validation of documented values belongs to the
//! contract schemas used by conformance, not to runtime parsing. A payload
//! that does not fit the selected event is reported as
//! `HookkitError::InvalidInputForHint`.
#![deny(missing_docs)]

pub mod environment;

pub use environment::AntigravityCommandEnvironment;

use hookkit_core::{
    ContractId, EventCategory, EventId, EventSelector, EventSpec, HarnessId, HarnessSpec,
    IdentificationDescriptor, NativeContext, NativeEventDescriptor, ProcessEmission, RawInvocation,
    SnapshotId, Utf8PathBuf,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Antigravity protocol documentation snapshot implemented by this crate.
pub const SNAPSHOT: SnapshotId = SnapshotId::builtin("docs-2026-09-30-r1");

/// Returns all Antigravity events with native command implementations.
pub fn events() -> Vec<NativeEventDescriptor> {
    vec![
        NativeEventDescriptor::command::<PreInvocation>(&["inject-reminder"]),
        NativeEventDescriptor::command::<PostInvocation>(&["default", "force-continue"]),
        NativeEventDescriptor::command::<PreToolUse>(&["ask"]),
        NativeEventDescriptor::command::<PostToolUse>(&["no-op"]),
        NativeEventDescriptor::command::<Stop>(&["continue"]),
    ]
}

/// Returns event-identification metadata for the Antigravity snapshot.
///
/// Invocation pairs and tool-use pairs share discriminator-free shapes and
/// require an explicit event hint. Stop has a distinct, validated shape.
pub fn identification_descriptors() -> Vec<IdentificationDescriptor> {
    vec![
        IdentificationDescriptor::ambiguous::<PreInvocation>(&["PostInvocation"]),
        IdentificationDescriptor::ambiguous::<PostInvocation>(&["PreInvocation"]),
        IdentificationDescriptor::ambiguous::<PreToolUse>(&["PostToolUse"]),
        IdentificationDescriptor::ambiguous::<PostToolUse>(&["PreToolUse"]),
        IdentificationDescriptor::sound_shape::<Stop>(&[]),
    ]
}

/// Discriminator-free PreInvocation input. Unknown additions are retained.
///
/// `PostInvocation` shares this wire shape; see [`PostInvocationInput`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct PreInvocationInput {
    /// Native conversation identifier (a UUID).
    pub conversation_id: String,
    /// Mounted workspace roots.
    ///
    /// The list may be empty, for example in a conversation launched without a
    /// workspace folder or a standalone Antigravity 2.0 conversation. The
    /// field itself is required.
    pub workspace_paths: Vec<Utf8PathBuf>,
    /// Path to the native `transcript.jsonl` conversation log.
    ///
    /// The reference calls it absolute, but every official example uses a
    /// literal `~/` prefix. HookKit does not expand it.
    pub transcript_path: Utf8PathBuf,
    /// Directory holding the conversation's artifacts and screenshots.
    ///
    /// Like `transcript_path`, it may carry a literal `~/` prefix.
    pub artifact_directory_path: Utf8PathBuf,
    /// Name or identifier of the model handling the invocation, for example
    /// `gemini-3.6-flash-medium`.
    ///
    /// `None` when the payload omits it: builds released before the field
    /// was documented may not send it. An explicit JSON `null` is also read
    /// as `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    /// Zero-based sequence number of the current model invocation. The first
    /// invocation is 0.
    pub invocation_num: u64,
    /// Number of steps currently present in the trajectory.
    pub initial_num_steps: u64,
    /// Unknown protocol fields retained for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Step to inject into the trajectory from an invocation hook.
///
/// Each step serializes as an object with exactly one of `toolCall`,
/// `userMessage`, or `ephemeralMessage`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum InjectStep {
    /// Injects a native tool-call object. The reference documents its shape
    /// only as "an object".
    ToolCall {
        /// Tool call serialized under the native `toolCall` key.
        #[serde(rename = "toolCall")]
        tool_call: serde_json::Map<String, serde_json::Value>,
    },
    /// Injects a persistent user message.
    UserMessage {
        /// Message serialized under the native `userMessage` key.
        #[serde(rename = "userMessage")]
        user_message: String,
    },
    /// Injects a transient system message.
    EphemeralMessage {
        /// Message serialized under the native `ephemeralMessage` key.
        #[serde(rename = "ephemeralMessage")]
        ephemeral_message: String,
    },
}

impl InjectStep {
    /// Creates a step that injects a native tool-call object.
    pub fn tool_call(tool_call: serde_json::Map<String, serde_json::Value>) -> Self {
        Self::ToolCall { tool_call }
    }

    /// Creates a step that injects a persistent user message.
    pub fn user_message(message: impl Into<String>) -> Self {
        Self::UserMessage {
            user_message: message.into(),
        }
    }

    /// Creates a step that injects a transient system message.
    pub fn ephemeral_message(message: impl Into<String>) -> Self {
        Self::EphemeralMessage {
            ephemeral_message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
/// Native response from a pre-invocation command hook.
pub struct PreInvocationOutput {
    /// Ordered steps to inject before the model is called. An empty list is
    /// omitted, so a no-op response is exactly `{}`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub inject_steps: Vec<InjectStep>,
}

impl PreInvocationOutput {
    /// Creates a response that injects no steps (`{}`).
    pub fn no_op() -> Self {
        Self::default()
    }

    /// Creates a response containing exactly one injected step.
    pub fn inject(step: InjectStep) -> Self {
        Self {
            inject_steps: vec![step],
        }
    }

    /// Creates a response that injects the given steps in order.
    pub fn inject_all(steps: impl IntoIterator<Item = InjectStep>) -> Self {
        Self {
            inject_steps: steps.into_iter().collect(),
        }
    }

    /// Appends one step after any already present.
    pub fn with_step(mut self, step: InjectStep) -> Self {
        self.inject_steps.push(step);
        self
    }
}

/// Native Antigravity `PreInvocation` command contract.
pub enum PreInvocation {}

impl EventSpec for PreInvocation {
    type Input = PreInvocationInput;
    type CommandEnvironment = AntigravityCommandEnvironment;
    type CommandOutput = PreInvocationOutput;

    const HARNESS: HarnessId = HarnessId::ANTIGRAVITY;
    const SNAPSHOT: SnapshotId = SNAPSHOT;
    const EVENT: EventId = EventId::builtin(Self::HARNESS, "PreInvocation");
    const CATEGORY: EventCategory = EventCategory::Model;
    const CONTRACT: ContractId =
        ContractId::builtin("antigravity/docs-2026-09-30-r1/PreInvocation");

    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        deserialize_input(invocation, Self::EVENT)
    }

    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        ProcessEmission::command_json(Self::CONTRACT, &output)
    }

    fn context(input: &Self::Input) -> NativeContext {
        invocation_context(input)
    }
}

/// Native post-invocation input, which shares the pre-invocation wire shape.
///
/// The reference does not say whether `initial_num_steps` counts steps
/// before or after the completed invocation.
pub type PostInvocationInput = PreInvocationInput;

/// Loop control after a post-invocation hook response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TerminationBehavior {
    /// `""`: use Antigravity's default behavior, as if the field were omitted.
    #[serde(rename = "")]
    Default,
    /// `force_continue`: force the execution loop to continue.
    ForceContinue,
    /// `terminate`: force the execution loop to terminate.
    Terminate,
}

impl TerminationBehavior {
    /// Returns the exact native wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "",
            Self::ForceContinue => "force_continue",
            Self::Terminate => "terminate",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
/// Native response from a post-invocation command hook.
pub struct PostInvocationOutput {
    /// Ordered steps to inject after the invocation completes.
    ///
    /// This list is always serialized, even when empty, matching the official
    /// `{"injectSteps": [], "terminationBehavior": ""}` example.
    pub inject_steps: Vec<InjectStep>,
    /// Optional loop control. `None` omits the field, which Antigravity
    /// treats like [`TerminationBehavior::Default`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub termination_behavior: Option<TerminationBehavior>,
}

impl PostInvocationOutput {
    /// Creates a response that injects nothing and keeps default loop
    /// behavior (`{"injectSteps":[]}`).
    pub fn no_op() -> Self {
        Self::default()
    }

    /// Creates a response containing exactly one injected step.
    pub fn inject(step: InjectStep) -> Self {
        Self {
            inject_steps: vec![step],
            termination_behavior: None,
        }
    }

    /// Creates a response that injects the given steps in order.
    pub fn inject_all(steps: impl IntoIterator<Item = InjectStep>) -> Self {
        Self {
            inject_steps: steps.into_iter().collect(),
            termination_behavior: None,
        }
    }

    /// Appends one step after any already present.
    pub fn with_step(mut self, step: InjectStep) -> Self {
        self.inject_steps.push(step);
        self
    }

    /// Sets the explicit loop-control value.
    pub fn with_termination_behavior(mut self, behavior: TerminationBehavior) -> Self {
        self.termination_behavior = Some(behavior);
        self
    }
}

/// Native Antigravity `PostInvocation` command contract.
///
/// Fires immediately after each model invocation completes.
pub enum PostInvocation {}

impl EventSpec for PostInvocation {
    type Input = PostInvocationInput;
    type CommandEnvironment = AntigravityCommandEnvironment;
    type CommandOutput = PostInvocationOutput;
    const HARNESS: HarnessId = HarnessId::ANTIGRAVITY;
    const SNAPSHOT: SnapshotId = SNAPSHOT;
    const EVENT: EventId = EventId::builtin(Self::HARNESS, "PostInvocation");
    const CATEGORY: EventCategory = EventCategory::Model;
    const CONTRACT: ContractId =
        ContractId::builtin("antigravity/docs-2026-09-30-r1/PostInvocation");
    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        deserialize_input(invocation, Self::EVENT)
    }
    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        ProcessEmission::command_json(Self::CONTRACT, &output)
    }
    fn context(input: &Self::Input) -> NativeContext {
        invocation_context(input)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "ToolCallWire", into = "ToolCallWire")]
#[non_exhaustive]
/// Native tool-call payload nested inside a tool event.
pub struct ToolCall {
    /// Harness-native tool name, for example `run_command` or `view_file`.
    ///
    /// The tool catalog is open: release notes add, retire, and extend tools
    /// that the reference does not list.
    pub name: String,
    /// Tool arguments as an exact JSON object, keyed by the tool's native
    /// PascalCase argument names (for example `CommandLine`, `AbsolutePath`).
    ///
    /// The reference types `args` as an object but lists tools that take no
    /// arguments ("Arguments: None", such as `list_permissions`), and a Go
    /// encoder writes a nil argument map as `null`, or leaves it out under
    /// `omitempty`. A missing key and a JSON `null` therefore both read as an
    /// empty object instead of failing the hook, and an unmodified call
    /// re-serializes exactly as it was sent: without the key, as `null`, or
    /// as `{}`.
    pub args: serde_json::Map<String, serde_json::Value>,
    /// Unknown tool-call fields retained for forward compatibility.
    pub extra: BTreeMap<String, serde_json::Value>,
    /// How the payload sent `args`, so an empty object can re-serialize in
    /// the same form.
    args_form: ArgsForm,
}

/// Wire form in which a [`ToolCall`] carried its `args`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArgsForm {
    /// A JSON object, possibly empty.
    Object,
    /// `"args": null`.
    Null,
    /// No `args` key.
    Absent,
}

/// Wire form of [`ToolCall`], which keeps a missing, `null`, and `{}` `args`
/// distinguishable so parsing stays lossless.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolCallWire {
    name: String,
    /// `None` when the key is absent, `Some(None)` for `null`.
    #[serde(
        default,
        deserialize_with = "present_nullable_object",
        skip_serializing_if = "Option::is_none"
    )]
    args: Option<Option<serde_json::Map<String, serde_json::Value>>>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}

/// Deserializes a present key whose value may be `null`; a missing key takes
/// the field's default (`None`) instead.
fn present_nullable_object<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<serde_json::Map<String, serde_json::Value>>>, D::Error> {
    Option::deserialize(deserializer).map(Some)
}

impl From<ToolCallWire> for ToolCall {
    fn from(wire: ToolCallWire) -> Self {
        let (args_form, args) = match wire.args {
            None => (ArgsForm::Absent, serde_json::Map::new()),
            Some(None) => (ArgsForm::Null, serde_json::Map::new()),
            Some(Some(args)) => (ArgsForm::Object, args),
        };
        Self {
            name: wire.name,
            args,
            extra: wire.extra,
            args_form,
        }
    }
}

impl From<ToolCall> for ToolCallWire {
    fn from(call: ToolCall) -> Self {
        // Arguments added to a call that arrived without any are serialized.
        let args = match call.args_form {
            ArgsForm::Absent if call.args.is_empty() => None,
            ArgsForm::Null if call.args.is_empty() => Some(None),
            _ => Some(Some(call.args)),
        };
        Self {
            name: call.name,
            args,
            extra: call.extra,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
/// Native Antigravity input observed before a tool runs.
pub struct PreToolUseInput {
    /// Native conversation identifier (a UUID).
    pub conversation_id: String,
    /// Mounted workspace roots. The list may be empty; see
    /// [`PreInvocationInput::workspace_paths`].
    pub workspace_paths: Vec<Utf8PathBuf>,
    /// Path to the native `transcript.jsonl` conversation log. It may carry a
    /// literal `~/` prefix.
    pub transcript_path: Utf8PathBuf,
    /// Directory holding the conversation's artifacts and screenshots. It may
    /// carry a literal `~/` prefix.
    pub artifact_directory_path: Utf8PathBuf,
    /// Name or identifier of the model handling the invocation; `None` when
    /// the payload omits it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    /// Tool call about to execute.
    pub tool_call: ToolCall,
    /// Zero-based index of the current step in the whole conversation
    /// trajectory. It is not an index within the current model invocation.
    pub step_idx: u64,
    /// Unknown protocol fields retained for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Authorization decision returned by a pre-tool hook.
///
/// Antigravity requires a decision on every response and documents no
/// pass-through or "no objection" value; an empty string is outside the
/// documented vocabulary. [`ToolDecision::Ask`] is the value that defers to
/// the user's normal permission policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ToolDecision {
    /// `allow`: automatically approve the tool call.
    ///
    /// This is an auto-approval, not a pass-through. It bypasses the prompts
    /// that the user's Ask presets and permission settings would otherwise
    /// show.
    Allow,
    /// `deny`: hard-block the tool call immediately.
    Deny,
    /// `ask`: prompt the user, but respect "Always Allow" settings and cached
    /// grants. This is the closest documented equivalent of deferring to
    /// normal policy.
    Ask,
    /// `force_ask`: always prompt the user, ignoring cached permissions.
    ForceAsk,
    /// `deny_unless_prior_grant`: deny unless a prior user grant already
    /// approved the resource.
    ///
    /// The unified reference documents it for every surface, but the IDE
    /// build described by the superseded IDE page (v2.5.5) may not accept it.
    DenyUnlessPriorGrant,
}

impl ToolDecision {
    /// Returns the exact native wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Ask => "ask",
            Self::ForceAsk => "force_ask",
            Self::DenyUnlessPriorGrant => "deny_unless_prior_grant",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
/// Native response from an Antigravity pre-tool command hook.
///
/// # No pass-through
///
/// Antigravity requires `decision` on every response and documents no
/// pass-through or "no objection" value. An empty or missing decision is
/// outside the documented protocol: CLI 1.0.16 tolerates an empty string, but
/// its effect is undocumented, so this type cannot express it.
///
/// [`PreToolUseOutput::allow`] is an auto-approval. It bypasses the prompts
/// that the user's Ask presets and permission settings would otherwise show,
/// so it is not a neutral answer. A hook with no objection that should leave
/// the call to the user's normal policy returns [`PreToolUseOutput::ask`],
/// which prompts only where no "Always Allow" setting already covers the
/// call.
pub struct PreToolUseOutput {
    /// Authorization decision for the pending tool call.
    pub decision: ToolDecision,
    /// Optional explanation shown to the agent or user. CLI 1.1.28 and later
    /// show it as a `Reason:` line in tool-approval prompts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Permission resources in `action(target)` form, for example
    /// `read_file(/path)` or `command(npm test)`, that override default tool
    /// permissions.
    ///
    /// Emission removes duplicate entries and keeps the first occurrence of
    /// each, so a duplicate never turns a decision into a hook failure.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub permission_overrides: Vec<String>,
}

impl PreToolUseOutput {
    /// Creates a response with `decision` and no reason or overrides.
    pub fn new(decision: ToolDecision) -> Self {
        Self {
            decision,
            reason: None,
            permission_overrides: Vec::new(),
        }
    }

    /// Creates an `allow` response, which auto-approves the call and bypasses
    /// Ask presets. It is not a pass-through.
    pub fn allow() -> Self {
        Self::new(ToolDecision::Allow)
    }

    /// Creates a `deny` response, which hard-blocks the call.
    pub fn deny() -> Self {
        Self::new(ToolDecision::Deny)
    }

    /// Creates an `ask` response, which prompts the user unless an "Always
    /// Allow" setting already covers the call.
    pub fn ask() -> Self {
        Self::new(ToolDecision::Ask)
    }

    /// Creates a `force_ask` response, which always prompts the user.
    pub fn force_ask() -> Self {
        Self::new(ToolDecision::ForceAsk)
    }

    /// Creates a `deny_unless_prior_grant` response.
    pub fn deny_unless_prior_grant() -> Self {
        Self::new(ToolDecision::DenyUnlessPriorGrant)
    }

    /// Sets the explanation shown to the agent or user.
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// Appends one permission override unless it is already present.
    pub fn with_permission_override(mut self, resource: impl Into<String>) -> Self {
        let resource = resource.into();
        if !self.permission_overrides.contains(&resource) {
            self.permission_overrides.push(resource);
        }
        self
    }

    /// Appends permission overrides in order, skipping entries already
    /// present.
    pub fn with_permission_overrides<I, S>(self, resources: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        resources.into_iter().fold(self, |output, resource| {
            output.with_permission_override(resource)
        })
    }
}

/// Native Antigravity `PreToolUse` command contract.
pub enum PreToolUse {}
impl EventSpec for PreToolUse {
    type Input = PreToolUseInput;
    type CommandEnvironment = AntigravityCommandEnvironment;
    type CommandOutput = PreToolUseOutput;
    const HARNESS: HarnessId = HarnessId::ANTIGRAVITY;
    const SNAPSHOT: SnapshotId = SNAPSHOT;
    const EVENT: EventId = EventId::builtin(Self::HARNESS, "PreToolUse");
    const CATEGORY: EventCategory = EventCategory::Tool;
    const CONTRACT: ContractId = ContractId::builtin("antigravity/docs-2026-09-30-r1/PreToolUse");
    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        deserialize_input(invocation, Self::EVENT)
    }
    fn emit(mut output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        let mut seen = BTreeSet::new();
        output
            .permission_overrides
            .retain(|resource| seen.insert(resource.clone()));
        ProcessEmission::command_json(Self::CONTRACT, &output)
    }
    fn context(input: &Self::Input) -> NativeContext {
        common_context(
            &input.conversation_id,
            &input.workspace_paths,
            &input.transcript_path,
            &input.artifact_directory_path,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
/// Native Antigravity input observed after a tool finishes.
pub struct PostToolUseInput {
    /// Native conversation identifier (a UUID).
    pub conversation_id: String,
    /// Mounted workspace roots. The list may be empty; see
    /// [`PreInvocationInput::workspace_paths`].
    pub workspace_paths: Vec<Utf8PathBuf>,
    /// Path to the native `transcript.jsonl` conversation log. It may carry a
    /// literal `~/` prefix.
    pub transcript_path: Utf8PathBuf,
    /// Directory holding the conversation's artifacts and screenshots. It may
    /// carry a literal `~/` prefix.
    pub artifact_directory_path: Utf8PathBuf,
    /// Name or identifier of the model handling the invocation; `None` when
    /// the payload omits it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    /// Tool call that finished executing.
    ///
    /// Current Antigravity 2.0 and CLI payloads are expected to carry it. It
    /// is `None` when the payload omits it, as the IDE reference example and
    /// earlier generic references did. A present `toolCall` must still carry
    /// `name`; its `args` may be missing or `null`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<ToolCall>,
    /// Zero-based index of the completed step in the whole conversation
    /// trajectory. It is not an index within the current model invocation.
    pub step_idx: u64,
    /// Raw runtime error text exactly as sent.
    ///
    /// Antigravity sends an empty string, or omits the field, when the call
    /// succeeded, so `Some("")` means success. Use
    /// [`PostToolUseInput::error_message`] or [`PostToolUseInput::failed`]
    /// rather than testing this field for `Some`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Unknown protocol fields retained for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl PostToolUseInput {
    /// Returns the tool failure message.
    ///
    /// Returns `None` when the call succeeded: `error` is absent, empty, or
    /// whitespace only.
    pub fn error_message(&self) -> Option<&str> {
        non_blank(self.error.as_deref())
    }

    /// Reports whether the tool call failed, meaning it carries a non-blank
    /// error message.
    pub fn failed(&self) -> bool {
        self.error_message().is_some()
    }
}

/// Empty successful response from an Antigravity post-tool command hook.
///
/// The reference defines no response fields, so stdout is always exactly
/// `{}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PostToolUseOutput {
    /// Optional exit-zero process diagnostics; never serialized into stdout.
    #[serde(skip)]
    protocol_stderr: Option<Vec<u8>>,
}

impl PostToolUseOutput {
    /// Creates the exact `{}` response.
    pub fn no_op() -> Self {
        Self::default()
    }

    /// Adds UTF-8 protocol stderr while preserving the required `{}` stdout.
    ///
    /// Antigravity does not define a structured `PostToolUse` message field.
    /// Exit-zero stderr is treated only as diagnostics and has no documented
    /// effect on the conversation. An already wrapped response is rejected.
    pub fn with_protocol_stderr(mut self, stderr: impl Into<String>) -> hookkit_core::Result<Self> {
        if self.protocol_stderr.is_some() {
            return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "protocol stderr can only wrap a successful output once",
            ));
        }
        self.protocol_stderr = Some(stderr.into().into_bytes());
        Ok(self)
    }
}
/// Native Antigravity `PostToolUse` command contract.
pub enum PostToolUse {}
impl EventSpec for PostToolUse {
    type Input = PostToolUseInput;
    type CommandEnvironment = AntigravityCommandEnvironment;
    type CommandOutput = PostToolUseOutput;
    const HARNESS: HarnessId = HarnessId::ANTIGRAVITY;
    const SNAPSHOT: SnapshotId = SNAPSHOT;
    const EVENT: EventId = EventId::builtin(Self::HARNESS, "PostToolUse");
    const CATEGORY: EventCategory = EventCategory::Tool;
    const CONTRACT: ContractId = ContractId::builtin("antigravity/docs-2026-09-30-r1/PostToolUse");
    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        deserialize_input(invocation, Self::EVENT)
    }
    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        let stdout = serde_json::to_vec(&serde_json::json!({}))?;
        Ok(ProcessEmission::command_unchecked(
            Self::CONTRACT,
            stdout,
            output.protocol_stderr.unwrap_or_default(),
            0,
        ))
    }
    fn context(input: &Self::Input) -> NativeContext {
        common_context(
            &input.conversation_id,
            &input.workspace_paths,
            &input.transcript_path,
            &input.artifact_directory_path,
        )
    }
}

/// Why an Antigravity execution is stopping.
///
/// The vocabulary is open. The reference gives `model_stop`,
/// `max_steps_exceeded`, and `error` only as examples, so any other value is
/// kept verbatim in [`TerminationReason::Unknown`] rather than failing the
/// parse.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TerminationReason {
    /// `model_stop`: the model ended its turn.
    ModelStop,
    /// `max_steps_exceeded`: the execution ran out of steps.
    MaxStepsExceeded,
    /// `error`: a system error ended the execution; see
    /// [`StopInput::error_message`].
    Error,
    /// A value this snapshot does not list, retained verbatim.
    ///
    /// Values produced by parsing never hold one of the listed strings.
    Unknown(String),
}

impl TerminationReason {
    /// Returns the exact native wire string.
    pub fn as_str(&self) -> &str {
        match self {
            Self::ModelStop => "model_stop",
            Self::MaxStepsExceeded => "max_steps_exceeded",
            Self::Error => "error",
            Self::Unknown(value) => value,
        }
    }
}

impl From<&str> for TerminationReason {
    fn from(value: &str) -> Self {
        match value {
            "model_stop" => Self::ModelStop,
            "max_steps_exceeded" => Self::MaxStepsExceeded,
            "error" => Self::Error,
            other => Self::Unknown(other.to_owned()),
        }
    }
}

impl From<String> for TerminationReason {
    fn from(value: String) -> Self {
        match value.as_str() {
            "model_stop" | "max_steps_exceeded" | "error" => Self::from(value.as_str()),
            _ => Self::Unknown(value),
        }
    }
}

impl fmt::Display for TerminationReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for TerminationReason {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TerminationReason {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::from)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
/// Native Antigravity input observed when an execution attempts to stop.
pub struct StopInput {
    /// Native conversation identifier (a UUID).
    pub conversation_id: String,
    /// Mounted workspace roots. The list may be empty; see
    /// [`PreInvocationInput::workspace_paths`].
    pub workspace_paths: Vec<Utf8PathBuf>,
    /// Path to the native `transcript.jsonl` conversation log. It may carry a
    /// literal `~/` prefix.
    pub transcript_path: Utf8PathBuf,
    /// Directory holding the conversation's artifacts and screenshots. It may
    /// carry a literal `~/` prefix.
    pub artifact_directory_path: Utf8PathBuf,
    /// Name or identifier of the model handling the invocation; `None` when
    /// the payload omits it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    /// Sequence number of the execution attempt.
    pub execution_num: u64,
    /// Harness-provided reason for the attempted termination (open
    /// vocabulary).
    pub termination_reason: TerminationReason,
    /// `true` when the agent and all background commands and asynchronous
    /// tasks have finished; `false` while background work is still running.
    pub fully_idle: bool,
    /// Raw system-error text exactly as sent.
    ///
    /// The official example sends an empty string with `model_stop`, so
    /// `Some("")` means no error. Use [`StopInput::error_message`] rather
    /// than testing this field for `Some`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Unknown protocol fields retained for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl StopInput {
    /// Returns the system error that caused termination.
    ///
    /// Returns `None` when `error` is absent, empty, or whitespace only.
    pub fn error_message(&self) -> Option<&str> {
        non_blank(self.error.as_deref())
    }
}

/// Decision returned by an Antigravity `Stop` hook.
///
/// Antigravity gives meaning to exactly one value: `continue` prevents the
/// stop. Every other value allows it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StopDecision {
    /// `continue`: prevent the stop and re-enter the execution loop.
    ///
    /// This is not a hard gate. CLI 1.1.9 ends the turn normally after a
    /// configurable number of consecutive continuations, and Antigravity 2.0
    /// v2.6.0 finishes the turn after repeated blocks. The limit is
    /// undocumented.
    Continue,
    /// `stop`: allow the stop. Antigravity assigns no meaning to the token;
    /// HookKit uses it because it is unambiguous and not `continue`.
    Stop,
    /// Any other raw decision, emitted verbatim; Antigravity allows the stop.
    ///
    /// Emission rejects an empty value. It also rejects any case or
    /// whitespace variant of `continue` other than the exact token, such as
    /// `Continue`: the value looks like a continue, but Antigravity treats it
    /// as allowing the stop.
    Other(String),
}

impl StopDecision {
    /// Returns the exact native wire string.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Continue => "continue",
            Self::Stop => "stop",
            Self::Other(value) => value,
        }
    }

    /// Reports whether Antigravity treats this decision as preventing the
    /// stop.
    pub fn prevents_stop(&self) -> bool {
        self.as_str() == "continue"
    }
}

impl From<&str> for StopDecision {
    fn from(value: &str) -> Self {
        match value {
            "continue" => Self::Continue,
            "stop" => Self::Stop,
            other => Self::Other(other.to_owned()),
        }
    }
}

impl From<String> for StopDecision {
    fn from(value: String) -> Self {
        match value.as_str() {
            "continue" | "stop" => Self::from(value.as_str()),
            _ => Self::Other(value),
        }
    }
}

impl fmt::Display for StopDecision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for StopDecision {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// Native response from an Antigravity stop command hook.
pub struct StopOutput {
    /// Stop decision; see [`StopDecision`].
    pub decision: StopDecision,
    /// Optional message. With [`StopDecision::Continue`] it is injected into
    /// the conversation as a system message. No effect is documented with any
    /// other decision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl StopOutput {
    /// Creates a response with `decision` and no reason.
    pub fn new(decision: impl Into<StopDecision>) -> Self {
        Self {
            decision: decision.into(),
            reason: None,
        }
    }

    /// Creates a response that allows the stop (`{"decision":"stop"}`).
    pub fn allow_stop() -> Self {
        Self::new(StopDecision::Stop)
    }

    /// Creates a response that prevents the stop and injects `reason` as a
    /// system message. The runtime caps consecutive continuations.
    pub fn continue_with(reason: impl Into<String>) -> Self {
        Self::new(StopDecision::Continue).with_reason(reason)
    }

    /// Sets the reason. Antigravity documents an effect only for
    /// [`StopDecision::Continue`].
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

/// Native Antigravity `Stop` command contract.
pub enum Stop {}
impl EventSpec for Stop {
    type Input = StopInput;
    type CommandEnvironment = AntigravityCommandEnvironment;
    type CommandOutput = StopOutput;
    const HARNESS: HarnessId = HarnessId::ANTIGRAVITY;
    const SNAPSHOT: SnapshotId = SNAPSHOT;
    const EVENT: EventId = EventId::builtin(Self::HARNESS, "Stop");
    const CATEGORY: EventCategory = EventCategory::Agent;
    const CONTRACT: ContractId = ContractId::builtin("antigravity/docs-2026-09-30-r1/Stop");
    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        deserialize_input(invocation, Self::EVENT)
    }
    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        let decision = output.decision.as_str();
        if decision.is_empty() {
            return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "Stop decision must not be empty",
            ));
        }
        if decision != "continue" && decision.trim().eq_ignore_ascii_case("continue") {
            return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "Stop decision is a variant of `continue` that Antigravity treats as allowing the stop; use StopDecision::Continue",
            ));
        }
        ProcessEmission::command_json(Self::CONTRACT, &output)
    }
    fn context(input: &Self::Input) -> NativeContext {
        common_context(
            &input.conversation_id,
            &input.workspace_paths,
            &input.transcript_path,
            &input.artifact_directory_path,
        )
    }
}

/// Deserializes the typed input of `event`, the event the caller selected.
///
/// Antigravity payloads carry no discriminator, so a payload that does not
/// fit the selected event's shape (a missing or mistyped field) is reported
/// as [`hookkit_core::HookkitError::InvalidInputForHint`], as the Claude Code
/// and Codex parsers report a schema violation of a known event.
fn deserialize_input<T: serde::de::DeserializeOwned>(
    invocation: &RawInvocation,
    event: EventId,
) -> hookkit_core::Result<T> {
    serde_json::from_value(invocation.json().clone()).map_err(|error| {
        hookkit_core::HookkitError::InvalidInputForHint {
            event,
            message: error.to_string(),
        }
    })
}

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

fn common_context(
    conversation_id: &str,
    workspace_paths: &[Utf8PathBuf],
    transcript_path: &Utf8PathBuf,
    artifact_directory_path: &Utf8PathBuf,
) -> NativeContext {
    NativeContext {
        workspace_roots: workspace_paths.to_vec(),
        conversation_id: hookkit_core::ConversationId::new(conversation_id.to_owned()).ok(),
        transcript_path: Some(transcript_path.clone()),
        artifact_directory: Some(artifact_directory_path.clone()),
        ..NativeContext::default()
    }
}

fn invocation_context(input: &PreInvocationInput) -> NativeContext {
    let session_boundary = (input.invocation_num == 0).then(|| {
        hookkit_core::SessionBoundaryContext::observed(
            hookkit_core::SessionBoundaryKind::InvocationStart,
        )
        .with_occurrence_key(format!(
            "{}\0{}",
            input.conversation_id, input.invocation_num
        ))
    });
    NativeContext {
        session_boundary,
        ..common_context(
            &input.conversation_id,
            &input.workspace_paths,
            &input.transcript_path,
            &input.artifact_directory_path,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
/// Compile-time selector for an implemented Antigravity event.
pub enum Event {
    /// Selects [`PreInvocation`].
    PreInvocation,
    /// Selects [`PostInvocation`].
    PostInvocation,
    /// Selects [`PreToolUse`].
    PreToolUse,
    /// Selects [`PostToolUse`].
    PostToolUse,
    /// Selects [`Stop`].
    Stop,
}

impl EventSelector for Event {
    fn event_id(&self) -> EventId {
        let name = match self {
            Self::PreInvocation => "PreInvocation",
            Self::PostInvocation => "PostInvocation",
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
            Self::Stop => "Stop",
        };
        EventId::builtin(HarnessId::ANTIGRAVITY, name)
    }
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
/// Lossless sum type over all implemented Antigravity inputs.
pub enum AnyInput {
    /// A pre-invocation input.
    PreInvocation(PreInvocationInput),
    /// A post-invocation input.
    PostInvocation(PostInvocationInput),
    /// A pre-tool input.
    PreToolUse(PreToolUseInput),
    /// A post-tool input.
    PostToolUse(PostToolUseInput),
    /// A stop input.
    Stop(StopInput),
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
/// Sum type over all implemented Antigravity command outputs.
pub enum AnyCommandOutput {
    /// A pre-invocation output.
    PreInvocation(PreInvocationOutput),
    /// A post-invocation output.
    PostInvocation(PostInvocationOutput),
    /// A pre-tool output.
    PreToolUse(PreToolUseOutput),
    /// A post-tool output.
    PostToolUse(PostToolUseOutput),
    /// A stop output.
    Stop(StopOutput),
}

/// Harness adapter implementing the documented Antigravity snapshot.
pub enum Antigravity {}

impl HarnessSpec for Antigravity {
    type AnyInput = AnyInput;
    type CommandEnvironment = AntigravityCommandEnvironment;
    type AnyCommandOutput = AnyCommandOutput;
    type EventSelector = Event;

    const ID: HarnessId = HarnessId::ANTIGRAVITY;
    const SNAPSHOT: SnapshotId = SNAPSHOT;

    fn identification_descriptors() -> Vec<IdentificationDescriptor> {
        identification_descriptors()
    }

    fn decode(event: &EventId, raw: &RawInvocation) -> hookkit_core::Result<Self::AnyInput> {
        match event.name() {
            "PreInvocation" => PreInvocation::parse(raw).map(AnyInput::PreInvocation),
            "PostInvocation" => PostInvocation::parse(raw).map(AnyInput::PostInvocation),
            "PreToolUse" => PreToolUse::parse(raw).map(AnyInput::PreToolUse),
            "PostToolUse" => PostToolUse::parse(raw).map(AnyInput::PostToolUse),
            "Stop" => Stop::parse(raw).map(AnyInput::Stop),
            _ => Err(hookkit_core::HookkitError::UnrecognizedEvent {
                harness: Self::ID,
                message: event.name().to_string(),
            }),
        }
    }

    fn input_event(input: &Self::AnyInput) -> EventId {
        match input {
            AnyInput::PreInvocation(_) => PreInvocation::EVENT,
            AnyInput::PostInvocation(_) => PostInvocation::EVENT,
            AnyInput::PreToolUse(_) => PreToolUse::EVENT,
            AnyInput::PostToolUse(_) => PostToolUse::EVENT,
            AnyInput::Stop(_) => Stop::EVENT,
        }
    }

    fn output_event(output: &Self::AnyCommandOutput) -> EventId {
        match output {
            AnyCommandOutput::PreInvocation(_) => PreInvocation::EVENT,
            AnyCommandOutput::PostInvocation(_) => PostInvocation::EVENT,
            AnyCommandOutput::PreToolUse(_) => PreToolUse::EVENT,
            AnyCommandOutput::PostToolUse(_) => PostToolUse::EVENT,
            AnyCommandOutput::Stop(_) => Stop::EVENT,
        }
    }

    fn encode_command_unchecked(
        output: Self::AnyCommandOutput,
    ) -> hookkit_core::Result<ProcessEmission> {
        match output {
            AnyCommandOutput::PreInvocation(output) => PreInvocation::emit(output),
            AnyCommandOutput::PostInvocation(output) => PostInvocation::emit(output),
            AnyCommandOutput::PreToolUse(output) => PreToolUse::emit(output),
            AnyCommandOutput::PostToolUse(output) => PostToolUse::emit(output),
            AnyCommandOutput::Stop(output) => Stop::emit(output),
        }
    }

    fn context(input: &Self::AnyInput) -> NativeContext {
        match input {
            AnyInput::PreInvocation(input) => PreInvocation::context(input),
            AnyInput::PostInvocation(input) => PostInvocation::context(input),
            AnyInput::PreToolUse(input) => PreToolUse::context(input),
            AnyInput::PostToolUse(input) => PostToolUse::context(input),
            AnyInput::Stop(input) => Stop::context(input),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invocation(json: &str) -> RawInvocation {
        RawInvocation::parse(json.as_bytes().to_vec()).unwrap()
    }

    #[test]
    fn contract_identities_point_at_the_current_snapshot() {
        assert_eq!(SNAPSHOT.as_str(), "docs-2026-09-30-r1");
        for contract in [
            PreInvocation::CONTRACT,
            PostInvocation::CONTRACT,
            PreToolUse::CONTRACT,
            PostToolUse::CONTRACT,
            Stop::CONTRACT,
        ] {
            assert!(
                contract
                    .as_str()
                    .starts_with("antigravity/docs-2026-09-30-r1/"),
                "{contract:?}"
            );
        }
    }

    #[test]
    fn invocation_events_are_model_category() {
        assert_eq!(PreInvocation::CATEGORY, EventCategory::Model);
        assert_eq!(PostInvocation::CATEGORY, EventCategory::Model);
    }

    #[test]
    fn parses_without_fabricated_discriminator_and_retains_unknown_fields() {
        let raw = invocation(
            r#"{"conversationId":"c1","workspacePaths":["/repo"],"transcriptPath":"/tmp/t.jsonl","artifactDirectoryPath":"/tmp/a","invocationNum":0,"initialNumSteps":0,"future":true}"#,
        );
        let input = PreInvocation::parse(&raw).unwrap();
        assert_eq!(input.extra["future"], true);
        assert_eq!(input.model_name, None);
        assert_eq!(
            PreInvocation::context(&input)
                .session_boundary
                .unwrap()
                .kind,
            hookkit_core::SessionBoundaryKind::InvocationStart
        );
    }

    #[test]
    fn model_name_is_typed_and_not_left_in_extra() {
        let raw = invocation(
            r#"{"conversationId":"c1","workspacePaths":["/repo"],"transcriptPath":"/tmp/t.jsonl","artifactDirectoryPath":"/tmp/a","invocationNum":1,"initialNumSteps":2,"modelName":"gemini-3.6-flash-medium"}"#,
        );
        let input = PostInvocation::parse(&raw).unwrap();
        assert_eq!(input.model_name.as_deref(), Some("gemini-3.6-flash-medium"));
        assert!(!input.extra.contains_key("modelName"));
        assert!(PostInvocation::context(&input).session_boundary.is_none());

        let invalid = invocation(
            r#"{"conversationId":"c1","workspacePaths":["/repo"],"transcriptPath":"/tmp/t.jsonl","artifactDirectoryPath":"/tmp/a","invocationNum":1,"initialNumSteps":2,"modelName":36}"#,
        );
        assert!(PostInvocation::parse(&invalid).is_err());
    }

    #[test]
    fn empty_workspace_is_accepted_by_every_native_parser() {
        let model = invocation(
            r#"{"conversationId":"c1","workspacePaths":[],"transcriptPath":"/tmp/t.jsonl","artifactDirectoryPath":"/tmp/a","invocationNum":0,"initialNumSteps":0}"#,
        );
        let pre = PreInvocation::parse(&model).unwrap();
        assert!(pre.workspace_paths.is_empty());
        assert!(PreInvocation::context(&pre).workspace_roots.is_empty());
        PostInvocation::parse(&model).unwrap();

        let tool = invocation(
            r#"{"conversationId":"c1","workspacePaths":[],"transcriptPath":"/tmp/t.jsonl","artifactDirectoryPath":"/tmp/a","toolCall":{"name":"run_command","args":{}},"stepIdx":0}"#,
        );
        PreToolUse::parse(&tool).unwrap();
        PostToolUse::parse(&tool).unwrap();

        let stop = invocation(
            r#"{"conversationId":"c1","workspacePaths":[],"transcriptPath":"/tmp/t.jsonl","artifactDirectoryPath":"/tmp/a","executionNum":0,"terminationReason":"model_stop","fullyIdle":true}"#,
        );
        let input = Stop::parse(&stop).unwrap();
        assert!(Stop::context(&input).workspace_roots.is_empty());
    }

    #[test]
    fn missing_workspace_field_is_still_rejected() {
        let raw = invocation(
            r#"{"conversationId":"c1","transcriptPath":"/tmp/t.jsonl","artifactDirectoryPath":"/tmp/a","invocationNum":0,"initialNumSteps":0}"#,
        );
        assert!(PreInvocation::parse(&raw).is_err());
        assert!(PostInvocation::parse(&raw).is_err());
    }

    #[test]
    fn inject_output_matches_exact_catalog_bytes() {
        let output = PreInvocationOutput::inject(InjectStep::ephemeral_message("Remember to lint"));
        let emission = PreInvocation::emit(output).unwrap();
        assert_eq!(
            emission.stdout(),
            br#"{"injectSteps":[{"ephemeralMessage":"Remember to lint"}]}"#
        );
        assert!(emission.stderr().is_empty());
        assert_eq!(emission.exit_code(), 0);
    }

    #[test]
    fn pre_invocation_no_op_is_an_empty_object() {
        let emission = PreInvocation::emit(PreInvocationOutput::no_op()).unwrap();
        assert_eq!(emission.stdout(), b"{}");
    }

    #[test]
    fn duplicate_permission_overrides_are_deduplicated_not_rejected() {
        // Regression: a duplicate override used to fail emission, which
        // turned a deny into a hook failure with undocumented behavior.
        let emission = PreToolUse::emit(PreToolUseOutput {
            decision: ToolDecision::Deny,
            reason: Some("blocked".into()),
            permission_overrides: vec![
                "read_file(/x)".into(),
                "command(ls)".into(),
                "read_file(/x)".into(),
            ],
        })
        .unwrap();
        assert_eq!(
            emission.stdout(),
            br#"{"decision":"deny","reason":"blocked","permissionOverrides":["read_file(/x)","command(ls)"]}"#
        );

        let built = PreToolUseOutput::ask()
            .with_permission_override("command(npm test)")
            .with_permission_overrides(["command(npm test)", "read_file(/y)"]);
        assert_eq!(
            built.permission_overrides,
            vec!["command(npm test)".to_owned(), "read_file(/y)".to_owned()]
        );
    }

    #[test]
    fn empty_stop_decision_is_rejected_at_emission() {
        assert!(Stop::emit(StopOutput::new(StopDecision::Other(String::new()))).is_err());
        assert!(Stop::emit(StopOutput::new("")).is_err());
    }

    #[test]
    fn continue_look_alikes_are_rejected_instead_of_silently_allowing_the_stop() {
        // Regression: `"Continue"` used to emit successfully, and Antigravity
        // treats it as allowing the stop while dropping the reason.
        for lookalike in ["Continue", "CONTINUE", " continue", "continue\n"] {
            let output = StopOutput::new(lookalike).with_reason("run tests first");
            assert_eq!(output.decision, StopDecision::Other(lookalike.to_owned()));
            assert!(Stop::emit(output).is_err(), "{lookalike:?}");
        }
        let exact = Stop::emit(StopOutput::new(StopDecision::Other("continue".into()))).unwrap();
        assert_eq!(exact.stdout(), br#"{"decision":"continue"}"#);
    }

    #[test]
    fn typed_stop_constructors_emit_documented_values() {
        assert_eq!(
            Stop::emit(StopOutput::continue_with("Not done yet"))
                .unwrap()
                .stdout(),
            br#"{"decision":"continue","reason":"Not done yet"}"#
        );
        assert_eq!(
            Stop::emit(StopOutput::allow_stop()).unwrap().stdout(),
            br#"{"decision":"stop"}"#
        );
        assert_eq!(
            Stop::emit(StopOutput::new(StopDecision::Other("done".into())))
                .unwrap()
                .stdout(),
            br#"{"decision":"done"}"#
        );
        assert_eq!(StopDecision::from("continue"), StopDecision::Continue);
        assert_eq!(StopDecision::from(String::from("stop")), StopDecision::Stop);
        assert!(StopDecision::Continue.prevents_stop());
        assert!(!StopDecision::from("Continue").prevents_stop());
    }

    #[test]
    fn grant_aware_tool_decision_matches_the_native_wire_value() {
        let emission = PreToolUse::emit(PreToolUseOutput::deny_unless_prior_grant()).unwrap();

        assert_eq!(
            emission.stdout(),
            br#"{"decision":"deny_unless_prior_grant"}"#
        );
    }

    #[test]
    fn tool_decision_constructors_cover_every_documented_value() {
        for (output, wire) in [
            (PreToolUseOutput::allow(), "allow"),
            (PreToolUseOutput::deny(), "deny"),
            (PreToolUseOutput::ask(), "ask"),
            (PreToolUseOutput::force_ask(), "force_ask"),
            (
                PreToolUseOutput::deny_unless_prior_grant(),
                "deny_unless_prior_grant",
            ),
        ] {
            assert_eq!(output.decision.as_str(), wire);
            let emission = PreToolUse::emit(output).unwrap();
            let value: serde_json::Value = serde_json::from_slice(emission.stdout()).unwrap();
            assert_eq!(value, serde_json::json!({ "decision": wire }));
        }
    }

    #[test]
    fn post_tool_use_types_the_originating_call_when_present() {
        let missing = invocation(
            r#"{"conversationId":"c1","workspacePaths":["/repo"],"transcriptPath":"/tmp/t.jsonl","artifactDirectoryPath":"/tmp/a","stepIdx":0}"#,
        );
        let input = PostToolUse::parse(&missing).unwrap();
        assert!(input.tool_call.is_none());
        assert!(!input.failed());

        let present = invocation(
            r#"{"conversationId":"c1","workspacePaths":["/repo"],"transcriptPath":"/tmp/t.jsonl","artifactDirectoryPath":"/tmp/a","toolCall":{"name":"run_command","args":{"CommandLine":"cargo test"}},"stepIdx":0}"#,
        );
        let input = PostToolUse::parse(&present).unwrap();
        let call = input.tool_call.as_ref().unwrap();
        assert_eq!(call.name, "run_command");
        assert_eq!(call.args["CommandLine"], "cargo test");

        // A present toolCall still needs its name, as the snapshot's
        // `invalid-tool-call` negative fixture pins.
        let malformed = invocation(
            r#"{"conversationId":"c1","workspacePaths":["/repo"],"transcriptPath":"/tmp/t.jsonl","artifactDirectoryPath":"/tmp/a","toolCall":{"args":{"CommandLine":"cargo test"}},"stepIdx":0}"#,
        );
        assert!(PostToolUse::parse(&malformed).is_err());
    }

    #[test]
    fn missing_or_null_tool_arguments_read_as_an_empty_object_and_round_trip() {
        // Regression: `"args": null`, how a Go nil map encodes a call without
        // arguments, and a call that leaves `args` out (list_permissions
        // takes none) failed every PreToolUse and PostToolUse hook.
        let json = r#"{"conversationId":"c1","workspacePaths":["/repo"],"transcriptPath":"/tmp/t.jsonl","artifactDirectoryPath":"/tmp/a","toolCall":{"name":"list_permissions","args":null},"stepIdx":2}"#;
        let null = invocation(json);
        let empty = invocation(&json.replace("null", "{}"));
        let missing = invocation(&json.replace(r#","args":null"#, ""));
        let mut parsed = Vec::new();
        for raw in [&null, &empty, &missing] {
            let pre = PreToolUse::parse(raw).unwrap();
            assert_eq!(pre.tool_call.name, "list_permissions");
            assert!(pre.tool_call.args.is_empty());
            assert_eq!(serde_json::to_value(&pre).unwrap(), *raw.json());
            let post = PostToolUse::parse(raw).unwrap();
            let call = post.tool_call.as_ref().unwrap();
            assert!(call.args.is_empty());
            assert_eq!(serde_json::to_value(&post).unwrap(), *raw.json());
            parsed.push(pre);
        }
        // The three wire forms stay distinct.
        assert_ne!(parsed[0], parsed[1]);
        assert_ne!(parsed[0], parsed[2]);
        assert_ne!(parsed[1], parsed[2]);

        // Arguments added to a call that arrived without any are serialized.
        for mut edited in parsed {
            edited.tool_call.args.insert("Scope".into(), "all".into());
            assert_eq!(
                serde_json::to_value(&edited).unwrap()["toolCall"]["args"],
                serde_json::json!({"Scope": "all"})
            );
        }

        // A value that is neither an object nor null is still rejected.
        for args in [r#""args":[]"#, r#""args":"x""#] {
            let malformed = invocation(&json.replace(r#""args":null"#, args));
            assert!(PreToolUse::parse(&malformed).is_err(), "{args}");
            assert!(PostToolUse::parse(&malformed).is_err(), "{args}");
        }
    }

    #[test]
    fn parse_failures_are_invalid_input_for_the_selected_event() {
        // Regression: every Antigravity parse failure used to be `InvalidJson`,
        // unlike a Claude Code or Codex payload that violates a known event.
        let raw = invocation(
            r#"{"conversationId":"c1","workspacePaths":["/repo"],"transcriptPath":"/tmp/t.jsonl","artifactDirectoryPath":"/tmp/a","initialNumSteps":0}"#,
        );
        for (result, expected) in [
            (PreInvocation::parse(&raw).map(|_| ()), PreInvocation::EVENT),
            (
                PostInvocation::parse(&raw).map(|_| ()),
                PostInvocation::EVENT,
            ),
            (PreToolUse::parse(&raw).map(|_| ()), PreToolUse::EVENT),
            (PostToolUse::parse(&raw).map(|_| ()), PostToolUse::EVENT),
            (Stop::parse(&raw).map(|_| ()), Stop::EVENT),
        ] {
            assert!(
                matches!(
                    &result,
                    Err(hookkit_core::HookkitError::InvalidInputForHint { event, message })
                        if *event == expected && message.contains("missing field")
                ),
                "{expected}: {result:?}"
            );
        }
    }

    #[test]
    fn selectors_and_sum_types_are_hashable_and_comparable() {
        let selected: BTreeSet<_> = [Event::Stop, Event::Stop, Event::PreToolUse]
            .iter()
            .map(EventSelector::event_id)
            .collect();
        assert_eq!(selected.len(), 2);
        let hashed: std::collections::HashSet<Event> = [Event::PreInvocation, Event::PreInvocation]
            .into_iter()
            .collect();
        assert_eq!(hashed.len(), 1);
        assert_eq!(
            AnyCommandOutput::Stop(StopOutput::allow_stop()),
            AnyCommandOutput::Stop(StopOutput::allow_stop())
        );
        let raw = invocation(
            r#"{"conversationId":"c1","workspacePaths":[],"transcriptPath":"/tmp/t.jsonl","artifactDirectoryPath":"/tmp/a","invocationNum":0,"initialNumSteps":0}"#,
        );
        assert_eq!(
            Antigravity::decode(&PreInvocation::EVENT, &raw).unwrap(),
            AnyInput::PreInvocation(PreInvocation::parse(&raw).unwrap())
        );
    }

    #[test]
    fn empty_error_means_success_but_is_retained_verbatim() {
        let success = invocation(
            r#"{"conversationId":"c1","workspacePaths":["/repo"],"transcriptPath":"/tmp/t.jsonl","artifactDirectoryPath":"/tmp/a","toolCall":{"name":"view_file","args":{"AbsolutePath":"/repo/README.md"}},"stepIdx":6,"error":""}"#,
        );
        let input = PostToolUse::parse(&success).unwrap();
        assert_eq!(input.error.as_deref(), Some(""));
        assert_eq!(input.error_message(), None);
        assert!(!input.failed());
        assert_eq!(
            serde_json::to_value(&input).unwrap(),
            *success.json(),
            "the raw empty string must round-trip"
        );

        let failure = invocation(
            r#"{"conversationId":"c1","workspacePaths":["/repo"],"transcriptPath":"/tmp/t.jsonl","artifactDirectoryPath":"/tmp/a","toolCall":{"name":"run_command","args":{}},"stepIdx":5,"error":"exit status 1"}"#,
        );
        let input = PostToolUse::parse(&failure).unwrap();
        assert_eq!(input.error_message(), Some("exit status 1"));
        assert!(input.failed());

        let stop = invocation(
            r#"{"conversationId":"c1","workspacePaths":["/repo"],"transcriptPath":"/tmp/t.jsonl","artifactDirectoryPath":"/tmp/a","executionNum":1,"terminationReason":"model_stop","error":"  ","fullyIdle":true}"#,
        );
        assert_eq!(Stop::parse(&stop).unwrap().error_message(), None);
    }

    #[test]
    fn absent_optional_fields_are_not_serialized_as_null() {
        let raw = invocation(
            r#"{"conversationId":"c1","workspacePaths":["/repo"],"transcriptPath":"/tmp/t.jsonl","artifactDirectoryPath":"/tmp/a","stepIdx":0}"#,
        );
        let input = PostToolUse::parse(&raw).unwrap();
        assert_eq!(serde_json::to_value(&input).unwrap(), *raw.json());
    }

    #[test]
    fn termination_reason_is_open_and_lossless() {
        assert_eq!(
            TerminationReason::from("model_stop"),
            TerminationReason::ModelStop
        );
        assert_eq!(
            TerminationReason::from(String::from("max_steps_exceeded")),
            TerminationReason::MaxStepsExceeded
        );
        let raw = invocation(
            r#"{"conversationId":"c1","workspacePaths":["/repo"],"transcriptPath":"/tmp/t.jsonl","artifactDirectoryPath":"/tmp/a","executionNum":3,"terminationReason":"user_cancelled","fullyIdle":false}"#,
        );
        let input = Stop::parse(&raw).unwrap();
        assert_eq!(
            input.termination_reason,
            TerminationReason::Unknown("user_cancelled".into())
        );
        assert_eq!(input.termination_reason.as_str(), "user_cancelled");
        assert_eq!(input.termination_reason.to_string(), "user_cancelled");
        assert_eq!(serde_json::to_value(&input).unwrap(), *raw.json());
    }

    #[test]
    fn post_tool_use_protocol_stderr_preserves_exact_empty_object_stdout() {
        let output = PostToolUseOutput::no_op()
            .with_protocol_stderr("lowering details were recorded")
            .unwrap();
        let emission = PostToolUse::emit(output).unwrap();

        assert_eq!(emission.stdout(), b"{}");
        assert_eq!(emission.stderr(), b"lowering details were recorded");
        assert_eq!(emission.exit_code(), 0);
    }

    #[test]
    fn post_tool_use_protocol_stderr_can_only_be_added_once() {
        let output = PostToolUseOutput::default()
            .with_protocol_stderr("first")
            .unwrap();
        assert!(output.with_protocol_stderr("second").is_err());
    }

    #[test]
    fn post_invocation_represents_typed_steps_and_explicit_default_behavior() {
        let emission = PostInvocation::emit(
            PostInvocationOutput::inject(InjectStep::user_message("Run one more check."))
                .with_termination_behavior(TerminationBehavior::Default),
        )
        .unwrap();

        assert_eq!(
            emission.stdout(),
            br#"{"injectSteps":[{"userMessage":"Run one more check."}],"terminationBehavior":""}"#
        );
        assert_eq!(
            PostInvocation::emit(PostInvocationOutput::no_op())
                .unwrap()
                .stdout(),
            br#"{"injectSteps":[]}"#
        );
        assert_eq!(
            TerminationBehavior::ForceContinue.as_str(),
            "force_continue"
        );
    }
}
