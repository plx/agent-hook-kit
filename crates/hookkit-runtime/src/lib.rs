//! I/O and execution plumbing for hookkit hook executables.

pub mod artifacts;
pub mod logging;
mod validate;

#[cfg(test)]
mod golden_tests;

use hookkit_claude::{ClaudeHookInput, ClaudeHookOutput};
use hookkit_codex::{CodexHookInput, CodexHookOutput};
use hookkit_core::{Harness, HookEventKey, HookkitError};
use hookkit_gemini::{GeminiHookInput, GeminiHookOutput};
use std::io::Read;

pub use validate::{validate_claude, validate_codex, validate_gemini};

/// Runtime context available to hook handlers.
pub struct RuntimeContext {
    pub harness: Harness,
    pub raw_input: serde_json::Value,
    pub stdin_bytes: Vec<u8>,
    pub cwd: String,
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

fn hook_event_key(input: &NativeHookInput) -> HookEventKey {
    match input {
        NativeHookInput::Claude(ev) => match ev {
            ClaudeHookInput::SessionStart(_) => HookEventKey::SessionStart,
            ClaudeHookInput::UserPromptSubmit(_) => HookEventKey::PromptSubmit,
            ClaudeHookInput::PreToolUse(_) => HookEventKey::PreToolUse,
            ClaudeHookInput::PostToolUse(_) => HookEventKey::PostToolUse,
            ClaudeHookInput::PostToolUseFailure(_) => HookEventKey::PostToolUseFailure,
            ClaudeHookInput::PermissionDenied(_) => HookEventKey::PermissionDenied,
            ClaudeHookInput::Stop(_) => HookEventKey::Stop,
            ClaudeHookInput::Notification(_) => HookEventKey::Notification,
            ClaudeHookInput::SessionEnd(_) => HookEventKey::SessionEnd,
            ClaudeHookInput::PermissionRequest(_) => HookEventKey::PermissionRequest,
            ClaudeHookInput::PreCompact(_) => HookEventKey::PreCompress,
            ClaudeHookInput::Unknown { event_name, .. } => HookEventKey::Other(event_name.clone()),
            ClaudeHookInput::SubagentStart(_) => HookEventKey::Other("SubagentStart".to_string()),
            ClaudeHookInput::SubagentStop(_) => HookEventKey::Other("SubagentStop".to_string()),
            ClaudeHookInput::TaskCreated(_) => HookEventKey::Other("TaskCreated".to_string()),
            ClaudeHookInput::TaskCompleted(_) => HookEventKey::Other("TaskCompleted".to_string()),
            ClaudeHookInput::TeammateIdle(_) => HookEventKey::Other("TeammateIdle".to_string()),
            ClaudeHookInput::ConfigChange(_) => HookEventKey::Other("ConfigChange".to_string()),
            ClaudeHookInput::CwdChanged(_) => HookEventKey::Other("CwdChanged".to_string()),
            ClaudeHookInput::FileChanged(_) => HookEventKey::Other("FileChanged".to_string()),
            ClaudeHookInput::PostCompact(_) => HookEventKey::Other("PostCompact".to_string()),
            ClaudeHookInput::InstructionsLoaded(_) => {
                HookEventKey::Other("InstructionsLoaded".to_string())
            }
            ClaudeHookInput::WorktreeCreate(_) => HookEventKey::Other("WorktreeCreate".to_string()),
            ClaudeHookInput::WorktreeRemove(_) => HookEventKey::Other("WorktreeRemove".to_string()),
            ClaudeHookInput::Elicitation(_) => HookEventKey::Other("Elicitation".to_string()),
            ClaudeHookInput::ElicitationResult(_) => {
                HookEventKey::Other("ElicitationResult".to_string())
            }
            ClaudeHookInput::StopFailure(_) => HookEventKey::Other("StopFailure".to_string()),
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

fn validate_native_output(input: &NativeHookInput, output: &NativeHookOutput) -> hookkit_core::Result<()> {
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
        cwd,
    };

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
/// Will be fully implemented in Phase 2 when hookkit-common is populated.
/// For now, this delegates through the native path.
pub fn run_common<F>(harness: Harness, handler: F) -> std::process::ExitCode
where
    F: FnOnce(NativeHookInput, &RuntimeContext) -> hookkit_core::Result<NativeHookOutput>,
{
    run_native(harness, handler)
}

#[cfg(test)]
mod tests {
    use super::*;

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
            assert_eq!(json["sessionId"], "abc-123-def");
            assert_eq!(json["hookEventName"], "SessionStart");
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
}
