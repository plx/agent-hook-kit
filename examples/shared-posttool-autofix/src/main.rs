use hookkit_claude::{ClaudeHookInput, ClaudeHookOutput};
use hookkit_codex::{CodexHookInput, CodexHookOutput};
use hookkit_common::input::{CommonHookInput, CommonPostToolUseInput};
use hookkit_common::output::CommonPostToolUseOutput;
use hookkit_core::Harness;
use hookkit_gemini::{GeminiHookInput, GeminiHookOutput};
use hookkit_runtime::{NativeHookInput, NativeHookOutput, RuntimeContext};

fn main() -> std::process::ExitCode {
    // Determine harness from CLI args or environment
    let harness = match std::env::args().nth(1).as_deref() {
        Some("--claude") => Harness::Claude,
        Some("--codex") => Harness::Codex,
        Some("--gemini") => Harness::Gemini,
        _ => {
            eprintln!("Usage: shared-posttool-autofix --claude|--codex|--gemini");
            return std::process::ExitCode::from(1);
        }
    };

    hookkit_runtime::run_native(harness, |input, ctx| handle(input, ctx, harness))
}

fn handle(
    input: NativeHookInput,
    _ctx: &RuntimeContext,
    harness: Harness,
) -> hookkit_core::Result<NativeHookOutput> {
    // Convert to common input if it's a post-tool event
    let common_input = match &input {
        NativeHookInput::Claude(ClaudeHookInput::PostToolUse(ev)) => Some(
            CommonHookInput::PostToolUse(CommonPostToolUseInput::Claude(ev.clone())),
        ),
        NativeHookInput::Codex(CodexHookInput::PostToolUse(ev)) => Some(
            CommonHookInput::PostToolUse(CommonPostToolUseInput::Codex(ev.clone())),
        ),
        NativeHookInput::Gemini(GeminiHookInput::AfterTool(ev)) => Some(
            CommonHookInput::PostToolUse(CommonPostToolUseInput::Gemini(ev.clone())),
        ),
        _ => None,
    };

    let Some(CommonHookInput::PostToolUse(post_tool)) = common_input else {
        return match harness {
            Harness::Claude => Ok(NativeHookOutput::Claude(ClaudeHookOutput::Empty)),
            Harness::Codex => Ok(NativeHookOutput::Codex(CodexHookOutput::Empty)),
            Harness::Gemini => Ok(NativeHookOutput::Gemini(GeminiHookOutput::Empty)),
        };
    };

    // Shared business logic: check if a file-modifying tool was used
    let tool_name = post_tool.tool_name().unwrap_or("");
    let is_file_tool = matches!(tool_name, "Write" | "Edit" | "shell");

    if !is_file_tool {
        return match harness {
            Harness::Claude => Ok(NativeHookOutput::Claude(ClaudeHookOutput::Empty)),
            Harness::Codex => Ok(NativeHookOutput::Codex(CodexHookOutput::Empty)),
            Harness::Gemini => Ok(NativeHookOutput::Gemini(GeminiHookOutput::Empty)),
        };
    }

    // Build a common output with context for the agent
    let output = CommonPostToolUseOutput::new()
        .with_agent_context("File was modified. Consider running formatter and linter.");

    // Convert to the target harness output
    match harness {
        Harness::Claude => Ok(NativeHookOutput::Claude(output.to_claude()?)),
        Harness::Codex => {
            // Codex doesn't support context injection — stay quiet
            Ok(NativeHookOutput::Codex(CodexHookOutput::Empty))
        }
        Harness::Gemini => Ok(NativeHookOutput::Gemini(output.to_gemini()?)),
    }
}
