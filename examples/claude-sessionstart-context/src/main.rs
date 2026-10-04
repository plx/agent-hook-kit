use hookkit_claude::protocol::{SessionStart, SessionStartOutput};

fn main() -> std::process::ExitCode {
    hookkit_runtime::typed::run_typed::<SessionStart, _>(|_input, environment, _ctx| {
        Ok(session_context(environment.project_dir.as_str()))
    })
}

/// Builds the session-start response.
///
/// The two Claude Code channels have different audiences, so each message
/// goes to the one that matches its reader:
///
/// - `additionalContext` is added to Claude's context. Workspace facts and
///   the instruction to run the tests belong here, phrased as information
///   Claude can act on.
/// - `systemMessage` is a warning shown only to the user; Claude never sees
///   it. It carries a short note about what the hook did.
fn session_context(project_dir: &str) -> SessionStartOutput {
    let context = format!(
        "Project directory: {project_dir}\n\
         This is a Rust workspace built with cargo.\n\
         Test command: cargo test --workspace\n\
         Lint command: cargo clippy --all-targets -- -D warnings\n\
         Run the test command after each change and before reporting work as done."
    );
    SessionStartOutput::with_context_and_system_message(
        context,
        "claude-sessionstart-context: added the Rust workspace conventions to Claude's context.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use hookkit_core::EventSpec;

    #[test]
    fn agent_instructions_go_to_additional_context_and_the_user_gets_a_note() {
        let emission = SessionStart::emit(session_context("/repo")).unwrap();
        let output: serde_json::Value = serde_json::from_slice(emission.stdout()).unwrap();
        let context = output["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(context.contains("Project directory: /repo"));
        assert!(context.contains("Run the test command after each change"));

        // Claude never reads systemMessage, so it must not carry instructions
        // meant for Claude.
        let notice = output["systemMessage"].as_str().unwrap();
        assert!(!notice.contains("Always run tests"));
        assert!(!notice.contains("cargo test"));
    }
}
