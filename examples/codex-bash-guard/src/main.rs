use hookkit_codex::input::CodexToolInput;
use hookkit_codex::{CodexHookInput, CodexHookOutput};
use hookkit_core::Harness;
use hookkit_runtime::{NativeHookInput, NativeHookOutput, RuntimeContext};

/// Patterns that should be denied.
const DENY_PATTERNS: &[&str] = &[
    "rm -rf /",
    "rm -rf ~",
    "rm -rf $HOME",
    "git push --force",
    "git push -f",
    "dd if=",
    "mkfs.",
    "> /dev/sd",
    "chmod -R 777",
];

fn main() -> std::process::ExitCode {
    hookkit_runtime::run_native(Harness::Codex, handle)
}

fn handle(input: NativeHookInput, _ctx: &RuntimeContext) -> hookkit_core::Result<NativeHookOutput> {
    let NativeHookInput::Codex(codex_input) = input else {
        return Ok(NativeHookOutput::Codex(CodexHookOutput::Empty));
    };

    match codex_input {
        CodexHookInput::PreToolUse(ev) => {
            if let Some(CodexToolInput::Bash(bash)) = ev.typed_tool_input() {
                for pattern in DENY_PATTERNS {
                    if bash.command.contains(pattern) {
                        eprintln!("[codex-bash-guard] blocked: {}", bash.command);
                        return Ok(NativeHookOutput::Codex(CodexHookOutput::BlockingDeny {
                            stderr: format!("Denied: command matches blocked pattern '{pattern}'"),
                        }));
                    }
                }
            }
            Ok(NativeHookOutput::Codex(CodexHookOutput::Empty))
        }
        _ => Ok(NativeHookOutput::Codex(CodexHookOutput::Empty)),
    }
}
