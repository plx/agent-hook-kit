//! I/O and execution plumbing for hookkit hook executables.

pub mod artifacts;
pub mod logging;
mod validate;

#[cfg(test)]
mod golden_tests;

use hookkit_claude::{ClaudeHookInput, ClaudeHookOutput};
use hookkit_codex::{CodexHookInput, CodexHookOutput};
use hookkit_common::input::{
    CommonHookInput, CommonNotificationInput, CommonPostToolUseInput, CommonPreCompressInput,
    CommonPreToolUseInput, CommonPromptSubmitInput, CommonSessionEndInput, CommonSessionStartInput,
    CommonStopInput,
};
use hookkit_common::output::{CommonHookOutput, LoweringAction, LoweringWarning};
use hookkit_core::{Harness, HookEventKey, HookkitError};
use hookkit_gemini::{GeminiHookInput, GeminiHookOutput};
use std::io::Read;

pub use validate::{validate_claude, validate_codex, validate_gemini};

/// Runtime context available to hook handlers.
pub struct RuntimeContext {
    pub harness: Harness,
    pub raw_input: serde_json::Value,
    pub stdin_bytes: Vec<u8>,
    pub cwd: hookkit_core::Utf8PathBuf,
}

impl RuntimeContext {
    pub fn hook_event_name(&self) -> Option<&str> {
        self.raw_input
            .get("hookEventName")
            .or_else(|| self.raw_input.get("hook_event_name"))
            .and_then(|v| v.as_str())
    }

    pub fn session_id(&self) -> Option<&str> {
        self.raw_input
            .get("sessionId")
            .or_else(|| self.raw_input.get("session_id"))
            .and_then(|v| v.as_str())
    }

    pub fn turn_id(&self) -> Option<&str> {
        self.raw_input
            .get("turnId")
            .or_else(|| self.raw_input.get("turn_id"))
            .and_then(|v| v.as_str())
    }

    pub fn tool_use_id(&self) -> Option<&str> {
        self.raw_input
            .get("toolUseId")
            .or_else(|| self.raw_input.get("tool_use_id"))
            .and_then(|v| v.as_str())
    }

    pub fn artifact_key(&self, label: impl Into<String>) -> artifacts::ArtifactKey {
        let mut key = artifacts::ArtifactKey::new(
            self.session_id().unwrap_or("unknown-session"),
            label.into(),
        );
        if let Some(turn_id) = self.turn_id() {
            key = key.with_turn(turn_id);
        }
        if let Some(tool_use_id) = self.tool_use_id() {
            key = key.with_tool_use(tool_use_id);
        }
        key
    }
}

/// Unified native input across harnesses.
#[derive(Debug, Clone)]
pub enum NativeHookInput {
    Claude(ClaudeHookInput),
    Codex(CodexHookInput),
    Gemini(GeminiHookInput),
}

/// Unified native output across harnesses.
pub enum NativeHookOutput {
    Claude(ClaudeHookOutput),
    Codex(CodexHookOutput),
    Gemini(GeminiHookOutput),
}

/// Native output plus side-channel messages produced by common lowering.
pub struct LoweredOutput {
    pub native: NativeHookOutput,
    pub stderr_messages: Vec<String>,
    pub warnings: Vec<LoweringWarning>,
}

impl LoweredOutput {
    fn new(native: NativeHookOutput) -> Self {
        Self {
            native,
            stderr_messages: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

fn hook_event_key(input: &NativeHookInput) -> HookEventKey {
    match input {
        NativeHookInput::Claude(ev) => match ev {
            ClaudeHookInput::SessionStart(_) => HookEventKey::SessionStart,
            ClaudeHookInput::UserPromptSubmit(_) => HookEventKey::PromptSubmit,
            ClaudeHookInput::UserPromptExpansion(_) => HookEventKey::PromptExpansion,
            ClaudeHookInput::PreToolUse(_) => HookEventKey::PreToolUse,
            ClaudeHookInput::PermissionRequest(_) => HookEventKey::PermissionRequest,
            ClaudeHookInput::PermissionDenied(_) => HookEventKey::PermissionDenied,
            ClaudeHookInput::PostToolUse(_) => HookEventKey::PostToolUse,
            ClaudeHookInput::PostToolUseFailure(_) => HookEventKey::PostToolUseFailure,
            ClaudeHookInput::PostToolBatch(_) => HookEventKey::PostToolBatch,
            ClaudeHookInput::Stop(_) => HookEventKey::Stop,
            ClaudeHookInput::StopFailure(_) => HookEventKey::StopFailure,
            ClaudeHookInput::Notification(_) => HookEventKey::Notification,
            ClaudeHookInput::SubagentStart(_) => HookEventKey::SubagentStart,
            ClaudeHookInput::SubagentStop(_) => HookEventKey::SubagentStop,
            ClaudeHookInput::TaskCreated(_) => HookEventKey::TaskCreated,
            ClaudeHookInput::TaskCompleted(_) => HookEventKey::TaskCompleted,
            ClaudeHookInput::TeammateIdle(_) => HookEventKey::TeammateIdle,
            ClaudeHookInput::InstructionsLoaded(_) => HookEventKey::InstructionsLoaded,
            ClaudeHookInput::ConfigChange(_) => HookEventKey::ConfigChange,
            ClaudeHookInput::CwdChanged(_) => HookEventKey::CwdChanged,
            ClaudeHookInput::FileChanged(_) => HookEventKey::FileChanged,
            ClaudeHookInput::WorktreeCreate(_) => HookEventKey::WorktreeCreate,
            ClaudeHookInput::WorktreeRemove(_) => HookEventKey::WorktreeRemove,
            ClaudeHookInput::PreCompact(_) => HookEventKey::PreCompact,
            ClaudeHookInput::PostCompact(_) => HookEventKey::PostCompact,
            ClaudeHookInput::Elicitation(_) => HookEventKey::Elicitation,
            ClaudeHookInput::ElicitationResult(_) => HookEventKey::ElicitationResult,
            ClaudeHookInput::SessionEnd(_) => HookEventKey::SessionEnd,
            ClaudeHookInput::Unknown { event_name, .. } => HookEventKey::Other(event_name.clone()),
        },
        NativeHookInput::Codex(ev) => match ev {
            CodexHookInput::SessionStart(_) => HookEventKey::SessionStart,
            CodexHookInput::PreToolUse(_) => HookEventKey::PreToolUse,
            CodexHookInput::PostToolUse(_) => HookEventKey::PostToolUse,
            CodexHookInput::UserPromptSubmit(_) => HookEventKey::PromptSubmit,
            CodexHookInput::Stop(_) => HookEventKey::Stop,
            CodexHookInput::Unknown { event_name, .. } => HookEventKey::Other(event_name.clone()),
        },
        NativeHookInput::Gemini(ev) => match ev {
            GeminiHookInput::SessionStart(_) => HookEventKey::SessionStart,
            GeminiHookInput::SessionEnd(_) => HookEventKey::SessionEnd,
            GeminiHookInput::BeforeAgent(_) => HookEventKey::PromptSubmit,
            GeminiHookInput::AfterAgent(_) => HookEventKey::Stop,
            GeminiHookInput::BeforeTool(_) => HookEventKey::PreToolUse,
            GeminiHookInput::AfterTool(_) => HookEventKey::PostToolUse,
            GeminiHookInput::Notification(_) => HookEventKey::Notification,
            GeminiHookInput::PreCompress(_) => HookEventKey::PreCompress,
            GeminiHookInput::BeforeModel(_) => HookEventKey::BeforeModel,
            GeminiHookInput::AfterModel(_) => HookEventKey::AfterModel,
            GeminiHookInput::BeforeToolSelection(_) => HookEventKey::BeforeToolSelection,
            GeminiHookInput::Unknown { event_name, .. } => HookEventKey::Other(event_name.clone()),
        },
    }
}

fn input_harness(input: &NativeHookInput) -> Harness {
    match input {
        NativeHookInput::Claude(_) => Harness::Claude,
        NativeHookInput::Codex(_) => Harness::Codex,
        NativeHookInput::Gemini(_) => Harness::Gemini,
    }
}

fn validate_native_output(
    input: &NativeHookInput,
    output: &NativeHookOutput,
) -> hookkit_core::Result<()> {
    let harness = input_harness(input);
    let event = hook_event_key(input);

    match (harness, output) {
        (Harness::Claude, NativeHookOutput::Claude(out)) => match out {
            ClaudeHookOutput::Empty => Ok(()),
            ClaudeHookOutput::Json(envelope) => validate_claude(&event, envelope),
            ClaudeHookOutput::BlockingError { .. } => Ok(()),
        },
        (Harness::Codex, NativeHookOutput::Codex(out)) => match out {
            CodexHookOutput::Empty => {
                if matches!(event, HookEventKey::Stop) {
                    return Err(HookkitError::InvalidOutputCombination {
                        harness: Harness::Codex,
                        event,
                        message: "Codex Stop must return JSON output on exit 0".to_string(),
                    });
                }
                Ok(())
            }
            CodexHookOutput::Json(envelope) => validate_codex(&event, envelope),
            CodexHookOutput::BlockingDeny { .. } => Ok(()),
        },
        (Harness::Gemini, NativeHookOutput::Gemini(out)) => match out {
            GeminiHookOutput::Empty => Ok(()),
            GeminiHookOutput::Json(envelope) => validate_gemini(&event, envelope),
            GeminiHookOutput::BlockingError { .. } => Ok(()),
        },
        (_, NativeHookOutput::Claude(_)) => Err(HookkitError::InvalidOutputCombination {
            harness,
            event,
            message: "handler returned Claude output for a different input harness".to_string(),
        }),
        (_, NativeHookOutput::Codex(_)) => Err(HookkitError::InvalidOutputCombination {
            harness,
            event,
            message: "handler returned Codex output for a different input harness".to_string(),
        }),
        (_, NativeHookOutput::Gemini(_)) => Err(HookkitError::InvalidOutputCombination {
            harness,
            event,
            message: "handler returned Gemini output for a different input harness".to_string(),
        }),
    }
}

fn hook_event_key_name(key: &HookEventKey) -> String {
    match key {
        HookEventKey::SessionStart => "SessionStart".to_string(),
        HookEventKey::SessionEnd => "SessionEnd".to_string(),
        HookEventKey::PromptSubmit => "PromptSubmit".to_string(),
        HookEventKey::PromptExpansion => "PromptExpansion".to_string(),
        HookEventKey::PreToolUse => "PreToolUse".to_string(),
        HookEventKey::PermissionRequest => "PermissionRequest".to_string(),
        HookEventKey::PermissionDenied => "PermissionDenied".to_string(),
        HookEventKey::PostToolUse => "PostToolUse".to_string(),
        HookEventKey::PostToolUseFailure => "PostToolUseFailure".to_string(),
        HookEventKey::PostToolBatch => "PostToolBatch".to_string(),
        HookEventKey::Stop => "Stop".to_string(),
        HookEventKey::StopFailure => "StopFailure".to_string(),
        HookEventKey::Notification => "Notification".to_string(),
        HookEventKey::SubagentStart => "SubagentStart".to_string(),
        HookEventKey::SubagentStop => "SubagentStop".to_string(),
        HookEventKey::TaskCreated => "TaskCreated".to_string(),
        HookEventKey::TaskCompleted => "TaskCompleted".to_string(),
        HookEventKey::TeammateIdle => "TeammateIdle".to_string(),
        HookEventKey::InstructionsLoaded => "InstructionsLoaded".to_string(),
        HookEventKey::ConfigChange => "ConfigChange".to_string(),
        HookEventKey::CwdChanged => "CwdChanged".to_string(),
        HookEventKey::FileChanged => "FileChanged".to_string(),
        HookEventKey::WorktreeCreate => "WorktreeCreate".to_string(),
        HookEventKey::WorktreeRemove => "WorktreeRemove".to_string(),
        HookEventKey::PreCompact => "PreCompact".to_string(),
        HookEventKey::PostCompact => "PostCompact".to_string(),
        HookEventKey::Elicitation => "Elicitation".to_string(),
        HookEventKey::ElicitationResult => "ElicitationResult".to_string(),
        HookEventKey::BeforeModel => "BeforeModel".to_string(),
        HookEventKey::AfterModel => "AfterModel".to_string(),
        HookEventKey::BeforeToolSelection => "BeforeToolSelection".to_string(),
        HookEventKey::PreCompress => "PreCompress".to_string(),
        HookEventKey::Other(name) => name.clone(),
    }
}

fn parsed_value(input: &NativeHookInput) -> serde_json::Value {
    match input {
        NativeHookInput::Claude(ev) => match ev {
            ClaudeHookInput::SessionStart(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::UserPromptSubmit(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::UserPromptExpansion(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::PreToolUse(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::PermissionRequest(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::PermissionDenied(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::PostToolUse(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::PostToolUseFailure(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::PostToolBatch(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::Stop(v) => serde_json::to_value(v).unwrap_or(serde_json::Value::Null),
            ClaudeHookInput::StopFailure(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::Notification(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::SubagentStart(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::SubagentStop(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::TaskCreated(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::TaskCompleted(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::TeammateIdle(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::InstructionsLoaded(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::ConfigChange(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::CwdChanged(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::FileChanged(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::WorktreeCreate(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::WorktreeRemove(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::PreCompact(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::PostCompact(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::Elicitation(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::ElicitationResult(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::SessionEnd(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            ClaudeHookInput::Unknown { raw, .. } => raw.as_value().clone(),
        },
        NativeHookInput::Codex(ev) => match ev {
            CodexHookInput::SessionStart(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            CodexHookInput::PreToolUse(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            CodexHookInput::PostToolUse(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            CodexHookInput::UserPromptSubmit(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            CodexHookInput::Stop(v) => serde_json::to_value(v).unwrap_or(serde_json::Value::Null),
            CodexHookInput::Unknown { raw, .. } => raw.as_value().clone(),
        },
        NativeHookInput::Gemini(ev) => match ev {
            GeminiHookInput::SessionStart(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            GeminiHookInput::SessionEnd(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            GeminiHookInput::BeforeAgent(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            GeminiHookInput::AfterAgent(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            GeminiHookInput::BeforeTool(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            GeminiHookInput::AfterTool(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            GeminiHookInput::Notification(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            GeminiHookInput::PreCompress(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            GeminiHookInput::BeforeModel(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            GeminiHookInput::AfterModel(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            GeminiHookInput::BeforeToolSelection(v) => {
                serde_json::to_value(v).unwrap_or(serde_json::Value::Null)
            }
            GeminiHookInput::Unknown { raw, .. } => raw.as_value().clone(),
        },
    }
}

fn format_dump_parsed(input: &NativeHookInput) -> String {
    let payload = serde_json::json!({
        "hookkitDebug": "dump-parsed",
        "harness": input_harness(input).to_string(),
        "event": hook_event_key_name(&hook_event_key(input)),
        "parsed": parsed_value(input),
    });
    serde_json::to_string(&payload).unwrap_or_else(|_| {
        "{\"hookkitDebug\":\"dump-parsed\",\"error\":\"serialization-failed\"}".to_string()
    })
}

fn dump_parsed_enabled() -> bool {
    std::env::args().any(|arg| arg == "--dump-parsed")
}

fn maybe_dump_parsed(input: &NativeHookInput) {
    if dump_parsed_enabled() {
        eprintln!("{}", format_dump_parsed(input));
    }
}

fn empty_output_for_harness(harness: Harness) -> NativeHookOutput {
    match harness {
        Harness::Claude => NativeHookOutput::Claude(ClaudeHookOutput::Empty),
        Harness::Codex => NativeHookOutput::Codex(CodexHookOutput::Empty),
        Harness::Gemini => NativeHookOutput::Gemini(GeminiHookOutput::Empty),
    }
}

fn native_to_common(input: NativeHookInput) -> hookkit_core::Result<CommonHookInput> {
    let harness = input_harness(&input);
    let event = hook_event_key(&input);

    match input {
        NativeHookInput::Claude(ev) => match ev {
            ClaudeHookInput::SessionStart(ev) => Ok(CommonHookInput::SessionStart(
                CommonSessionStartInput::Claude(ev),
            )),
            ClaudeHookInput::UserPromptSubmit(ev) => Ok(CommonHookInput::PromptSubmit(
                CommonPromptSubmitInput::Claude(ev),
            )),
            ClaudeHookInput::PreToolUse(ev) => Ok(CommonHookInput::PreToolUse(
                CommonPreToolUseInput::Claude(ev),
            )),
            ClaudeHookInput::PostToolUse(ev) => Ok(CommonHookInput::PostToolUse(
                CommonPostToolUseInput::Claude(ev),
            )),
            ClaudeHookInput::Stop(ev) => Ok(CommonHookInput::Stop(CommonStopInput::Claude(ev))),
            ClaudeHookInput::Notification(ev) => Ok(CommonHookInput::Notification(
                CommonNotificationInput::Claude(ev),
            )),
            ClaudeHookInput::SessionEnd(ev) => Ok(CommonHookInput::SessionEnd(
                CommonSessionEndInput::Claude(ev),
            )),
            ClaudeHookInput::PreCompact(ev) => Ok(CommonHookInput::PreCompress(
                CommonPreCompressInput::Claude(ev),
            )),
            _ => Err(HookkitError::UnsupportedCapability {
                harness,
                event,
                capability: "common-wrapper conversion for this event is not yet implemented",
            }),
        },
        NativeHookInput::Codex(ev) => match ev {
            CodexHookInput::SessionStart(ev) => Ok(CommonHookInput::SessionStart(
                CommonSessionStartInput::Codex(ev),
            )),
            CodexHookInput::UserPromptSubmit(ev) => Ok(CommonHookInput::PromptSubmit(
                CommonPromptSubmitInput::Codex(ev),
            )),
            CodexHookInput::PreToolUse(ev) => Ok(CommonHookInput::PreToolUse(
                CommonPreToolUseInput::Codex(ev),
            )),
            CodexHookInput::PostToolUse(ev) => Ok(CommonHookInput::PostToolUse(
                CommonPostToolUseInput::Codex(ev),
            )),
            CodexHookInput::Stop(ev) => Ok(CommonHookInput::Stop(CommonStopInput::Codex(ev))),
            _ => Err(HookkitError::UnsupportedCapability {
                harness,
                event,
                capability: "common-wrapper conversion for this event is not yet implemented",
            }),
        },
        NativeHookInput::Gemini(ev) => match ev {
            GeminiHookInput::SessionStart(ev) => Ok(CommonHookInput::SessionStart(
                CommonSessionStartInput::Gemini(ev),
            )),
            GeminiHookInput::BeforeAgent(ev) => Ok(CommonHookInput::PromptSubmit(
                CommonPromptSubmitInput::Gemini(ev),
            )),
            GeminiHookInput::BeforeTool(ev) => Ok(CommonHookInput::PreToolUse(
                CommonPreToolUseInput::Gemini(ev),
            )),
            GeminiHookInput::AfterTool(ev) => Ok(CommonHookInput::PostToolUse(
                CommonPostToolUseInput::Gemini(ev),
            )),
            GeminiHookInput::AfterAgent(ev) => {
                Ok(CommonHookInput::Stop(CommonStopInput::Gemini(ev)))
            }
            GeminiHookInput::Notification(ev) => Ok(CommonHookInput::Notification(
                CommonNotificationInput::Gemini(ev),
            )),
            GeminiHookInput::SessionEnd(ev) => Ok(CommonHookInput::SessionEnd(
                CommonSessionEndInput::Gemini(ev),
            )),
            GeminiHookInput::PreCompress(ev) => Ok(CommonHookInput::PreCompress(
                CommonPreCompressInput::Gemini(ev),
            )),
            _ => Err(HookkitError::UnsupportedCapability {
                harness,
                event,
                capability: "common-wrapper conversion for this event is not yet implemented",
            }),
        },
    }
}

fn common_to_native(
    harness: Harness,
    output: CommonHookOutput,
) -> hookkit_core::Result<LoweredOutput> {
    match output {
        CommonHookOutput::Empty => Ok(LoweredOutput::new(empty_output_for_harness(harness))),
        CommonHookOutput::SessionStart(out) => match harness {
            Harness::Claude => Ok(LoweredOutput::new(NativeHookOutput::Claude(
                out.to_claude()?,
            ))),
            Harness::Codex => Ok(LoweredOutput::new(NativeHookOutput::Codex(out.to_codex()?))),
            Harness::Gemini => Ok(LoweredOutput::new(NativeHookOutput::Gemini(
                out.to_gemini()?,
            ))),
        },
        CommonHookOutput::Notification(out) => match harness {
            Harness::Claude => Ok(LoweredOutput::new(NativeHookOutput::Claude(
                out.to_claude()?,
            ))),
            Harness::Codex => Err(HookkitError::UnsupportedCapability {
                harness: Harness::Codex,
                event: HookEventKey::Notification,
                capability: "notification output conversion (not supported by Codex)",
            }),
            Harness::Gemini => Ok(LoweredOutput::new(NativeHookOutput::Gemini(
                out.to_gemini()?,
            ))),
        },
        CommonHookOutput::SessionEnd(out) => match harness {
            Harness::Claude => Ok(LoweredOutput::new(NativeHookOutput::Claude(
                out.to_claude()?,
            ))),
            Harness::Codex => Err(HookkitError::UnsupportedCapability {
                harness: Harness::Codex,
                event: HookEventKey::SessionEnd,
                capability: "session_end output conversion (not supported by Codex)",
            }),
            Harness::Gemini => Ok(LoweredOutput::new(NativeHookOutput::Gemini(
                out.to_gemini()?,
            ))),
        },
        CommonHookOutput::PreCompress(out) => match harness {
            Harness::Claude => Ok(LoweredOutput::new(NativeHookOutput::Claude(
                out.to_claude()?,
            ))),
            Harness::Codex => Ok(LoweredOutput::new(NativeHookOutput::Codex(out.to_codex()?))),
            Harness::Gemini => Ok(LoweredOutput::new(NativeHookOutput::Gemini(
                out.to_gemini()?,
            ))),
        },
        CommonHookOutput::PostToolUse(out) => match harness {
            Harness::Claude => {
                let lowered = out.to_claude_lowered()?;
                Ok(LoweredOutput {
                    native: NativeHookOutput::Claude(lowered.native),
                    stderr_messages: lowered.stderr_messages,
                    warnings: lowered.warnings,
                })
            }
            Harness::Codex => {
                let lowered = out.to_codex_lowered()?;
                Ok(LoweredOutput {
                    native: NativeHookOutput::Codex(lowered.native),
                    stderr_messages: lowered.stderr_messages,
                    warnings: lowered.warnings,
                })
            }
            Harness::Gemini => {
                let lowered = out.to_gemini_lowered()?;
                Ok(LoweredOutput {
                    native: NativeHookOutput::Gemini(lowered.native),
                    stderr_messages: lowered.stderr_messages,
                    warnings: lowered.warnings,
                })
            }
        },
        CommonHookOutput::PreToolUse(out) => match harness {
            Harness::Claude => Ok(LoweredOutput::new(NativeHookOutput::Claude(
                out.to_claude(),
            ))),
            Harness::Codex => Ok(LoweredOutput::new(NativeHookOutput::Codex(out.to_codex()?))),
            Harness::Gemini => Ok(LoweredOutput::new(NativeHookOutput::Gemini(
                out.to_gemini(),
            ))),
        },
        CommonHookOutput::PromptSubmit(out) => match harness {
            Harness::Claude => Ok(LoweredOutput::new(NativeHookOutput::Claude(
                out.to_claude(),
            ))),
            Harness::Codex => Ok(LoweredOutput::new(NativeHookOutput::Codex(out.to_codex()))),
            Harness::Gemini => Ok(LoweredOutput::new(NativeHookOutput::Gemini(
                out.to_gemini(),
            ))),
        },
        CommonHookOutput::Stop(out) => match harness {
            Harness::Claude => Ok(LoweredOutput::new(NativeHookOutput::Claude(
                out.to_claude(),
            ))),
            Harness::Codex => Ok(LoweredOutput::new(NativeHookOutput::Codex(out.to_codex()))),
            Harness::Gemini => Ok(LoweredOutput::new(NativeHookOutput::Gemini(
                out.to_gemini(),
            ))),
        },
    }
}

/// Read stdin into bytes.
fn read_stdin() -> hookkit_core::Result<Vec<u8>> {
    let mut buf = Vec::new();
    std::io::stdin().read_to_end(&mut buf)?;
    Ok(buf)
}

/// Parse JSON bytes as a native hook input for the given harness.
pub fn parse_native(
    harness: Harness,
    bytes: &[u8],
) -> hookkit_core::Result<(serde_json::Value, NativeHookInput)> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    let input = match harness {
        Harness::Claude => NativeHookInput::Claude(hookkit_claude::input::parse(&value)?),
        Harness::Codex => NativeHookInput::Codex(hookkit_codex::input::parse(&value)?),
        Harness::Gemini => NativeHookInput::Gemini(hookkit_gemini::input::parse(&value)?),
    };
    Ok((value, input))
}

/// Validate and emit the output for a native hook result.
///
/// Validates the output before writing, then writes JSON to stdout,
/// messages to stderr, and returns the exit code.
fn emit_output(output: NativeHookOutput) -> std::process::ExitCode {
    match output {
        NativeHookOutput::Claude(o) => emit_claude_output(o),
        NativeHookOutput::Codex(o) => emit_codex_output(o),
        NativeHookOutput::Gemini(o) => emit_gemini_output(o),
    }
}

fn emit_lowered_output(output: LoweredOutput) -> std::process::ExitCode {
    for message in output.stderr_messages {
        eprintln!("{message}");
    }
    for warning in output.warnings {
        eprintln!("{}", format_lowering_warning(&warning));
    }
    emit_output(output.native)
}

fn format_lowering_warning(warning: &LoweringWarning) -> String {
    let action = match warning.action {
        LoweringAction::Dropped => "dropped",
        LoweringAction::RedirectedToStderr => "redirected to stderr",
        LoweringAction::Converted => "converted",
    };
    format!(
        "hookkit: lowering warning: {} {:?} intent `{}` {}",
        warning.harness, warning.event, warning.intent, action
    )
}

fn emit_claude_output(output: ClaudeHookOutput) -> std::process::ExitCode {
    match output {
        ClaudeHookOutput::Empty => std::process::ExitCode::SUCCESS,
        ClaudeHookOutput::Json(envelope) => {
            let json = serde_json::to_string(&envelope).expect("failed to serialize output");
            println!("{json}");
            std::process::ExitCode::SUCCESS
        }
        ClaudeHookOutput::BlockingError { stderr } => {
            eprintln!("{stderr}");
            std::process::ExitCode::from(2)
        }
    }
}

fn emit_codex_output(output: CodexHookOutput) -> std::process::ExitCode {
    match output {
        CodexHookOutput::Empty => std::process::ExitCode::SUCCESS,
        CodexHookOutput::Json(envelope) => {
            let json = serde_json::to_string(&envelope).expect("failed to serialize output");
            println!("{json}");
            std::process::ExitCode::SUCCESS
        }
        CodexHookOutput::BlockingDeny { stderr } => {
            eprintln!("{stderr}");
            std::process::ExitCode::from(2)
        }
    }
}

fn emit_gemini_output(output: GeminiHookOutput) -> std::process::ExitCode {
    match output {
        GeminiHookOutput::Empty => std::process::ExitCode::SUCCESS,
        GeminiHookOutput::Json(envelope) => {
            let json = serde_json::to_string(&envelope).expect("failed to serialize output");
            println!("{json}");
            std::process::ExitCode::SUCCESS
        }
        GeminiHookOutput::BlockingError { stderr } => {
            eprintln!("{stderr}");
            std::process::ExitCode::from(2)
        }
    }
}

/// Run a native hook handler.
///
/// Reads stdin, parses it for the specified harness, calls the handler,
/// validates the output, and emits the correct stdout/stderr/exit code.
pub fn run_native<F>(harness: Harness, handler: F) -> std::process::ExitCode
where
    F: FnOnce(NativeHookInput, &RuntimeContext) -> hookkit_core::Result<NativeHookOutput>,
{
    let bytes = match read_stdin() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("hookkit: failed to read stdin: {e}");
            return std::process::ExitCode::from(1);
        }
    };

    let (raw_input, input) = match parse_native(harness, &bytes) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("hookkit: failed to parse input: {e}");
            return std::process::ExitCode::from(1);
        }
    };

    let cwd = raw_input
        .get("cwd")
        .and_then(|v| v.as_str())
        .unwrap_or(".")
        .to_string();

    let ctx = RuntimeContext {
        harness,
        raw_input,
        stdin_bytes: bytes,
        cwd: hookkit_core::Utf8PathBuf::from(cwd),
    };

    maybe_dump_parsed(&input);
    let input_for_validation = input.clone();
    match handler(input, &ctx) {
        Ok(output) => {
            if let Err(e) = validate_native_output(&input_for_validation, &output) {
                eprintln!("hookkit: output validation failed: {e}");
                return std::process::ExitCode::from(1);
            }
            emit_output(output)
        }
        Err(e) => {
            eprintln!("hookkit: handler error: {e}");
            std::process::ExitCode::from(1)
        }
    }
}

/// Placeholder for the common (cross-harness) runtime.
///
/// Parses native input, converts to `CommonHookInput`, invokes the common
/// handler, converts `CommonHookOutput` back to the target native output,
/// validates emission rules, then writes stdout/stderr/exit code.
pub fn run_common<F>(harness: Harness, handler: F) -> std::process::ExitCode
where
    F: FnOnce(CommonHookInput, &RuntimeContext) -> hookkit_core::Result<CommonHookOutput>,
{
    let bytes = match read_stdin() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("hookkit: failed to read stdin: {e}");
            return std::process::ExitCode::from(1);
        }
    };

    let (raw_input, native_input) = match parse_native(harness, &bytes) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("hookkit: failed to parse input: {e}");
            return std::process::ExitCode::from(1);
        }
    };

    let native_for_validation = native_input.clone();
    maybe_dump_parsed(&native_input);

    let cwd = raw_input
        .get("cwd")
        .and_then(|v| v.as_str())
        .unwrap_or(".")
        .to_string();

    let ctx = RuntimeContext {
        harness,
        raw_input,
        stdin_bytes: bytes,
        cwd: hookkit_core::Utf8PathBuf::from(cwd),
    };

    let common_input = match native_to_common(native_input) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("hookkit: failed to convert to common input: {e}");
            return std::process::ExitCode::from(1);
        }
    };

    match handler(common_input, &ctx) {
        Ok(common_output) => {
            let lowered_output = match common_to_native(harness, common_output) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("hookkit: failed to convert common output: {e}");
                    return std::process::ExitCode::from(1);
                }
            };
            if let Err(e) = validate_native_output(&native_for_validation, &lowered_output.native) {
                eprintln!("hookkit: output validation failed: {e}");
                return std::process::ExitCode::from(1);
            }
            emit_lowered_output(lowered_output)
        }
        Err(e) => {
            eprintln!("hookkit: handler error: {e}");
            std::process::ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hookkit_common::output::{
        CommonPostToolUseOutput, CommonPromptSubmitOutput, CommonStopOutput,
    };

    fn load_fixture(harness: &str, name: &str) -> Vec<u8> {
        let path = format!(
            "{}/fixtures/{harness}/{name}",
            env!("CARGO_MANIFEST_DIR").replace("/crates/hookkit-runtime", "")
        );
        std::fs::read(path).expect("fixture should exist")
    }

    #[test]
    fn parse_claude_fixture() {
        let bytes = load_fixture("claude", "session_start.json");
        let (_, input) = parse_native(Harness::Claude, &bytes).expect("should parse");
        assert!(matches!(input, NativeHookInput::Claude(_)));
    }

    #[test]
    fn parse_codex_fixture() {
        let bytes = load_fixture("codex", "pre_tool_use.json");
        let (_, input) = parse_native(Harness::Codex, &bytes).expect("should parse");
        assert!(matches!(input, NativeHookInput::Codex(_)));
    }

    #[test]
    fn parse_gemini_fixture() {
        let bytes = load_fixture("gemini", "before_tool.json");
        let (_, input) = parse_native(Harness::Gemini, &bytes).expect("should parse");
        assert!(matches!(input, NativeHookInput::Gemini(_)));
    }

    #[test]
    fn parse_invalid_json() {
        let result = parse_native(Harness::Claude, b"not json");
        assert!(result.is_err());
    }

    #[test]
    fn parse_missing_event_name() {
        let result = parse_native(Harness::Claude, b"{}");
        assert!(result.is_err());
    }

    // Round-trip tests: parse fixture → re-serialize event → verify shape
    #[test]
    fn roundtrip_claude_session_start() {
        let bytes = load_fixture("claude", "session_start.json");
        let (_, input) = parse_native(Harness::Claude, &bytes).unwrap();
        if let NativeHookInput::Claude(ClaudeHookInput::SessionStart(ev)) = input {
            let json = serde_json::to_value(&ev).unwrap();
            assert_eq!(json["session_id"], "abc-123-def");
            assert_eq!(json["hook_event_name"], "SessionStart");
            assert_eq!(json["cwd"], "/home/user/project");
        } else {
            panic!("expected Claude SessionStart");
        }
    }

    #[test]
    fn roundtrip_codex_pre_tool_use() {
        let bytes = load_fixture("codex", "pre_tool_use.json");
        let (_, input) = parse_native(Harness::Codex, &bytes).unwrap();
        if let NativeHookInput::Codex(CodexHookInput::PreToolUse(ev)) = input {
            let json = serde_json::to_value(&ev).unwrap();
            assert_eq!(json["hookEventName"], "PreToolUse");
            assert_eq!(json["toolName"], "Bash");
        } else {
            panic!("expected Codex PreToolUse");
        }
    }

    #[test]
    fn roundtrip_gemini_before_tool() {
        let bytes = load_fixture("gemini", "before_tool.json");
        let (_, input) = parse_native(Harness::Gemini, &bytes).unwrap();
        if let NativeHookInput::Gemini(GeminiHookInput::BeforeTool(ev)) = input {
            let json = serde_json::to_value(&ev).unwrap();
            assert_eq!(json["hookEventName"], "BeforeTool");
            assert_eq!(json["toolName"], "shell");
        } else {
            panic!("expected Gemini BeforeTool");
        }
    }

    #[test]
    fn native_to_common_prompt_submit_is_supported_for_all_harnesses() {
        let claude = serde_json::from_slice::<serde_json::Value>(&load_fixture(
            "claude",
            "user_prompt_submit.json",
        ))
        .unwrap();
        let codex = serde_json::from_slice::<serde_json::Value>(&load_fixture(
            "codex",
            "user_prompt_submit.json",
        ))
        .unwrap();
        let gemini = serde_json::from_slice::<serde_json::Value>(&load_fixture(
            "gemini",
            "before_agent.json",
        ))
        .unwrap();

        let c = native_to_common(NativeHookInput::Claude(
            hookkit_claude::input::parse(&claude).unwrap(),
        ))
        .unwrap();
        let x = native_to_common(NativeHookInput::Codex(
            hookkit_codex::input::parse(&codex).unwrap(),
        ))
        .unwrap();
        let g = native_to_common(NativeHookInput::Gemini(
            hookkit_gemini::input::parse(&gemini).unwrap(),
        ))
        .unwrap();

        assert!(matches!(c, CommonHookInput::PromptSubmit(_)));
        assert!(matches!(x, CommonHookInput::PromptSubmit(_)));
        assert!(matches!(g, CommonHookInput::PromptSubmit(_)));
    }

    #[test]
    fn native_to_common_post_tool_is_supported_for_all_harnesses() {
        let claude = serde_json::from_slice::<serde_json::Value>(&load_fixture(
            "claude",
            "post_tool_use.json",
        ))
        .unwrap();
        let codex = serde_json::from_slice::<serde_json::Value>(&load_fixture(
            "codex",
            "post_tool_use.json",
        ))
        .unwrap();
        let gemini =
            serde_json::from_slice::<serde_json::Value>(&load_fixture("gemini", "after_tool.json"))
                .unwrap();

        let c = native_to_common(NativeHookInput::Claude(
            hookkit_claude::input::parse(&claude).unwrap(),
        ))
        .unwrap();
        let x = native_to_common(NativeHookInput::Codex(
            hookkit_codex::input::parse(&codex).unwrap(),
        ))
        .unwrap();
        let g = native_to_common(NativeHookInput::Gemini(
            hookkit_gemini::input::parse(&gemini).unwrap(),
        ))
        .unwrap();

        assert!(matches!(c, CommonHookInput::PostToolUse(_)));
        assert!(matches!(x, CommonHookInput::PostToolUse(_)));
        assert!(matches!(g, CommonHookInput::PostToolUse(_)));
    }

    #[test]
    fn common_output_conversion_prompt_submit_and_post_tool() {
        let prompt = CommonHookOutput::PromptSubmit(CommonPromptSubmitOutput::allow());
        let post = CommonHookOutput::PostToolUse(CommonPostToolUseOutput::new());

        let claude_prompt = common_to_native(Harness::Claude, prompt.clone()).unwrap();
        let codex_prompt = common_to_native(Harness::Codex, prompt.clone()).unwrap();
        let gemini_prompt = common_to_native(Harness::Gemini, prompt).unwrap();
        assert!(matches!(
            claude_prompt.native,
            NativeHookOutput::Claude(ClaudeHookOutput::Empty)
        ));
        assert!(matches!(
            codex_prompt.native,
            NativeHookOutput::Codex(CodexHookOutput::Empty)
        ));
        assert!(matches!(
            gemini_prompt.native,
            NativeHookOutput::Gemini(GeminiHookOutput::Empty)
        ));

        let claude_post = common_to_native(Harness::Claude, post.clone()).unwrap();
        let codex_post = common_to_native(Harness::Codex, post.clone()).unwrap();
        let gemini_post = common_to_native(Harness::Gemini, post).unwrap();
        assert!(matches!(
            claude_post.native,
            NativeHookOutput::Claude(ClaudeHookOutput::Empty)
        ));
        assert!(matches!(
            codex_post.native,
            NativeHookOutput::Codex(CodexHookOutput::Empty)
        ));
        assert!(matches!(
            gemini_post.native,
            NativeHookOutput::Gemini(GeminiHookOutput::Empty)
        ));
    }

    #[test]
    fn codex_stop_empty_stdout_fails_native_validation() {
        let bytes = load_fixture("codex", "stop.json");
        let (_, input) = parse_native(Harness::Codex, &bytes).unwrap();
        let output = NativeHookOutput::Codex(CodexHookOutput::Empty);
        assert!(validate_native_output(&input, &output).is_err());
    }

    #[test]
    fn codex_stop_allow_common_output_validates_as_json() {
        let bytes = load_fixture("codex", "stop.json");
        let (_, input) = parse_native(Harness::Codex, &bytes).unwrap();
        let output = common_to_native(
            Harness::Codex,
            CommonHookOutput::Stop(CommonStopOutput::allow_stop()),
        )
        .unwrap();

        assert!(matches!(
            output.native,
            NativeHookOutput::Codex(CodexHookOutput::Json(_))
        ));
        assert!(validate_native_output(&input, &output.native).is_ok());
    }

    #[test]
    fn runtime_context_key_helpers() {
        let raw = serde_json::json!({
            "sessionId": "sess-42",
            "turnId": "turn-7",
            "toolUseId": "tool-9",
            "hookEventName": "PostToolUse",
        });
        let ctx = RuntimeContext {
            harness: Harness::Claude,
            raw_input: raw,
            stdin_bytes: Vec::new(),
            cwd: hookkit_core::Utf8PathBuf::from("/tmp"),
        };
        assert_eq!(ctx.session_id(), Some("sess-42"));
        assert_eq!(ctx.turn_id(), Some("turn-7"));
        assert_eq!(ctx.tool_use_id(), Some("tool-9"));
        assert_eq!(ctx.hook_event_name(), Some("PostToolUse"));

        let key = ctx.artifact_key("diag");
        assert_eq!(key.session_id, "sess-42");
        assert_eq!(key.turn_id.as_deref(), Some("turn-7"));
        assert_eq!(key.tool_use_id.as_deref(), Some("tool-9"));
        assert_eq!(key.label, "diag");
    }

    #[test]
    fn format_dump_parsed_contains_harness_event_and_payload() {
        let bytes = load_fixture("codex", "pre_tool_use.json");
        let (_, input) = parse_native(Harness::Codex, &bytes).unwrap();
        let line = format_dump_parsed(&input);
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["hookkitDebug"], "dump-parsed");
        assert_eq!(v["harness"], "Codex");
        assert_eq!(v["event"], "PreToolUse");
        assert_eq!(v["parsed"]["hookEventName"], "PreToolUse");
    }
}
