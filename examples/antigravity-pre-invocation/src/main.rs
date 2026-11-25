use hookkit_antigravity::{InjectStep, PreInvocation, PreInvocationOutput};

fn main() -> std::process::ExitCode {
    hookkit_runtime::typed::run_typed::<PreInvocation, _>(|input, _environment, _context| {
        let output = if input.workspace_paths.len() > 1 {
            PreInvocationOutput::inject(InjectStep::system_message(
                "Remember to check every workspace root.",
            ))
        } else {
            PreInvocationOutput::no_op()
        };
        Ok(output)
    })
}
