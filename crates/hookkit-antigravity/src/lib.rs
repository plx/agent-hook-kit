//! Native Antigravity hook contracts.
#![deny(missing_docs)]

pub mod environment;

pub use environment::AntigravityCommandEnvironment;

use hookkit_core::{
    ContractId, EventCategory, EventId, EventSelector, EventSpec, HarnessId, HarnessSpec,
    IdentificationDescriptor, NativeContext, NativeEventDescriptor, ProcessEmission, RawInvocation,
    SnapshotId,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Antigravity protocol documentation snapshot implemented by this crate.
pub const SNAPSHOT: SnapshotId = SnapshotId::builtin("docs-2026-08-04-r1");

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
/// Invocation events share a discriminator-free shape and require an explicit
/// event hint. Tool and stop events have distinct, validated shapes.
pub fn identification_descriptors() -> Vec<IdentificationDescriptor> {
    vec![
        IdentificationDescriptor::ambiguous::<PreInvocation>(&["PostInvocation"]),
        IdentificationDescriptor::ambiguous::<PostInvocation>(&["PreInvocation"]),
        IdentificationDescriptor::sound_shape::<PreToolUse>(&[]),
        IdentificationDescriptor::sound_shape::<PostToolUse>(&[]),
        IdentificationDescriptor::sound_shape::<Stop>(&[]),
    ]
}

/// Discriminator-free PreInvocation input. Unknown additions are retained.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreInvocationInput {
    /// Native conversation identifier.
    pub conversation_id: String,
    /// Non-empty set of active workspace roots.
    pub workspace_paths: Vec<hookkit_core::Utf8PathBuf>,
    /// Path to the native conversation transcript.
    pub transcript_path: hookkit_core::Utf8PathBuf,
    /// Directory where hooks may write diagnostic artifacts.
    pub artifact_directory_path: hookkit_core::Utf8PathBuf,
    /// Zero-based invocation number; zero marks an invocation-session boundary.
    pub invocation_num: u64,
    /// Number of steps currently present in the trajectory.
    pub initial_num_steps: u64,
    /// Unknown protocol fields retained for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Step to inject before an Antigravity invocation begins.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum InjectStep {
    /// Injects a native tool-call object.
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
    /// Injects a message visible only for the current invocation.
    EphemeralMessage {
        /// Message serialized under the native `ephemeralMessage` key.
        #[serde(rename = "ephemeralMessage")]
        ephemeral_message: String,
    },
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
/// Native response from a pre-invocation command hook.
pub struct PreInvocationOutput {
    /// Ordered steps to inject; an empty list is omitted from JSON.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub inject_steps: Vec<InjectStep>,
}

impl PreInvocationOutput {
    /// Creates a response that injects no steps.
    pub fn no_op() -> Self {
        Self::default()
    }

    /// Creates a response containing exactly one injected step.
    pub fn inject(step: InjectStep) -> Self {
        Self {
            inject_steps: vec![step],
        }
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
    const CATEGORY: EventCategory = EventCategory::Agent;
    const CONTRACT: ContractId =
        ContractId::builtin("antigravity/docs-2026-08-04-r1/PreInvocation");

    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        let input: Self::Input = serde_json::from_value(invocation.json().clone())?;
        require_workspace(&input.workspace_paths, Self::EVENT)?;
        Ok(input)
    }

    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        ProcessEmission::command_json(Self::CONTRACT, &output)
    }

    fn context(input: &Self::Input) -> NativeContext {
        invocation_context(input)
    }
}

/// Native post-invocation input, which shares the pre-invocation wire shape.
pub type PostInvocationInput = PreInvocationInput;

/// Whether Antigravity should end after a post-invocation hook response.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminationBehavior {
    /// Use Antigravity's default post-invocation behavior.
    #[serde(rename = "")]
    Default,
    /// Continue execution even if the invocation would otherwise terminate.
    ForceContinue,
    /// Terminate the invocation.
    Terminate,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
/// Native response from a post-invocation command hook.
pub struct PostInvocationOutput {
    /// Ordered native steps to inject after the invocation completes.
    pub inject_steps: Vec<InjectStep>,
    /// Optional override for the invocation's termination behavior.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub termination_behavior: Option<TerminationBehavior>,
}

/// Native Antigravity `PostInvocation` command contract.
pub enum PostInvocation {}

impl EventSpec for PostInvocation {
    type Input = PostInvocationInput;
    type CommandEnvironment = AntigravityCommandEnvironment;
    type CommandOutput = PostInvocationOutput;
    const HARNESS: HarnessId = HarnessId::ANTIGRAVITY;
    const SNAPSHOT: SnapshotId = SNAPSHOT;
    const EVENT: EventId = EventId::builtin(Self::HARNESS, "PostInvocation");
    const CATEGORY: EventCategory = EventCategory::Agent;
    const CONTRACT: ContractId =
        ContractId::builtin("antigravity/docs-2026-08-04-r1/PostInvocation");
    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        let input: Self::Input = serde_json::from_value(invocation.json().clone())?;
        require_workspace(&input.workspace_paths, Self::EVENT)?;
        Ok(input)
    }
    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        ProcessEmission::command_json(Self::CONTRACT, &output)
    }
    fn context(input: &Self::Input) -> NativeContext {
        invocation_context(input)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Native tool-call payload nested inside a pre-tool event.
pub struct ToolCall {
    /// Harness-native tool name.
    pub name: String,
    /// Tool arguments as an exact JSON object.
    pub args: serde_json::Map<String, serde_json::Value>,
    /// Unknown tool-call fields retained for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Native Antigravity input observed before a tool runs.
pub struct PreToolUseInput {
    /// Native conversation identifier.
    pub conversation_id: String,
    /// Non-empty set of active workspace roots.
    pub workspace_paths: Vec<hookkit_core::Utf8PathBuf>,
    /// Path to the native conversation transcript.
    pub transcript_path: hookkit_core::Utf8PathBuf,
    /// Directory where hooks may write diagnostic artifacts.
    pub artifact_directory_path: hookkit_core::Utf8PathBuf,
    /// Tool call about to execute.
    pub tool_call: ToolCall,
    /// Zero-based step index within the invocation.
    pub step_idx: u64,
    /// Unknown protocol fields retained for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Authorization decision returned by a pre-tool hook.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolDecision {
    /// Allow the tool call without prompting.
    Allow,
    /// Reject the tool call.
    Deny,
    /// Ask the user for permission under normal harness policy.
    Ask,
    /// Require a user permission prompt even if policy would bypass one.
    ForceAsk,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
/// Native response from an Antigravity pre-tool command hook.
pub struct PreToolUseOutput {
    /// Authorization decision for the pending tool call.
    pub decision: ToolDecision,
    /// Optional human-readable explanation of the decision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Permission identifiers to override for this decision.
    ///
    /// Emission rejects duplicate entries.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub permission_overrides: Vec<String>,
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
    const CONTRACT: ContractId = ContractId::builtin("antigravity/docs-2026-08-04-r1/PreToolUse");
    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        let input: Self::Input = serde_json::from_value(invocation.json().clone())?;
        require_workspace(&input.workspace_paths, Self::EVENT)?;
        Ok(input)
    }
    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        if output
            .permission_overrides
            .iter()
            .collect::<BTreeSet<_>>()
            .len()
            != output.permission_overrides.len()
        {
            return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "permissionOverrides must not contain duplicates",
            ));
        }
        ProcessEmission::command_json(Self::CONTRACT, &output)
    }
    fn context(input: &Self::Input) -> NativeContext {
        NativeContext {
            workspace_roots: input.workspace_paths.clone(),
            conversation_id: hookkit_core::ConversationId::new(input.conversation_id.clone()).ok(),
            transcript_path: Some(input.transcript_path.clone()),
            artifact_directory: Some(input.artifact_directory_path.clone()),
            ..NativeContext::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Native Antigravity input observed after a tool finishes.
pub struct PostToolUseInput {
    /// Native conversation identifier.
    pub conversation_id: String,
    /// Non-empty set of active workspace roots.
    pub workspace_paths: Vec<hookkit_core::Utf8PathBuf>,
    /// Path to the native conversation transcript.
    pub transcript_path: hookkit_core::Utf8PathBuf,
    /// Directory where hooks may write diagnostic artifacts.
    pub artifact_directory_path: hookkit_core::Utf8PathBuf,
    /// Zero-based step index within the invocation.
    pub step_idx: u64,
    /// Tool failure text, or `None` when the call succeeded.
    #[serde(default)]
    pub error: Option<String>,
    /// Unknown protocol fields retained for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Empty successful response from an Antigravity post-tool command hook.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct PostToolUseOutput {}
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
    const CONTRACT: ContractId = ContractId::builtin("antigravity/docs-2026-08-04-r1/PostToolUse");
    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        let input: Self::Input = serde_json::from_value(invocation.json().clone())?;
        require_workspace(&input.workspace_paths, Self::EVENT)?;
        Ok(input)
    }
    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        ProcessEmission::command_json(Self::CONTRACT, &output)
    }
    fn context(input: &Self::Input) -> NativeContext {
        NativeContext {
            workspace_roots: input.workspace_paths.clone(),
            conversation_id: hookkit_core::ConversationId::new(input.conversation_id.clone()).ok(),
            transcript_path: Some(input.transcript_path.clone()),
            artifact_directory: Some(input.artifact_directory_path.clone()),
            ..NativeContext::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Native Antigravity input observed when an invocation attempts to stop.
pub struct StopInput {
    /// Native conversation identifier.
    pub conversation_id: String,
    /// Non-empty set of active workspace roots.
    pub workspace_paths: Vec<hookkit_core::Utf8PathBuf>,
    /// Path to the native conversation transcript.
    pub transcript_path: hookkit_core::Utf8PathBuf,
    /// Directory where hooks may write diagnostic artifacts.
    pub artifact_directory_path: hookkit_core::Utf8PathBuf,
    /// Sequence number of the execution attempt.
    pub execution_num: u64,
    /// Harness-provided explanation for the attempted termination.
    pub termination_reason: String,
    /// Whether every agent and tool is idle.
    pub fully_idle: bool,
    /// Invocation failure text, if termination follows an error.
    #[serde(default)]
    pub error: Option<String>,
    /// Unknown protocol fields retained for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
/// Native response from an Antigravity stop command hook.
pub struct StopOutput {
    /// Native stop decision string; emission rejects an empty value.
    pub decision: String,
    /// Optional human-readable explanation of the decision.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
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
    const CONTRACT: ContractId = ContractId::builtin("antigravity/docs-2026-08-04-r1/Stop");
    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        let input: Self::Input = serde_json::from_value(invocation.json().clone())?;
        require_workspace(&input.workspace_paths, Self::EVENT)?;
        Ok(input)
    }
    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        if output.decision.is_empty() {
            return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "Stop decision must not be empty",
            ));
        }
        ProcessEmission::command_json(Self::CONTRACT, &output)
    }
    fn context(input: &Self::Input) -> NativeContext {
        NativeContext {
            workspace_roots: input.workspace_paths.clone(),
            conversation_id: hookkit_core::ConversationId::new(input.conversation_id.clone()).ok(),
            transcript_path: Some(input.transcript_path.clone()),
            artifact_directory: Some(input.artifact_directory_path.clone()),
            ..NativeContext::default()
        }
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
        workspace_roots: input.workspace_paths.clone(),
        conversation_id: hookkit_core::ConversationId::new(input.conversation_id.clone()).ok(),
        transcript_path: Some(input.transcript_path.clone()),
        artifact_directory: Some(input.artifact_directory_path.clone()),
        session_boundary,
        ..NativeContext::default()
    }
}

fn require_workspace(
    workspace_paths: &[hookkit_core::Utf8PathBuf],
    event: EventId,
) -> hookkit_core::Result<()> {
    if workspace_paths.is_empty() {
        return Err(hookkit_core::HookkitError::InvalidInputForHint {
            event,
            message: "workspacePaths must contain at least one path".into(),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
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

    #[test]
    fn parses_without_fabricated_discriminator_and_retains_unknown_fields() {
        let raw = RawInvocation::parse(
            br#"{"conversationId":"c1","workspacePaths":["/repo"],"transcriptPath":"/tmp/t.jsonl","artifactDirectoryPath":"/tmp/a","invocationNum":0,"initialNumSteps":0,"future":true}"#.to_vec(),
        )
        .unwrap();
        let input = PreInvocation::parse(&raw).unwrap();
        assert_eq!(input.extra["future"], true);
        assert_eq!(
            PreInvocation::context(&input)
                .session_boundary
                .unwrap()
                .kind,
            hookkit_core::SessionBoundaryKind::InvocationStart
        );
    }

    #[test]
    fn inject_output_matches_exact_catalog_bytes() {
        let output = PreInvocationOutput::inject(InjectStep::EphemeralMessage {
            ephemeral_message: "Remember to lint".into(),
        });
        let emission = PreInvocation::emit(output).unwrap();
        assert_eq!(
            emission.stdout(),
            br#"{"injectSteps":[{"ephemeralMessage":"Remember to lint"}]}"#
        );
        assert!(emission.stderr().is_empty());
        assert_eq!(emission.exit_code(), 0);
    }

    #[test]
    fn empty_workspace_is_rejected_by_native_parsers() {
        let raw = RawInvocation::parse(
            br#"{"conversationId":"c1","workspacePaths":[],"transcriptPath":"/tmp/t.jsonl","artifactDirectoryPath":"/tmp/a","invocationNum":0,"initialNumSteps":0}"#.to_vec(),
        )
        .unwrap();
        assert!(PreInvocation::parse(&raw).is_err());
        assert!(PostInvocation::parse(&raw).is_err());
    }

    #[test]
    fn output_constraints_are_revalidated_at_emission() {
        assert!(
            PreToolUse::emit(PreToolUseOutput {
                decision: ToolDecision::Ask,
                reason: None,
                permission_overrides: vec!["same".into(), "same".into()],
            })
            .is_err()
        );
        assert!(
            Stop::emit(StopOutput {
                decision: String::new(),
                reason: None,
            })
            .is_err()
        );
    }

    #[test]
    fn post_invocation_represents_typed_steps_and_explicit_default_behavior() {
        let emission = PostInvocation::emit(PostInvocationOutput {
            inject_steps: vec![InjectStep::UserMessage {
                user_message: "Run one more check.".into(),
            }],
            termination_behavior: Some(TerminationBehavior::Default),
        })
        .unwrap();

        assert_eq!(
            emission.stdout(),
            br#"{"injectSteps":[{"userMessage":"Run one more check."}],"terminationBehavior":""}"#
        );
    }
}
