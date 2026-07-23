//! Implemented Codex native event contracts and dynamic harness adapter.

use hookkit_core::{
    ContractId, EventCategory, EventId, EventSelector, EventSpec, HarnessId, HarnessSpec,
    IdentificationDescriptor, NativeContext, ProcessEmission, RawInvocation, SessionId, SnapshotId,
    ToolCallId, TurnId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::CodexCommandEnvironment;

/// Codex source snapshot implemented by this crate.
pub const SNAPSHOT_ID: SnapshotId = SnapshotId::builtin("commit-9e552e9-r2");

/// Returns every Codex event with a native command implementation.
pub fn events() -> Vec<hookkit_core::NativeEventDescriptor> {
    let mut events = vec![
        hookkit_core::NativeEventDescriptor::command::<PreToolUse>(&[
            "no-op",
            "deny-json",
            "deny-stderr",
        ]),
        hookkit_core::NativeEventDescriptor::command::<PostToolUse>(&["structured", "exit-2"]),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Native Codex input observed before a tool invocation.
pub struct PreToolUseInput {
    /// Native session identifier.
    pub session_id: String,
    /// Transcript path; the key is required but its value may be JSON `null`.
    pub transcript_path: Option<hookkit_core::Utf8PathBuf>,
    /// Current workspace directory.
    pub cwd: hookkit_core::Utf8PathBuf,
    /// Authoritative native event discriminator.
    pub hook_event_name: String,
    /// Model configured for the current turn.
    pub model: String,
    /// Native turn identifier.
    pub turn_id: String,
    /// Permission policy active for the tool invocation.
    pub permission_mode: PermissionMode,
    /// Harness-native tool name.
    pub tool_name: String,
    /// Native tool-call identifier.
    pub tool_use_id: String,
    /// Tool arguments in their native JSON shape.
    pub tool_input: serde_json::Value,
    /// Unknown protocol fields retained for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Codex permission policy active for a hook event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    /// Use the default interactive permission policy.
    Default,
    /// Automatically accept file-edit operations.
    AcceptEdits,
    /// Restrict the agent to planning behavior.
    Plan,
    /// Do not prompt for otherwise disallowed operations.
    DontAsk,
    /// Bypass normal permission checks.
    BypassPermissions,
}

/// Native response from a Codex pre-tool command hook.
#[derive(Debug, Clone)]
pub enum PreToolUseOutput {
    /// Emit no stdout and exit successfully.
    NoOp,
    /// Block with the legacy top-level decision shape.
    Block {
        /// Human-readable reason presented by Codex.
        reason: String,
    },
    /// Allow using the hook-specific permission decision shape.
    Allow,
    /// Ask the user for permission.
    Ask {
        /// Optional explanation for the permission prompt.
        reason: Option<String>,
    },
    /// Approve using the legacy top-level decision shape.
    Approve,
    /// Deny using the hook-specific permission decision shape.
    Deny {
        /// Optional human-readable denial reason.
        reason: Option<String>,
    },
    /// Allow the call after replacing its tool input.
    Rewrite {
        /// Complete replacement for the native tool-input object.
        updated_input: serde_json::Map<String, serde_json::Value>,
    },
    /// Add context without making a permission decision.
    AdditionalContext {
        /// Context appended to the agent conversation.
        context: String,
    },
    /// Block by writing a required message to stderr and exiting with code 2.
    DenyStderr {
        /// Non-empty stderr message; emptiness is checked during emission.
        message: String,
    },
}

impl PreToolUseOutput {
    /// Creates an empty successful response.
    pub fn no_op() -> Self {
        Self::NoOp
    }
    /// Creates a legacy top-level block response.
    pub fn block(reason: impl Into<String>) -> Self {
        Self::Block {
            reason: reason.into(),
        }
    }
    /// Creates a hook-specific allow response.
    pub fn allow() -> Self {
        Self::Allow
    }
    /// Creates a hook-specific ask response.
    pub fn ask(reason: Option<String>) -> Self {
        Self::Ask { reason }
    }
    /// Creates a legacy top-level approve response.
    pub fn approve() -> Self {
        Self::Approve
    }
    /// Creates a hook-specific deny response.
    pub fn deny(reason: Option<String>) -> Self {
        Self::Deny { reason }
    }
    /// Creates an allow response that replaces the tool input.
    pub fn rewrite(updated_input: serde_json::Map<String, serde_json::Value>) -> Self {
        Self::Rewrite { updated_input }
    }
    /// Creates a response that adds agent context.
    pub fn with_context(context: impl Into<String>) -> Self {
        Self::AdditionalContext {
            context: context.into(),
        }
    }
    /// Creates a code-2 blocking response with required stderr text.
    pub fn deny_stderr(message: impl Into<String>) -> Self {
        Self::DenyStderr {
            message: message.into(),
        }
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
    const CONTRACT: ContractId = ContractId::builtin("codex/commit-9e552e9-r2/PreToolUse");

    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        require_event(invocation, "PreToolUse")?;
        require_field(invocation, "transcript_path", "PreToolUse")?;
        serde_json::from_value(invocation.json().clone()).map_err(Into::into)
    }

    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        let output = match output {
            PreToolUseOutput::NoOp => {
                return Ok(ProcessEmission::command_empty(Self::CONTRACT));
            }
            PreToolUseOutput::DenyStderr { message } => {
                return ProcessEmission::command_required_stderr(Self::CONTRACT, message, 2);
            }
            output => output,
        };
        let value = match output {
            PreToolUseOutput::NoOp | PreToolUseOutput::DenyStderr { .. } => {
                unreachable!("handled above")
            }
            PreToolUseOutput::Block { reason } => {
                serde_json::json!({"decision":"block","reason":reason})
            }
            PreToolUseOutput::Allow => {
                serde_json::json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow"}})
            }
            PreToolUseOutput::Ask { reason } => {
                let mut specific = serde_json::Map::from_iter([
                    (
                        "hookEventName".into(),
                        serde_json::Value::String("PreToolUse".into()),
                    ),
                    (
                        "permissionDecision".into(),
                        serde_json::Value::String("ask".into()),
                    ),
                ]);
                if let Some(reason) = reason {
                    specific.insert(
                        "permissionDecisionReason".into(),
                        serde_json::Value::String(reason),
                    );
                }
                serde_json::json!({"hookSpecificOutput": specific})
            }
            PreToolUseOutput::Approve => serde_json::json!({"decision":"approve"}),
            PreToolUseOutput::Deny { reason } => {
                let mut specific = serde_json::Map::from_iter([
                    (
                        "hookEventName".into(),
                        serde_json::Value::String("PreToolUse".into()),
                    ),
                    (
                        "permissionDecision".into(),
                        serde_json::Value::String("deny".into()),
                    ),
                ]);
                if let Some(reason) = reason {
                    specific.insert(
                        "permissionDecisionReason".into(),
                        serde_json::Value::String(reason),
                    );
                }
                serde_json::json!({"hookSpecificOutput": specific})
            }
            PreToolUseOutput::Rewrite { updated_input } => {
                serde_json::json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow","updatedInput":updated_input}})
            }
            PreToolUseOutput::AdditionalContext { context } => {
                serde_json::json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":context}})
            }
        };
        ProcessEmission::command_json(Self::CONTRACT, &value)
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

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Native Codex input observed after a tool invocation.
pub struct PostToolUseInput {
    /// Native session identifier.
    pub session_id: String,
    /// Transcript path; the key is required but its value may be JSON `null`.
    pub transcript_path: Option<hookkit_core::Utf8PathBuf>,
    /// Current workspace directory.
    pub cwd: hookkit_core::Utf8PathBuf,
    /// Authoritative native event discriminator.
    pub hook_event_name: String,
    /// Model configured for the current turn.
    pub model: String,
    /// Native turn identifier.
    pub turn_id: String,
    /// Permission policy active for the tool invocation.
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
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Subagent type when the tool ran on behalf of a subagent.
    #[serde(default)]
    pub agent_type: Option<String>,
    /// Unknown protocol fields retained for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Native response from a Codex post-tool command hook.
#[derive(Debug, Clone)]
pub enum PostToolUseOutput {
    /// Emit an empty JSON object and exit successfully.
    NoOp,
    /// Emit a structured JSON response.
    Structured(StructuredPostToolUseOutput),
    /// Write a required message to stderr and exit with code 2.
    BlockingError {
        /// Non-empty error message; emptiness is checked during emission.
        message: String,
    },
    /// Add UTF-8 protocol stderr to one otherwise successful response.
    WithProtocolStderr {
        /// Successful response to encode as stdout.
        output: Box<PostToolUseOutput>,
        /// UTF-8 stderr bytes, validated during emission.
        stderr: Vec<u8>,
    },
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
/// Structured Codex post-tool response built through [`PostToolUseOutput`].
///
/// Fields are private so callers cannot construct combinations that bypass the
/// builder's ordering checks.
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
    suppress_output: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hook_specific_output: Option<PostToolUseSpecific>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PostToolUseSpecific {
    hook_event_name: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    additional_context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_mcp_tool_output: Option<serde_json::Value>,
}

impl PostToolUseOutput {
    /// Creates an empty JSON-object response.
    pub fn no_op() -> Self {
        Self::NoOp
    }

    /// Creates a structured response that appends agent context.
    pub fn with_context(context: impl Into<String>) -> Self {
        Self::Structured(StructuredPostToolUseOutput {
            hook_specific_output: Some(PostToolUseSpecific {
                hook_event_name: "PostToolUse",
                additional_context: Some(context.into()),
                updated_mcp_tool_output: None,
            }),
            ..StructuredPostToolUseOutput::default()
        })
    }

    /// Creates a structured legacy block response.
    pub fn block(reason: impl Into<String>) -> Self {
        Self::Structured(StructuredPostToolUseOutput {
            decision: Some("block"),
            reason: Some(reason.into()),
            ..StructuredPostToolUseOutput::default()
        })
    }

    /// Adds a legacy block decision to a successful structured response.
    ///
    /// `NoOp` is promoted to an empty structured response. Blocking-error and
    /// stderr-wrapped outputs are rejected because their structure is final.
    pub fn with_block(mut self, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        let output = self.structured_mut()?;
        output.decision = Some("block");
        output.reason = Some(reason.into());
        Ok(self)
    }

    /// Creates a code-2 response with required stderr text.
    pub fn blocking_error(message: impl Into<String>) -> Self {
        Self::BlockingError {
            message: message.into(),
        }
    }

    /// Adds protocol stderr to a successful response.
    ///
    /// A blocking error or an already wrapped response is rejected. The text
    /// must be valid UTF-8 when emitted.
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

    /// Replaces the MCP tool output in a structured hook-specific response.
    pub fn with_updated_mcp_tool_output(
        mut self,
        output: serde_json::Value,
    ) -> hookkit_core::Result<Self> {
        self.specific_mut()?.updated_mcp_tool_output = Some(output);
        Ok(self)
    }

    /// Sets Codex's top-level `continue` control on a structured response.
    pub fn with_continue(mut self, continue_session: bool) -> hookkit_core::Result<Self> {
        self.structured_mut()?.continue_session = Some(continue_session);
        Ok(self)
    }

    /// Sets the top-level stop reason on a structured response.
    pub fn with_stop_reason(mut self, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        self.structured_mut()?.stop_reason = Some(reason.into());
        Ok(self)
    }

    /// Sets whether Codex suppresses ordinary tool output.
    pub fn with_suppress_output(mut self, suppress_output: bool) -> hookkit_core::Result<Self> {
        self.structured_mut()?.suppress_output = Some(suppress_output);
        Ok(self)
    }

    /// Sets a top-level system message on a structured response.
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

    fn specific_mut(&mut self) -> hookkit_core::Result<&mut PostToolUseSpecific> {
        let structured = self.structured_mut()?;
        Ok(structured
            .hook_specific_output
            .get_or_insert(PostToolUseSpecific {
                hook_event_name: "PostToolUse",
                additional_context: None,
                updated_mcp_tool_output: None,
            }))
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
    const CONTRACT: ContractId = ContractId::builtin("codex/commit-9e552e9-r2/PostToolUse");

    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        require_event(invocation, "PostToolUse")?;
        require_field(invocation, "transcript_path", "PostToolUse")?;
        serde_json::from_value(invocation.json().clone()).map_err(Into::into)
    }

    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        match output {
            PostToolUseOutput::NoOp => {
                ProcessEmission::command_json(Self::CONTRACT, &serde_json::json!({}))
            }
            PostToolUseOutput::Structured(output) => {
                ProcessEmission::command_json(Self::CONTRACT, &output)
            }
            PostToolUseOutput::BlockingError { message } => {
                ProcessEmission::command_required_stderr(Self::CONTRACT, message, 2)
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

pub(crate) fn require_field(
    invocation: &RawInvocation,
    field: &str,
    event: &'static str,
) -> hookkit_core::Result<()> {
    if invocation.json().get(field).is_some() {
        return Ok(());
    }
    Err(hookkit_core::HookkitError::InvalidForHint {
        harness: HarnessId::CODEX,
        event: EventId::builtin(HarnessId::CODEX, event),
        message: format!("missing required field {field}"),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Compile-time selector for an implemented Codex event.
pub enum Event {
    /// Selects [`PreToolUse`].
    PreToolUse,
    /// Selects [`PostToolUse`].
    PostToolUse,
    /// Selects [`crate::catalog::PermissionRequest`].
    PermissionRequest,
    /// Selects [`crate::catalog::PostCompact`].
    PostCompact,
    /// Selects [`crate::catalog::PreCompact`].
    PreCompact,
    /// Selects [`crate::catalog::SessionStart`].
    SessionStart,
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
            Self::PermissionRequest => "PermissionRequest",
            Self::PostCompact => "PostCompact",
            Self::PreCompact => "PreCompact",
            Self::SessionStart => "SessionStart",
            Self::Stop => "Stop",
            Self::SubagentStart => "SubagentStart",
            Self::SubagentStop => "SubagentStop",
            Self::UserPromptSubmit => "UserPromptSubmit",
        };
        EventId::builtin(HarnessId::CODEX, name)
    }
}

#[derive(Debug, Clone)]
/// Lossless sum type over all implemented Codex inputs.
pub enum AnyInput {
    /// A pre-tool input.
    PreToolUse(PreToolUseInput),
    /// A post-tool input.
    PostToolUse(PostToolUseInput),
    /// An input for another implemented catalog event.
    Catalog(crate::catalog::CatalogInput),
}

#[derive(Debug, Clone)]
/// Sum type over all implemented Codex command outputs.
pub enum AnyCommandOutput {
    /// A pre-tool output.
    PreToolUse(PreToolUseOutput),
    /// A post-tool output.
    PostToolUse(PostToolUseOutput),
    /// An output for another implemented catalog event.
    Catalog(crate::catalog::CatalogOutput),
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
    #[test]
    fn rewrite_cannot_be_emitted_without_allow() {
        let emission = PreToolUse::emit(PreToolUseOutput::rewrite(serde_json::Map::new())).unwrap();
        let value: serde_json::Value = serde_json::from_slice(emission.stdout()).unwrap();
        assert_eq!(value["hookSpecificOutput"]["permissionDecision"], "allow");
    }

    #[test]
    fn deny_without_reason_omits_reason_instead_of_emitting_null() {
        let emission = PreToolUse::emit(PreToolUseOutput::deny(None)).unwrap();
        let value: serde_json::Value = serde_json::from_slice(emission.stdout()).unwrap();
        assert!(
            value["hookSpecificOutput"]
                .get("permissionDecisionReason")
                .is_none()
        );
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
        let value: serde_json::Value = serde_json::from_slice(emission.stdout()).unwrap();
        assert_eq!(value["hookSpecificOutput"]["hookEventName"], "PostToolUse");
        assert_eq!(
            value["hookSpecificOutput"]["additionalContext"],
            "review changes"
        );
    }

    #[test]
    fn post_tool_use_requires_nullable_transcript_field_to_be_present() {
        let raw = RawInvocation::parse(
            br#"{"session_id":"s","cwd":"/repo","hook_event_name":"PostToolUse","model":"gpt-test","turn_id":"t","permission_mode":"default","tool_name":"Bash","tool_use_id":"call","tool_input":{},"tool_response":{}}"#.to_vec(),
        )
        .unwrap();
        assert!(PostToolUse::parse(&raw).is_err());
    }

    #[test]
    fn post_tool_use_no_op_is_required_empty_json_object() {
        let emission = PostToolUse::emit(PostToolUseOutput::no_op()).unwrap();
        assert_eq!(emission.stdout(), b"{}");
    }

    #[test]
    fn post_tool_use_protocol_stderr_preserves_json_stdout() {
        let output = PostToolUseOutput::with_context("review changes")
            .with_protocol_stderr("user diagnostic")
            .unwrap();
        let emission = PostToolUse::emit(output).unwrap();
        assert_eq!(emission.stderr(), b"user diagnostic");
        assert_eq!(emission.exit_code(), 0);
        let value: serde_json::Value = serde_json::from_slice(emission.stdout()).unwrap();
        assert_eq!(value["hookSpecificOutput"]["hookEventName"], "PostToolUse");
    }

    #[test]
    fn permission_modes_are_schema_constrained() {
        let raw = RawInvocation::parse(
            br#"{"session_id":"s","transcript_path":null,"cwd":"/repo","hook_event_name":"PreToolUse","model":"gpt-test","turn_id":"t","permission_mode":"invalid","tool_name":"Bash","tool_use_id":"u","tool_input":{}}"#.to_vec(),
        )
        .unwrap();
        assert!(PreToolUse::parse(&raw).is_err());
    }

    #[test]
    fn ask_and_approve_are_distinct_schema_valid_outputs() {
        let ask = PreToolUse::emit(PreToolUseOutput::ask(Some("confirm".into()))).unwrap();
        let ask: serde_json::Value = serde_json::from_slice(ask.stdout()).unwrap();
        assert_eq!(ask["hookSpecificOutput"]["permissionDecision"], "ask");
        let approve = PreToolUse::emit(PreToolUseOutput::approve()).unwrap();
        let approve: serde_json::Value = serde_json::from_slice(approve.stdout()).unwrap();
        assert_eq!(approve["decision"], "approve");
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
}
