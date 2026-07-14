use hookkit_core::{
    ContractId, EventCategory, EventId, EventSelector, EventSpec, HarnessId, HarnessSpec,
    IdentificationDescriptor, NativeContext, ProcessEmission, RawInvocation, SessionId, SnapshotId,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::GeminiCommandEnvironment;

pub const SNAPSHOT_ID: SnapshotId = SnapshotId::builtin("commit-f354eeb-r2");

pub fn events() -> Vec<hookkit_core::NativeEventDescriptor> {
    vec![
        hookkit_core::NativeEventDescriptor::command::<BeforeTool>(&["structured", "exit-2"]),
        hookkit_core::NativeEventDescriptor::command::<AfterTool>(&["structured", "exit-2"]),
        hookkit_core::NativeEventDescriptor::command::<BeforeToolSelection>(&[
            "no-op",
            "disable-tools",
        ]),
    ]
}

pub fn identification_descriptors() -> Vec<IdentificationDescriptor> {
    macro_rules! catalog {
        ($event:literal) => {
            IdentificationDescriptor::catalog_definitive(
                EventId::builtin(HarnessId::GEMINI_CLI, $event),
                SNAPSHOT_ID,
                ContractId::builtin(concat!("gemini-cli/commit-f354eeb-r2/", $event)),
                "/hook_event_name",
                $event,
            )
        };
    }
    vec![
        catalog!("SessionStart"),
        catalog!("BeforeAgent"),
        catalog!("BeforeModel"),
        IdentificationDescriptor::definitive::<BeforeToolSelection>(
            "/hook_event_name",
            "BeforeToolSelection",
        ),
        IdentificationDescriptor::definitive::<BeforeTool>("/hook_event_name", "BeforeTool"),
        IdentificationDescriptor::definitive::<AfterTool>("/hook_event_name", "AfterTool"),
        catalog!("AfterModel"),
        catalog!("AfterAgent"),
        catalog!("Notification"),
        catalog!("PreCompress"),
        catalog!("SessionEnd"),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeforeToolInput {
    pub session_id: String,
    pub transcript_path: hookkit_core::Utf8PathBuf,
    pub cwd: hookkit_core::Utf8PathBuf,
    pub hook_event_name: String,
    pub timestamp: String,
    pub tool_name: String,
    pub tool_input: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub mcp_context: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    pub original_request_name: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
pub enum BeforeToolOutput {
    NoOp,
    Structured(StructuredBeforeToolOutput),
    BlockingError { message: String },
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuredBeforeToolOutput {
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
    hook_specific_output: Option<BeforeToolSpecific>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BeforeToolSpecific {
    hook_event_name: &'static str,
    #[serde(rename = "tool_input", skip_serializing_if = "Option::is_none")]
    tool_input: Option<serde_json::Map<String, serde_json::Value>>,
}

impl BeforeToolOutput {
    pub fn no_op() -> Self {
        Self::NoOp
    }

    pub fn allow() -> Self {
        Self::Structured(StructuredBeforeToolOutput {
            decision: Some("allow"),
            hook_specific_output: Some(BeforeToolSpecific {
                hook_event_name: "BeforeTool",
                tool_input: None,
            }),
            ..StructuredBeforeToolOutput::default()
        })
    }

    pub fn deny(reason: impl Into<String>) -> Self {
        Self::Structured(StructuredBeforeToolOutput {
            decision: Some("deny"),
            reason: Some(reason.into()),
            hook_specific_output: Some(BeforeToolSpecific {
                hook_event_name: "BeforeTool",
                tool_input: None,
            }),
            ..StructuredBeforeToolOutput::default()
        })
    }

    pub fn block(reason: impl Into<String>) -> Self {
        Self::Structured(StructuredBeforeToolOutput {
            decision: Some("block"),
            reason: Some(reason.into()),
            hook_specific_output: Some(BeforeToolSpecific {
                hook_event_name: "BeforeTool",
                tool_input: None,
            }),
            ..StructuredBeforeToolOutput::default()
        })
    }

    pub fn rewrite_tool_input(tool_input: serde_json::Map<String, serde_json::Value>) -> Self {
        Self::Structured(StructuredBeforeToolOutput {
            hook_specific_output: Some(BeforeToolSpecific {
                hook_event_name: "BeforeTool",
                tool_input: Some(tool_input),
            }),
            ..StructuredBeforeToolOutput::default()
        })
    }

    pub fn deny_and_rewrite(
        reason: impl Into<String>,
        tool_input: serde_json::Map<String, serde_json::Value>,
    ) -> Self {
        Self::Structured(StructuredBeforeToolOutput {
            decision: Some("deny"),
            reason: Some(reason.into()),
            hook_specific_output: Some(BeforeToolSpecific {
                hook_event_name: "BeforeTool",
                tool_input: Some(tool_input),
            }),
            ..StructuredBeforeToolOutput::default()
        })
    }

    pub fn blocking_error(message: impl Into<String>) -> Self {
        Self::BlockingError {
            message: message.into(),
        }
    }

    pub fn with_continue(mut self, continue_session: bool) -> hookkit_core::Result<Self> {
        self.structured_mut()?.continue_session = Some(continue_session);
        Ok(self)
    }

    pub fn with_stop_reason(mut self, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        self.structured_mut()?.stop_reason = Some(reason.into());
        Ok(self)
    }

    pub fn with_suppress_output(mut self, suppress_output: bool) -> hookkit_core::Result<Self> {
        self.structured_mut()?.suppress_output = Some(suppress_output);
        Ok(self)
    }

    pub fn with_system_message(mut self, message: impl Into<String>) -> hookkit_core::Result<Self> {
        self.structured_mut()?.system_message = Some(message.into());
        Ok(self)
    }

    fn structured_mut(&mut self) -> hookkit_core::Result<&mut StructuredBeforeToolOutput> {
        if matches!(self, Self::NoOp) {
            *self = Self::Structured(StructuredBeforeToolOutput::default());
        }
        match self {
            Self::Structured(output) => Ok(output),
            Self::NoOp => unreachable!("NoOp is converted to Structured above"),
            Self::BlockingError { .. } => Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "structured fields cannot be added to a blocking error",
            )),
        }
    }
}

pub enum BeforeTool {}

impl EventSpec for BeforeTool {
    type Input = BeforeToolInput;
    type CommandEnvironment = GeminiCommandEnvironment;
    type CommandOutput = BeforeToolOutput;
    const HARNESS: HarnessId = HarnessId::GEMINI_CLI;
    const SNAPSHOT: SnapshotId = SNAPSHOT_ID;
    const EVENT: EventId = EventId::builtin(HarnessId::GEMINI_CLI, "BeforeTool");
    const CATEGORY: EventCategory = EventCategory::Tool;
    const CONTRACT: ContractId = ContractId::builtin("gemini-cli/commit-f354eeb-r2/BeforeTool");

    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        require_event(invocation, "BeforeTool")?;
        serde_json::from_value(invocation.json().clone()).map_err(Into::into)
    }

    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        match output {
            BeforeToolOutput::NoOp => {
                ProcessEmission::command_json(Self::CONTRACT, &serde_json::json!({}))
            }
            BeforeToolOutput::Structured(output) => {
                ProcessEmission::command_json(Self::CONTRACT, &output)
            }
            BeforeToolOutput::BlockingError { message } => {
                ProcessEmission::command_required_stderr(Self::CONTRACT, message, 2)
            }
        }
    }

    fn context(input: &Self::Input) -> NativeContext {
        NativeContext {
            workspace_roots: vec![input.cwd.clone()],
            session_id: SessionId::new(&input.session_id).ok(),
            transcript_path: Some(input.transcript_path.clone()),
            ..NativeContext::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AfterToolInput {
    pub session_id: String,
    pub transcript_path: hookkit_core::Utf8PathBuf,
    pub cwd: hookkit_core::Utf8PathBuf,
    pub hook_event_name: String,
    pub timestamp: String,
    pub tool_name: String,
    pub tool_input: serde_json::Map<String, serde_json::Value>,
    pub tool_response: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub mcp_context: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    pub original_request_name: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
pub enum AfterToolOutput {
    NoOp,
    Structured(StructuredAfterToolOutput),
    BlockingError {
        message: String,
    },
    WithProtocolStderr {
        output: Box<AfterToolOutput>,
        stderr: Vec<u8>,
    },
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuredAfterToolOutput {
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
    hook_specific_output: Option<AfterToolSpecific>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AfterToolSpecific {
    hook_event_name: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    additional_context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tail_tool_call_request: Option<TailToolCallRequest>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TailToolCallRequest {
    name: String,
    args: serde_json::Map<String, serde_json::Value>,
}

impl AfterToolOutput {
    pub fn no_op() -> Self {
        Self::NoOp
    }

    pub fn with_context(context: impl Into<String>) -> Self {
        Self::Structured(StructuredAfterToolOutput {
            hook_specific_output: Some(AfterToolSpecific {
                hook_event_name: "AfterTool",
                additional_context: Some(context.into()),
                tail_tool_call_request: None,
            }),
            ..StructuredAfterToolOutput::default()
        })
    }

    pub fn allow() -> Self {
        Self::Structured(StructuredAfterToolOutput {
            decision: Some("allow"),
            hook_specific_output: Some(AfterToolSpecific {
                hook_event_name: "AfterTool",
                additional_context: None,
                tail_tool_call_request: None,
            }),
            ..StructuredAfterToolOutput::default()
        })
    }

    pub fn deny(reason: impl Into<String>) -> Self {
        Self::Structured(StructuredAfterToolOutput {
            decision: Some("deny"),
            reason: Some(reason.into()),
            hook_specific_output: Some(AfterToolSpecific {
                hook_event_name: "AfterTool",
                additional_context: None,
                tail_tool_call_request: None,
            }),
            ..StructuredAfterToolOutput::default()
        })
    }

    pub fn with_tail_tool_call(
        name: impl Into<String>,
        args: serde_json::Map<String, serde_json::Value>,
    ) -> Self {
        Self::Structured(StructuredAfterToolOutput {
            hook_specific_output: Some(AfterToolSpecific {
                hook_event_name: "AfterTool",
                additional_context: None,
                tail_tool_call_request: Some(TailToolCallRequest {
                    name: name.into(),
                    args,
                }),
            }),
            ..StructuredAfterToolOutput::default()
        })
    }

    pub fn and_tail_tool_call(
        mut self,
        name: impl Into<String>,
        args: serde_json::Map<String, serde_json::Value>,
    ) -> hookkit_core::Result<Self> {
        let specific = self.specific_mut()?;
        specific.tail_tool_call_request = Some(TailToolCallRequest {
            name: name.into(),
            args,
        });
        Ok(self)
    }

    pub fn block(reason: impl Into<String>) -> Self {
        Self::Structured(StructuredAfterToolOutput {
            decision: Some("block"),
            reason: Some(reason.into()),
            hook_specific_output: Some(AfterToolSpecific {
                hook_event_name: "AfterTool",
                additional_context: None,
                tail_tool_call_request: None,
            }),
            ..StructuredAfterToolOutput::default()
        })
    }

    pub fn blocking_error(message: impl Into<String>) -> Self {
        Self::BlockingError {
            message: message.into(),
        }
    }

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

    pub fn with_continue(mut self, continue_session: bool) -> hookkit_core::Result<Self> {
        self.structured_mut()?.continue_session = Some(continue_session);
        Ok(self)
    }

    pub fn with_stop_reason(mut self, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        self.structured_mut()?.stop_reason = Some(reason.into());
        Ok(self)
    }

    pub fn with_suppress_output(mut self, suppress_output: bool) -> hookkit_core::Result<Self> {
        self.structured_mut()?.suppress_output = Some(suppress_output);
        Ok(self)
    }

    pub fn with_system_message(mut self, message: impl Into<String>) -> hookkit_core::Result<Self> {
        self.structured_mut()?.system_message = Some(message.into());
        Ok(self)
    }

    fn structured_mut(&mut self) -> hookkit_core::Result<&mut StructuredAfterToolOutput> {
        if matches!(self, Self::NoOp) {
            *self = Self::Structured(StructuredAfterToolOutput::default());
        }
        match self {
            Self::Structured(output) => Ok(output),
            Self::NoOp => unreachable!("NoOp is converted to Structured above"),
            Self::BlockingError { .. } | Self::WithProtocolStderr { .. } => {
                Err(hookkit_core::HookkitError::InvalidProcessEmission(
                    "structured fields must be added before blocking/protocol-stderr output",
                ))
            }
        }
    }

    fn specific_mut(&mut self) -> hookkit_core::Result<&mut AfterToolSpecific> {
        let output = self.structured_mut()?;
        Ok(output
            .hook_specific_output
            .get_or_insert(AfterToolSpecific {
                hook_event_name: "AfterTool",
                additional_context: None,
                tail_tool_call_request: None,
            }))
    }
}

pub enum AfterTool {}

impl EventSpec for AfterTool {
    type Input = AfterToolInput;
    type CommandEnvironment = GeminiCommandEnvironment;
    type CommandOutput = AfterToolOutput;
    const HARNESS: HarnessId = HarnessId::GEMINI_CLI;
    const SNAPSHOT: SnapshotId = SNAPSHOT_ID;
    const EVENT: EventId = EventId::builtin(HarnessId::GEMINI_CLI, "AfterTool");
    const CATEGORY: EventCategory = EventCategory::Tool;
    const CONTRACT: ContractId = ContractId::builtin("gemini-cli/commit-f354eeb-r2/AfterTool");

    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        require_event(invocation, "AfterTool")?;
        serde_json::from_value(invocation.json().clone()).map_err(Into::into)
    }

    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        match output {
            AfterToolOutput::NoOp => {
                ProcessEmission::command_json(Self::CONTRACT, &serde_json::json!({}))
            }
            AfterToolOutput::Structured(output) => {
                ProcessEmission::command_json(Self::CONTRACT, &output)
            }
            AfterToolOutput::BlockingError { message } => {
                ProcessEmission::command_required_stderr(Self::CONTRACT, message, 2)
            }
            AfterToolOutput::WithProtocolStderr { output, stderr } => {
                if matches!(
                    output.as_ref(),
                    AfterToolOutput::BlockingError { .. }
                        | AfterToolOutput::WithProtocolStderr { .. }
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
            transcript_path: Some(input.transcript_path.clone()),
            ..NativeContext::default()
        }
    }
}

fn require_event(invocation: &RawInvocation, expected: &'static str) -> hookkit_core::Result<()> {
    if invocation
        .json()
        .get("hook_event_name")
        .and_then(serde_json::Value::as_str)
        == Some(expected)
    {
        return Ok(());
    }
    Err(hookkit_core::HookkitError::InvalidForHint {
        harness: HarnessId::GEMINI_CLI,
        event: EventId::builtin(HarnessId::GEMINI_CLI, expected),
        message: format!("expected hook_event_name={expected}"),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeforeToolSelectionInput {
    pub session_id: String,
    pub transcript_path: hookkit_core::Utf8PathBuf,
    pub cwd: hookkit_core::Utf8PathBuf,
    pub hook_event_name: String,
    pub timestamp: String,
    pub llm_request: LlmRequest,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRequest {
    pub model: String,
    pub messages: Vec<serde_json::Value>,
    pub config: serde_json::Map<String, serde_json::Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ToolMode {
    Auto,
    Any,
    None,
}

#[derive(Debug, Clone)]
pub struct BeforeToolSelectionOutput(BeforeToolSelectionOutcome);

#[derive(Debug, Clone)]
enum BeforeToolSelectionOutcome {
    NoOp,
    Configure {
        mode: Option<ToolMode>,
        allowed_function_names: Vec<String>,
    },
}

impl BeforeToolSelectionOutput {
    pub fn no_op() -> Self {
        Self(BeforeToolSelectionOutcome::NoOp)
    }
    pub fn configure(
        mode: Option<ToolMode>,
        allowed_function_names: Vec<String>,
    ) -> hookkit_core::Result<Self> {
        if mode.is_none() && allowed_function_names.is_empty() {
            return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "toolConfig must contain mode or allowedFunctionNames",
            ));
        }
        if allowed_function_names.iter().collect::<BTreeSet<_>>().len()
            != allowed_function_names.len()
        {
            return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "allowedFunctionNames must not contain duplicates",
            ));
        }
        Ok(Self(BeforeToolSelectionOutcome::Configure {
            mode,
            allowed_function_names,
        }))
    }
}

pub enum BeforeToolSelection {}

impl EventSpec for BeforeToolSelection {
    type Input = BeforeToolSelectionInput;
    type CommandEnvironment = GeminiCommandEnvironment;
    type CommandOutput = BeforeToolSelectionOutput;
    const HARNESS: HarnessId = HarnessId::GEMINI_CLI;
    const SNAPSHOT: SnapshotId = SNAPSHOT_ID;
    const EVENT: EventId = EventId::builtin(HarnessId::GEMINI_CLI, "BeforeToolSelection");
    const CATEGORY: EventCategory = EventCategory::Model;
    const CONTRACT: ContractId =
        ContractId::builtin("gemini-cli/commit-f354eeb-r2/BeforeToolSelection");

    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        if invocation
            .json()
            .get("hook_event_name")
            .and_then(serde_json::Value::as_str)
            != Some("BeforeToolSelection")
        {
            return Err(hookkit_core::HookkitError::InvalidForHint {
                harness: HarnessId::GEMINI_CLI,
                event: Self::EVENT,
                message: "expected hook_event_name=BeforeToolSelection".into(),
            });
        }
        serde_json::from_value(invocation.json().clone()).map_err(Into::into)
    }

    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        let value = match output.0 {
            BeforeToolSelectionOutcome::NoOp => serde_json::json!({}),
            BeforeToolSelectionOutcome::Configure {
                mode,
                allowed_function_names,
            } => {
                let mut config = serde_json::Map::new();
                if let Some(mode) = mode {
                    config.insert("mode".into(), serde_json::to_value(mode)?);
                }
                if !allowed_function_names.is_empty() {
                    config.insert(
                        "allowedFunctionNames".into(),
                        serde_json::json!(allowed_function_names),
                    );
                }
                serde_json::json!({"hookSpecificOutput":{"hookEventName":"BeforeToolSelection","toolConfig":config}})
            }
        };
        ProcessEmission::command_json(Self::CONTRACT, &value)
    }

    fn context(input: &Self::Input) -> NativeContext {
        NativeContext {
            workspace_roots: vec![input.cwd.clone()],
            session_id: SessionId::new(&input.session_id).ok(),
            transcript_path: Some(input.transcript_path.clone()),
            ..NativeContext::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    BeforeTool,
    AfterTool,
    BeforeToolSelection,
}

impl EventSelector for Event {
    fn event_id(&self) -> EventId {
        let name = match self {
            Self::BeforeTool => "BeforeTool",
            Self::AfterTool => "AfterTool",
            Self::BeforeToolSelection => "BeforeToolSelection",
        };
        EventId::builtin(HarnessId::GEMINI_CLI, name)
    }
}

#[derive(Debug, Clone)]
pub enum AnyInput {
    BeforeTool(BeforeToolInput),
    AfterTool(AfterToolInput),
    BeforeToolSelection(BeforeToolSelectionInput),
}

#[derive(Debug, Clone)]
pub enum AnyCommandOutput {
    BeforeTool(BeforeToolOutput),
    AfterTool(AfterToolOutput),
    BeforeToolSelection(BeforeToolSelectionOutput),
}

pub enum GeminiCli {}

impl HarnessSpec for GeminiCli {
    type AnyInput = AnyInput;
    type CommandEnvironment = GeminiCommandEnvironment;
    type AnyCommandOutput = AnyCommandOutput;
    type EventSelector = Event;

    const ID: HarnessId = HarnessId::GEMINI_CLI;
    const SNAPSHOT: SnapshotId = SNAPSHOT_ID;

    fn identification_descriptors() -> Vec<IdentificationDescriptor> {
        identification_descriptors()
    }

    fn decode(event: &EventId, raw: &RawInvocation) -> hookkit_core::Result<Self::AnyInput> {
        match event.name() {
            "BeforeTool" => BeforeTool::parse(raw).map(AnyInput::BeforeTool),
            "AfterTool" => AfterTool::parse(raw).map(AnyInput::AfterTool),
            "BeforeToolSelection" => {
                BeforeToolSelection::parse(raw).map(AnyInput::BeforeToolSelection)
            }
            _ => Err(hookkit_core::HookkitError::UnrecognizedEvent {
                harness: Self::ID,
                message: event.name().to_string(),
            }),
        }
    }

    fn input_event(input: &Self::AnyInput) -> EventId {
        match input {
            AnyInput::BeforeTool(_) => BeforeTool::EVENT,
            AnyInput::AfterTool(_) => AfterTool::EVENT,
            AnyInput::BeforeToolSelection(_) => BeforeToolSelection::EVENT,
        }
    }

    fn output_event(output: &Self::AnyCommandOutput) -> EventId {
        match output {
            AnyCommandOutput::BeforeTool(_) => BeforeTool::EVENT,
            AnyCommandOutput::AfterTool(_) => AfterTool::EVENT,
            AnyCommandOutput::BeforeToolSelection(_) => BeforeToolSelection::EVENT,
        }
    }

    fn encode_command_unchecked(
        output: Self::AnyCommandOutput,
    ) -> hookkit_core::Result<ProcessEmission> {
        match output {
            AnyCommandOutput::BeforeTool(output) => BeforeTool::emit(output),
            AnyCommandOutput::AfterTool(output) => AfterTool::emit(output),
            AnyCommandOutput::BeforeToolSelection(output) => BeforeToolSelection::emit(output),
        }
    }

    fn context(input: &Self::AnyInput) -> NativeContext {
        match input {
            AnyInput::BeforeTool(input) => BeforeTool::context(input),
            AnyInput::AfterTool(input) => AfterTool::context(input),
            AnyInput::BeforeToolSelection(input) => BeforeToolSelection::context(input),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn event_name_is_fixed() {
        let output =
            BeforeToolSelectionOutput::configure(Some(ToolMode::Any), vec!["run_shell".into()])
                .unwrap();
        let emission = BeforeToolSelection::emit(output).unwrap();
        let value: serde_json::Value = serde_json::from_slice(emission.stdout()).unwrap();
        assert_eq!(
            value["hookSpecificOutput"]["hookEventName"],
            "BeforeToolSelection"
        );
    }

    #[test]
    fn before_tool_parses_open_real_shell_payload() {
        let raw = RawInvocation::parse(
            br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"BeforeTool","timestamp":"2026-07-12T00:00:00Z","tool_name":"run_shell_command","tool_input":{"command":"cargo test","future":true},"future_top":true}"#.to_vec(),
        )
        .unwrap();
        let input = BeforeTool::parse(&raw).unwrap();
        assert_eq!(input.tool_name, "run_shell_command");
        assert_eq!(input.tool_input["command"], "cargo test");
        assert_eq!(input.extra["future_top"], true);
    }

    #[test]
    fn before_tool_rewrite_stamps_exact_discriminator() {
        let input = serde_json::Map::from_iter([(
            "command".into(),
            serde_json::Value::String("echo safe".into()),
        )]);
        let emission = BeforeTool::emit(BeforeToolOutput::rewrite_tool_input(input)).unwrap();
        let value: serde_json::Value = serde_json::from_slice(emission.stdout()).unwrap();
        assert_eq!(value["hookSpecificOutput"]["hookEventName"], "BeforeTool");
        assert_eq!(
            value["hookSpecificOutput"]["tool_input"]["command"],
            "echo safe"
        );
    }

    #[test]
    fn before_tool_decisions_stamp_exact_discriminator() {
        for output in [
            BeforeToolOutput::allow(),
            BeforeToolOutput::deny("policy"),
            BeforeToolOutput::block("policy"),
        ] {
            let emission = BeforeTool::emit(output).unwrap();
            let value: serde_json::Value = serde_json::from_slice(emission.stdout()).unwrap();
            assert_eq!(value["hookSpecificOutput"]["hookEventName"], "BeforeTool");
        }
    }

    #[test]
    fn before_tool_no_op_is_empty_json_object() {
        let emission = BeforeTool::emit(BeforeToolOutput::no_op()).unwrap();
        assert_eq!(emission.stdout(), b"{}");
    }

    #[test]
    fn after_tool_context_stamps_exact_discriminator() {
        let emission = AfterTool::emit(AfterToolOutput::with_context("review changes")).unwrap();
        let value: serde_json::Value = serde_json::from_slice(emission.stdout()).unwrap();
        assert_eq!(value["hookSpecificOutput"]["hookEventName"], "AfterTool");
        assert_eq!(
            value["hookSpecificOutput"]["additionalContext"],
            "review changes"
        );
    }

    #[test]
    fn after_tool_block_stamps_exact_discriminator() {
        let emission = AfterTool::emit(AfterToolOutput::block("policy")).unwrap();
        let value: serde_json::Value = serde_json::from_slice(emission.stdout()).unwrap();
        assert_eq!(value["hookSpecificOutput"]["hookEventName"], "AfterTool");
    }

    #[test]
    fn after_tool_no_op_is_required_empty_json_object() {
        let emission = AfterTool::emit(AfterToolOutput::no_op()).unwrap();
        assert_eq!(emission.stdout(), b"{}");
    }

    #[test]
    fn after_tool_protocol_stderr_preserves_json_stdout() {
        let output = AfterToolOutput::with_context("review changes")
            .with_protocol_stderr("user diagnostic")
            .unwrap();
        let emission = AfterTool::emit(output).unwrap();
        assert_eq!(emission.stderr(), b"user diagnostic");
        assert_eq!(emission.exit_code(), 0);
        let value: serde_json::Value = serde_json::from_slice(emission.stdout()).unwrap();
        assert_eq!(value["hookSpecificOutput"]["hookEventName"], "AfterTool");
    }

    #[test]
    fn after_tool_requires_object_tool_payloads() {
        let raw = RawInvocation::parse(
            br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"AfterTool","timestamp":"2026-07-12T00:00:00Z","tool_name":"run_shell_command","tool_input":{"command":"cargo test"},"tool_response":"not-an-object"}"#.to_vec(),
        )
        .unwrap();
        assert!(AfterTool::parse(&raw).is_err());
    }

    #[test]
    fn tool_selection_rejects_empty_and_duplicate_configurations() {
        assert!(BeforeToolSelectionOutput::configure(None, Vec::new()).is_err());
        assert!(
            BeforeToolSelectionOutput::configure(
                Some(ToolMode::Any),
                vec!["read_file".into(), "read_file".into()]
            )
            .is_err()
        );
    }

    #[test]
    fn invalid_builder_order_and_raw_protocol_stderr_return_errors() {
        assert!(
            AfterToolOutput::blocking_error("blocked")
                .and_tail_tool_call("read_file", serde_json::Map::new())
                .is_err()
        );
        let invalid = AfterToolOutput::WithProtocolStderr {
            output: Box::new(AfterToolOutput::no_op()),
            stderr: vec![0xff],
        };
        assert!(AfterTool::emit(invalid).is_err());
    }
}
