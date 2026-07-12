use hookkit_codex::protocol::{PreToolUse, PreToolUseOutput};

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
    hookkit_runtime::typed::run_typed::<PreToolUse, _>(|input, _ctx| {
        let command = if input.tool_name == "Bash" {
            input
                .tool_input
                .get("command")
                .and_then(serde_json::Value::as_str)
        } else {
            None
        };
        if let Some(command) = command {
            for pattern in DENY_PATTERNS {
                if command.contains(pattern) {
                    return Ok(PreToolUseOutput::deny(Some(format!(
                        "Denied: command matches blocked pattern '{pattern}'"
                    ))));
                }
            }
        }
        Ok(PreToolUseOutput::no_op())
    })
}
