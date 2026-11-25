use hookkit_antigravity::{InjectStep, PreInvocation, PreInvocationOutput};
use hookkit_core::Utf8PathBuf;

fn main() -> std::process::ExitCode {
    hookkit_runtime::typed::run_typed::<PreInvocation, _>(|input, _environment, _context| {
        Ok(reminder(&input.workspace_paths))
    })
}

/// Reminds the agent to check every root of a multi-root workspace.
///
/// The reminder is a `systemMessage` step, which the Antigravity reference
/// prefers to the deprecated `ephemeralMessage` step.
fn reminder(workspace_paths: &[Utf8PathBuf]) -> PreInvocationOutput {
    if workspace_paths.len() > 1 {
        PreInvocationOutput::inject(InjectStep::system_message(
            "Remember to check every workspace root.",
        ))
    } else {
        PreInvocationOutput::no_op()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hookkit_core::EventSpec;

    fn emitted(workspace_paths: &[&str]) -> String {
        let paths: Vec<Utf8PathBuf> = workspace_paths.iter().map(Utf8PathBuf::from).collect();
        let emission = PreInvocation::emit(reminder(&paths)).unwrap();
        String::from_utf8(emission.stdout().to_vec()).unwrap()
    }

    #[test]
    fn multi_root_workspaces_get_a_system_message_step() {
        let stdout = emitted(&["/repo/app", "/repo/lib"]);
        assert!(stdout.contains(r#""systemMessage""#), "{stdout}");
        assert!(
            stdout.contains("Remember to check every workspace root."),
            "{stdout}"
        );
        assert!(!stdout.contains("ephemeralMessage"), "{stdout}");

        let single = emitted(&["/repo"]);
        assert!(!single.contains("systemMessage"), "{single}");
    }
}
