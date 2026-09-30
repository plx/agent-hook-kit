//! Claude Code `PreModelSwitch` and `PostModelSwitch` command contracts
//! (Claude Code v2.1.251 or later).

use hookkit_core::{
    ContractId, EventCategory, EventId, EventSpec, HarnessId, NativeContext, ProcessEmission,
    RawInvocation, SessionId, SnapshotId, Utf8PathBuf,
};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;

use crate::ClaudeCommandEnvironment;
use crate::catalog::{
    CatalogOutput, block_builders, blocking_error, context_builders, into_blocking_error, no_op,
    nonblocking_error, text_context, universal_builders,
};
use crate::protocol::{SNAPSHOT_ID, contract_id, deserialize_input, invalid_input, require_event};
use crate::values::{CacheTtl, Effort, ModelSwitchPricing, ModelSwitchSource, PermissionMode};

/// Native input shared by `PreModelSwitch` and `PostModelSwitch`.
///
/// Every model-switch field is required. `requested_model` must be present
/// but may be `null`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ModelSwitchInput {
    /// Native session identifier.
    pub session_id: String,
    /// Path to the native conversation transcript.
    pub transcript_path: Utf8PathBuf,
    /// Current workspace directory.
    pub cwd: Utf8PathBuf,
    /// Authoritative native event discriminator.
    pub hook_event_name: String,
    /// Model ID the switch changes from.
    pub from_model: String,
    /// Model ID the switch changes to. Matchers compare its canonical name.
    pub to_model: String,
    /// Model the request named: an alias, a full model ID, or `None` for the
    /// default model. `None` for an automatic `PostModelSwitch`.
    #[serde(deserialize_with = "required_nullable")]
    pub requested_model: Option<String>,
    /// Where the switch came from.
    pub source: ModelSwitchSource,
    /// Tokens the next request re-sends as its prompt; `0` before the first
    /// response.
    pub context_tokens: u64,
    /// Whether the current model's prompt cache is likely still warm.
    pub prompt_cache_warm: bool,
    /// Prompt-cache lifetime Claude Code requests for the session.
    pub cache_ttl: CacheTtl,
    /// Estimated US-dollar cost of writing `context_tokens` to the new
    /// model's prompt cache; never negative.
    #[serde(serialize_with = "crate::values::serialize_js_number")]
    pub estimated_cache_write_usd: f64,
    /// How Claude Code priced `estimated_cache_write_usd`.
    pub pricing: ModelSwitchPricing,
    /// Subagent identifier when the event ran on behalf of a subagent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Subagent type when the event ran on behalf of a subagent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    /// Optional nested effort setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,
    /// Permission policy active for the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
    /// Prompt identifier associated with the current turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_id: Option<String>,
    /// Session scratchpad directory (Claude Code v2.1.257 or later).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scratchpad_dir: Option<Utf8PathBuf>,
    /// Unknown protocol fields retained for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Deserializes a key that must be present but may be `null`.
fn required_nullable<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

fn parse_model_switch(
    invocation: &RawInvocation,
    event: &'static str,
) -> hookkit_core::Result<ModelSwitchInput> {
    require_event(invocation, event)?;
    let input: ModelSwitchInput = deserialize_input(invocation, event)?;
    if input.estimated_cache_write_usd < 0.0 {
        return Err(invalid_input(
            event,
            "estimated_cache_write_usd must be non-negative",
        ));
    }
    Ok(input)
}

fn model_switch_context(input: &ModelSwitchInput) -> NativeContext {
    NativeContext {
        workspace_roots: vec![input.cwd.clone()],
        session_id: SessionId::new(&input.session_id).ok(),
        transcript_path: Some(input.transcript_path.clone()),
        ..NativeContext::default()
    }
}

fn validate_model_switch_environment(
    event: &EventId,
    input: &ModelSwitchInput,
    environment: &ClaudeCommandEnvironment,
) -> hookkit_core::Result<()> {
    environment.validate_input(
        event,
        &input.session_id,
        input.effort.as_ref().map(|effort| effort.level.as_str()),
    )
}

/// Native Claude Code `PreModelSwitch` command contract.
///
/// Runs before a switch a user or client requested, never for automatic
/// fallback or a model restored on resume. A hook that times out blocks the
/// switch; a parse failure (exit 1 without JSON) does not.
pub enum PreModelSwitch {}

impl EventSpec for PreModelSwitch {
    type Input = ModelSwitchInput;
    type CommandEnvironment = ClaudeCommandEnvironment;
    type CommandOutput = PreModelSwitchOutput;
    const HARNESS: HarnessId = HarnessId::CLAUDE_CODE;
    const SNAPSHOT: SnapshotId = SNAPSHOT_ID;
    const EVENT: EventId = EventId::builtin(HarnessId::CLAUDE_CODE, "PreModelSwitch");
    const CATEGORY: EventCategory = EventCategory::Model;
    const CONTRACT: ContractId = contract_id!("PreModelSwitch");

    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        parse_model_switch(invocation, "PreModelSwitch")
    }

    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        output.0.emit()
    }

    fn validate_command_environment(
        input: &Self::Input,
        environment: &Self::CommandEnvironment,
    ) -> hookkit_core::Result<()> {
        validate_model_switch_environment(&Self::EVENT, input, environment)
    }

    fn context(input: &Self::Input) -> NativeContext {
        model_switch_context(input)
    }
}

/// Native response from a Claude Code `PreModelSwitch` command hook.
///
/// When several hooks disagree, Claude Code applies `deny` over `ask` over
/// `allow`. `ask` is treated as a refusal everywhere except interactive
/// `/model`.
#[derive(Debug, Clone)]
pub struct PreModelSwitchOutput(CatalogOutput);

no_op!(PreModelSwitch, PreModelSwitchOutput);
block_builders!(PreModelSwitch, PreModelSwitchOutput, "Cancels the switch.");
blocking_error!(
    PreModelSwitch,
    PreModelSwitchOutput,
    "Cancels the switch; stderr is shown to the user."
);
into_blocking_error!(PreModelSwitchOutput);
nonblocking_error!(PreModelSwitch, PreModelSwitchOutput);
universal_builders!(PreModelSwitchOutput: continue_session, stop_reason, system_message, terminal_sequence);

impl PreModelSwitchOutput {
    fn with_decision(decision: &'static str, reason: Option<String>) -> Self {
        let reason = reason.map(|reason| ("permissionDecisionReason", reason.into()));
        Self(CatalogOutput::hook_specific::<PreModelSwitch>(
            std::iter::once(("permissionDecision", decision.into())).chain(reason),
        ))
    }

    /// Lets the switch proceed and skips the confirmation Claude Code shows
    /// while the prompt cache is warm.
    pub fn allow() -> Self {
        Self::with_decision("allow", None)
    }

    /// Cancels the switch; `reason` is shown to the user, or returned as the
    /// error for an SDK `set_model` request.
    pub fn deny(reason: impl Into<String>) -> Self {
        Self::with_decision("deny", Some(reason.into()))
    }

    /// Asks the user to confirm the switch; `reason` is shown in the
    /// confirmation prompt. Outside interactive `/model`, Claude Code treats
    /// `ask` as a refusal.
    pub fn ask(reason: impl Into<String>) -> Self {
        Self::with_decision("ask", Some(reason.into()))
    }

    /// Sets `permissionDecisionReason` on a `deny` or `ask` decision.
    /// Rejected for `allow`, where Claude Code ignores it, and when no
    /// decision was made.
    pub fn with_decision_reason(self, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        match self
            .0
            .specific_field("permissionDecision")
            .and_then(serde_json::Value::as_str)
        {
            Some("deny" | "ask") => self
                .0
                .with_specific("permissionDecisionReason", reason.into().into())
                .map(Self),
            _ => Err(hookkit_core::HookkitError::InvalidProcessEmission(
                "permissionDecisionReason applies only to deny and ask decisions",
            )),
        }
    }
}

impl From<PreModelSwitchOutput> for crate::protocol::AnyCommandOutput {
    fn from(output: PreModelSwitchOutput) -> Self {
        Self::PreModelSwitch(output)
    }
}

/// Native Claude Code `PostModelSwitch` command contract.
///
/// Runs after any change of the session's model, including automatic
/// fallback and a model restored on resume. It cannot block.
pub enum PostModelSwitch {}

impl EventSpec for PostModelSwitch {
    type Input = ModelSwitchInput;
    type CommandEnvironment = ClaudeCommandEnvironment;
    type CommandOutput = PostModelSwitchOutput;
    const HARNESS: HarnessId = HarnessId::CLAUDE_CODE;
    const SNAPSHOT: SnapshotId = SNAPSHOT_ID;
    const EVENT: EventId = EventId::builtin(HarnessId::CLAUDE_CODE, "PostModelSwitch");
    const CATEGORY: EventCategory = EventCategory::Model;
    const CONTRACT: ContractId = contract_id!("PostModelSwitch");

    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        parse_model_switch(invocation, "PostModelSwitch")
    }

    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        output.0.emit()
    }

    fn validate_command_environment(
        input: &Self::Input,
        environment: &Self::CommandEnvironment,
    ) -> hookkit_core::Result<()> {
        validate_model_switch_environment(&Self::EVENT, input, environment)
    }

    fn context(input: &Self::Input) -> NativeContext {
        model_switch_context(input)
    }
}

/// Native response from a Claude Code `PostModelSwitch` command hook.
///
/// Context is delivered with the next request after the switch; only the
/// last switch's output is delivered.
#[derive(Debug, Clone)]
pub struct PostModelSwitchOutput(CatalogOutput);

no_op!(PostModelSwitch, PostModelSwitchOutput);
context_builders!(
    PostModelSwitch,
    PostModelSwitchOutput,
    "with the next request after the switch"
);
text_context!(PostModelSwitch, PostModelSwitchOutput);
nonblocking_error!(PostModelSwitch, PostModelSwitchOutput);
universal_builders!(PostModelSwitchOutput: continue_session, stop_reason, system_message, terminal_sequence);

impl From<PostModelSwitchOutput> for crate::protocol::AnyCommandOutput {
    fn from(output: PostModelSwitchOutput) -> Self {
        Self::PostModelSwitch(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn switch(event: &str) -> serde_json::Value {
        serde_json::json!({
            "session_id": "s",
            "transcript_path": "/tmp/t",
            "cwd": "/repo",
            "hook_event_name": event,
            "from_model": "claude-sonnet-5",
            "to_model": "claude-opus-5",
            "requested_model": "opus",
            "source": "command",
            "context_tokens": 182340,
            "prompt_cache_warm": true,
            "cache_ttl": "1h",
            "estimated_cache_write_usd": 1.1396,
            "pricing": "configured",
        })
    }

    fn raw(value: serde_json::Value) -> RawInvocation {
        RawInvocation::parse(serde_json::to_vec(&value).unwrap()).unwrap()
    }

    fn stdout_json(emission: &ProcessEmission) -> serde_json::Value {
        serde_json::from_slice(emission.stdout()).unwrap()
    }

    #[test]
    fn both_events_parse_the_shared_input() {
        let input = PreModelSwitch::parse(&raw(switch("PreModelSwitch"))).unwrap();
        assert_eq!(input.source, ModelSwitchSource::Command);
        assert_eq!(input.cache_ttl, CacheTtl::OneHour);
        assert_eq!(input.pricing, ModelSwitchPricing::Configured);
        assert_eq!(input.requested_model.as_deref(), Some("opus"));
        assert_eq!(
            serde_json::to_value(&input).unwrap(),
            switch("PreModelSwitch")
        );

        let mut automatic = switch("PostModelSwitch");
        automatic["source"] = "auto".into();
        automatic["requested_model"] = serde_json::Value::Null;
        let input = PostModelSwitch::parse(&raw(automatic.clone())).unwrap();
        assert_eq!(input.source, ModelSwitchSource::Auto);
        assert_eq!(input.requested_model, None);
        assert_eq!(serde_json::to_value(&input).unwrap(), automatic);

        assert!(PostModelSwitch::parse(&raw(switch("PreModelSwitch"))).is_err());
    }

    #[test]
    fn requested_model_must_be_present_even_when_null() {
        let mut value = switch("PreModelSwitch");
        value.as_object_mut().unwrap().remove("requested_model");
        assert!(PreModelSwitch::parse(&raw(value)).is_err());
    }

    #[test]
    fn negative_costs_are_rejected_and_future_values_parse() {
        let mut value = switch("PreModelSwitch");
        value["estimated_cache_write_usd"] = (-0.5).into();
        assert!(PreModelSwitch::parse(&raw(value)).is_err());

        let mut value = switch("PreModelSwitch");
        value["context_tokens"] = (-1).into();
        assert!(PreModelSwitch::parse(&raw(value)).is_err());

        let mut value = switch("PreModelSwitch");
        value["source"] = "schedule".into();
        value["cache_ttl"] = "24h".into();
        value["pricing"] = "negotiated".into();
        let input = PreModelSwitch::parse(&raw(value.clone())).unwrap();
        assert!(!input.source.is_documented());
        assert_eq!(input.cache_ttl.as_str(), "24h");
        assert_eq!(serde_json::to_value(&input).unwrap(), value);
    }

    #[test]
    fn pre_model_switch_decisions_and_reasons() {
        let value =
            stdout_json(&PreModelSwitch::emit(PreModelSwitchOutput::deny("retired")).unwrap());
        assert_eq!(
            value,
            serde_json::json!({"hookSpecificOutput": {
                "hookEventName": "PreModelSwitch",
                "permissionDecision": "deny",
                "permissionDecisionReason": "retired",
            }})
        );
        assert!(
            PreModelSwitchOutput::allow()
                .with_decision_reason("r")
                .is_err()
        );
        assert!(
            PreModelSwitchOutput::no_op()
                .with_decision_reason("r")
                .is_err()
        );
        assert!(
            PreModelSwitchOutput::ask("a")
                .with_decision_reason("b")
                .is_ok()
        );

        let emission = PreModelSwitch::emit(PreModelSwitchOutput::blocking_error("no")).unwrap();
        assert_eq!((emission.exit_code(), emission.stderr()), (2, &b"no"[..]));
    }

    #[test]
    fn post_model_switch_context_and_notices() {
        let emission =
            PostModelSwitch::emit(PostModelSwitchOutput::text_context("Use subagents.")).unwrap();
        assert_eq!(emission.stdout(), b"Use subagents.");
        let value =
            stdout_json(&PostModelSwitch::emit(PostModelSwitchOutput::text_context("{}")).unwrap());
        assert_eq!(value["hookSpecificOutput"]["additionalContext"], "{}");
        let emission =
            PostModelSwitch::emit(PostModelSwitchOutput::nonblocking_error("oops")).unwrap();
        assert_eq!(emission.exit_code(), 1);
    }
}
