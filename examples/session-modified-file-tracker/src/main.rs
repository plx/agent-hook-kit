fn main() -> std::process::ExitCode {
    // A usage error prints its diagnostic and exits 1, which Claude Code and
    // Codex treat as a non-blocking hook error; `--help` prints and exits 0.
    match hookkit_tool_runner::parse_file_activity_args() {
        Ok(cli) => hookkit_tool_runner::run_file_activity_observer(cli),
        Err(error) => error.exit(),
    }
}
