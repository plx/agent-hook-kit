fn main() -> std::process::ExitCode {
    match hookkit_tool_runner::parse_turn_completion_args() {
        Ok(cli) => hookkit_tool_runner::run_turn_completion_runner(cli),
        Err(error) => error.exit(),
    }
}
