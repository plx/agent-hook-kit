//! Implemented Codex native event contracts and dynamic harness adapter.

use hookkit_core::{
    ContractId, EventCategory, EventId, EventSelector, EventSpec, HarnessId, HarnessSpec,
    IdentificationDescriptor, NativeContext, ProcessEmission, RawInvocation, SessionId, SnapshotId,
    ToolCallId, TurnId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::CodexCommandEnvironment;
use crate::wire::{self, contract_id, open_string_enum, snapshot_literal};

/// Codex source snapshot implemented by this crate: openai/codex `rust-v0.159.2`
/// (`ff6aec96948b70d94983af2641a6b67c94faeff5`).
pub const SNAPSHOT_ID: SnapshotId = SnapshotId::builtin(snapshot_literal!());

/// Returns every Codex event with a native command implementation.
pub fn events() -> Vec<hookkit_core::NativeEventDescriptor> {
    let mut events = vec![
        hookkit_core::NativeEventDescriptor::command::<PreToolUse>(&[
            "no-op",
            "deny-json",
            "deny-stderr",
        ]),
        hookkit_core::NativeEventDescriptor::command::<PostToolUse>(&[
            "structured",
            "no-op",
            "exit-2",
        ]),
    ];
    events.extend(crate::catalog::events());
    events
}

/// Returns discriminator-based identification metadata for the Codex snapshot.
pub fn identification_descriptors() -> Vec<IdentificationDescriptor> {
    let mut descriptors = vec![
        IdentificationDescriptor::definitive::<PreToolUse>("/hook_event_name", "PreToolUse"),
        IdentificationDescriptor::definitive::<PostToolUse>("/hook_event_name", "PostToolUse"),
    ];
    descriptors.extend(crate::catalog::identification_descriptors());
    descriptors
}

open_string_enum! {
    /// Codex permission policy active for a hook event.
    ///
    /// Since Codex 0.156.0 the tool events (`PreToolUse`, `PermissionRequest`,
    /// `PostToolUse`) report the mode captured for the individual step, so it
    /// can change within one turn.
    pub enum PermissionMode {
        /// Use the default interactive permission policy.
        Default => "default",
        /// Automatically accept file-edit operations.
        AcceptEdits => "acceptEdits",
        /// Restrict the agent to planning behavior.
        Plan => "plan",
        /// Do not prompt for otherwise disallowed operations.
        DontAsk => "dontAsk",
        /// Bypass normal permission checks.
        BypassPermissions => "bypassPermissions",
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
/// Native Codex input observed before a tool invocation.
///
/// Since Codex 0.157.0 `cwd` (and the hook's process working directory) is the
/// step's local-environment directory when one is selected, falling back to
/// the turn directory, so it can differ from the `cwd` of turn-scoped events.
pub struct PreToolUseInput {
    /// Native session identifier.
    pub session_id: String,
    /// Transcript path; the key is required but its value may be JSON `null`.
    pub transcript_path: Option<hookkit_core::Utf8PathBuf>,
    /// Step-local working directory of the tool call.
    pub cwd: hookkit_core::Utf8PathBuf,
    /// Authoritative native event discriminator.
    pub hook_event_name: String,
    /// Model captured for the step that issued the tool call.
    pub model: String,
    /// Native turn identifier.
    pub turn_id: String,
    /// Permission policy captured for the step that issued the tool call.
    pub permission_mode: PermissionMode,
    /// Harness-native tool name.
    pub tool_name: String,
    /// Native tool-call identifier.
    pub tool_use_id: String,
    /// Tool arguments in their native JSON shape.
    pub tool_input: serde_json::Value,
    /// Subagent identifier when the tool runs on behalf of a subagent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Subagent type when the tool runs on behalf of a subagent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    /// Unknown protocol fields retained for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Native response from a Codex pre-tool command hook.
///
/// A decision constructor ([`Self::deny`], [`Self::rewrite`], [`Self::block`])
/// or [`Self::no_op`] can be combined with an agent-facing
/// [`Self::with_additional_context`] and a user-facing
/// [`Self::with_system_message`], matching what Codex's parser accepts. Codex
/// rejects `continue: false`, `stopReason`, `suppressOutput`,
/// `permissionDecision: "ask"`, a bare `allow`, and legacy
/// `decision: "approve"` for this event, so none can be built here.
///
/// Codex ignores every decision from a handler configured with
/// `async: true`; such handlers only deliver additional context and system
/// messages.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PreToolUseOutput {
    /// Emit no stdout and exit successfully; the tool call proceeds.
    NoOp,
    /// Emit one structured JSON response built by this type's constructors.
    Structured(StructuredPreToolUseOutput),
    /// Block by writing a required message to stderr and exiting with code 2.
    DenyStderr {
        /// Stderr message; it must be non-empty after trimming, which is
        /// checked during emission because Codex ignores a blank exit-2 block.
        message: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq)]
/// Structured Codex pre-tool response built through [`PreToolUseOutput`].
///
/// Fields are private so callers cannot construct combinations that Codex
/// rejects. A value with no members emits empty stdout.
pub struct StructuredPreToolUseOutput {
    decision: Option<PreToolUseDecision>,
    additional_context: Option<String>,
    system_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum PreToolUseDecision {
    Deny(String),
    Block(String),
    Rewrite(serde_json::Map<String, serde_json::Value>),
}

impl PreToolUseOutput {
    /// Creates an empty successful response.
    pub fn no_op() -> Self {
        Self::NoOp
    }

    /// Creates a legacy top-level `decision: "block"` response.
    ///
    /// The reason must be non-empty after trimming; this is checked during
    /// emission. Prefer [`Self::deny`], the current hook-specific form.
    pub fn block(reason: impl Into<String>) -> Self {
        Self::decision(PreToolUseDecision::Block(reason.into()))
    }

    /// Creates a hook-specific `permissionDecision: "deny"` response.
    ///
    /// The reason must be non-empty after trimming; this is checked during
    /// emission.
    pub fn deny(reason: impl Into<String>) -> Self {
        Self::decision(PreToolUseDecision::Deny(reason.into()))
    }

    /// Creates an allow response that replaces the tool input.
    ///
    /// For Bash and `apply_patch` the replacement must contain a string
    /// `command`; for MCP and other function tools it is the complete
    /// replacement arguments object.
    pub fn rewrite(updated_input: serde_json::Map<String, serde_json::Value>) -> Self {
        Self::decision(PreToolUseDecision::Rewrite(updated_input))
    }

    /// Creates a response that adds agent context without a decision.
    pub fn with_context(context: impl Into<String>) -> Self {
        Self::Structured(StructuredPreToolUseOutput {
            additional_context: Some(context.into()),
            ..StructuredPreToolUseOutput::default()
        })
    }

    /// Creates a nonblocking response that surfaces a user-facing warning.
    pub fn system_message(message: impl Into<String>) -> Self {
        Self::Structured(StructuredPreToolUseOutput {
            system_message: Some(message.into()),
            ..StructuredPreToolUseOutput::default()
        })
    }

    /// Creates a code-2 blocking response with required stderr text.
    ///
    /// The message must be non-empty after trimming; this is checked during
    /// emission.
    pub fn deny_stderr(message: impl Into<String>) -> Self {
        Self::DenyStderr {
            message: message.into(),
        }
    }

    /// Adds agent-facing `hookSpecificOutput.additionalContext`.
    ///
    /// Valid on [`Self::no_op`] and every structured response; stderr-only
    /// responses are rejected because their shape is final.
    pub fn with_additional_context(
        mut self,
        context: impl Into<String>,
    ) -> hookkit_core::Result<Self> {
        self.structured_mut()?.additional_context = Some(context.into());
        Ok(self)
    }

    /// Adds a user-facing top-level `systemMessage`, shown as a warning.
    ///
    /// Valid on [`Self::no_op`] and every structured response; stderr-only
    /// responses are rejected because their shape is final.
    pub fn with_system_message(mut self, message: impl Into<String>) -> hookkit_core::Result<Self> {
        self.structured_mut()?.system_message = Some(message.into());
        Ok(self)
    }

    fn decision(decision: PreToolUseDecision) -> Self {
        Self::Structured(StructuredPreToolUseOutput {
            decision: Some(decision),
            ..StructuredPreToolUseOutput::default()
        })
    }

    fn structured_mut(&mut self) -> hookkit_core::Result<&mut StructuredPreToolUseOutput> {
        if matches!(self, Self::NoOp) {
            *self = Self::Structured(StructuredPreToolUseOutput::default());
        }
        match self {
            Self::Structured(output) => Ok(output),
            Self::NoOp => unreachable!("NoOp is converted to Structured above"),
            Self::DenyStderr { .. } => Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "structured fields cannot be added to a stderr-only PreToolUse block",
            )),
        }
    }
}

impl StructuredPreToolUseOutput {
    fn into_json(self) -> hookkit_core::Result<serde_json::Map<String, serde_json::Value>> {
        let mut output = serde_json::Map::new();
        let mut specific = serde_json::Map::new();
        match self.decision {
            Some(PreToolUseDecision::Deny(reason)) => {
                wire::require_nonblank(
                    &reason,
                    "PreToolUse denial reason must be non-empty after trimming",
                )?;
                specific.insert("permissionDecision".into(), "deny".into());
                specific.insert("permissionDecisionReason".into(), reason.into());
            }
            Some(PreToolUseDecision::Rewrite(updated_input)) => {
                specific.insert("permissionDecision".into(), "allow".into());
                specific.insert("updatedInput".into(), updated_input.into());
            }
            Some(PreToolUseDecision::Block(reason)) => {
                wire::require_nonblank(
                    &reason,
                    "PreToolUse block reason must be non-empty after trimming",
                )?;
                output.insert("decision".into(), "block".into());
                output.insert("reason".into(), reason.into());
            }
            None => {}
        }
        if let Some(context) = self.additional_context {
            specific.insert("additionalContext".into(), context.into());
        }
        if !specific.is_empty() {
            specific.insert("hookEventName".into(), "PreToolUse".into());
            output.insert("hookSpecificOutput".into(), specific.into());
        }
        if let Some(message) = self.system_message {
            output.insert("systemMessage".into(), message.into());
        }
        Ok(output)
    }
}

/// Native Codex `PreToolUse` command contract.
pub enum PreToolUse {}

impl EventSpec for PreToolUse {
    type Input = PreToolUseInput;
    type CommandEnvironment = CodexCommandEnvironment;
    type CommandOutput = PreToolUseOutput;
    const HARNESS: HarnessId = HarnessId::CODEX;
    const SNAPSHOT: SnapshotId = SNAPSHOT_ID;
    const EVENT: EventId = EventId::builtin(HarnessId::CODEX, "PreToolUse");
    const CATEGORY: EventCategory = EventCategory::Tool;
    const CONTRACT: ContractId = contract_id!("PreToolUse");

    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        require_event(invocation, "PreToolUse")?;
        require_field(invocation, "transcript_path", "PreToolUse")?;
        deserialize_input(invocation, "PreToolUse")
    }

    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        match output {
            PreToolUseOutput::NoOp => Ok(ProcessEmission::command_empty(Self::CONTRACT)),
            PreToolUseOutput::DenyStderr { message } => {
                wire::blocking_stderr(Self::CONTRACT, message)
            }
            PreToolUseOutput::Structured(output) => {
                wire::structured(Self::CONTRACT, &output.into_json()?)
            }
        }
    }

    fn context(input: &Self::Input) -> NativeContext {
        NativeContext {
            workspace_roots: vec![input.cwd.clone()],
            session_id: SessionId::new(&input.session_id).ok(),
            turn_id: TurnId::new(&input.turn_id).ok(),
            transcript_path: input.transcript_path.clone(),
            tool_call_id: ToolCallId::new(&input.tool_use_id).ok(),
            ..NativeContext::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
/// Native Codex input observed after a tool invocation.
///
/// `cwd`, `model`, and `permission_mode` describe the step that ran the tool
/// (see [`PreToolUseInput`]).
pub struct PostToolUseInput {
    /// Native session identifier.
    pub session_id: String,
    /// Transcript path; the key is required but its value may be JSON `null`.
    pub transcript_path: Option<hookkit_core::Utf8PathBuf>,
    /// Step-local working directory of the tool call.
    pub cwd: hookkit_core::Utf8PathBuf,
    /// Authoritative native event discriminator.
    pub hook_event_name: String,
    /// Model captured for the step that issued the tool call.
    pub model: String,
    /// Native turn identifier.
    pub turn_id: String,
    /// Permission policy captured for the step that issued the tool call.
    pub permission_mode: PermissionMode,
    /// Harness-native tool name.
    pub tool_name: String,
    /// Native tool-call identifier.
    pub tool_use_id: String,
    /// Tool arguments in their native JSON shape.
    pub tool_input: serde_json::Value,
    /// Tool result in its native JSON shape.
    pub tool_response: serde_json::Value,
    /// Subagent identifier when the tool ran on behalf of a subagent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Subagent type when the tool ran on behalf of a subagent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    /// Unknown protocol fields retained for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Native response from a Codex post-tool command hook.
///
/// Codex ignores the block decision, `continue: false`, and exit 2 from a
/// handler configured with `async: true`; such handlers only deliver
/// additional context and system messages.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PostToolUseOutput {
    /// Emit no stdout and exit successfully.
    NoOp,
    /// Emit a structured JSON response.
    Structured(StructuredPostToolUseOutput),
    /// Write a required message to stderr and exit with code 2, replacing the
    /// tool result the model sees.
    BlockingError {
        /// Stderr message; it must be non-empty after trimming, which is
        /// checked during emission because Codex ignores a blank exit-2 block.
        message: String,
    },
    /// Add UTF-8 stderr to one otherwise successful response.
    ///
    /// Codex never reads stderr from a hook that exits 0, so this text is
    /// discarded by the harness; see [`PostToolUseOutput::with_protocol_stderr`].
    WithProtocolStderr {
        /// Successful response to encode as stdout.
        output: Box<PostToolUseOutput>,
        /// UTF-8 stderr bytes, validated during emission.
        stderr: Vec<u8>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
/// Structured Codex post-tool response built through [`PostToolUseOutput`].
///
/// Fields are private so callers cannot construct combinations that bypass the
/// builder's ordering checks. A value with no members emits empty stdout.
pub struct StructuredPostToolUseOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    decision: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(rename = "continue", skip_serializing_if = "Option::is_none")]
    continue_session: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hook_specific_output: Option<PostToolUseSpecific>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct PostToolUseSpecific {
    hook_event_name: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    additional_context: Option<String>,
}

impl PostToolUseOutput {
    /// Creates an empty successful response (no stdout).
    pub fn no_op() -> Self {
        Self::NoOp
    }

    /// Creates a structured response that appends agent context.
    pub fn with_context(context: impl Into<String>) -> Self {
        Self::Structured(StructuredPostToolUseOutput {
            hook_specific_output: Some(PostToolUseSpecific {
                hook_event_name: "PostToolUse",
                additional_context: Some(context.into()),
            }),
            ..StructuredPostToolUseOutput::default()
        })
    }

    /// Creates a structured `decision: "block"` response, Codex's JSON form
    /// for replacing the tool result with feedback for the model.
    ///
    /// The reason must be non-empty after trimming unless `continue: false`
    /// is also set; this is checked during emission. With `continue: false`
    /// a blank block is left out of the JSON: it would change nothing but
    /// make Codex drop the response's additional context.
    pub fn block(reason: impl Into<String>) -> Self {
        Self::Structured(StructuredPostToolUseOutput {
            decision: Some("block"),
            reason: Some(reason.into()),
            ..StructuredPostToolUseOutput::default()
        })
    }

    /// Adds a `decision: "block"` to a successful structured response.
    ///
    /// `NoOp` is promoted to an empty structured response. Blocking-error and
    /// stderr-wrapped outputs are rejected because their structure is final.
    /// The reason must be non-empty after trimming unless `continue: false` is
    /// also set, as for [`Self::block`]; this is checked during emission.
    pub fn with_block(mut self, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        let output = self.structured_mut()?;
        output.decision = Some("block");
        output.reason = Some(reason.into());
        Ok(self)
    }

    /// Adds agent-facing `hookSpecificOutput.additionalContext` to a
    /// successful structured response.
    ///
    /// Codex discards the context of a response whose block reason is blank,
    /// even under `continue: false`, so emission never sends such a block.
    pub fn with_additional_context(
        mut self,
        context: impl Into<String>,
    ) -> hookkit_core::Result<Self> {
        self.structured_mut()?.hook_specific_output = Some(PostToolUseSpecific {
            hook_event_name: "PostToolUse",
            additional_context: Some(context.into()),
        });
        Ok(self)
    }

    /// Creates a code-2 response with required stderr text.
    ///
    /// The message must be non-empty after trimming; this is checked during
    /// emission.
    pub fn blocking_error(message: impl Into<String>) -> Self {
        Self::BlockingError {
            message: message.into(),
        }
    }

    /// Adds stderr to a successful (exit-0) response.
    ///
    /// Codex never reads stderr from a hook that exits 0, so the text reaches
    /// neither the user nor the model; it is only useful to tooling that
    /// captures the process's stderr. Use [`Self::with_system_message`] for a
    /// user-visible notice. A blocking error or an already wrapped response is
    /// rejected. The text must be valid UTF-8 when emitted.
    pub fn with_protocol_stderr(self, stderr: impl Into<String>) -> hookkit_core::Result<Self> {
        if matches!(
            self,
            Self::BlockingError { .. } | Self::WithProtocolStderr { .. }
        ) {
            return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "protocol stderr can only wrap a successful output once",
            ));
        }
        Ok(Self::WithProtocolStderr {
            output: Box::new(self),
            stderr: stderr.into().into_bytes(),
        })
    }

    /// Sets Codex's top-level `continue` control on a structured response.
    ///
    /// For `PostToolUse`, `continue: false` neither ends the turn nor undoes
    /// the tool call. Codex replaces the tool result the model sees with the
    /// block `reason` when it is non-blank, else [`Self::with_stop_reason`],
    /// else `PostToolUse hook stopped execution`, and the model carries on
    /// from there; code mode's tool promise still resolves. It takes
    /// precedence over a block decision. To halt, deny the call in
    /// `PreToolUse`, or use `continue: false` on `Stop` or
    /// `UserPromptSubmit`.
    pub fn with_continue(mut self, continue_session: bool) -> hookkit_core::Result<Self> {
        self.structured_mut()?.continue_session = Some(continue_session);
        Ok(self)
    }

    /// Sets the top-level `stopReason`, used when `continue` is `false`.
    ///
    /// Codex records it as the run's stop text and, when the response has no
    /// non-blank block reason, it becomes the tool result the model sees.
    pub fn with_stop_reason(mut self, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        self.structured_mut()?.stop_reason = Some(reason.into());
        Ok(self)
    }

    /// Sets a user-facing top-level `systemMessage`, shown as a warning.
    pub fn with_system_message(mut self, message: impl Into<String>) -> hookkit_core::Result<Self> {
        self.structured_mut()?.system_message = Some(message.into());
        Ok(self)
    }

    fn structured_mut(&mut self) -> hookkit_core::Result<&mut StructuredPostToolUseOutput> {
        if matches!(self, Self::NoOp) {
            *self = Self::Structured(StructuredPostToolUseOutput::default());
        }
        Ok(match self {
            Self::Structured(output) => output,
            Self::NoOp => unreachable!("NoOp is converted to Structured above"),
            Self::BlockingError { .. } | Self::WithProtocolStderr { .. } => {
                return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                    "structured fields must be added before blocking/protocol-stderr output",
                ));
            }
        })
    }
}

/// Native Codex `PostToolUse` command contract.
pub enum PostToolUse {}

impl EventSpec for PostToolUse {
    type Input = PostToolUseInput;
    type CommandEnvironment = CodexCommandEnvironment;
    type CommandOutput = PostToolUseOutput;
    const HARNESS: HarnessId = HarnessId::CODEX;
    const SNAPSHOT: SnapshotId = SNAPSHOT_ID;
    const EVENT: EventId = EventId::builtin(HarnessId::CODEX, "PostToolUse");
    const CATEGORY: EventCategory = EventCategory::Tool;
    const CONTRACT: ContractId = contract_id!("PostToolUse");

    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        require_event(invocation, "PostToolUse")?;
        require_field(invocation, "transcript_path", "PostToolUse")?;
        deserialize_input(invocation, "PostToolUse")
    }

    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        match output {
            PostToolUseOutput::NoOp => Ok(ProcessEmission::command_empty(Self::CONTRACT)),
            PostToolUseOutput::Structured(output) => {
                let serde_json::Value::Object(output) = serde_json::to_value(&output)? else {
                    unreachable!("StructuredPostToolUseOutput serializes as an object")
                };
                wire::structured(Self::CONTRACT, &output)
            }
            PostToolUseOutput::BlockingError { message } => {
                wire::blocking_stderr(Self::CONTRACT, message)
            }
            PostToolUseOutput::WithProtocolStderr { output, stderr } => {
                if matches!(
                    output.as_ref(),
                    PostToolUseOutput::BlockingError { .. }
                        | PostToolUseOutput::WithProtocolStderr { .. }
                ) || std::str::from_utf8(&stderr).is_err()
                {
                    return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                        "protocol stderr wrapper must contain one successful output and UTF-8 text",
                    ));
                }
                let emission = Self::emit(*output)?;
                if emission.exit_code() != 0 {
                    return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                        "protocol stderr cannot wrap a nonzero output",
                    ));
                }
                Ok(ProcessEmission::command_unchecked(
                    Self::CONTRACT,
                    emission.stdout().to_vec(),
                    stderr,
                    0,
                ))
            }
        }
    }

    fn context(input: &Self::Input) -> NativeContext {
        NativeContext {
            workspace_roots: vec![input.cwd.clone()],
            session_id: SessionId::new(&input.session_id).ok(),
            turn_id: TurnId::new(&input.turn_id).ok(),
            transcript_path: input.transcript_path.clone(),
            tool_call_id: ToolCallId::new(&input.tool_use_id).ok(),
            ..NativeContext::default()
        }
    }
}

pub(crate) fn require_event(
    invocation: &RawInvocation,
    expected: &'static str,
) -> hookkit_core::Result<()> {
    if invocation
        .json()
        .get("hook_event_name")
        .and_then(serde_json::Value::as_str)
        == Some(expected)
    {
        return Ok(());
    }
    Err(hookkit_core::HookkitError::InvalidForHint {
        harness: HarnessId::CODEX,
        event: EventId::builtin(HarnessId::CODEX, expected),
        message: format!("expected hook_event_name={expected}"),
    })
}

/// Deserializes the typed input of `event` after its discriminator matched.
///
/// Like the Claude and Antigravity parsers, a schema violation in a payload
/// whose event is already known (a missing or mistyped field, whether caught
/// by [`require_field`], a field-type check, or deserialization) is reported
/// as [`hookkit_core::HookkitError::InvalidInputForHint`]; only a
/// discriminator mismatch ([`require_event`]) uses `InvalidForHint`.
pub(crate) fn deserialize_input<T: serde::de::DeserializeOwned>(
    invocation: &RawInvocation,
    event: &'static str,
) -> hookkit_core::Result<T> {
    serde_json::from_value(invocation.json().clone())
        .map_err(|error| invalid_input(event, error.to_string()))
}

/// Rejects input for `event` whose required key is absent, as
/// [`hookkit_core::HookkitError::InvalidInputForHint`] (see
/// [`deserialize_input`]).
pub(crate) fn require_field(
    invocation: &RawInvocation,
    field: &str,
    event: &'static str,
) -> hookkit_core::Result<()> {
    if invocation.json().get(field).is_some() {
        return Ok(());
    }
    Err(invalid_input(
        event,
        format!("missing required field {field}"),
    ))
}

/// Builds the error for a payload that violates the known event's schema.
pub(crate) fn invalid_input(event: &'static str, message: String) -> hookkit_core::HookkitError {
    hookkit_core::HookkitError::InvalidInputForHint {
        event: EventId::builtin(HarnessId::CODEX, event),
        message,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
/// Compile-time selector for an implemented Codex event.
pub enum Event {
    /// Selects [`PreToolUse`].
    PreToolUse,
    /// Selects [`PostToolUse`].
    PostToolUse,
    /// Selects [`crate::catalog::Interrupt`].
    Interrupt,
    /// Selects [`crate::catalog::PermissionRequest`].
    PermissionRequest,
    /// Selects [`crate::catalog::PostCompact`].
    PostCompact,
    /// Selects [`crate::catalog::PreCompact`].
    PreCompact,
    /// Selects [`crate::catalog::SessionStart`].
    SessionStart,
    /// Selects [`crate::catalog::SessionEnd`].
    SessionEnd,
    /// Selects [`crate::catalog::Stop`].
    Stop,
    /// Selects [`crate::catalog::SubagentStart`].
    SubagentStart,
    /// Selects [`crate::catalog::SubagentStop`].
    SubagentStop,
    /// Selects [`crate::catalog::UserPromptSubmit`].
    UserPromptSubmit,
}

impl EventSelector for Event {
    fn event_id(&self) -> EventId {
        let name = match self {
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
            Self::Interrupt => "Interrupt",
            Self::PermissionRequest => "PermissionRequest",
            Self::PostCompact => "PostCompact",
            Self::PreCompact => "PreCompact",
            Self::SessionStart => "SessionStart",
            Self::SessionEnd => "SessionEnd",
            Self::Stop => "Stop",
            Self::SubagentStart => "SubagentStart",
            Self::SubagentStop => "SubagentStop",
            Self::UserPromptSubmit => "UserPromptSubmit",
        };
        EventId::builtin(HarnessId::CODEX, name)
    }
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
/// Lossless sum type over all implemented Codex inputs.
pub enum AnyInput {
    /// A pre-tool input.
    PreToolUse(PreToolUseInput),
    /// A post-tool input.
    PostToolUse(PostToolUseInput),
    /// An input for another implemented catalog event.
    Catalog(crate::catalog::CatalogInput),
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
/// Sum type over all implemented Codex command outputs.
pub enum AnyCommandOutput {
    /// A pre-tool output.
    PreToolUse(PreToolUseOutput),
    /// A post-tool output.
    PostToolUse(PostToolUseOutput),
    /// An output for another implemented catalog event.
    Catalog(crate::catalog::CatalogOutput),
}

impl From<PreToolUseOutput> for AnyCommandOutput {
    fn from(output: PreToolUseOutput) -> Self {
        Self::PreToolUse(output)
    }
}

impl From<PostToolUseOutput> for AnyCommandOutput {
    fn from(output: PostToolUseOutput) -> Self {
        Self::PostToolUse(output)
    }
}

/// Harness adapter implementing the pinned Codex snapshot.
pub enum Codex {}

impl HarnessSpec for Codex {
    type AnyInput = AnyInput;
    type CommandEnvironment = CodexCommandEnvironment;
    type AnyCommandOutput = AnyCommandOutput;
    type EventSelector = Event;

    const ID: HarnessId = HarnessId::CODEX;
    const SNAPSHOT: SnapshotId = SNAPSHOT_ID;

    fn identification_descriptors() -> Vec<IdentificationDescriptor> {
        identification_descriptors()
    }

    fn decode(event: &EventId, raw: &RawInvocation) -> hookkit_core::Result<Self::AnyInput> {
        match event.name() {
            "PreToolUse" => PreToolUse::parse(raw).map(AnyInput::PreToolUse),
            "PostToolUse" => PostToolUse::parse(raw).map(AnyInput::PostToolUse),
            _ => crate::catalog::decode(event, raw)?
                .map(AnyInput::Catalog)
                .ok_or_else(|| hookkit_core::HookkitError::UnrecognizedEvent {
                    harness: Self::ID,
                    message: event.name().to_string(),
                }),
        }
    }

    fn input_event(input: &Self::AnyInput) -> EventId {
        match input {
            AnyInput::PreToolUse(_) => PreToolUse::EVENT,
            AnyInput::PostToolUse(_) => PostToolUse::EVENT,
            AnyInput::Catalog(input) => input.event_id(),
        }
    }

    fn output_event(output: &Self::AnyCommandOutput) -> EventId {
        match output {
            AnyCommandOutput::PreToolUse(_) => PreToolUse::EVENT,
            AnyCommandOutput::PostToolUse(_) => PostToolUse::EVENT,
            AnyCommandOutput::Catalog(output) => output.event_id(),
        }
    }

    fn encode_command_unchecked(
        output: Self::AnyCommandOutput,
    ) -> hookkit_core::Result<ProcessEmission> {
        match output {
            AnyCommandOutput::PreToolUse(output) => PreToolUse::emit(output),
            AnyCommandOutput::PostToolUse(output) => PostToolUse::emit(output),
            AnyCommandOutput::Catalog(output) => output.emit(),
        }
    }

    fn context(input: &Self::AnyInput) -> NativeContext {
        match input {
            AnyInput::PreToolUse(input) => PreToolUse::context(input),
            AnyInput::PostToolUse(input) => PostToolUse::context(input),
            AnyInput::Catalog(input) => input.context(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdout_json(emission: &ProcessEmission) -> serde_json::Value {
        serde_json::from_slice(emission.stdout()).unwrap()
    }

    fn pre_tool_use(permission_mode: &str, extra: &str) -> RawInvocation {
        RawInvocation::parse(
            format!(
                r#"{{"session_id":"s","transcript_path":null,"cwd":"/repo","hook_event_name":"PreToolUse","model":"gpt-test","turn_id":"t","permission_mode":"{permission_mode}","tool_name":"Bash","tool_use_id":"u","tool_input":{{}}{extra}}}"#
            )
            .into_bytes(),
        )
        .unwrap()
    }

    #[test]
    fn contract_ids_name_the_implemented_snapshot() {
        assert_eq!(SNAPSHOT_ID.as_str(), "commit-ff6aec9-r1");
        assert_eq!(
            PreToolUse::CONTRACT.as_str(),
            "codex/commit-ff6aec9-r1/PreToolUse"
        );
        assert_eq!(
            PostToolUse::CONTRACT.as_str(),
            "codex/commit-ff6aec9-r1/PostToolUse"
        );
    }

    #[test]
    fn rewrite_cannot_be_emitted_without_allow() {
        let emission = PreToolUse::emit(PreToolUseOutput::rewrite(serde_json::Map::new())).unwrap();
        let value = stdout_json(&emission);
        assert_eq!(value["hookSpecificOutput"]["permissionDecision"], "allow");
    }

    #[test]
    fn deny_requires_a_nonblank_reason() {
        assert!(PreToolUse::emit(PreToolUseOutput::deny("  ")).is_err());
        assert!(PreToolUse::emit(PreToolUseOutput::block("\t\n")).is_err());
    }

    #[test]
    fn exit_two_stderr_must_be_nonblank_after_trimming() {
        // Codex fails (and does not block) an exit-2 run whose stderr is blank
        // after trimming, so HookKit refuses to emit one.
        assert!(PreToolUse::emit(PreToolUseOutput::deny_stderr(" \n")).is_err());
        assert!(PostToolUse::emit(PostToolUseOutput::blocking_error("\t")).is_err());
        let emission = PreToolUse::emit(PreToolUseOutput::deny_stderr("blocked")).unwrap();
        assert_eq!(emission.stderr(), b"blocked");
        assert_eq!(emission.exit_code(), 2);
    }

    #[test]
    fn pre_tool_use_decisions_combine_with_context_and_system_message() {
        let deny = PreToolUseOutput::deny("use the cleanup skill")
            .with_additional_context("see skills/cleanup")
            .unwrap()
            .with_system_message("rm -rf blocked")
            .unwrap();
        assert_eq!(
            stdout_json(&PreToolUse::emit(deny).unwrap()),
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": "use the cleanup skill",
                    "additionalContext": "see skills/cleanup"
                },
                "systemMessage": "rm -rf blocked"
            })
        );

        let mut updated = serde_json::Map::new();
        updated.insert("command".into(), "cargo test --locked".into());
        let rewrite = PreToolUseOutput::rewrite(updated)
            .with_additional_context("added --locked")
            .unwrap();
        let value = stdout_json(&PreToolUse::emit(rewrite).unwrap());
        assert_eq!(value["hookSpecificOutput"]["permissionDecision"], "allow");
        assert_eq!(
            value["hookSpecificOutput"]["additionalContext"],
            "added --locked"
        );

        // A legacy block stays honored when hookSpecificOutput carries only
        // additionalContext.
        let block = PreToolUseOutput::block("destructive")
            .with_additional_context("why")
            .unwrap();
        assert_eq!(
            stdout_json(&PreToolUse::emit(block).unwrap()),
            serde_json::json!({
                "decision": "block",
                "reason": "destructive",
                "hookSpecificOutput": {"hookEventName": "PreToolUse", "additionalContext": "why"}
            })
        );

        let notice = PreToolUseOutput::no_op()
            .with_system_message("policy checked")
            .unwrap();
        assert_eq!(
            stdout_json(&PreToolUse::emit(notice).unwrap()),
            serde_json::json!({"systemMessage": "policy checked"})
        );
        assert!(
            PreToolUseOutput::deny_stderr("blocked")
                .with_system_message("x")
                .is_err()
        );
    }

    #[test]
    fn empty_outputs_emit_empty_stdout() {
        for emission in [
            PreToolUse::emit(PreToolUseOutput::no_op()).unwrap(),
            PreToolUse::emit(PreToolUseOutput::Structured(Default::default())).unwrap(),
            PostToolUse::emit(PostToolUseOutput::no_op()).unwrap(),
            PostToolUse::emit(PostToolUseOutput::Structured(Default::default())).unwrap(),
        ] {
            assert!(emission.stdout().is_empty());
            assert!(emission.stderr().is_empty());
            assert_eq!(emission.exit_code(), 0);
        }
    }

    #[test]
    fn post_tool_use_parses_canonical_snake_case() {
        let raw = RawInvocation::parse(
            br#"{"session_id":"s","transcript_path":null,"cwd":"/repo","hook_event_name":"PostToolUse","model":"gpt-test","turn_id":"t","permission_mode":"default","tool_name":"Bash","tool_use_id":"call","tool_input":{},"tool_response":{},"future":true}"#.to_vec(),
        )
        .unwrap();
        let input = PostToolUse::parse(&raw).unwrap();
        assert_eq!(input.tool_name, "Bash");
        assert_eq!(input.extra["future"], true);
    }

    #[test]
    fn post_tool_use_context_stamps_exact_discriminator() {
        let emission =
            PostToolUse::emit(PostToolUseOutput::with_context("review changes")).unwrap();
        let value = stdout_json(&emission);
        assert_eq!(value["hookSpecificOutput"]["hookEventName"], "PostToolUse");
        assert_eq!(
            value["hookSpecificOutput"]["additionalContext"],
            "review changes"
        );
    }

    #[test]
    fn post_tool_use_block_requires_a_nonblank_reason_unless_stopping() {
        assert!(PostToolUse::emit(PostToolUseOutput::block("")).is_err());
        assert!(PostToolUse::emit(PostToolUseOutput::no_op().with_block(" ").unwrap()).is_err());
        let stopping = PostToolUseOutput::block("")
            .with_continue(false)
            .unwrap()
            .with_stop_reason("stopping")
            .unwrap();
        let value = stdout_json(&PostToolUse::emit(stopping).unwrap());
        assert_eq!(
            value,
            serde_json::json!({"continue": false, "stopReason": "stopping"})
        );

        // Regression: Codex discards additionalContext when the block reason
        // is blank, even under `continue: false`, so the blank block is left
        // out and the context reaches the model.
        let stopping_with_context = PostToolUseOutput::block(" ")
            .with_continue(false)
            .unwrap()
            .with_additional_context("see skills/lint")
            .unwrap();
        let value = stdout_json(&PostToolUse::emit(stopping_with_context).unwrap());
        assert!(value.get("decision").is_none() && value.get("reason").is_none());
        assert_eq!(value["continue"], false);
        assert_eq!(
            value["hookSpecificOutput"]["additionalContext"],
            "see skills/lint"
        );

        // A non-blank reason stays: it is the model-visible tool result.
        let stopping_with_reason = PostToolUseOutput::block("secret written")
            .with_continue(false)
            .unwrap();
        let value = stdout_json(&PostToolUse::emit(stopping_with_reason).unwrap());
        assert_eq!(value["decision"], "block");
        assert_eq!(value["reason"], "secret written");

        let block_with_context = PostToolUseOutput::block("fix it")
            .with_additional_context("lint failed")
            .unwrap();
        let value = stdout_json(&PostToolUse::emit(block_with_context).unwrap());
        assert_eq!(value["decision"], "block");
        assert_eq!(
            value["hookSpecificOutput"]["additionalContext"],
            "lint failed"
        );
    }

    #[test]
    fn post_tool_use_requires_nullable_transcript_field_to_be_present() {
        let raw = RawInvocation::parse(
            br#"{"session_id":"s","cwd":"/repo","hook_event_name":"PostToolUse","model":"gpt-test","turn_id":"t","permission_mode":"default","tool_name":"Bash","tool_use_id":"call","tool_input":{},"tool_response":{}}"#.to_vec(),
        )
        .unwrap();
        // A schema violation in a known event is reported like the Claude and
        // Antigravity parsers report it; only a discriminator mismatch is an
        // `InvalidForHint`.
        assert!(matches!(
            PostToolUse::parse(&raw),
            Err(hookkit_core::HookkitError::InvalidInputForHint { event, message })
                if event == PostToolUse::EVENT && message == "missing required field transcript_path"
        ));
        assert!(matches!(
            PreToolUse::parse(&raw),
            Err(hookkit_core::HookkitError::InvalidForHint { .. })
        ));
    }

    #[test]
    fn every_schema_violation_of_a_known_event_is_invalid_input_for_the_hint() {
        // Regression: a missing key that only deserialization checks, such as
        // `model`, used to surface as `InvalidJson` while `transcript_path`
        // was `InvalidInputForHint`.
        let without = |field: &str| {
            let mut value = pre_tool_use("default", "").json().clone();
            value.as_object_mut().unwrap().remove(field);
            RawInvocation::parse(serde_json::to_vec(&value).unwrap()).unwrap()
        };
        for field in ["model", "session_id", "cwd", "tool_input", "turn_id"] {
            assert!(
                matches!(
                    PreToolUse::parse(&without(field)),
                    Err(hookkit_core::HookkitError::InvalidInputForHint { event, message })
                        if event == PreToolUse::EVENT && message.contains(field)
                ),
                "{field}"
            );
        }
        let mut mistyped = pre_tool_use("default", "").json().clone();
        mistyped["model"] = 7.into();
        assert!(matches!(
            PreToolUse::parse(
                &RawInvocation::parse(serde_json::to_vec(&mistyped).unwrap()).unwrap()
            ),
            Err(hookkit_core::HookkitError::InvalidInputForHint { .. })
        ));

        let session_end = RawInvocation::parse(
            br#"{"transcript_path":null,"cwd":"/repo","hook_event_name":"SessionEnd","reason":"other"}"#
                .to_vec(),
        )
        .unwrap();
        assert!(matches!(
            crate::catalog::SessionEnd::parse(&session_end),
            Err(hookkit_core::HookkitError::InvalidInputForHint { event, message })
                if event == crate::catalog::SessionEnd::EVENT && message.contains("session_id")
        ));
    }

    #[test]
    fn post_tool_use_protocol_stderr_preserves_json_stdout() {
        let output = PostToolUseOutput::with_context("review changes")
            .with_protocol_stderr("user diagnostic")
            .unwrap();
        let emission = PostToolUse::emit(output).unwrap();
        assert_eq!(emission.stderr(), b"user diagnostic");
        assert_eq!(emission.exit_code(), 0);
        let value = stdout_json(&emission);
        assert_eq!(value["hookSpecificOutput"]["hookEventName"], "PostToolUse");
    }

    #[test]
    fn unknown_permission_modes_parse_and_round_trip() {
        let input = PreToolUse::parse(&pre_tool_use("autoReview", "")).unwrap();
        assert_eq!(
            input.permission_mode,
            PermissionMode::Unknown("autoReview".into())
        );
        assert_eq!(input.permission_mode.as_str(), "autoReview");
        let value = serde_json::to_value(&input).unwrap();
        assert_eq!(value["permission_mode"], "autoReview");

        let input = PreToolUse::parse(&pre_tool_use("plan", "")).unwrap();
        assert_eq!(input.permission_mode, PermissionMode::Plan);
        assert!(input.permission_mode.is_known());
    }

    #[test]
    fn pre_tool_use_types_subagent_fields_and_serializes_losslessly() {
        let input = PreToolUse::parse(&pre_tool_use(
            "default",
            r#","agent_id":"a1","agent_type":"worker","future":1"#,
        ))
        .unwrap();
        assert_eq!(input.agent_id.as_deref(), Some("a1"));
        assert_eq!(input.agent_type.as_deref(), Some("worker"));
        assert!(!input.extra.contains_key("agent_id"));

        let plain = PreToolUse::parse(&pre_tool_use("default", "")).unwrap();
        let value = serde_json::to_value(&plain).unwrap();
        assert!(value.get("agent_id").is_none());
        assert!(value.get("agent_type").is_none());
        assert_eq!(&value, pre_tool_use("default", "").json());
    }

    #[test]
    fn invalid_builder_order_and_raw_protocol_stderr_return_errors() {
        assert!(
            PostToolUseOutput::blocking_error("blocked")
                .with_continue(true)
                .is_err()
        );
        let invalid = PostToolUseOutput::WithProtocolStderr {
            output: Box::new(PostToolUseOutput::no_op()),
            stderr: vec![0xff],
        };
        assert!(PostToolUse::emit(invalid).is_err());
    }

    #[test]
    fn selector_covers_interrupt() {
        assert_eq!(
            Event::Interrupt.event_id(),
            EventId::builtin(HarnessId::CODEX, "Interrupt")
        );
    }
}
