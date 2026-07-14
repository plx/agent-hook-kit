//! Native Antigravity hook contracts.

pub mod environment;

pub use environment::AntigravityCommandEnvironment;

use hookkit_core::{
    ContractId, EventCategory, EventId, EventSelector, EventSpec, HarnessId, HarnessSpec,
    IdentificationDescriptor, NativeContext, NativeEventDescriptor, ProcessEmission, RawInvocation,
    SnapshotId,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SNAPSHOT: SnapshotId = SnapshotId::builtin("docs-2026-07-12-r2");

pub fn events() -> Vec<NativeEventDescriptor> {
    vec![
        NativeEventDescriptor::command::<PreInvocation>(&["inject-reminder"]),
        NativeEventDescriptor::command::<PostInvocation>(&["force-continue"]),
        NativeEventDescriptor::command::<PreToolUse>(&["ask"]),
        NativeEventDescriptor::command::<PostToolUse>(&["no-op"]),
        NativeEventDescriptor::command::<Stop>(&["continue"]),
    ]
}

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
    pub conversation_id: String,
    pub workspace_paths: Vec<hookkit_core::Utf8PathBuf>,
    pub transcript_path: hookkit_core::Utf8PathBuf,
    pub artifact_directory_path: hookkit_core::Utf8PathBuf,
    pub invocation_num: u64,
    pub initial_num_steps: u64,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum InjectStep {
    ToolCall {
        #[serde(rename = "toolCall")]
        tool_call: serde_json::Map<String, serde_json::Value>,
    },
    UserMessage {
        #[serde(rename = "userMessage")]
        user_message: String,
    },
    EphemeralMessage {
        #[serde(rename = "ephemeralMessage")]
        ephemeral_message: String,
    },
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreInvocationOutput {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub inject_steps: Vec<InjectStep>,
}

impl PreInvocationOutput {
    pub fn no_op() -> Self {
        Self::default()
    }

    pub fn inject(step: InjectStep) -> Self {
        Self {
            inject_steps: vec![step],
        }
    }
}

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
        ContractId::builtin("antigravity/docs-2026-07-12-r2/PreInvocation");

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

pub type PostInvocationInput = PreInvocationInput;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminationBehavior {
    ForceContinue,
    Terminate,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostInvocationOutput {
    pub inject_steps: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub termination_behavior: Option<TerminationBehavior>,
}

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
        ContractId::builtin("antigravity/docs-2026-07-12-r2/PostInvocation");
    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        let input: Self::Input = serde_json::from_value(invocation.json().clone())?;
        require_workspace(&input.workspace_paths, Self::EVENT)?;
        Ok(input)
    }
    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        if output.inject_steps.iter().any(|step| !step.is_object()) {
            return Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "PostInvocation injectSteps entries must be objects",
            ));
        }
        ProcessEmission::command_json(Self::CONTRACT, &output)
    }
    fn context(input: &Self::Input) -> NativeContext {
        invocation_context(input)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub name: String,
    pub args: serde_json::Map<String, serde_json::Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreToolUseInput {
    pub conversation_id: String,
    pub workspace_paths: Vec<hookkit_core::Utf8PathBuf>,
    pub transcript_path: hookkit_core::Utf8PathBuf,
    pub artifact_directory_path: hookkit_core::Utf8PathBuf,
    pub tool_call: ToolCall,
    pub step_idx: u64,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolDecision {
    Allow,
    Deny,
    Ask,
    ForceAsk,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreToolUseOutput {
    pub decision: ToolDecision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub permission_overrides: Vec<String>,
}

pub enum PreToolUse {}
impl EventSpec for PreToolUse {
    type Input = PreToolUseInput;
    type CommandEnvironment = AntigravityCommandEnvironment;
    type CommandOutput = PreToolUseOutput;
    const HARNESS: HarnessId = HarnessId::ANTIGRAVITY;
    const SNAPSHOT: SnapshotId = SNAPSHOT;
    const EVENT: EventId = EventId::builtin(Self::HARNESS, "PreToolUse");
    const CATEGORY: EventCategory = EventCategory::Tool;
    const CONTRACT: ContractId = ContractId::builtin("antigravity/docs-2026-07-12-r2/PreToolUse");
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
pub struct PostToolUseInput {
    pub conversation_id: String,
    pub workspace_paths: Vec<hookkit_core::Utf8PathBuf>,
    pub transcript_path: hookkit_core::Utf8PathBuf,
    pub artifact_directory_path: hookkit_core::Utf8PathBuf,
    pub step_idx: u64,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct PostToolUseOutput {}
pub enum PostToolUse {}
impl EventSpec for PostToolUse {
    type Input = PostToolUseInput;
    type CommandEnvironment = AntigravityCommandEnvironment;
    type CommandOutput = PostToolUseOutput;
    const HARNESS: HarnessId = HarnessId::ANTIGRAVITY;
    const SNAPSHOT: SnapshotId = SNAPSHOT;
    const EVENT: EventId = EventId::builtin(Self::HARNESS, "PostToolUse");
    const CATEGORY: EventCategory = EventCategory::Tool;
    const CONTRACT: ContractId = ContractId::builtin("antigravity/docs-2026-07-12-r2/PostToolUse");
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
pub struct StopInput {
    pub conversation_id: String,
    pub workspace_paths: Vec<hookkit_core::Utf8PathBuf>,
    pub transcript_path: hookkit_core::Utf8PathBuf,
    pub artifact_directory_path: hookkit_core::Utf8PathBuf,
    pub execution_num: u64,
    pub termination_reason: String,
    pub fully_idle: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StopOutput {
    pub decision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
pub enum Stop {}
impl EventSpec for Stop {
    type Input = StopInput;
    type CommandEnvironment = AntigravityCommandEnvironment;
    type CommandOutput = StopOutput;
    const HARNESS: HarnessId = HarnessId::ANTIGRAVITY;
    const SNAPSHOT: SnapshotId = SNAPSHOT;
    const EVENT: EventId = EventId::builtin(Self::HARNESS, "Stop");
    const CATEGORY: EventCategory = EventCategory::Agent;
    const CONTRACT: ContractId = ContractId::builtin("antigravity/docs-2026-07-12-r2/Stop");
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
    NativeContext {
        workspace_roots: input.workspace_paths.clone(),
        conversation_id: hookkit_core::ConversationId::new(input.conversation_id.clone()).ok(),
        transcript_path: Some(input.transcript_path.clone()),
        artifact_directory: Some(input.artifact_directory_path.clone()),
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
pub enum Event {
    PreInvocation,
    PostInvocation,
    PreToolUse,
    PostToolUse,
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
pub enum AnyInput {
    PreInvocation(PreInvocationInput),
    PostInvocation(PostInvocationInput),
    PreToolUse(PreToolUseInput),
    PostToolUse(PostToolUseInput),
    Stop(StopInput),
}

#[derive(Debug, Clone)]
pub enum AnyCommandOutput {
    PreInvocation(PreInvocationOutput),
    PostInvocation(PostInvocationOutput),
    PreToolUse(PreToolUseOutput),
    PostToolUse(PostToolUseOutput),
    Stop(StopOutput),
}

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
            PostInvocation::emit(PostInvocationOutput {
                inject_steps: vec![serde_json::json!("not-an-object")],
                termination_behavior: None,
            })
            .is_err()
        );
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
}
