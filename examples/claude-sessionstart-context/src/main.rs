use hookkit_claude::output::OutputEnvelope;
use hookkit_claude::{ClaudeHookInput, ClaudeHookOutput};
use hookkit_core::Harness;
use hookkit_runtime::{NativeHookInput, NativeHookOutput, RuntimeContext};

fn main() -> std::process::ExitCode {
    hookkit_runtime::run_native(Harness::Claude, handle)
}

fn handle(input: NativeHookInput, ctx: &RuntimeContext) -> hookkit_core::Result<NativeHookOutput> {
    let NativeHookInput::Claude(claude_input) = input else {
        return Ok(NativeHookOutput::Claude(ClaudeHookOutput::Empty));
    };

    match claude_input {
        ClaudeHookInput::SessionStart(_ev) => {
            // Inject project-specific context at session start
            let context = format!(
                "Project directory: {}\n\
                 Build system: cargo\n\
                 Test command: cargo test --workspace\n\
                 Lint command: cargo clippy --all-targets -- -D warnings",
                ctx.cwd
            );

            Ok(NativeHookOutput::Claude(ClaudeHookOutput::Json(
                OutputEnvelope::with_context(context).with_system_message(
                    "You are working in a Rust workspace. Always run tests after changes.",
                ),
            )))
        }
        _ => Ok(NativeHookOutput::Claude(ClaudeHookOutput::Empty)),
    }
}
