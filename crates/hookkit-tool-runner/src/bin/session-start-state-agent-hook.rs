fn main() -> std::process::ExitCode {
    match hookkit_tool_runner::parse_session_start_args() {
        Ok(cli) => hookkit_tool_runner::run_session_start_observer(cli),
        Err(error) => error.exit(),
    }
}
