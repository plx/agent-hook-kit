use hookkit_core::Harness;
use hookkit_gemini::input::GeminiToolInput;
use hookkit_gemini::output::OutputEnvelope;
use hookkit_gemini::{GeminiHookInput, GeminiHookOutput};
use hookkit_runtime::{NativeHookInput, NativeHookOutput, RuntimeContext};

fn main() -> std::process::ExitCode {
    hookkit_runtime::run_native(Harness::Gemini, handle)
}

fn handle(input: NativeHookInput, _ctx: &RuntimeContext) -> hookkit_core::Result<NativeHookOutput> {
    let NativeHookInput::Gemini(gemini_input) = input else {
        return Ok(NativeHookOutput::Gemini(GeminiHookOutput::Empty));
    };

    match gemini_input {
        GeminiHookInput::BeforeTool(ev) => {
            if let Some(GeminiToolInput::Shell(shell)) = ev.typed_tool_input() {
                let joined = shell.command.join(" ");

                // Deny outright dangerous commands
                if joined.contains("rm -rf /") || joined.contains("mkfs.") {
                    eprintln!("[gemini-policy] denied: {joined}");
                    return Ok(NativeHookOutput::Gemini(GeminiHookOutput::Json(
                        OutputEnvelope::deny(format!("Blocked dangerous command: {joined}")),
                    )));
                }

                // Rewrite: replace `curl | sh` with a safe alternative
                if joined.contains("curl") && joined.contains("| sh") {
                    eprintln!("[gemini-policy] rewriting curl-pipe-sh to safe download");
                    let safe_cmd = vec!["echo".to_string(), "curl-pipe-sh blocked".to_string()];
                    return Ok(NativeHookOutput::Gemini(GeminiHookOutput::Json(
                        OutputEnvelope::rewrite_tool_input(
                            serde_json::json!({"command": safe_cmd}),
                        ),
                    )));
                }
            }

            Ok(NativeHookOutput::Gemini(GeminiHookOutput::Empty))
        }
        _ => Ok(NativeHookOutput::Gemini(GeminiHookOutput::Empty)),
    }
}
