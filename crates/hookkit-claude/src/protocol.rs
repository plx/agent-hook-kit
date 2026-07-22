//! Contract-first event specifications for the selected catalog snapshot.

use hookkit_core::{
    ContractId, EventCategory, EventId, EventSelector, EventSpec, HarnessId, HarnessSpec,
    IdentificationDescriptor, NativeContext, ProcessEmission, RawInvocation,
    SessionBoundaryContext, SessionBoundaryKind, SessionId, SnapshotId, ToolCallId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::ClaudeCommandEnvironment;

/// Claude Code protocol documentation snapshot implemented by this crate.
pub const SNAPSHOT_ID: SnapshotId = SnapshotId::builtin("docs-2026-07-12-r2");

/// Returns every Claude Code event with a native command implementation.
pub fn events() -> Vec<hookkit_core::NativeEventDescriptor> {
    let mut events = vec![
        hookkit_core::NativeEventDescriptor::command::<SessionStart>(&[
            "command-structured",
            "command-text",
        ]),
        hookkit_core::NativeEventDescriptor::command::<PostToolUse>(&["command-structured"]),
        hookkit_core::NativeEventDescriptor::command::<WorktreeCreate>(&[
            "command-created",
            "command-failed",
        ]),
    ];
    events.extend(crate::catalog::events());
    events
}

/// Returns discriminator-based identification metadata for this snapshot.
pub fn identification_descriptors() -> Vec<IdentificationDescriptor> {
    let mut descriptors = vec![
        IdentificationDescriptor::definitive::<SessionStart>("/hook_event_name", "SessionStart"),
        IdentificationDescriptor::definitive::<PostToolUse>("/hook_event_name", "PostToolUse"),
        IdentificationDescriptor::definitive::<WorktreeCreate>(
            "/hook_event_name",
            "WorktreeCreate",
        ),
    ];
    descriptors.extend(crate::catalog::identification_descriptors());
    descriptors
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Native Claude Code input observed at a session boundary.
pub struct SessionStartInput {
    /// Native session identifier.
    pub session_id: String,
    /// Path to the native conversation transcript.
    pub transcript_path: hookkit_core::Utf8PathBuf,
    /// Current workspace directory.
    pub cwd: hookkit_core::Utf8PathBuf,
    /// Authoritative native event discriminator.
    pub hook_event_name: String,
    /// Cause of the session boundary.
    pub source: SessionSource,
    /// Subagent identifier when a subagent session starts.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Subagent type when a subagent session starts.
    #[serde(default)]
    pub agent_type: Option<String>,
    /// Optional nested effort setting.
    #[serde(default)]
    pub effort: Option<Effort>,
    /// Model configured for the session.
    #[serde(default)]
    pub model: Option<String>,
    /// Permission policy active for the session.
    #[serde(default)]
    pub permission_mode: Option<PermissionMode>,
    /// Prompt identifier associated with session startup.
    #[serde(default)]
    pub prompt_id: Option<String>,
    /// Existing or requested session title.
    #[serde(default)]
    pub session_title: Option<String>,
    /// Unknown protocol fields retained for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Native cause of a Claude Code session-start event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionSource {
    /// A newly started session.
    Startup,
    /// A previously persisted session was resumed.
    Resume,
    /// The current conversation context was cleared.
    Clear,
    /// The current conversation context was compacted.
    Compact,
}

/// Claude Code permission policy active for a hook event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    /// Use the default interactive permission policy.
    Default,
    /// Restrict the agent to planning behavior.
    Plan,
    /// Automatically accept file-edit operations.
    AcceptEdits,
    /// Automatically choose permissions under Claude's automatic policy.
    Auto,
    /// Do not prompt for otherwise disallowed operations.
    DontAsk,
    /// Bypass normal permission checks.
    BypassPermissions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Nested effort object used by Claude native event payloads.
pub struct Effort {
    /// Requested effort level.
    pub level: EffortLevel,
}

/// Known Claude effort levels.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EffortLevel {
    /// Low effort.
    Low,
    /// Medium effort.
    Medium,
    /// High effort.
    High,
    /// Extra-high effort.
    Xhigh,
    /// Maximum effort.
    Max,
}

impl EffortLevel {
    /// Returns the native lowercase wire spelling.
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
/// Native response from a Claude Code session-start command hook.
pub enum SessionStartOutput {
    /// Structured JSON response.
    Structured(StructuredSessionStartOutput),
    /// Plain-text context response, emitted without a trailing newline.
    Text(String),
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
/// Structured session-start response built through [`SessionStartOutput`].
///
/// Fields are private so the required hook-specific discriminator cannot be
/// omitted when hook-specific values are present.
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
    /// Creates an empty structured response.
    pub fn no_op() -> Self {
        Self::Structured(StructuredSessionStartOutput::default())
    }

    /// Creates a structured response that appends agent context.
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

    /// Creates a response containing both agent context and a system message.
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

    /// Creates a successful plain-text context response.
    pub fn text_context(context: impl Into<String>) -> Self {
        Self::Text(context.into())
    }

    /// Creates a structured response that injects an initial user message.
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

    /// Creates a structured response with all session-start controls.
    ///
    /// Empty `watch_paths` are omitted. The values are retained verbatim and
    /// are not checked for existence or uniqueness.
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

/// Native Claude Code `SessionStart` command contract.
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
        let boundary_kind = match input.source {
            SessionSource::Startup => SessionBoundaryKind::Startup,
            SessionSource::Resume => SessionBoundaryKind::Resume,
            SessionSource::Clear => SessionBoundaryKind::Clear,
            SessionSource::Compact => SessionBoundaryKind::Compact,
        };
        NativeContext {
            workspace_roots: vec![input.cwd.clone()],
            session_id: SessionId::new(&input.session_id).ok(),
            transcript_path: Some(input.transcript_path.clone()),
            session_boundary: Some(SessionBoundaryContext::observed(boundary_kind)),
            ..NativeContext::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Native Claude Code input observed after a tool invocation.
pub struct PostToolUseInput {
    /// Native session identifier.
    pub session_id: String,
    /// Path to the native conversation transcript.
    pub transcript_path: hookkit_core::Utf8PathBuf,
    /// Current workspace directory.
    pub cwd: hookkit_core::Utf8PathBuf,
    /// Authoritative native event discriminator.
    pub hook_event_name: String,
    /// Harness-native tool name.
    pub tool_name: String,
    /// Tool arguments in their native JSON shape.
    pub tool_input: serde_json::Value,
    /// Native tool-call identifier.
    pub tool_use_id: String,
    /// Tool result in its native JSON shape.
    pub tool_response: serde_json::Value,
    /// Subagent identifier when the tool ran on behalf of a subagent.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Subagent type when the tool ran on behalf of a subagent.
    #[serde(default)]
    pub agent_type: Option<String>,
    /// Tool duration in milliseconds; when present it must be non-negative.
    #[serde(default)]
    pub duration_ms: Option<f64>,
    /// Optional nested effort setting.
    #[serde(default)]
    pub effort: Option<Effort>,
    /// Permission policy active for the tool invocation.
    #[serde(default)]
    pub permission_mode: Option<PermissionMode>,
    /// Prompt identifier associated with the current turn.
    #[serde(default)]
    pub prompt_id: Option<String>,
    /// Unknown protocol fields retained for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Native response from a Claude Code post-tool command hook.
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
/// Structured post-tool response built through [`PostToolUseOutput`].
///
/// Fields are private so callers cannot bypass the builder's ordering checks.
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
                updated_tool_output: None,
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

    /// Creates a code-2 blocking response with required stderr text.
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

    /// Replaces the non-MCP tool output in a structured hook-specific response.
    pub fn with_updated_tool_output(
        mut self,
        output: serde_json::Value,
    ) -> hookkit_core::Result<Self> {
        self.specific_mut()?.updated_tool_output = Some(output);
        Ok(self)
    }

    /// Sets Claude's top-level `continue` control on a structured response.
    pub fn with_continue(mut self, continue_session: bool) -> hookkit_core::Result<Self> {
        self.structured_mut()?.continue_session = Some(continue_session);
        Ok(self)
    }

    /// Sets the top-level stop reason on a structured response.
    pub fn with_stop_reason(mut self, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        self.structured_mut()?.stop_reason = Some(reason.into());
        Ok(self)
    }

    /// Sets whether Claude suppresses ordinary tool output.
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
                updated_tool_output: None,
            }))
    }
}

/// Native Claude Code `PostToolUse` command contract.
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
/// Native Claude Code input requesting creation of a worktree.
pub struct WorktreeCreateInput {
    /// Native session identifier.
    pub session_id: String,
    /// Path to the native conversation transcript.
    pub transcript_path: hookkit_core::Utf8PathBuf,
    /// Current workspace directory.
    pub cwd: hookkit_core::Utf8PathBuf,
    /// Authoritative native event discriminator.
    pub hook_event_name: String,
    /// Requested worktree name.
    pub name: String,
    /// Unknown protocol fields retained for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
/// Native response from a Claude Code worktree-create command hook.
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
    /// Creates a successful response containing an absolute worktree path.
    ///
    /// The path is emitted as exact UTF-8 text without a trailing newline.
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

    /// Creates a successful absolute-path response with one trailing newline.
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

    /// Creates a failed response written to stderr.
    ///
    /// `exit_code` must be nonzero. Unlike required-stderr outcomes, the
    /// message may be empty because the native worktree contract permits it.
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

/// Native Claude Code `WorktreeCreate` command contract.
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

pub(crate) fn require_event(
    invocation: &RawInvocation,
    expected: &'static str,
) -> hookkit_core::Result<()> {
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
/// Compile-time selector for an implemented Claude Code event.
pub enum Event {
    /// Selects [`SessionStart`].
    SessionStart,
    /// Selects [`PostToolUse`].
    PostToolUse,
    /// Selects [`WorktreeCreate`].
    WorktreeCreate,
    /// Selects [`crate::catalog::ConfigChange`].
    ConfigChange,
    /// Selects [`crate::catalog::CwdChanged`].
    CwdChanged,
    /// Selects [`crate::catalog::Elicitation`].
    Elicitation,
    /// Selects [`crate::catalog::ElicitationResult`].
    ElicitationResult,
    /// Selects [`crate::catalog::FileChanged`].
    FileChanged,
    /// Selects [`crate::catalog::InstructionsLoaded`].
    InstructionsLoaded,
    /// Selects [`crate::catalog::MessageDisplay`].
    MessageDisplay,
    /// Selects [`crate::catalog::Notification`].
    Notification,
    /// Selects [`crate::catalog::PermissionDenied`].
    PermissionDenied,
    /// Selects [`crate::catalog::PermissionRequest`].
    PermissionRequest,
    /// Selects [`crate::catalog::PostCompact`].
    PostCompact,
    /// Selects [`crate::catalog::PostToolBatch`].
    PostToolBatch,
    /// Selects [`crate::catalog::PostToolUseFailure`].
    PostToolUseFailure,
    /// Selects [`crate::catalog::PreCompact`].
    PreCompact,
    /// Selects [`crate::catalog::PreToolUse`].
    PreToolUse,
    /// Selects [`crate::catalog::SessionEnd`].
    SessionEnd,
    /// Selects [`crate::catalog::Setup`].
    Setup,
    /// Selects [`crate::catalog::Stop`].
    Stop,
    /// Selects [`crate::catalog::StopFailure`].
    StopFailure,
    /// Selects [`crate::catalog::SubagentStart`].
    SubagentStart,
    /// Selects [`crate::catalog::SubagentStop`].
    SubagentStop,
    /// Selects [`crate::catalog::TaskCompleted`].
    TaskCompleted,
    /// Selects [`crate::catalog::TaskCreated`].
    TaskCreated,
    /// Selects [`crate::catalog::TeammateIdle`].
    TeammateIdle,
    /// Selects [`crate::catalog::UserPromptExpansion`].
    UserPromptExpansion,
    /// Selects [`crate::catalog::UserPromptSubmit`].
    UserPromptSubmit,
    /// Selects [`crate::catalog::WorktreeRemove`].
    WorktreeRemove,
}

impl EventSelector for Event {
    fn event_id(&self) -> EventId {
        let name = match self {
            Self::SessionStart => "SessionStart",
            Self::PostToolUse => "PostToolUse",
            Self::WorktreeCreate => "WorktreeCreate",
            Self::ConfigChange => "ConfigChange",
            Self::CwdChanged => "CwdChanged",
            Self::Elicitation => "Elicitation",
            Self::ElicitationResult => "ElicitationResult",
            Self::FileChanged => "FileChanged",
            Self::InstructionsLoaded => "InstructionsLoaded",
            Self::MessageDisplay => "MessageDisplay",
            Self::Notification => "Notification",
            Self::PermissionDenied => "PermissionDenied",
            Self::PermissionRequest => "PermissionRequest",
            Self::PostCompact => "PostCompact",
            Self::PostToolBatch => "PostToolBatch",
            Self::PostToolUseFailure => "PostToolUseFailure",
            Self::PreCompact => "PreCompact",
            Self::PreToolUse => "PreToolUse",
            Self::SessionEnd => "SessionEnd",
            Self::Setup => "Setup",
            Self::Stop => "Stop",
            Self::StopFailure => "StopFailure",
            Self::SubagentStart => "SubagentStart",
            Self::SubagentStop => "SubagentStop",
            Self::TaskCompleted => "TaskCompleted",
            Self::TaskCreated => "TaskCreated",
            Self::TeammateIdle => "TeammateIdle",
            Self::UserPromptExpansion => "UserPromptExpansion",
            Self::UserPromptSubmit => "UserPromptSubmit",
            Self::WorktreeRemove => "WorktreeRemove",
        };
        EventId::builtin(HarnessId::CLAUDE_CODE, name)
    }
}

#[derive(Debug, Clone)]
/// Lossless sum type over all implemented Claude Code inputs.
pub enum AnyInput {
    /// A session-start input.
    SessionStart(SessionStartInput),
    /// A post-tool input.
    PostToolUse(PostToolUseInput),
    /// A worktree-create input.
    WorktreeCreate(WorktreeCreateInput),
    /// An input for another implemented catalog event.
    Catalog(crate::catalog::CatalogInput),
}

#[derive(Debug, Clone)]
/// Sum type over all implemented Claude Code command outputs.
pub enum AnyCommandOutput {
    /// A session-start output.
    SessionStart(SessionStartOutput),
    /// A post-tool output.
    PostToolUse(PostToolUseOutput),
    /// A worktree-create output.
    WorktreeCreate(WorktreeCreateOutput),
    /// An output for another implemented catalog event.
    Catalog(crate::catalog::CatalogOutput),
}

/// Harness adapter implementing the pinned Claude Code snapshot.
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
            AnyInput::SessionStart(_) => SessionStart::EVENT,
            AnyInput::PostToolUse(_) => PostToolUse::EVENT,
            AnyInput::WorktreeCreate(_) => WorktreeCreate::EVENT,
            AnyInput::Catalog(input) => input.event_id(),
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
            AnyInput::Catalog(input) => input.validate_environment(environment),
        }
    }

    fn output_event(output: &Self::AnyCommandOutput) -> EventId {
        match output {
            AnyCommandOutput::SessionStart(_) => SessionStart::EVENT,
            AnyCommandOutput::PostToolUse(_) => PostToolUse::EVENT,
            AnyCommandOutput::WorktreeCreate(_) => WorktreeCreate::EVENT,
            AnyCommandOutput::Catalog(output) => output.event_id(),
        }
    }

    fn encode_command_unchecked(
        output: Self::AnyCommandOutput,
    ) -> hookkit_core::Result<ProcessEmission> {
        match output {
            AnyCommandOutput::SessionStart(output) => SessionStart::emit(output),
            AnyCommandOutput::PostToolUse(output) => PostToolUse::emit(output),
            AnyCommandOutput::WorktreeCreate(output) => WorktreeCreate::emit(output),
            AnyCommandOutput::Catalog(output) => output.emit(),
        }
    }

    fn context(input: &Self::AnyInput) -> NativeContext {
        match input {
            AnyInput::SessionStart(input) => SessionStart::context(input),
            AnyInput::PostToolUse(input) => PostToolUse::context(input),
            AnyInput::WorktreeCreate(input) => WorktreeCreate::context(input),
            AnyInput::Catalog(input) => input.context(),
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
    fn session_start_context_exposes_typed_boundary_cause() {
        let raw = RawInvocation::parse(
            br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"SessionStart","source":"resume"}"#.to_vec(),
        )
        .unwrap();
        let context = SessionStart::context(&SessionStart::parse(&raw).unwrap());
        assert_eq!(
            context.session_boundary.unwrap().kind,
            SessionBoundaryKind::Resume
        );
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
