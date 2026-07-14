//! Contract-first event specifications for the selected catalog snapshot.

use hookkit_core::{
    ContractId, EventCategory, EventId, EventSelector, EventSpec, HarnessId, HarnessSpec,
    IdentificationDescriptor, NativeContext, ProcessEmission, RawInvocation, SessionId, SnapshotId,
    ToolCallId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::ClaudeCommandEnvironment;

pub const SNAPSHOT_ID: SnapshotId = SnapshotId::builtin("docs-2026-07-12-r2");

pub fn events() -> Vec<hookkit_core::NativeEventDescriptor> {
    vec![
        hookkit_core::NativeEventDescriptor::command::<SessionStart>(&[
            "command-structured",
            "command-text",
        ]),
        hookkit_core::NativeEventDescriptor::command::<PostToolUse>(&["command-structured"]),
        hookkit_core::NativeEventDescriptor::command::<WorktreeCreate>(&[
            "command-created",
            "command-failed",
        ]),
    ]
}

pub fn identification_descriptors() -> Vec<IdentificationDescriptor> {
    macro_rules! catalog {
        ($event:literal) => {
            IdentificationDescriptor::catalog_definitive(
                EventId::builtin(HarnessId::CLAUDE_CODE, $event),
                SNAPSHOT_ID,
                ContractId::builtin(concat!("claude-code/docs-2026-07-12-r2/", $event)),
                "/hook_event_name",
                $event,
            )
        };
    }
    vec![
        IdentificationDescriptor::definitive::<SessionStart>("/hook_event_name", "SessionStart"),
        catalog!("Setup"),
        catalog!("InstructionsLoaded"),
        catalog!("UserPromptSubmit"),
        catalog!("UserPromptExpansion"),
        catalog!("MessageDisplay"),
        catalog!("PreToolUse"),
        catalog!("PermissionRequest"),
        IdentificationDescriptor::definitive::<PostToolUse>("/hook_event_name", "PostToolUse"),
        catalog!("PostToolUseFailure"),
        catalog!("PostToolBatch"),
        catalog!("PermissionDenied"),
        catalog!("Notification"),
        catalog!("SubagentStart"),
        catalog!("SubagentStop"),
        catalog!("TaskCreated"),
        catalog!("TaskCompleted"),
        catalog!("Stop"),
        catalog!("StopFailure"),
        catalog!("TeammateIdle"),
        catalog!("ConfigChange"),
        catalog!("CwdChanged"),
        catalog!("FileChanged"),
        IdentificationDescriptor::definitive::<WorktreeCreate>(
            "/hook_event_name",
            "WorktreeCreate",
        ),
        catalog!("WorktreeRemove"),
        catalog!("PreCompact"),
        catalog!("PostCompact"),
        catalog!("SessionEnd"),
        catalog!("Elicitation"),
        catalog!("ElicitationResult"),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStartInput {
    pub session_id: String,
    pub transcript_path: hookkit_core::Utf8PathBuf,
    pub cwd: hookkit_core::Utf8PathBuf,
    pub hook_event_name: String,
    pub source: SessionSource,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub agent_type: Option<String>,
    #[serde(default)]
    pub effort: Option<Effort>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<PermissionMode>,
    #[serde(default)]
    pub prompt_id: Option<String>,
    #[serde(default)]
    pub session_title: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionSource {
    Startup,
    Resume,
    Clear,
    Compact,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    Default,
    Plan,
    AcceptEdits,
    Auto,
    DontAsk,
    BypassPermissions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Effort {
    pub level: EffortLevel,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EffortLevel {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl EffortLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }
}

#[derive(Debug, Clone)]
pub enum SessionStartOutput {
    Structured(StructuredSessionStartOutput),
    Text(String),
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuredSessionStartOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    hook_specific_output: Option<SessionStartSpecific>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionStartSpecific {
    hook_event_name: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    additional_context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    initial_user_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reload_skills: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_title: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    watch_paths: Vec<hookkit_core::Utf8PathBuf>,
}

impl SessionStartOutput {
    pub fn no_op() -> Self {
        Self::Structured(StructuredSessionStartOutput::default())
    }

    pub fn with_context(context: impl Into<String>) -> Self {
        Self::Structured(StructuredSessionStartOutput {
            hook_specific_output: Some(SessionStartSpecific {
                hook_event_name: "SessionStart",
                additional_context: Some(context.into()),
                initial_user_message: None,
                reload_skills: None,
                session_title: None,
                watch_paths: Vec::new(),
            }),
            system_message: None,
        })
    }

    pub fn with_context_and_system_message(
        context: impl Into<String>,
        system_message: impl Into<String>,
    ) -> Self {
        Self::Structured(StructuredSessionStartOutput {
            hook_specific_output: Some(SessionStartSpecific {
                hook_event_name: "SessionStart",
                additional_context: Some(context.into()),
                initial_user_message: None,
                reload_skills: None,
                session_title: None,
                watch_paths: Vec::new(),
            }),
            system_message: Some(system_message.into()),
        })
    }

    pub fn text_context(context: impl Into<String>) -> Self {
        Self::Text(context.into())
    }

    pub fn with_initial_user_message(message: impl Into<String>) -> Self {
        Self::Structured(StructuredSessionStartOutput {
            hook_specific_output: Some(SessionStartSpecific {
                hook_event_name: "SessionStart",
                additional_context: None,
                initial_user_message: Some(message.into()),
                reload_skills: None,
                session_title: None,
                watch_paths: Vec::new(),
            }),
            system_message: None,
        })
    }

    pub fn structured(
        context: Option<String>,
        reload_skills: Option<bool>,
        session_title: Option<String>,
        watch_paths: Vec<hookkit_core::Utf8PathBuf>,
    ) -> Self {
        Self::Structured(StructuredSessionStartOutput {
            hook_specific_output: Some(SessionStartSpecific {
                hook_event_name: "SessionStart",
                additional_context: context,
                initial_user_message: None,
                reload_skills,
                session_title,
                watch_paths,
            }),
            system_message: None,
        })
    }
}

pub enum SessionStart {}

impl EventSpec for SessionStart {
    type Input = SessionStartInput;
    type CommandEnvironment = ClaudeCommandEnvironment;
    type CommandOutput = SessionStartOutput;
    const HARNESS: HarnessId = HarnessId::CLAUDE_CODE;
    const SNAPSHOT: SnapshotId = SNAPSHOT_ID;
    const EVENT: EventId = EventId::builtin(HarnessId::CLAUDE_CODE, "SessionStart");
    const CATEGORY: EventCategory = EventCategory::Session;
    const CONTRACT: ContractId = ContractId::builtin("claude-code/docs-2026-07-12-r2/SessionStart");

    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        require_event(invocation, "SessionStart")?;
        serde_json::from_value(invocation.json().clone()).map_err(Into::into)
    }

    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        match output {
            SessionStartOutput::Structured(output) => {
                ProcessEmission::command_json(Self::CONTRACT, &output)
            }
            SessionStartOutput::Text(context) => {
                Ok(ProcessEmission::command_text(Self::CONTRACT, context))
            }
        }
    }

    fn validate_command_environment(
        input: &Self::Input,
        environment: &Self::CommandEnvironment,
    ) -> hookkit_core::Result<()> {
        environment.validate_input(
            &Self::EVENT,
            &input.session_id,
            input.effort.as_ref().map(|effort| effort.level.as_str()),
        )
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
pub struct PostToolUseInput {
    pub session_id: String,
    pub transcript_path: hookkit_core::Utf8PathBuf,
    pub cwd: hookkit_core::Utf8PathBuf,
    pub hook_event_name: String,
    pub tool_name: String,
    pub tool_input: serde_json::Value,
    pub tool_use_id: String,
    pub tool_response: serde_json::Value,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub agent_type: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<f64>,
    #[serde(default)]
    pub effort: Option<Effort>,
    #[serde(default)]
    pub permission_mode: Option<PermissionMode>,
    #[serde(default)]
    pub prompt_id: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
pub enum PostToolUseOutput {
    NoOp,
    Structured(StructuredPostToolUseOutput),
    BlockingError {
        message: String,
    },
    WithProtocolStderr {
        output: Box<PostToolUseOutput>,
        stderr: Vec<u8>,
    },
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_tool_output: Option<serde_json::Value>,
}

impl PostToolUseOutput {
    pub fn no_op() -> Self {
        Self::NoOp
    }

    pub fn with_context(context: impl Into<String>) -> Self {
        Self::Structured(StructuredPostToolUseOutput {
            hook_specific_output: Some(PostToolUseSpecific {
                hook_event_name: "PostToolUse",
                additional_context: Some(context.into()),
                updated_mcp_tool_output: None,
                updated_tool_output: None,
            }),
            ..StructuredPostToolUseOutput::default()
        })
    }

    pub fn block(reason: impl Into<String>) -> Self {
        Self::Structured(StructuredPostToolUseOutput {
            decision: Some("block"),
            reason: Some(reason.into()),
            ..StructuredPostToolUseOutput::default()
        })
    }

    pub fn with_block(mut self, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        let output = self.structured_mut()?;
        output.decision = Some("block");
        output.reason = Some(reason.into());
        Ok(self)
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

    pub fn with_updated_mcp_tool_output(
        mut self,
        output: serde_json::Value,
    ) -> hookkit_core::Result<Self> {
        self.specific_mut()?.updated_mcp_tool_output = Some(output);
        Ok(self)
    }

    pub fn with_updated_tool_output(
        mut self,
        output: serde_json::Value,
    ) -> hookkit_core::Result<Self> {
        self.specific_mut()?.updated_tool_output = Some(output);
        Ok(self)
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
                updated_tool_output: None,
            }))
    }
}

pub enum PostToolUse {}

impl EventSpec for PostToolUse {
    type Input = PostToolUseInput;
    type CommandEnvironment = ClaudeCommandEnvironment;
    type CommandOutput = PostToolUseOutput;
    const HARNESS: HarnessId = HarnessId::CLAUDE_CODE;
    const SNAPSHOT: SnapshotId = SNAPSHOT_ID;
    const EVENT: EventId = EventId::builtin(HarnessId::CLAUDE_CODE, "PostToolUse");
    const CATEGORY: EventCategory = EventCategory::Tool;
    const CONTRACT: ContractId = ContractId::builtin("claude-code/docs-2026-07-12-r2/PostToolUse");

    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        require_event(invocation, "PostToolUse")?;
        let input: Self::Input = serde_json::from_value(invocation.json().clone())?;
        if input.duration_ms.is_some_and(|duration| duration < 0.0) {
            return Err(invalid_input(
                "PostToolUse",
                "duration_ms must be non-negative",
            ));
        }
        Ok(input)
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

    fn validate_command_environment(
        input: &Self::Input,
        environment: &Self::CommandEnvironment,
    ) -> hookkit_core::Result<()> {
        environment.validate_input(
            &Self::EVENT,
            &input.session_id,
            input.effort.as_ref().map(|effort| effort.level.as_str()),
        )
    }

    fn context(input: &Self::Input) -> NativeContext {
        NativeContext {
            workspace_roots: vec![input.cwd.clone()],
            session_id: SessionId::new(&input.session_id).ok(),
            transcript_path: Some(input.transcript_path.clone()),
            tool_call_id: ToolCallId::new(&input.tool_use_id).ok(),
            ..NativeContext::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeCreateInput {
    pub session_id: String,
    pub transcript_path: hookkit_core::Utf8PathBuf,
    pub cwd: hookkit_core::Utf8PathBuf,
    pub hook_event_name: String,
    pub name: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct WorktreeCreateOutput(WorktreeCreateOutcome);

#[derive(Debug, Clone)]
enum WorktreeCreateOutcome {
    Created {
        path: hookkit_core::Utf8PathBuf,
        trailing_newline: bool,
    },
    Failed {
        message: String,
        exit_code: u8,
    },
}

impl WorktreeCreateOutput {
    pub fn path(path: hookkit_core::Utf8PathBuf) -> hookkit_core::Result<Self> {
        if !path.is_absolute() {
            return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "worktree path must be absolute",
            ));
        }
        Ok(Self(WorktreeCreateOutcome::Created {
            path,
            trailing_newline: false,
        }))
    }

    pub fn path_with_newline(path: hookkit_core::Utf8PathBuf) -> hookkit_core::Result<Self> {
        let output = Self::path(path)?;
        Ok(Self(match output.0 {
            WorktreeCreateOutcome::Created { path, .. } => WorktreeCreateOutcome::Created {
                path,
                trailing_newline: true,
            },
            WorktreeCreateOutcome::Failed { .. } => unreachable!(),
        }))
    }

    pub fn failed(message: impl Into<String>, exit_code: u8) -> hookkit_core::Result<Self> {
        if exit_code == 0 {
            return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "worktree failure exit code must be nonzero",
            ));
        }
        Ok(Self(WorktreeCreateOutcome::Failed {
            message: message.into(),
            exit_code,
        }))
    }
}

pub enum WorktreeCreate {}

impl EventSpec for WorktreeCreate {
    type Input = WorktreeCreateInput;
    type CommandEnvironment = ClaudeCommandEnvironment;
    type CommandOutput = WorktreeCreateOutput;
    const HARNESS: HarnessId = HarnessId::CLAUDE_CODE;
    const SNAPSHOT: SnapshotId = SNAPSHOT_ID;
    const EVENT: EventId = EventId::builtin(HarnessId::CLAUDE_CODE, "WorktreeCreate");
    const CATEGORY: EventCategory = EventCategory::Worktree;
    const CONTRACT: ContractId =
        ContractId::builtin("claude-code/docs-2026-07-12-r2/WorktreeCreate");

    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        require_event(invocation, "WorktreeCreate")?;
        let input: Self::Input = serde_json::from_value(invocation.json().clone())?;
        if input.session_id.is_empty() || input.cwd.as_str().is_empty() || input.name.is_empty() {
            return Err(invalid_input(
                "WorktreeCreate",
                "session_id, cwd, and name must not be empty",
            ));
        }
        Ok(input)
    }

    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        Ok(match output.0 {
            WorktreeCreateOutcome::Created {
                path,
                trailing_newline,
            } => {
                let mut bytes = path.as_str().as_bytes().to_vec();
                if trailing_newline {
                    bytes.push(b'\n');
                }
                ProcessEmission::command_text(Self::CONTRACT, bytes)
            }
            WorktreeCreateOutcome::Failed { message, exit_code } => {
                return ProcessEmission::command_stderr(Self::CONTRACT, message, exit_code);
            }
        })
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
    let actual = invocation
        .json()
        .get("hook_event_name")
        .and_then(serde_json::Value::as_str)
        .ok_or(hookkit_core::HookkitError::MissingHookEventName)?;
    if actual != expected {
        return Err(hookkit_core::HookkitError::InvalidForHint {
            harness: HarnessId::CLAUDE_CODE,
            event: EventId::builtin(HarnessId::CLAUDE_CODE, expected),
            message: format!("expected hook_event_name={expected}, got {actual}"),
        });
    }
    Ok(())
}

fn invalid_input(event: &'static str, message: impl Into<String>) -> hookkit_core::HookkitError {
    hookkit_core::HookkitError::InvalidInputForHint {
        event: EventId::builtin(HarnessId::CLAUDE_CODE, event),
        message: message.into(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    SessionStart,
    PostToolUse,
    WorktreeCreate,
}

impl EventSelector for Event {
    fn event_id(&self) -> EventId {
        let name = match self {
            Self::SessionStart => "SessionStart",
            Self::PostToolUse => "PostToolUse",
            Self::WorktreeCreate => "WorktreeCreate",
        };
        EventId::builtin(HarnessId::CLAUDE_CODE, name)
    }
}

#[derive(Debug, Clone)]
pub enum AnyInput {
    SessionStart(SessionStartInput),
    PostToolUse(PostToolUseInput),
    WorktreeCreate(WorktreeCreateInput),
}

#[derive(Debug, Clone)]
pub enum AnyCommandOutput {
    SessionStart(SessionStartOutput),
    PostToolUse(PostToolUseOutput),
    WorktreeCreate(WorktreeCreateOutput),
}

pub enum ClaudeCode {}

impl HarnessSpec for ClaudeCode {
    type AnyInput = AnyInput;
    type CommandEnvironment = ClaudeCommandEnvironment;
    type AnyCommandOutput = AnyCommandOutput;
    type EventSelector = Event;

    const ID: HarnessId = HarnessId::CLAUDE_CODE;
    const SNAPSHOT: SnapshotId = SNAPSHOT_ID;

    fn identification_descriptors() -> Vec<IdentificationDescriptor> {
        identification_descriptors()
    }

    fn decode(event: &EventId, raw: &RawInvocation) -> hookkit_core::Result<Self::AnyInput> {
        match event.name() {
            "SessionStart" => SessionStart::parse(raw).map(AnyInput::SessionStart),
            "PostToolUse" => PostToolUse::parse(raw).map(AnyInput::PostToolUse),
            "WorktreeCreate" => WorktreeCreate::parse(raw).map(AnyInput::WorktreeCreate),
            _ => Err(hookkit_core::HookkitError::UnrecognizedEvent {
                harness: Self::ID,
                message: event.name().to_string(),
            }),
        }
    }

    fn input_event(input: &Self::AnyInput) -> EventId {
        match input {
            AnyInput::SessionStart(_) => SessionStart::EVENT,
            AnyInput::PostToolUse(_) => PostToolUse::EVENT,
            AnyInput::WorktreeCreate(_) => WorktreeCreate::EVENT,
        }
    }

    fn validate_command_environment(
        input: &Self::AnyInput,
        environment: &Self::CommandEnvironment,
    ) -> hookkit_core::Result<()> {
        match input {
            AnyInput::SessionStart(input) => {
                SessionStart::validate_command_environment(input, environment)
            }
            AnyInput::PostToolUse(input) => {
                PostToolUse::validate_command_environment(input, environment)
            }
            AnyInput::WorktreeCreate(input) => {
                WorktreeCreate::validate_command_environment(input, environment)
            }
        }
    }

    fn output_event(output: &Self::AnyCommandOutput) -> EventId {
        match output {
            AnyCommandOutput::SessionStart(_) => SessionStart::EVENT,
            AnyCommandOutput::PostToolUse(_) => PostToolUse::EVENT,
            AnyCommandOutput::WorktreeCreate(_) => WorktreeCreate::EVENT,
        }
    }

    fn encode_command_unchecked(
        output: Self::AnyCommandOutput,
    ) -> hookkit_core::Result<ProcessEmission> {
        match output {
            AnyCommandOutput::SessionStart(output) => SessionStart::emit(output),
            AnyCommandOutput::PostToolUse(output) => PostToolUse::emit(output),
            AnyCommandOutput::WorktreeCreate(output) => WorktreeCreate::emit(output),
        }
    }

    fn context(input: &Self::AnyInput) -> NativeContext {
        match input {
            AnyInput::SessionStart(input) => SessionStart::context(input),
            AnyInput::PostToolUse(input) => PostToolUse::context(input),
            AnyInput::WorktreeCreate(input) => WorktreeCreate::context(input),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_start_discriminator_is_fixed_by_output_type() {
        let emission = SessionStart::emit(SessionStartOutput::with_context("ctx")).unwrap();
        let value: serde_json::Value = serde_json::from_slice(emission.stdout()).unwrap();
        assert_eq!(value["hookSpecificOutput"]["hookEventName"], "SessionStart");
    }

    #[test]
    fn session_start_context_and_system_message_keep_session_discriminator() {
        let emission = SessionStart::emit(SessionStartOutput::with_context_and_system_message(
            "ctx", "system",
        ))
        .unwrap();
        let value: serde_json::Value = serde_json::from_slice(emission.stdout()).unwrap();
        assert_eq!(value["hookSpecificOutput"]["hookEventName"], "SessionStart");
        assert_eq!(value["systemMessage"], "system");
    }

    #[test]
    fn post_tool_use_parses_snake_case_and_retains_unknown_fields() {
        let raw = RawInvocation::parse(
            br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"PostToolUse","tool_name":"Bash","tool_input":{"command":"cargo test"},"tool_use_id":"call-1","tool_response":{},"future":true}"#.to_vec(),
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
        assert!(!emission.stdout().ends_with(b"\n"));
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
    fn session_start_text_is_not_json_or_newline_normalized() {
        let emission = SessionStart::emit(SessionStartOutput::text_context("context")).unwrap();
        assert_eq!(emission.stdout(), b"context");
    }

    #[test]
    fn worktree_command_is_absolute_plain_text_without_newline() {
        let output = WorktreeCreateOutput::path("/tmp/worktree".into()).unwrap();
        let emission = WorktreeCreate::emit(output).unwrap();
        assert_eq!(emission.stdout(), b"/tmp/worktree");
        assert!(!emission.stdout().ends_with(b"\n"));
    }

    #[test]
    fn worktree_failure_is_stderr_only() {
        let output = WorktreeCreateOutput::failed("cannot create", 7).unwrap();
        let emission = WorktreeCreate::emit(output).unwrap();
        assert!(emission.stdout().is_empty());
        assert_eq!(emission.stderr(), b"cannot create");
        assert_eq!(emission.exit_code(), 7);
    }

    #[test]
    fn input_constraints_reject_invalid_enum_range_and_min_length() {
        let session = RawInvocation::parse(
            br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"SessionStart","source":"startup","permission_mode":"invalid"}"#.to_vec(),
        )
        .unwrap();
        assert!(SessionStart::parse(&session).is_err());

        let post = RawInvocation::parse(
            br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"PostToolUse","tool_name":"Bash","tool_input":{},"tool_use_id":"u","tool_response":{},"duration_ms":-1}"#.to_vec(),
        )
        .unwrap();
        assert!(PostToolUse::parse(&post).is_err());

        let worktree = RawInvocation::parse(
            br#"{"session_id":"","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"WorktreeCreate","name":"w"}"#.to_vec(),
        )
        .unwrap();
        assert!(WorktreeCreate::parse(&worktree).is_err());
    }

    #[test]
    fn invalid_builder_order_and_raw_protocol_stderr_return_errors() {
        assert!(
            PostToolUseOutput::blocking_error("blocked")
                .with_block("again")
                .is_err()
        );
        let invalid = PostToolUseOutput::WithProtocolStderr {
            output: Box::new(PostToolUseOutput::no_op()),
            stderr: vec![0xff],
        };
        assert!(PostToolUse::emit(invalid).is_err());
    }
}
