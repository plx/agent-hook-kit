fn main() -> std::process::ExitCode {
    let Ok(cli) = hookkit_tool_runner::parse_args() else {
        return std::process::ExitCode::from(1);
    };
    hookkit_tool_runner::run_runner(cli)
}
