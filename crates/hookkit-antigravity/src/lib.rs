//! Native Antigravity hook contracts.

use hookkit_core::{EventCategory, EventId, EventSpec, HarnessId, ProcessEmission, RawInvocation};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SNAPSHOT_ID: &str = "docs-2026-07-12-r1";
pub static EVENTS: &[hookkit_core::NativeEventDescriptor] = &[
    hookkit_core::NativeEventDescriptor {
        contract_id: "antigravity/docs-2026-07-12-r1/PreInvocation",
        harness: "antigravity",
        event: "PreInvocation",
        native_input: true,
        native_output: true,
        bindings: &[hookkit_core::HandlerKind::Command],
        conformance_cases: &["inject-reminder"],
    },
    descriptor(
        "antigravity/docs-2026-07-12-r1/PostInvocation",
        "PostInvocation",
        &["force-continue"],
    ),
    descriptor(
        "antigravity/docs-2026-07-12-r1/PreToolUse",
        "PreToolUse",
        &["ask"],
    ),
    descriptor(
        "antigravity/docs-2026-07-12-r1/PostToolUse",
        "PostToolUse",
        &["no-op"],
    ),
    descriptor("antigravity/docs-2026-07-12-r1/Stop", "Stop", &["continue"]),
];

const fn descriptor(
    contract_id: &'static str,
    event: &'static str,
    cases: &'static [&'static str],
) -> hookkit_core::NativeEventDescriptor {
    hookkit_core::NativeEventDescriptor {
        contract_id,
        harness: "antigravity",
        event,
        native_input: true,
        native_output: true,
        bindings: &[hookkit_core::HandlerKind::Command],
        conformance_cases: cases,
    }
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
    type CommandOutput = PreInvocationOutput;

    const HARNESS: HarnessId = HarnessId::ANTIGRAVITY;
    const EVENT: EventId = EventId::builtin("PreInvocation");
    const CATEGORY: EventCategory = EventCategory::Agent;
    const CONTRACT_ID: &'static str = "antigravity/docs-2026-07-12-r1/PreInvocation";

    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        serde_json::from_value(invocation.json().clone()).map_err(Into::into)
    }

    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        ProcessEmission::success_json(&output)
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub inject_steps: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub termination_behavior: Option<TerminationBehavior>,
}

pub enum PostInvocation {}

impl EventSpec for PostInvocation {
    type Input = PostInvocationInput;
    type CommandOutput = PostInvocationOutput;
    const HARNESS: HarnessId = HarnessId::ANTIGRAVITY;
    const EVENT: EventId = EventId::builtin("PostInvocation");
    const CATEGORY: EventCategory = EventCategory::Agent;
    const CONTRACT_ID: &'static str = "antigravity/docs-2026-07-12-r1/PostInvocation";
    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        serde_json::from_value(invocation.json().clone()).map_err(Into::into)
    }
    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        ProcessEmission::success_json(&output)
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
    type CommandOutput = PreToolUseOutput;
    const HARNESS: HarnessId = HarnessId::ANTIGRAVITY;
    const EVENT: EventId = EventId::builtin("PreToolUse");
    const CATEGORY: EventCategory = EventCategory::Tool;
    const CONTRACT_ID: &'static str = "antigravity/docs-2026-07-12-r1/PreToolUse";
    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        serde_json::from_value(invocation.json().clone()).map_err(Into::into)
    }
    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        ProcessEmission::success_json(&output)
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
    type CommandOutput = PostToolUseOutput;
    const HARNESS: HarnessId = HarnessId::ANTIGRAVITY;
    const EVENT: EventId = EventId::builtin("PostToolUse");
    const CATEGORY: EventCategory = EventCategory::Tool;
    const CONTRACT_ID: &'static str = "antigravity/docs-2026-07-12-r1/PostToolUse";
    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        serde_json::from_value(invocation.json().clone()).map_err(Into::into)
    }
    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        ProcessEmission::success_json(&output)
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
    type CommandOutput = StopOutput;
    const HARNESS: HarnessId = HarnessId::ANTIGRAVITY;
    const EVENT: EventId = EventId::builtin("Stop");
    const CATEGORY: EventCategory = EventCategory::Agent;
    const CONTRACT_ID: &'static str = "antigravity/docs-2026-07-12-r1/Stop";
    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        serde_json::from_value(invocation.json().clone()).map_err(Into::into)
    }
    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        ProcessEmission::success_json(&output)
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
}
