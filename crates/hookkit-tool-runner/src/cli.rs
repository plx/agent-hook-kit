//! Command-line parsing for the four bundled hook binaries.
//!
//! Every parser accepts exactly one harness, either as `--claude`, `--codex`,
//! or `--antigravity`, or through the compatibility alias
//! `--harness=claude|codex|antigravity` (`claude-code` is accepted as an alias
//! for `claude`). Parse failures never exit with status 2, because Claude Code
//! and Codex treat exit code 2 as a deliberate blocking hook result; help and
//! version output exit 0 and usage errors exit 1 with a diagnostic on stderr.

use clap::{Args, Parser, ValueEnum};
use hookkit_core::HarnessId;
use std::ffi::OsString;
use std::path::PathBuf;

const POST_TOOL_USE_BINARY_NAME: &str = "post-tool-use-agent-hook";
const TURN_COMPLETION_BINARY_NAME: &str = "turn-completion-agent-hook";
const SESSION_START_BINARY_NAME: &str = "session-start-state-agent-hook";
const FILE_ACTIVITY_BINARY_NAME: &str = "file-activity-agent-hook";

/// CLI options parsed from process args.
#[derive(Debug, Clone)]
pub struct Cli {
    /// Harness whose native post-tool event is read from standard input.
    pub harness: HarnessId,
    /// Explicit Pkl file, or `None` to use layered discovery.
    pub config_path: Option<PathBuf>,
}

/// CLI options for the stop-time batch runner.
#[derive(Debug, Clone)]
pub struct TurnCompletionCli {
    /// Harness whose native turn-completion event is read from standard input.
    pub harness: HarnessId,
    /// Explicit Pkl file, or `None` to use layered discovery.
    pub config_path: Option<PathBuf>,
    /// Session-state directory override. A relative path is resolved against
    /// the harness project root (`CLAUDE_PROJECT_DIR` for Claude Code), not
    /// the hook process's current directory.
    pub state_dir: Option<PathBuf>,
}

/// CLI options for the library-owned precise session-start observer.
#[derive(Debug, Clone)]
pub struct SessionStartCli {
    /// Harness whose native session-start event is read from standard input.
    pub harness: HarnessId,
    /// Session-state directory override, resolved like
    /// [`TurnCompletionCli::state_dir`].
    pub state_dir: Option<PathBuf>,
}

/// CLI options for the quiet post-tool file-activity observer.
#[derive(Debug, Clone)]
pub struct FileActivityCli {
    /// Harness whose native post-tool event (on Claude Code, `PostToolUse` or
    /// `PostToolUseFailure`) is read from standard input.
    pub harness: HarnessId,
    /// Session-state directory override, resolved like
    /// [`TurnCompletionCli::state_dir`].
    pub state_dir: Option<PathBuf>,
}

/// A command line that did not produce runnable options.
///
/// Usage errors are rendered with the name of the binary that was invoked, so
/// a misconfigured hook command is identifiable from the harness's error
/// output; [`CliError::exit`] prints the text and chooses a harness-safe exit
/// code.
#[derive(Debug)]
pub struct CliError {
    binary: &'static str,
    error: clap::Error,
}

impl CliError {
    fn new(binary: &'static str, error: clap::Error) -> Self {
        Self { binary, error }
    }

    /// Whether this is requested help or version output rather than a failure.
    pub fn is_informational(&self) -> bool {
        matches!(
            self.error.kind(),
            clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
        )
    }

    /// Print the diagnostic (help and version to stdout, errors to stderr)
    /// and return exit code 0 for informational output or 1 for usage errors.
    pub fn exit(self) -> std::process::ExitCode {
        if self.is_informational() {
            let _ = self.error.print();
            return std::process::ExitCode::SUCCESS;
        }
        eprint!("{self}");
        std::process::ExitCode::from(1)
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_informational() {
            return self.error.fmt(f);
        }
        write!(f, "{}: {}", self.binary, self.error)
    }
}

impl std::error::Error for CliError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum HarnessArg {
    #[value(alias = "claude-code")]
    Claude,
    Codex,
    Antigravity,
}

impl HarnessArg {
    fn id(self) -> HarnessId {
        match self {
            Self::Claude => HarnessId::CLAUDE_CODE,
            Self::Codex => HarnessId::CODEX,
            Self::Antigravity => HarnessId::ANTIGRAVITY,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum SessionStartHarnessArg {
    #[value(alias = "claude-code")]
    Claude,
    Codex,
}

#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
struct HarnessSelection {
    /// Read a native Claude Code hook event.
    #[arg(long)]
    claude: bool,
    /// Read a native Codex hook event.
    #[arg(long)]
    codex: bool,
    /// Read a native Antigravity hook event.
    #[arg(long)]
    antigravity: bool,
    /// Select the harness by name.
    #[arg(long, value_enum, value_name = "HARNESS")]
    harness: Option<HarnessArg>,
}

impl HarnessSelection {
    fn id(&self) -> HarnessId {
        if self.claude {
            HarnessId::CLAUDE_CODE
        } else if self.codex {
            HarnessId::CODEX
        } else if self.antigravity {
            HarnessId::ANTIGRAVITY
        } else {
            self.harness
                .expect("clap requires exactly one harness selection")
                .id()
        }
    }
}

#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
struct SessionStartHarnessSelection {
    /// Read a native Claude Code SessionStart event.
    #[arg(long)]
    claude: bool,
    /// Read a native Codex SessionStart event.
    #[arg(long)]
    codex: bool,
    /// Select the harness by name. Antigravity has no SessionStart event.
    #[arg(long, value_enum, value_name = "HARNESS")]
    harness: Option<SessionStartHarnessArg>,
}

impl SessionStartHarnessSelection {
    fn id(&self) -> HarnessId {
        if self.claude {
            HarnessId::CLAUDE_CODE
        } else if self.codex {
            HarnessId::CODEX
        } else {
            match self
                .harness
                .expect("clap requires exactly one harness selection")
            {
                SessionStartHarnessArg::Claude => HarnessId::CLAUDE_CODE,
                SessionStartHarnessArg::Codex => HarnessId::CODEX,
            }
        }
    }
}

/// Run configured formatters and linters on files changed by one tool call.
#[derive(Debug, Parser)]
#[command(name = POST_TOOL_USE_BINARY_NAME, version)]
struct PostToolUseArgs {
    #[command(flatten)]
    harness: HarnessSelection,
    /// Load only this Pkl file instead of layered discovery.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
}

/// Record file activity from one tool call without producing output.
#[derive(Debug, Parser)]
#[command(name = FILE_ACTIVITY_BINARY_NAME, version)]
struct FileActivityArgs {
    #[command(flatten)]
    harness: HarnessSelection,
    /// Session-state root; relative paths resolve against the project root.
    #[arg(long, value_name = "PATH")]
    state_dir: Option<PathBuf>,
}

/// Check files touched during the turn before the agent stops.
#[derive(Debug, Parser)]
#[command(name = TURN_COMPLETION_BINARY_NAME, version)]
struct TurnCompletionArgs {
    #[command(flatten)]
    harness: HarnessSelection,
    /// Load only this Pkl file instead of layered discovery.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
    /// Session-state root; relative paths resolve against the project root.
    #[arg(long, value_name = "PATH")]
    state_dir: Option<PathBuf>,
}

/// Record precise session-start metadata for the stateful hooks.
#[derive(Debug, Parser)]
#[command(name = SESSION_START_BINARY_NAME, version)]
struct SessionStartArgs {
    #[command(flatten)]
    harness: SessionStartHarnessSelection,
    /// Session-state root; relative paths resolve against the project root.
    #[arg(long, value_name = "PATH")]
    state_dir: Option<PathBuf>,
}

/// Parse `post-tool-use-agent-hook` options from the process arguments.
pub fn parse_args() -> Result<Cli, CliError> {
    parse_args_from(std::env::args_os())
}

/// Parse `post-tool-use-agent-hook` options; the first item is the binary name.
pub fn parse_args_from<I, T>(args: I) -> Result<Cli, CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = PostToolUseArgs::try_parse_from(args)
        .map_err(|error| CliError::new(POST_TOOL_USE_BINARY_NAME, error))?;
    Ok(Cli {
        harness: args.harness.id(),
        config_path: args.config,
    })
}

/// Parse `file-activity-agent-hook` options from the process arguments.
pub fn parse_file_activity_args() -> Result<FileActivityCli, CliError> {
    parse_file_activity_args_from(std::env::args_os())
}

/// Parse `file-activity-agent-hook` options; the first item is the binary name.
pub fn parse_file_activity_args_from<I, T>(args: I) -> Result<FileActivityCli, CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = FileActivityArgs::try_parse_from(args)
        .map_err(|error| CliError::new(FILE_ACTIVITY_BINARY_NAME, error))?;
    Ok(FileActivityCli {
        harness: args.harness.id(),
        state_dir: args.state_dir,
    })
}

/// Parse `turn-completion-agent-hook` options from the process arguments.
pub fn parse_turn_completion_args() -> Result<TurnCompletionCli, CliError> {
    parse_turn_completion_args_from(std::env::args_os())
}

/// Parse `turn-completion-agent-hook` options; the first item is the binary name.
pub fn parse_turn_completion_args_from<I, T>(args: I) -> Result<TurnCompletionCli, CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = TurnCompletionArgs::try_parse_from(args)
        .map_err(|error| CliError::new(TURN_COMPLETION_BINARY_NAME, error))?;
    Ok(TurnCompletionCli {
        harness: args.harness.id(),
        config_path: args.config,
        state_dir: args.state_dir,
    })
}

/// Parse `session-start-state-agent-hook` options from the process arguments.
pub fn parse_session_start_args() -> Result<SessionStartCli, CliError> {
    parse_session_start_args_from(std::env::args_os())
}

/// Parse `session-start-state-agent-hook` options; the first item is the
/// binary name. Only Claude Code and Codex have a native SessionStart event.
pub fn parse_session_start_args_from<I, T>(args: I) -> Result<SessionStartCli, CliError>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = SessionStartArgs::try_parse_from(args)
        .map_err(|error| CliError::new(SESSION_START_BINARY_NAME, error))?;
    Ok(SessionStartCli {
        harness: args.harness.id(),
        state_dir: args.state_dir,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory as _;

    #[test]
    fn clap_definitions_are_internally_consistent() {
        PostToolUseArgs::command().debug_assert();
        FileActivityArgs::command().debug_assert();
        TurnCompletionArgs::command().debug_assert();
        SessionStartArgs::command().debug_assert();
    }

    #[test]
    fn every_harness_spelling_selects_exactly_one_harness() {
        for (arguments, expected) in [
            (vec!["--claude"], HarnessId::CLAUDE_CODE),
            (vec!["--codex"], HarnessId::CODEX),
            (vec!["--antigravity"], HarnessId::ANTIGRAVITY),
            (vec!["--harness=claude"], HarnessId::CLAUDE_CODE),
            (vec!["--harness=claude-code"], HarnessId::CLAUDE_CODE),
            (vec!["--harness", "codex"], HarnessId::CODEX),
            (vec!["--harness=antigravity"], HarnessId::ANTIGRAVITY),
        ] {
            let parsed = parse_turn_completion_args_from(
                std::iter::once(TURN_COMPLETION_BINARY_NAME).chain(arguments.iter().copied()),
            )
            .unwrap();
            assert_eq!(parsed.harness, expected, "{arguments:?}");
        }
    }

    #[test]
    fn options_are_parsed_in_both_spellings() {
        let parsed = parse_turn_completion_args_from([
            TURN_COMPLETION_BINARY_NAME,
            "--codex",
            "--config=custom.pkl",
            "--state-dir",
            ".context/state",
        ])
        .unwrap();
        assert_eq!(parsed.config_path, Some(PathBuf::from("custom.pkl")));
        assert_eq!(parsed.state_dir, Some(PathBuf::from(".context/state")));

        let parsed =
            parse_args_from([POST_TOOL_USE_BINARY_NAME, "--claude", "--config", "x.pkl"]).unwrap();
        assert_eq!(parsed.config_path, Some(PathBuf::from("x.pkl")));
    }

    #[test]
    fn misconfigurations_fail_with_a_diagnostic_naming_the_right_binary() {
        let typo =
            parse_turn_completion_args_from([TURN_COMPLETION_BINARY_NAME, "--harness=claud"])
                .unwrap_err();
        assert!(!typo.is_informational());
        let rendered = typo.to_string();
        assert!(rendered.contains("claud"), "{rendered}");
        assert!(rendered.contains(TURN_COMPLETION_BINARY_NAME), "{rendered}");

        let conflict =
            parse_turn_completion_args_from([TURN_COMPLETION_BINARY_NAME, "--claude", "--codex"])
                .unwrap_err()
                .to_string();
        assert!(conflict.contains(TURN_COMPLETION_BINARY_NAME), "{conflict}");
        assert!(!conflict.contains(POST_TOOL_USE_BINARY_NAME), "{conflict}");

        let missing = parse_file_activity_args_from([FILE_ACTIVITY_BINARY_NAME]).unwrap_err();
        assert!(!missing.is_informational());
    }

    #[test]
    fn session_start_rejects_antigravity_at_parse_time() {
        for arguments in [
            vec![SESSION_START_BINARY_NAME, "--harness=antigravity"],
            vec![SESSION_START_BINARY_NAME, "--antigravity"],
        ] {
            let error = parse_session_start_args_from(arguments.clone()).unwrap_err();
            assert!(!error.is_informational(), "{arguments:?}");
            assert!(
                error.to_string().contains("antigravity"),
                "{arguments:?}: {error}"
            );
        }
        let parsed =
            parse_session_start_args_from([SESSION_START_BINARY_NAME, "--harness=codex"]).unwrap();
        assert_eq!(parsed.harness, HarnessId::CODEX);
    }

    #[test]
    fn help_is_informational_and_names_the_binary() {
        let help =
            parse_session_start_args_from([SESSION_START_BINARY_NAME, "--help"]).unwrap_err();
        assert!(help.is_informational());
        assert!(help.to_string().contains(SESSION_START_BINARY_NAME));
    }
}
