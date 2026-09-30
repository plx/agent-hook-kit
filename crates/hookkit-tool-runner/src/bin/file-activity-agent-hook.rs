fn main() -> std::process::ExitCode {
    match hookkit_tool_runner::parse_file_activity_args() {
        Ok(cli) => hookkit_tool_runner::run_file_activity_observer(cli),
        Err(error) => error.exit(),
    }
}
