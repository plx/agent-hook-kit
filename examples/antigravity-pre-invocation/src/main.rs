use hookkit_antigravity::{InjectStep, PreInvocation, PreInvocationOutput};
use hookkit_core::Utf8PathBuf;

fn main() -> std::process::ExitCode {
    hookkit_runtime::typed::run_typed::<PreInvocation, _>(|input, _environment, _context| {
        Ok(reminder(input.invocation_num, &input.workspace_paths))
    })
}

/// Reminds the agent, once, to check every root of a multi-root workspace.
///
/// The reminder is a `systemMessage` step, which the Antigravity reference
/// prefers to the deprecated `ephemeralMessage` step. Unlike the transient
/// `ephemeralMessage`, an injected `systemMessage` stays in the conversation
/// history for later turns, and `PreInvocation` fires before every model
/// call. The reference therefore advises a hook that runs on every
/// invocation to inject a message only when its content changes: repeating
/// the same text on each call adds up and fills the context window. This
/// reminder never changes, so it is injected only on the first invocation
/// (`invocationNum` 0) and every later invocation is a no-op. Whether
/// `invocationNum` restarts at 0 for each user turn is undocumented, so this
/// injects at most once per conversation, or at worst once per turn.
///
/// On builds that send `SessionStart`, which fires once before the first
/// model invocation, the same reminder fits better in a `SessionStart` hook
/// (`SessionStartOutput::inject(InjectStep::system_message(..))`); this
/// example stays a `PreInvocation` demo, which older builds also run.
fn reminder(invocation_num: u64, workspace_paths: &[Utf8PathBuf]) -> PreInvocationOutput {
    if invocation_num == 0 && workspace_paths.len() > 1 {
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

    fn emitted(invocation_num: u64, workspace_paths: &[&str]) -> String {
        let paths: Vec<Utf8PathBuf> = workspace_paths.iter().map(Utf8PathBuf::from).collect();
        let emission = PreInvocation::emit(reminder(invocation_num, &paths)).unwrap();
        String::from_utf8(emission.stdout().to_vec()).unwrap()
    }

    #[test]
    fn multi_root_workspaces_get_a_system_message_step_on_the_first_invocation() {
        let stdout = emitted(0, &["/repo/app", "/repo/lib"]);
        assert!(stdout.contains(r#""systemMessage""#), "{stdout}");
        assert!(
            stdout.contains("Remember to check every workspace root."),
            "{stdout}"
        );
        assert!(!stdout.contains("ephemeralMessage"), "{stdout}");

        let single = emitted(0, &["/repo"]);
        assert_eq!(single, "{}", "a single root needs no reminder");
    }

    /// The injected step stays in the conversation, so later invocations
    /// must not repeat it.
    #[test]
    fn later_invocations_do_not_repeat_the_reminder() {
        for invocation_num in [1, 3, 57] {
            assert_eq!(
                emitted(invocation_num, &["/repo/app", "/repo/lib"]),
                "{}",
                "invocation {invocation_num}"
            );
        }
    }
}
