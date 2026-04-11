//! I/O and execution plumbing for hookkit hook executables.

mod validate;

#[cfg(test)]
mod golden_tests;

use hookkit_claude::{ClaudeHookInput, ClaudeHookOutput};
use hookkit_codex::{CodexHookInput, CodexHookOutput};
use hookkit_core::Harness;
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
#[derive(Debug)]
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
            if let Err(e) = validate_claude(&envelope) {
                eprintln!("hookkit: output validation failed: {e}");
                return std::process::ExitCode::from(1);
            }
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
            if let Err(e) = validate_codex(&envelope) {
                eprintln!("hookkit: output validation failed: {e}");
                return std::process::ExitCode::from(1);
            }
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
            if let Err(e) = validate_gemini(&envelope) {
                eprintln!("hookkit: output validation failed: {e}");
                return std::process::ExitCode::from(1);
            }
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

    match handler(input, &ctx) {
        Ok(output) => emit_output(output),
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
