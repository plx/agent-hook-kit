use hookkit_core::{EventCategory, EventId, EventSpec, HarnessId, ProcessEmission, RawInvocation};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SNAPSHOT_ID: &str = "commit-9e552e9-r1";
pub static EVENTS: &[hookkit_core::NativeEventDescriptor] =
    &[hookkit_core::NativeEventDescriptor {
        contract_id: "codex/commit-9e552e9-r1/PreToolUse",
        harness: "codex",
        event: "PreToolUse",
        native_input: true,
        native_output: true,
        bindings: &[hookkit_core::HandlerKind::Command],
        conformance_cases: &["no-op", "deny-json", "deny-stderr"],
    }];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreToolUseInput {
    pub session_id: String,
    pub transcript_path: Option<hookkit_core::Utf8PathBuf>,
    pub cwd: hookkit_core::Utf8PathBuf,
    pub hook_event_name: String,
    pub model: String,
    pub turn_id: String,
    pub permission_mode: String,
    pub tool_name: String,
    pub tool_use_id: String,
    pub tool_input: serde_json::Value,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
pub enum PreToolUseOutput {
    NoOp,
    Block {
        reason: String,
    },
    Allow,
    Deny {
        reason: Option<String>,
    },
    Rewrite {
        updated_input: serde_json::Map<String, serde_json::Value>,
    },
    AdditionalContext {
        context: String,
    },
    DenyStderr {
        message: String,
    },
}

impl PreToolUseOutput {
    pub fn no_op() -> Self {
        Self::NoOp
    }
    pub fn block(reason: impl Into<String>) -> Self {
        Self::Block {
            reason: reason.into(),
        }
    }
    pub fn allow() -> Self {
        Self::Allow
    }
    pub fn deny(reason: Option<String>) -> Self {
        Self::Deny { reason }
    }
    pub fn rewrite(updated_input: serde_json::Map<String, serde_json::Value>) -> Self {
        Self::Rewrite { updated_input }
    }
    pub fn with_context(context: impl Into<String>) -> Self {
        Self::AdditionalContext {
            context: context.into(),
        }
    }
    pub fn deny_stderr(message: impl Into<String>) -> Self {
        Self::DenyStderr {
            message: message.into(),
        }
    }
}

pub enum PreToolUse {}

impl EventSpec for PreToolUse {
    type Input = PreToolUseInput;
    type CommandOutput = PreToolUseOutput;
    const HARNESS: HarnessId = HarnessId::CODEX;
    const EVENT: EventId = EventId::builtin("PreToolUse");
    const CATEGORY: EventCategory = EventCategory::Tool;
    const CONTRACT_ID: &'static str = "codex/commit-9e552e9-r1/PreToolUse";

    fn parse(invocation: &RawInvocation) -> hookkit_core::Result<Self::Input> {
        if invocation
            .json()
            .get("hook_event_name")
            .and_then(serde_json::Value::as_str)
            != Some("PreToolUse")
        {
            return Err(hookkit_core::HookkitError::InvalidForHint {
                harness: HarnessId::CODEX,
                event: Self::EVENT,
                message: "expected hook_event_name=PreToolUse".into(),
            });
        }
        serde_json::from_value(invocation.json().clone()).map_err(Into::into)
    }

    fn emit(output: Self::CommandOutput) -> hookkit_core::Result<ProcessEmission> {
        let output = match output {
            PreToolUseOutput::NoOp => return Ok(ProcessEmission::success_empty()),
            PreToolUseOutput::DenyStderr { message } => {
                return Ok(ProcessEmission::protocol_error(message, 2));
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
            PreToolUseOutput::Deny { reason } => {
                serde_json::json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":reason}})
            }
            PreToolUseOutput::Rewrite { updated_input } => {
                serde_json::json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow","updatedInput":updated_input}})
            }
            PreToolUseOutput::AdditionalContext { context } => {
                serde_json::json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":context}})
            }
        };
        ProcessEmission::success_json(&value)
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
}
