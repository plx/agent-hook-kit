fn main() -> std::process::ExitCode {
    match hookkit_tool_runner::parse_args() {
        Ok(cli) => hookkit_tool_runner::run_runner(cli),
        Err(error) => error.exit(),
    }
}
