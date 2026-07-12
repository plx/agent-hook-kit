use hookkit_gemini::protocol::{BeforeTool, BeforeToolOutput};

fn main() -> std::process::ExitCode {
    hookkit_runtime::typed::run_typed::<BeforeTool, _>(|input, _ctx| {
        if input.tool_name != "run_shell_command" {
            return Ok(BeforeToolOutput::no_op());
        }

        let Some(command) = input
            .tool_input
            .get("command")
            .and_then(serde_json::Value::as_str)
        else {
            return Ok(BeforeToolOutput::no_op());
        };

        if command.contains("rm -rf /") || command.contains("mkfs.") {
            return Ok(BeforeToolOutput::deny(format!(
                "Blocked dangerous command: {command}"
            )));
        }

        if command.contains("curl") && command.contains("| sh") {
            let safe_input = serde_json::Map::from_iter([(
                "command".to_string(),
                serde_json::Value::String("echo 'curl-pipe-sh blocked'".to_string()),
            )]);
            return Ok(BeforeToolOutput::rewrite_tool_input(safe_input));
        }

        Ok(BeforeToolOutput::no_op())
    })
}
