//! Contract-first event specifications for the selected catalog snapshot.

use hookkit_core::{
    ContractId, EventCategory, EventId, EventSelector, EventSpec, HarnessId, HarnessSpec,
    IdentificationDescriptor, NativeContext, ProcessEmission, RawInvocation,
    SessionBoundaryContext, SessionBoundaryKind, SessionId, SnapshotId, ToolCallId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::ClaudeCommandEnvironment;
use crate::model_switch::{
    ModelSwitchInput, PostModelSwitch, PostModelSwitchOutput, PreModelSwitch, PreModelSwitchOutput,
};

pub use crate::values::{
    CacheTtl, CompactTrigger, Effort, EffortLevel, McpServer, McpServerSource, ModelSwitchPricing,
    ModelSwitchSource, NotificationType, PermissionMode, SessionEndReason, SessionSource,
    StopFailureError,
};

/// Claude Code protocol documentation snapshot implemented by this crate.
pub const SNAPSHOT_ID: SnapshotId = SnapshotId::builtin("docs-2026-09-29-r1");

/// Expands to the [`ContractId`] of a Claude event in the selected snapshot.
macro_rules! contract_id {
    ($name:literal) => {
        hookkit_core::ContractId::builtin(concat!("claude-code/docs-2026-09-29-r1/", $name))
    };
}
pub(crate) use contract_id;

/// Returns every Claude Code event with a native command implementation.
///
/// Each descriptor names the snapshot process fixtures that a typed
/// constructor reproduces exactly; see [`crate::catalog::events`].
pub fn events() -> Vec<hookkit_core::NativeEventDescriptor> {
    use hookkit_core::NativeEventDescriptor as D;
    let mut events = vec![
        D::command::<SessionStart>(&[
            "command-structured",
            "command-text",
            "command-text-open-brace",
            "command-nonzero-unstructured",
        ]),
        D::command::<PostToolUse>(&[
            "command-structured",
            "command-classifier-context",
            "command-exit-2",
            "command-exit-2-structured",
            "command-nonzero-unstructured",
        ]),
        D::command::<WorktreeCreate>(&["command-created", "command-failed"]),
        D::command::<PreModelSwitch>(&[
            "command-structured",
            "command-block",
            "command-exit-2",
            "command-exit-2-structured",
            "command-nonzero-unstructured",
        ]),
        D::command::<PostModelSwitch>(&[
            "command-structured",
            "command-text",
            "command-text-open-brace",
            "command-nonzero-unstructured",
        ]),
    ];
    events.extend(crate::catalog::events());
    events
}

/// Returns discriminator-based identification metadata for this snapshot.
pub fn identification_descriptors() -> Vec<IdentificationDescriptor> {
    const POINTER: &str = "/hook_event_name";
    let mut descriptors = vec![
        IdentificationDescriptor::definitive::<SessionStart>(POINTER, "SessionStart"),
        IdentificationDescriptor::definitive::<PostToolUse>(POINTER, "PostToolUse"),
        IdentificationDescriptor::definitive::<WorktreeCreate>(POINTER, "WorktreeCreate"),
        IdentificationDescriptor::definitive::<PreModelSwitch>(POINTER, "PreModelSwitch"),
        IdentificationDescriptor::definitive::<PostModelSwitch>(POINTER, "PostModelSwitch"),
    ];
    descriptors.extend(crate::catalog::identification_descriptors());
    descriptors
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Subagent type when a subagent session starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    /// Optional nested effort setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,
    /// Model configured for the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Permission policy active for the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
    /// Prompt identifier associated with session startup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_id: Option<String>,
    /// Existing or requested session title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_title: Option<String>,
    /// Session scratchpad directory (Claude Code v2.1.257 or later).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scratchpad_dir: Option<hookkit_core::Utf8PathBuf>,
    /// Seconds since the last response. Sent only for `resume` and `fork`
    /// when the transcript holds a response (Claude Code v2.1.251 or later).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::values::serialize_optional_js_number"
    )]
    pub seconds_since_last_response: Option<f64>,
    /// Tokens the next request re-sends. Sent only for `resume` and `fork`
    /// when the transcript holds a response (Claude Code v2.1.251 or later).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,
    /// Whether the prompt cache has likely expired. Sent only for `resume`
    /// and `fork` when the transcript holds a response (Claude Code v2.1.251
    /// or later).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_likely_expired: Option<bool>,
    /// Estimated US-dollar cost of re-caching the context. Sent only for
    /// `resume` and `fork` when the transcript holds a response (Claude Code
    /// v2.1.251 or later).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::values::serialize_optional_js_number"
    )]
    pub estimated_cache_write_usd: Option<f64>,
    /// Unknown protocol fields retained for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
/// Native response from a Claude Code session-start command hook.
///
/// Build it with the constructors below and refine structured responses with
/// the `with_*` builders.
pub struct SessionStartOutput(SessionStartOutcome);

#[derive(Debug, Clone)]
enum SessionStartOutcome {
    Structured(StructuredSessionStartOutput),
    Text(String),
    NonblockingError(String),
}

/// Structured session-start response built through [`SessionStartOutput`].
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct StructuredSessionStartOutput {
    #[serde(rename = "continue", skip_serializing_if = "Option::is_none")]
    continue_session: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    suppress_output: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hook_specific_output: Option<SessionStartSpecific>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    terminal_sequence: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    watch_paths: Option<Vec<hookkit_core::Utf8PathBuf>>,
}

impl SessionStartSpecific {
    fn new() -> Self {
        Self {
            hook_event_name: "SessionStart",
            additional_context: None,
            initial_user_message: None,
            reload_skills: None,
            session_title: None,
            watch_paths: None,
        }
    }
}

fn absolute_watch_paths(
    paths: Vec<hookkit_core::Utf8PathBuf>,
) -> hookkit_core::Result<Vec<hookkit_core::Utf8PathBuf>> {
    if paths.iter().any(|path| !path.is_absolute()) {
        return Err(hookkit_core::HookkitError::InvalidProcessEmission(
            "SessionStart watch paths must be absolute",
        ));
    }
    Ok(paths)
}

impl SessionStartOutput {
    /// Creates an empty structured response.
    pub fn no_op() -> Self {
        Self(SessionStartOutcome::Structured(
            StructuredSessionStartOutput::default(),
        ))
    }

    /// Creates a structured response that adds `additionalContext` for
    /// Claude at the start of the conversation, before the first prompt.
    ///
    /// Refine it with the `with_*` builders, for example
    /// [`Self::with_session_title`] or [`Self::with_initial_user_message`].
    pub fn with_context(context: impl Into<String>) -> Self {
        Self::structured_with(Some(context.into()), None)
    }

    /// Creates a response containing both agent context and a system message.
    pub fn with_context_and_system_message(
        context: impl Into<String>,
        system_message: impl Into<String>,
    ) -> Self {
        Self::structured_with(Some(context.into()), Some(system_message.into()))
    }

    fn structured_with(context: Option<String>, system_message: Option<String>) -> Self {
        let specific = context.map(|context| SessionStartSpecific {
            additional_context: Some(context),
            ..SessionStartSpecific::new()
        });
        Self(SessionStartOutcome::Structured(
            StructuredSessionStartOutput {
                hook_specific_output: specific,
                system_message,
                ..StructuredSessionStartOutput::default()
            },
        ))
    }

    /// Creates a successful plain-text context response.
    ///
    /// Claude Code v2.1.248 and later parse stdout whose trimmed text starts
    /// with `{` and ends with `}` as JSON and drop it when it is not a valid
    /// response. Such text is therefore emitted as structured
    /// `additionalContext` instead, so the context always reaches Claude.
    /// Other text is written verbatim, without a trailing newline.
    pub fn text_context(context: impl Into<String>) -> Self {
        let context = context.into();
        if crate::catalog::parsed_as_json(&context) {
            Self::with_context(context)
        } else {
            Self(SessionStartOutcome::Text(context))
        }
    }

    /// Reports a non-blocking hook error: exits 1 with required stderr, which
    /// Claude Code shows the user as a hook error notice. Claude does not see
    /// it and the session proceeds.
    pub fn nonblocking_error(message: impl Into<String>) -> Self {
        Self(SessionStartOutcome::NonblockingError(message.into()))
    }

    /// Creates a structured response setting the additional-context,
    /// skill-reload, session-title, and watch-path controls.
    ///
    /// Empty `watch_paths` are omitted. Every path must be absolute.
    #[deprecated(
        note = "use `no_op()` or `with_context(..)` with `with_reload_skills`, `with_session_title`, and `with_watch_paths`"
    )]
    pub fn structured(
        context: Option<String>,
        reload_skills: Option<bool>,
        session_title: Option<String>,
        watch_paths: Vec<hookkit_core::Utf8PathBuf>,
    ) -> hookkit_core::Result<Self> {
        let watch_paths = absolute_watch_paths(watch_paths)?;
        let mut specific = SessionStartSpecific::new();
        specific.additional_context = context;
        specific.reload_skills = reload_skills;
        specific.session_title = session_title;
        specific.watch_paths = (!watch_paths.is_empty()).then_some(watch_paths);
        Ok(Self(SessionStartOutcome::Structured(
            StructuredSessionStartOutput {
                hook_specific_output: Some(specific),
                ..StructuredSessionStartOutput::default()
            },
        )))
    }

    /// Sets `additionalContext`, replacing any earlier value.
    pub fn with_additional_context(
        mut self,
        context: impl Into<String>,
    ) -> hookkit_core::Result<Self> {
        self.specific_mut()?.additional_context = Some(context.into());
        Ok(self)
    }

    /// Sets `initialUserMessage`, the first user message of a non-interactive
    /// `-p` session. It creates a turn even when no prompt was provided.
    pub fn with_initial_user_message(
        mut self,
        message: impl Into<String>,
    ) -> hookkit_core::Result<Self> {
        self.specific_mut()?.initial_user_message = Some(message.into());
        Ok(self)
    }

    /// Sets `reloadSkills`. When `true`, Claude Code re-scans skill and
    /// command directories after SessionStart hooks complete.
    pub fn with_reload_skills(mut self, reload: bool) -> hookkit_core::Result<Self> {
        self.specific_mut()?.reload_skills = Some(reload);
        Ok(self)
    }

    /// Sets `sessionTitle`, with the same effect as `/rename`. Claude Code
    /// applies it for `startup`, `resume`, and `fork` and ignores it on
    /// `clear` and `compact`.
    pub fn with_session_title(mut self, title: impl Into<String>) -> hookkit_core::Result<Self> {
        self.specific_mut()?.session_title = Some(title.into());
        Ok(self)
    }

    /// Sets `watchPaths`, the absolute paths FileChanged watches during the
    /// session. Every path must be absolute; values are otherwise retained
    /// verbatim and are not checked for existence or uniqueness.
    pub fn with_watch_paths(
        mut self,
        paths: Vec<hookkit_core::Utf8PathBuf>,
    ) -> hookkit_core::Result<Self> {
        let paths = absolute_watch_paths(paths)?;
        self.specific_mut()?.watch_paths = Some(paths);
        Ok(self)
    }

    /// Sets Claude's universal top-level `continue` control.
    pub fn with_continue(mut self, continue_session: bool) -> hookkit_core::Result<Self> {
        self.structured_mut()?.continue_session = Some(continue_session);
        Ok(self)
    }

    /// Sets the universal top-level `stopReason`, shown to the user when
    /// `continue` is `false`.
    pub fn with_stop_reason(mut self, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        self.structured_mut()?.stop_reason = Some(reason.into());
        Ok(self)
    }

    /// Sets the universal `suppressOutput` field, which Claude Code accepts
    /// but ignores.
    #[deprecated(
        note = "Claude Code accepts `suppressOutput` but ignores it on every event (claude-code/docs-2026-09-29-r1)"
    )]
    pub fn with_suppress_output(mut self, suppress: bool) -> hookkit_core::Result<Self> {
        self.structured_mut()?.suppress_output = Some(suppress);
        Ok(self)
    }

    /// Sets the universal top-level `systemMessage`, a warning shown to the
    /// user.
    pub fn with_system_message(mut self, message: impl Into<String>) -> hookkit_core::Result<Self> {
        self.structured_mut()?.system_message = Some(message.into());
        Ok(self)
    }

    /// Requests emission of an allowlisted terminal notification sequence.
    pub fn with_terminal_sequence(
        mut self,
        sequence: impl Into<String>,
    ) -> hookkit_core::Result<Self> {
        self.structured_mut()?.terminal_sequence = Some(sequence.into());
        Ok(self)
    }

    fn structured_mut(&mut self) -> hookkit_core::Result<&mut StructuredSessionStartOutput> {
        match &mut self.0 {
            SessionStartOutcome::Structured(output) => Ok(output),
            _ => Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "structured fields cannot be added to plain-text or error SessionStart output",
            )),
        }
    }

    fn specific_mut(&mut self) -> hookkit_core::Result<&mut SessionStartSpecific> {
        Ok(self
            .structured_mut()?
            .hook_specific_output
            .get_or_insert_with(SessionStartSpecific::new))
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
    const CONTRACT: ContractId = contract_id!("SessionStart");

    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        require_event(invocation, "SessionStart")?;
        deserialize_input(invocation, "SessionStart")
    }

    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        match output.0 {
            SessionStartOutcome::Structured(output) => {
                ProcessEmission::command_json(Self::CONTRACT, &output)
            }
            SessionStartOutcome::Text(context) => {
                Ok(ProcessEmission::command_text(Self::CONTRACT, context))
            }
            SessionStartOutcome::NonblockingError(message) => {
                ProcessEmission::command_required_stderr(Self::CONTRACT, message, 1)
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
            SessionSource::Startup => Some(SessionBoundaryKind::Startup),
            SessionSource::Resume => Some(SessionBoundaryKind::Resume),
            SessionSource::Fork => Some(SessionBoundaryKind::Fork),
            SessionSource::Clear => Some(SessionBoundaryKind::Clear),
            SessionSource::Compact => Some(SessionBoundaryKind::Compact),
            SessionSource::Unknown(_) => None,
        };
        NativeContext {
            workspace_roots: vec![input.cwd.clone()],
            session_id: SessionId::new(&input.session_id).ok(),
            transcript_path: Some(input.transcript_path.clone()),
            session_boundary: boundary_kind.map(SessionBoundaryContext::observed),
            ..NativeContext::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
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
    /// MCP server that owns the tool, for MCP tools (Claude Code v2.1.274 or
    /// later).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_server: Option<McpServer>,
    /// Subagent identifier when the tool ran on behalf of a subagent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Subagent type when the tool ran on behalf of a subagent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    /// Tool duration in milliseconds; when present it must be non-negative.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::values::serialize_optional_js_number"
    )]
    pub duration_ms: Option<f64>,
    /// Optional nested effort setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,
    /// Permission policy active for the tool invocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
    /// Prompt identifier associated with the current turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_id: Option<String>,
    /// Session scratchpad directory (Claude Code v2.1.257 or later).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scratchpad_dir: Option<hookkit_core::Utf8PathBuf>,
    /// Unknown protocol fields retained for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Native response from a Claude Code post-tool command hook.
///
/// Build it with the constructors below and refine structured responses with
/// the `with_*` builders.
#[derive(Debug, Clone)]
pub struct PostToolUseOutput(PostToolUseOutcome);

#[derive(Debug, Clone)]
enum PostToolUseOutcome {
    NoOp,
    Structured(Box<StructuredPostToolUseOutput>),
    /// Exit 2 with required stderr, which Claude sees as feedback.
    FeedbackError(String),
    /// Exit 1 with required stderr, shown to the user as a hook error notice.
    NonblockingError(String),
    /// Exit 0 with the wrapped successful stdout and debug-log stderr.
    WithProtocolStderr {
        output: Box<PostToolUseOutcome>,
        stderr: String,
    },
    /// Exit 2 with the wrapped successful stdout and required stderr, which
    /// Claude sees as feedback while Claude Code still reads the JSON.
    StructuredFeedback {
        output: Box<PostToolUseOutcome>,
        stderr: String,
    },
}

/// Structured post-tool response built through [`PostToolUseOutput`].
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct StructuredPostToolUseOutput {
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
    terminal_sequence: Option<String>,
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
    classifier_context: Option<String>,
    #[serde(
        rename = "updatedMCPToolOutput",
        skip_serializing_if = "Option::is_none"
    )]
    updated_mcp_tool_output: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_tool_output: Option<serde_json::Value>,
}

impl PostToolUseSpecific {
    fn new() -> Self {
        Self {
            hook_event_name: "PostToolUse",
            additional_context: None,
            classifier_context: None,
            updated_mcp_tool_output: None,
            updated_tool_output: None,
        }
    }
}

impl PostToolUseOutput {
    /// Creates an empty JSON-object response.
    pub fn no_op() -> Self {
        Self(PostToolUseOutcome::NoOp)
    }

    /// Creates a structured response that adds `additionalContext` for
    /// Claude next to the tool result.
    pub fn with_context(context: impl Into<String>) -> Self {
        Self(PostToolUseOutcome::Structured(Box::new(
            StructuredPostToolUseOutput {
                hook_specific_output: Some(PostToolUseSpecific {
                    additional_context: Some(context.into()),
                    ..PostToolUseSpecific::new()
                }),
                ..StructuredPostToolUseOutput::default()
            },
        )))
    }

    /// Creates a top-level `decision: "block"` response. The tool already
    /// ran; Claude Code adds `reason` next to the tool result, and Claude
    /// still sees the original output. This is the current PostToolUse
    /// format, not a legacy one.
    pub fn block(reason: impl Into<String>) -> Self {
        Self(PostToolUseOutcome::Structured(Box::new(
            StructuredPostToolUseOutput {
                decision: Some("block"),
                reason: Some(reason.into()),
                ..StructuredPostToolUseOutput::default()
            },
        )))
    }

    /// Adds a top-level `decision: "block"` and `reason` to a structured
    /// response.
    ///
    /// `NoOp` is promoted to an empty structured response. Error and
    /// stderr-wrapped outputs are rejected because their structure is final.
    pub fn with_block(mut self, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        let output = self.structured_mut()?;
        output.decision = Some("block");
        output.reason = Some(reason.into());
        Ok(self)
    }

    /// Creates a code-2 feedback response with required stderr text.
    ///
    /// The tool has already completed, so Claude Code shows this feedback to
    /// Claude but does not undo or block the tool call.
    pub fn feedback_error(message: impl Into<String>) -> Self {
        Self(PostToolUseOutcome::FeedbackError(message.into()))
    }

    /// Creates a code-2 feedback response with required stderr text.
    #[deprecated(note = "PostToolUse exit 2 does not block; use PostToolUseOutput::feedback_error")]
    pub fn blocking_error(message: impl Into<String>) -> Self {
        Self::feedback_error(message)
    }

    /// Reports a non-blocking hook error: exits 1 with required stderr, which
    /// Claude Code shows the user as a hook error notice. Claude does not see
    /// it.
    pub fn nonblocking_error(message: impl Into<String>) -> Self {
        Self(PostToolUseOutcome::NonblockingError(message.into()))
    }

    /// Adds protocol stderr to a successful response.
    ///
    /// Claude Code writes exit-0 stderr only to its debug log, so the user
    /// never sees it; use [`Self::with_system_message`] for a user notice.
    /// An error or an already wrapped response is rejected.
    pub fn with_protocol_stderr(self, stderr: impl Into<String>) -> hookkit_core::Result<Self> {
        let output =
            self.into_successful("protocol stderr can only wrap a successful output once")?;
        Ok(Self(PostToolUseOutcome::WithProtocolStderr {
            output: Box::new(output),
            stderr: stderr.into(),
        }))
    }

    /// Emits this structured response on stdout while exiting 2 with
    /// `message` on stderr.
    ///
    /// Claude Code reads JSON on every exit code, so the structured fields
    /// still apply, and it shows `message` to Claude as feedback. The tool
    /// has already run, so nothing is blocked. `message` may be empty only
    /// when the response makes a `block` decision, whose `reason` is then the
    /// feedback; otherwise an empty `message` is rejected here, so the
    /// handler can still choose another response. No field can be added
    /// afterwards.
    pub fn into_feedback_error(self, message: impl Into<String>) -> hookkit_core::Result<Self> {
        let output =
            self.into_successful("only a successful structured response can exit 2 with stdout")?;
        let stderr = message.into();
        if stderr.is_empty() && !output.blocks() {
            return Err(empty_feedback_error());
        }
        Ok(Self(PostToolUseOutcome::StructuredFeedback {
            output: Box::new(output),
            stderr,
        }))
    }

    fn into_successful(self, message: &'static str) -> hookkit_core::Result<PostToolUseOutcome> {
        match self.0 {
            output @ (PostToolUseOutcome::NoOp | PostToolUseOutcome::Structured(_)) => Ok(output),
            _ => Err(hookkit_core::HookkitError::InvalidProcessEmission(message)),
        }
    }

    /// Sets `additionalContext`, replacing any earlier value.
    pub fn with_additional_context(
        mut self,
        context: impl Into<String>,
    ) -> hookkit_core::Result<Self> {
        self.specific_mut()?.additional_context = Some(context.into());
        Ok(self)
    }

    /// Sets `classifierContext`, a short note about this call's result for
    /// the auto-mode classifier rather than for Claude (Claude Code v2.1.236
    /// or later).
    ///
    /// Claude Code caps the note at 2,000 UTF-16 code units shared by every
    /// hook for one call and ignores it for async hooks; longer text is not
    /// rejected here.
    pub fn with_classifier_context(
        mut self,
        context: impl Into<String>,
    ) -> hookkit_core::Result<Self> {
        self.specific_mut()?.classifier_context = Some(context.into());
        Ok(self)
    }

    /// Sets `updatedMCPToolOutput`, replacing an MCP tool's output before
    /// Claude sees it. Claude Code recommends
    /// [`Self::with_updated_tool_output`], which works for every tool.
    pub fn with_updated_mcp_tool_output(
        mut self,
        output: serde_json::Value,
    ) -> hookkit_core::Result<Self> {
        self.specific_mut()?.updated_mcp_tool_output = Some(output);
        Ok(self)
    }

    /// Sets `updatedToolOutput`, replacing the tool's output before Claude
    /// sees it. For built-in tools the value must match the tool's output
    /// shape or Claude Code ignores it.
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

    /// Sets the top-level `stopReason`, shown to the user when `continue` is
    /// `false`.
    pub fn with_stop_reason(mut self, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        self.structured_mut()?.stop_reason = Some(reason.into());
        Ok(self)
    }

    /// Sets the universal `suppressOutput` field, which Claude Code accepts
    /// but ignores.
    #[deprecated(
        note = "Claude Code accepts `suppressOutput` but ignores it on every event (claude-code/docs-2026-09-29-r1)"
    )]
    pub fn with_suppress_output(mut self, suppress_output: bool) -> hookkit_core::Result<Self> {
        self.structured_mut()?.suppress_output = Some(suppress_output);
        Ok(self)
    }

    /// Sets a top-level `systemMessage`, a warning shown to the user.
    pub fn with_system_message(mut self, message: impl Into<String>) -> hookkit_core::Result<Self> {
        self.structured_mut()?.system_message = Some(message.into());
        Ok(self)
    }

    /// Requests emission of an allowlisted terminal notification sequence.
    pub fn with_terminal_sequence(
        mut self,
        sequence: impl Into<String>,
    ) -> hookkit_core::Result<Self> {
        self.structured_mut()?.terminal_sequence = Some(sequence.into());
        Ok(self)
    }

    fn structured_mut(&mut self) -> hookkit_core::Result<&mut StructuredPostToolUseOutput> {
        if matches!(self.0, PostToolUseOutcome::NoOp) {
            self.0 = PostToolUseOutcome::Structured(Box::default());
        }
        match &mut self.0 {
            PostToolUseOutcome::Structured(output) => Ok(output),
            _ => Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "structured fields must be added before error/protocol-stderr output",
            )),
        }
    }

    fn specific_mut(&mut self) -> hookkit_core::Result<&mut PostToolUseSpecific> {
        Ok(self
            .structured_mut()?
            .hook_specific_output
            .get_or_insert_with(PostToolUseSpecific::new))
    }
}

impl PostToolUseOutcome {
    /// Reports whether a successful outcome makes a `block` decision, whose
    /// reason Claude Code uses as the exit-2 feedback.
    fn blocks(&self) -> bool {
        matches!(self, Self::Structured(output) if output.decision == Some("block"))
    }
}

fn empty_feedback_error() -> hookkit_core::HookkitError {
    hookkit_core::HookkitError::InvalidProcessEmission(
        "an exit-2 response needs a non-empty message unless its JSON makes a blocking decision",
    )
}

fn emit_post_tool_use(outcome: PostToolUseOutcome) -> hookkit_core::Result<ProcessEmission> {
    let contract = PostToolUse::CONTRACT;
    match outcome {
        PostToolUseOutcome::NoOp => ProcessEmission::command_json(contract, &serde_json::json!({})),
        PostToolUseOutcome::Structured(output) => ProcessEmission::command_json(contract, &output),
        PostToolUseOutcome::FeedbackError(message) => {
            ProcessEmission::command_required_stderr(contract, message, 2)
        }
        PostToolUseOutcome::NonblockingError(message) => {
            ProcessEmission::command_required_stderr(contract, message, 1)
        }
        PostToolUseOutcome::WithProtocolStderr { output, stderr } => {
            let emission = emit_post_tool_use(*output)?;
            if emission.exit_code() != 0 {
                return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                    "protocol stderr cannot wrap a nonzero output",
                ));
            }
            Ok(ProcessEmission::command_unchecked(
                contract,
                emission.stdout().to_vec(),
                stderr.into_bytes(),
                0,
            ))
        }
        PostToolUseOutcome::StructuredFeedback { output, stderr } => {
            if stderr.is_empty() && !output.blocks() {
                return Err(empty_feedback_error());
            }
            let emission = emit_post_tool_use(*output)?;
            if emission.exit_code() != 0 {
                return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                    "feedback stderr cannot wrap a nonzero output",
                ));
            }
            Ok(ProcessEmission::command_unchecked(
                contract,
                emission.stdout().to_vec(),
                stderr.into_bytes(),
                2,
            ))
        }
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
    const CONTRACT: ContractId = contract_id!("PostToolUse");

    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        require_event(invocation, "PostToolUse")?;
        let input: PostToolUseInput = deserialize_input(invocation, "PostToolUse")?;
        if input.duration_ms.is_some_and(|duration| duration < 0.0) {
            return Err(invalid_input(
                "PostToolUse",
                "duration_ms must be non-negative",
            ));
        }
        Ok(input)
    }

    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        emit_post_tool_use(output.0)
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
#[non_exhaustive]
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
    /// Requested worktree name, a slug such as `bold-oak-a3f2`.
    pub name: String,
    /// Subagent identifier when the event ran on behalf of a subagent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Subagent type when the event ran on behalf of a subagent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    /// Optional nested effort setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,
    /// Permission policy active for the event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
    /// Prompt identifier associated with the current turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_id: Option<String>,
    /// Session scratchpad directory (Claude Code v2.1.257 or later).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scratchpad_dir: Option<hookkit_core::Utf8PathBuf>,
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
    /// Creates a successful response containing the created worktree path.
    ///
    /// Claude Code reads the last non-empty stdout line after stripping ANSI
    /// escape codes, resolves a relative path against the hook's working
    /// directory (collapsing `.` and `..`), and refuses an absolute path that
    /// contains `.` or `..` segments (v2.1.216 or later). The path is
    /// therefore rejected when it is empty, contains a line break or an
    /// escape character, or is absolute with a `.` or `..` component. Claude
    /// Code also refuses a path that passes through a symlink below the
    /// repository root; that needs filesystem access and is not checked here.
    /// The path is emitted as exact UTF-8 text without a trailing newline.
    pub fn path(path: hookkit_core::Utf8PathBuf) -> hookkit_core::Result<Self> {
        validate_worktree_path(&path)?;
        Ok(Self(WorktreeCreateOutcome::Created {
            path,
            trailing_newline: false,
        }))
    }

    /// Creates a successful path response with one trailing newline, under
    /// the same rules as [`Self::path`].
    pub fn path_with_newline(path: hookkit_core::Utf8PathBuf) -> hookkit_core::Result<Self> {
        validate_worktree_path(&path)?;
        Ok(Self(WorktreeCreateOutcome::Created {
            path,
            trailing_newline: true,
        }))
    }

    /// Creates a failed response written to stderr, which Claude Code shows
    /// the user.
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

fn validate_worktree_path(path: &hookkit_core::Utf8Path) -> hookkit_core::Result<()> {
    let text = path.as_str();
    if text.trim().is_empty() {
        return Err(hookkit_core::HookkitError::InvalidProcessEmission(
            "worktree path must not be empty",
        ));
    }
    if text.contains(['\n', '\r', '\u{1b}']) {
        return Err(hookkit_core::HookkitError::InvalidProcessEmission(
            "worktree path must not contain line breaks or escape characters",
        ));
    }
    if path.is_absolute()
        && text
            .split(['/', '\\'])
            .any(|segment| segment == "." || segment == "..")
    {
        return Err(hookkit_core::HookkitError::InvalidProcessEmission(
            "absolute worktree path must not contain . or .. segments",
        ));
    }
    Ok(())
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
    const CONTRACT: ContractId = contract_id!("WorktreeCreate");

    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        require_event(invocation, "WorktreeCreate")?;
        let input: WorktreeCreateInput = deserialize_input(invocation, "WorktreeCreate")?;
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
                let mut bytes = path.into_string().into_bytes();
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

pub(crate) fn invalid_input(
    event: &'static str,
    message: impl Into<String>,
) -> hookkit_core::HookkitError {
    hookkit_core::HookkitError::InvalidInputForHint {
        event: EventId::builtin(HarnessId::CLAUDE_CODE, event),
        message: message.into(),
    }
}

/// Deserializes the typed input of `event` after its discriminator matched.
///
/// A payload that violates the event's shape (a missing or mistyped field)
/// is reported as [`hookkit_core::HookkitError::InvalidInputForHint`], as
/// the required-field checks report it and as the Codex and Antigravity
/// parsers do; only a discriminator mismatch is `InvalidForHint`.
pub(crate) fn deserialize_input<'de, T: Deserialize<'de>>(
    invocation: &'de RawInvocation,
    event: &'static str,
) -> hookkit_core::Result<T> {
    T::deserialize(invocation.json()).map_err(|error| invalid_input(event, error.to_string()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
/// Compile-time selector for an implemented Claude Code event.
pub enum Event {
    /// Selects [`SessionStart`].
    SessionStart,
    /// Selects [`PostToolUse`].
    PostToolUse,
    /// Selects [`WorktreeCreate`].
    WorktreeCreate,
    /// Selects [`crate::model_switch::PreModelSwitch`].
    PreModelSwitch,
    /// Selects [`crate::model_switch::PostModelSwitch`].
    PostModelSwitch,
    /// Selects [`crate::catalog::ConfigChange`].
    ConfigChange,
    /// Selects [`crate::catalog::CwdChanged`].
    CwdChanged,
    /// Selects [`crate::catalog::DirectoryAdded`].
    DirectoryAdded,
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

impl Event {
    /// Returns the native wire event name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SessionStart => "SessionStart",
            Self::PostToolUse => "PostToolUse",
            Self::WorktreeCreate => "WorktreeCreate",
            Self::PreModelSwitch => "PreModelSwitch",
            Self::PostModelSwitch => "PostModelSwitch",
            Self::ConfigChange => "ConfigChange",
            Self::CwdChanged => "CwdChanged",
            Self::DirectoryAdded => "DirectoryAdded",
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
        }
    }
}

impl EventSelector for Event {
    fn event_id(&self) -> EventId {
        EventId::builtin(HarnessId::CLAUDE_CODE, self.as_str())
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
/// Sum type over all implemented Claude Code inputs.
pub enum AnyInput {
    /// A session-start input.
    SessionStart(SessionStartInput),
    /// A post-tool input.
    PostToolUse(PostToolUseInput),
    /// A worktree-create input.
    WorktreeCreate(WorktreeCreateInput),
    /// A pre-model-switch input.
    PreModelSwitch(ModelSwitchInput),
    /// A post-model-switch input.
    PostModelSwitch(ModelSwitchInput),
    /// An input for another implemented catalog event.
    Catalog(crate::catalog::CatalogInput),
}

#[derive(Debug, Clone)]
#[non_exhaustive]
/// Sum type over all implemented Claude Code command outputs.
pub enum AnyCommandOutput {
    /// A session-start output.
    SessionStart(SessionStartOutput),
    /// A post-tool output.
    PostToolUse(PostToolUseOutput),
    /// A worktree-create output.
    WorktreeCreate(WorktreeCreateOutput),
    /// A pre-model-switch output.
    PreModelSwitch(PreModelSwitchOutput),
    /// A post-model-switch output.
    PostModelSwitch(PostModelSwitchOutput),
    /// An output for another implemented catalog event.
    Catalog(crate::catalog::CatalogOutput),
}

impl From<SessionStartOutput> for AnyCommandOutput {
    fn from(output: SessionStartOutput) -> Self {
        Self::SessionStart(output)
    }
}

impl From<PostToolUseOutput> for AnyCommandOutput {
    fn from(output: PostToolUseOutput) -> Self {
        Self::PostToolUse(output)
    }
}

impl From<WorktreeCreateOutput> for AnyCommandOutput {
    fn from(output: WorktreeCreateOutput) -> Self {
        Self::WorktreeCreate(output)
    }
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
            "PreModelSwitch" => PreModelSwitch::parse(raw).map(AnyInput::PreModelSwitch),
            "PostModelSwitch" => PostModelSwitch::parse(raw).map(AnyInput::PostModelSwitch),
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
            AnyInput::PreModelSwitch(_) => PreModelSwitch::EVENT,
            AnyInput::PostModelSwitch(_) => PostModelSwitch::EVENT,
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
            AnyInput::PreModelSwitch(input) => {
                PreModelSwitch::validate_command_environment(input, environment)
            }
            AnyInput::PostModelSwitch(input) => {
                PostModelSwitch::validate_command_environment(input, environment)
            }
            AnyInput::Catalog(input) => input.validate_environment(environment),
        }
    }

    fn output_event(output: &Self::AnyCommandOutput) -> EventId {
        match output {
            AnyCommandOutput::SessionStart(_) => SessionStart::EVENT,
            AnyCommandOutput::PostToolUse(_) => PostToolUse::EVENT,
            AnyCommandOutput::WorktreeCreate(_) => WorktreeCreate::EVENT,
            AnyCommandOutput::PreModelSwitch(_) => PreModelSwitch::EVENT,
            AnyCommandOutput::PostModelSwitch(_) => PostModelSwitch::EVENT,
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
            AnyCommandOutput::PreModelSwitch(output) => PreModelSwitch::emit(output),
            AnyCommandOutput::PostModelSwitch(output) => PostModelSwitch::emit(output),
            AnyCommandOutput::Catalog(output) => output.emit(),
        }
    }

    fn context(input: &Self::AnyInput) -> NativeContext {
        match input {
            AnyInput::SessionStart(input) => SessionStart::context(input),
            AnyInput::PostToolUse(input) => PostToolUse::context(input),
            AnyInput::WorktreeCreate(input) => WorktreeCreate::context(input),
            AnyInput::PreModelSwitch(input) => PreModelSwitch::context(input),
            AnyInput::PostModelSwitch(input) => PostModelSwitch::context(input),
            AnyInput::Catalog(input) => input.context(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(value: serde_json::Value) -> RawInvocation {
        RawInvocation::parse(serde_json::to_vec(&value).unwrap()).unwrap()
    }

    fn session_start(source: &str) -> serde_json::Value {
        serde_json::json!({
            "session_id": "s",
            "transcript_path": "/tmp/t",
            "cwd": "/repo",
            "hook_event_name": "SessionStart",
            "source": source,
        })
    }

    fn post_tool_use() -> serde_json::Value {
        serde_json::json!({
            "session_id": "s",
            "transcript_path": "/tmp/t",
            "cwd": "/repo",
            "hook_event_name": "PostToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": "cargo test"},
            "tool_use_id": "call-1",
            "tool_response": {},
        })
    }

    fn stdout_json(emission: &ProcessEmission) -> serde_json::Value {
        serde_json::from_slice(emission.stdout()).unwrap()
    }

    #[test]
    fn session_start_discriminator_is_fixed_by_output_type() {
        let emission = SessionStart::emit(SessionStartOutput::with_context("ctx")).unwrap();
        let value = stdout_json(&emission);
        assert_eq!(value["hookSpecificOutput"]["hookEventName"], "SessionStart");
        assert_eq!(value["hookSpecificOutput"]["additionalContext"], "ctx");
    }

    #[test]
    fn session_start_context_exposes_typed_boundary_cause() {
        for (source, kind) in [
            ("startup", SessionBoundaryKind::Startup),
            ("resume", SessionBoundaryKind::Resume),
            ("fork", SessionBoundaryKind::Fork),
            ("clear", SessionBoundaryKind::Clear),
            ("compact", SessionBoundaryKind::Compact),
        ] {
            let input = SessionStart::parse(&raw(session_start(source))).unwrap();
            assert_eq!(input.source.as_str(), source);
            let context = SessionStart::context(&input);
            assert_eq!(context.session_boundary.unwrap().kind, kind);
        }
    }

    #[test]
    fn an_undocumented_session_source_parses_without_a_boundary_kind() {
        let input = SessionStart::parse(&raw(session_start("branch"))).unwrap();
        assert_eq!(input.source, SessionSource::Unknown("branch".into()));
        assert!(SessionStart::context(&input).session_boundary.is_none());
        assert_eq!(
            serde_json::to_value(&input).unwrap(),
            session_start("branch")
        );
    }

    #[test]
    fn undocumented_permission_modes_and_effort_values_do_not_fail_parsing() {
        // A parse failure exits 1, which Claude Code treats as a non-blocking
        // error: a policy hook would silently fail open.
        let mut value = post_tool_use();
        value["permission_mode"] = "manual".into();
        value["effort"] = serde_json::json!({"level": "ultra", "source": "x"});
        let input = PostToolUse::parse(&raw(value.clone())).unwrap();
        assert_eq!(
            input.permission_mode,
            Some(PermissionMode::Unknown("manual".into()))
        );
        let effort = input.effort.as_ref().unwrap();
        assert_eq!(effort.level.as_str(), "ultra");
        assert_eq!(effort.extra["source"], "x");
        assert_eq!(serde_json::to_value(&input).unwrap(), value);

        let mut value = session_start("startup");
        value["permission_mode"] = "invalid".into();
        assert!(SessionStart::parse(&raw(value)).is_ok());
    }

    #[test]
    fn session_start_builders_combine_every_hook_specific_field() {
        let output = SessionStartOutput::no_op()
            .with_initial_user_message("Run the nightly report.")
            .unwrap()
            .with_session_title("nightly")
            .unwrap()
            .with_additional_context("ctx")
            .unwrap()
            .with_reload_skills(true)
            .unwrap()
            .with_watch_paths(vec!["/repo/.env".into()])
            .unwrap()
            .with_system_message("notice")
            .unwrap();
        let value = stdout_json(&SessionStart::emit(output).unwrap());
        assert_eq!(
            value,
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "SessionStart",
                    "additionalContext": "ctx",
                    "initialUserMessage": "Run the nightly report.",
                    "reloadSkills": true,
                    "sessionTitle": "nightly",
                    "watchPaths": ["/repo/.env"],
                },
                "systemMessage": "notice",
            })
        );
    }

    #[test]
    fn session_start_context_and_system_message_keep_session_discriminator() {
        let emission = SessionStart::emit(SessionStartOutput::with_context_and_system_message(
            "ctx", "system",
        ))
        .unwrap();
        let value = stdout_json(&emission);
        assert_eq!(value["hookSpecificOutput"]["hookEventName"], "SessionStart");
        assert_eq!(value["systemMessage"], "system");
    }

    #[test]
    fn session_start_rejects_relative_watch_paths_and_structured_fields_on_text() {
        let emission = SessionStart::emit(
            SessionStartOutput::no_op()
                .with_terminal_sequence("\u{7}")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(stdout_json(&emission)["terminalSequence"], "\u{7}");
        assert!(
            SessionStartOutput::no_op()
                .with_watch_paths(vec!["relative/.env".into()])
                .is_err()
        );
        assert!(
            SessionStartOutput::text_context("plain")
                .with_session_title("t")
                .is_err()
        );
        assert!(
            SessionStartOutput::nonblocking_error("oops")
                .with_system_message("m")
                .is_err()
        );
    }

    #[test]
    fn session_start_text_is_not_json_or_newline_normalized() {
        let emission = SessionStart::emit(SessionStartOutput::text_context("context")).unwrap();
        assert_eq!(emission.stdout(), b"context");
        let emission =
            SessionStart::emit(SessionStartOutput::text_context("{ unclosed\n")).unwrap();
        assert_eq!(emission.stdout(), b"{ unclosed\n");
    }

    #[test]
    fn json_looking_text_context_is_emitted_as_structured_context() {
        // Claude Code v2.1.248+ parses such stdout as JSON and drops it.
        for text in [r#"{"branch":"main"}"#, " \n{}\n ", "\u{feff}{ a }"] {
            let emission = SessionStart::emit(SessionStartOutput::text_context(text)).unwrap();
            let value = stdout_json(&emission);
            assert_eq!(value["hookSpecificOutput"]["hookEventName"], "SessionStart");
            assert_eq!(value["hookSpecificOutput"]["additionalContext"], text);
        }
    }

    #[test]
    fn post_tool_use_parses_snake_case_and_retains_unknown_fields() {
        let mut value = post_tool_use();
        value["future"] = true.into();
        value["duration_ms"] = 12.into();
        value["mcp_server"] = serde_json::json!({"name": "db", "source": "team", "extra": 1});
        let input = PostToolUse::parse(&raw(value.clone())).unwrap();
        assert_eq!(input.tool_name, "Bash");
        assert_eq!(input.extra["future"], true);
        assert_eq!(input.duration_ms, Some(12.0));
        let server = input.mcp_server.as_ref().unwrap();
        assert!(!server.source.is_sdk());
        assert_eq!(server.extra["extra"], 1);
        // Integral numbers keep their integer spelling on re-serialization.
        assert_eq!(serde_json::to_value(&input).unwrap(), value);
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
        assert!(!emission.stdout().ends_with(b"\n"));
    }

    #[test]
    fn updated_mcp_tool_output_uses_the_documented_wire_key() {
        let emission = PostToolUse::emit(
            PostToolUseOutput::no_op()
                .with_updated_mcp_tool_output(serde_json::json!({"content": []}))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            emission.stdout(),
            br#"{"hookSpecificOutput":{"hookEventName":"PostToolUse","updatedMCPToolOutput":{"content":[]}}}"#
        );
    }

    #[test]
    fn post_tool_use_context_can_follow_a_block() {
        let value = stdout_json(
            &PostToolUse::emit(
                PostToolUseOutput::block("review")
                    .with_additional_context("ctx")
                    .unwrap()
                    .with_classifier_context("staging")
                    .unwrap(),
            )
            .unwrap(),
        );
        assert_eq!(value["decision"], "block");
        assert_eq!(value["reason"], "review");
        assert_eq!(value["hookSpecificOutput"]["additionalContext"], "ctx");
        assert_eq!(value["hookSpecificOutput"]["classifierContext"], "staging");
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
        assert_eq!(
            stdout_json(&emission)["hookSpecificOutput"]["hookEventName"],
            "PostToolUse"
        );
    }

    #[test]
    fn post_tool_use_structured_feedback_keeps_json_and_exits_2() {
        let emission = PostToolUse::emit(
            PostToolUseOutput::with_context("ctx")
                .into_feedback_error("feedback")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(emission.exit_code(), 2);
        assert_eq!(emission.stderr(), b"feedback");
        assert_eq!(
            stdout_json(&emission)["hookSpecificOutput"]["additionalContext"],
            "ctx"
        );

        // Without a block decision stderr is the only feedback, so an empty
        // message is rejected when the response is built, not at emission.
        assert!(PostToolUseOutput::no_op().into_feedback_error("").is_err());
        assert!(
            PostToolUseOutput::with_context("ctx")
                .into_feedback_error("")
                .is_err()
        );

        // With a block decision its reason is the feedback, and the snapshot
        // allows exit 2 with empty stderr next to JSON.
        let emission = PostToolUse::emit(
            PostToolUseOutput::block("lint failed")
                .into_feedback_error("")
                .unwrap(),
        )
        .unwrap();
        assert_eq!((emission.exit_code(), emission.stderr()), (2, &b""[..]));
        assert_eq!(stdout_json(&emission)["reason"], "lint failed");
    }

    #[test]
    fn invalid_builder_order_returns_errors() {
        assert!(
            PostToolUseOutput::feedback_error("blocked")
                .with_block("again")
                .is_err()
        );
        assert!(
            PostToolUseOutput::nonblocking_error("oops")
                .into_feedback_error("again")
                .is_err()
        );
        let wrapped = PostToolUseOutput::no_op()
            .with_protocol_stderr("debug")
            .unwrap();
        assert!(wrapped.clone().with_protocol_stderr("again").is_err());
        assert!(wrapped.clone().into_feedback_error("again").is_err());
        assert!(wrapped.with_system_message("late").is_err());
        let feedback = PostToolUseOutput::no_op().into_feedback_error("f").unwrap();
        assert!(feedback.with_additional_context("late").is_err());
    }

    #[test]
    fn input_constraints_reject_negative_numbers_and_empty_required_values() {
        let mut post = post_tool_use();
        post["duration_ms"] = (-1).into();
        assert!(PostToolUse::parse(&raw(post)).is_err());

        let worktree = serde_json::json!({
            "session_id": "",
            "transcript_path": "/tmp/t",
            "cwd": "/repo",
            "hook_event_name": "WorktreeCreate",
            "name": "w",
        });
        assert!(WorktreeCreate::parse(&raw(worktree)).is_err());

        assert!(matches!(
            SessionStart::parse(&raw(post_tool_use())),
            Err(hookkit_core::HookkitError::InvalidForHint { .. })
        ));
    }

    #[test]
    fn worktree_command_is_plain_text_without_newline() {
        let output = WorktreeCreateOutput::path("/tmp/worktree".into()).unwrap();
        let emission = WorktreeCreate::emit(output).unwrap();
        assert_eq!(emission.stdout(), b"/tmp/worktree");

        let output = WorktreeCreateOutput::path_with_newline("/tmp/worktree".into()).unwrap();
        assert_eq!(
            WorktreeCreate::emit(output).unwrap().stdout(),
            b"/tmp/worktree\n"
        );
    }

    #[test]
    fn worktree_command_accepts_relative_paths_with_dot_segments() {
        for path in [".claude/worktrees/feature", "../worktrees/./feature"] {
            let output = WorktreeCreateOutput::path(path.into()).unwrap();
            let emission = WorktreeCreate::emit(output).unwrap();
            assert_eq!(emission.stdout(), path.as_bytes());
        }
    }

    #[test]
    fn worktree_paths_claude_code_would_misread_or_refuse_are_rejected() {
        for path in [
            "",
            "  ",
            "/tmp/a\nrelative",
            "/tmp/a\r",
            "\u{1b}[1m/tmp/a",
            "/repo/../wt",
            "/repo/./wt",
            "/repo/wt/..",
        ] {
            assert!(
                WorktreeCreateOutput::path(path.into()).is_err(),
                "{path:?} should be rejected"
            );
            assert!(WorktreeCreateOutput::path_with_newline(path.into()).is_err());
        }
    }

    #[test]
    fn worktree_failure_is_stderr_only() {
        let output = WorktreeCreateOutput::failed("cannot create", 7).unwrap();
        let emission = WorktreeCreate::emit(output).unwrap();
        assert!(emission.stdout().is_empty());
        assert_eq!(emission.stderr(), b"cannot create");
        assert_eq!(emission.exit_code(), 7);
        assert!(WorktreeCreateOutput::failed("x", 0).is_err());
    }

    #[test]
    fn every_protocol_output_converts_into_the_harness_sum_type() {
        let outputs: [AnyCommandOutput; 5] = [
            SessionStartOutput::no_op().into(),
            PostToolUseOutput::no_op().into(),
            WorktreeCreateOutput::failed("", 1).unwrap().into(),
            PreModelSwitchOutput::allow().into(),
            PostModelSwitchOutput::no_op().into(),
        ];
        let events: Vec<_> = outputs.iter().map(ClaudeCode::output_event).collect();
        assert_eq!(
            events,
            [
                SessionStart::EVENT,
                PostToolUse::EVENT,
                WorktreeCreate::EVENT,
                PreModelSwitch::EVENT,
                PostModelSwitch::EVENT,
            ]
        );
    }

    #[test]
    fn event_selectors_cover_every_descriptor() {
        let selectors = [
            Event::SessionStart,
            Event::PostToolUse,
            Event::WorktreeCreate,
            Event::PreModelSwitch,
            Event::PostModelSwitch,
            Event::ConfigChange,
            Event::CwdChanged,
            Event::DirectoryAdded,
            Event::Elicitation,
            Event::ElicitationResult,
            Event::FileChanged,
            Event::InstructionsLoaded,
            Event::MessageDisplay,
            Event::Notification,
            Event::PermissionDenied,
            Event::PermissionRequest,
            Event::PostCompact,
            Event::PostToolBatch,
            Event::PostToolUseFailure,
            Event::PreCompact,
            Event::PreToolUse,
            Event::SessionEnd,
            Event::Setup,
            Event::Stop,
            Event::StopFailure,
            Event::SubagentStart,
            Event::SubagentStop,
            Event::TaskCompleted,
            Event::TaskCreated,
            Event::TeammateIdle,
            Event::UserPromptExpansion,
            Event::UserPromptSubmit,
            Event::WorktreeRemove,
        ];
        let selected: std::collections::BTreeSet<_> =
            selectors.iter().map(|event| event.event_id()).collect();
        let described: std::collections::BTreeSet<_> = events()
            .iter()
            .map(|descriptor| descriptor.event().clone())
            .collect();
        assert_eq!(selected, described);
        assert_eq!(selectors.len(), described.len());
    }
}
