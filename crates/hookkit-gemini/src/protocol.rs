use hookkit_core::{EventCategory, EventId, EventSpec, HarnessId, ProcessEmission, RawInvocation};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SNAPSHOT_ID: &str = "commit-f354eeb-r1";
pub static EVENTS: &[hookkit_core::NativeEventDescriptor] =
    &[hookkit_core::NativeEventDescriptor {
        contract_id: "gemini-cli/commit-f354eeb-r1/BeforeToolSelection",
        harness: "gemini-cli",
        event: "BeforeToolSelection",
        native_input: true,
        native_output: true,
        bindings: &[hookkit_core::HandlerKind::Command],
        conformance_cases: &["no-op", "disable-tools"],
    }];

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
pub enum BeforeToolSelectionOutput {
    NoOp,
    Configure {
        mode: Option<ToolMode>,
        allowed_function_names: Vec<String>,
    },
}

impl BeforeToolSelectionOutput {
    pub fn no_op() -> Self {
        Self::NoOp
    }
    pub fn configure(
        mode: Option<ToolMode>,
        allowed_function_names: Vec<String>,
    ) -> hookkit_core::Result<Self> {
        if mode.is_none() && allowed_function_names.is_empty() {
            return Err(hookkit_core::HookkitError::InvalidOutputCombination {
                harness: hookkit_core::Harness::Gemini,
                event: hookkit_core::HookEventKey::BeforeToolSelection,
                message: "toolConfig must contain mode or allowedFunctionNames".into(),
            });
        }
        Ok(Self::Configure {
            mode,
            allowed_function_names,
        })
    }
}

pub enum BeforeToolSelection {}

impl EventSpec for BeforeToolSelection {
    type Input = BeforeToolSelectionInput;
    type CommandOutput = BeforeToolSelectionOutput;
    const HARNESS: HarnessId = HarnessId::GEMINI_CLI;
    const EVENT: EventId = EventId::builtin("BeforeToolSelection");
    const CATEGORY: EventCategory = EventCategory::Model;
    const CONTRACT_ID: &'static str = "gemini-cli/commit-f354eeb-r1/BeforeToolSelection";

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
        let value = match output {
            BeforeToolSelectionOutput::NoOp => serde_json::json!({}),
            BeforeToolSelectionOutput::Configure {
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
        ProcessEmission::success_json(&value)
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
}
