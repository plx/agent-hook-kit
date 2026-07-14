use hookkit_claude::protocol::{SessionStart, SessionStartOutput};

fn main() -> std::process::ExitCode {
    hookkit_runtime::typed::run_typed::<SessionStart, _>(|_input, environment, _ctx| {
        let context = format!(
            "Project directory: {}\n\
             Build system: cargo\n\
             Test command: cargo test --workspace\n\
             Lint command: cargo clippy --all-targets -- -D warnings",
            environment.project_dir
        );

        Ok(SessionStartOutput::with_context_and_system_message(
            context,
            "You are working in a Rust workspace. Always run tests after changes.",
        ))
    })
}
