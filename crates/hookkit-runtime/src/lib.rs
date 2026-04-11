//! I/O and execution plumbing for hookkit hook executables.

use hookkit_claude::{ClaudeHookInput, ClaudeHookOutput};
use hookkit_codex::{CodexHookInput, CodexHookOutput};
use hookkit_core::Harness;
use hookkit_gemini::{GeminiHookInput, GeminiHookOutput};
use std::io::Read;

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

/// Parse stdin JSON as a native hook input for the given harness.
pub fn parse_stdin_native(
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

/// Emit the output for a native hook result.
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
/// and emits the correct stdout/stderr/exit code.
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

    let (raw_input, input) = match parse_stdin_native(harness, &bytes) {
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
