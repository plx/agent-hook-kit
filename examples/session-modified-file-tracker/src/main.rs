use clap::{Parser, ValueEnum};
use hookkit_common::{PostToolUseInput, PostToolUseOutput};
use hookkit_core::{HarnessId, RuntimeContext};
use hookkit_file_activity::{FileActivityStore, observe_post_tool};
use hookkit_session_state::StateRoot;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Harness {
    Claude,
    Codex,
    Gemini,
}

impl Harness {
    fn id(self) -> HarnessId {
        match self {
            Self::Claude => HarnessId::CLAUDE_CODE,
            Self::Codex => HarnessId::CODEX,
            Self::Gemini => HarnessId::GEMINI_CLI,
        }
    }
}

#[derive(Debug, Parser)]
#[command(about = "Accumulate best-effort file activity during one agent session")]
struct Cli {
    /// Hook harness whose native post-tool contract should be used.
    #[arg(long, value_enum)]
    harness: Harness,

    /// Override the root directory for per-session tracker state.
    #[arg(long)]
    state_dir: Option<PathBuf>,
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    hookkit_runtime::aligned::run_aligned_event::<hookkit_runtime::aligned::PostToolUse, _>(
        cli.harness.id(),
        move |input, _environment, context| {
            handle_post_tool(input, context, cli.state_dir.as_deref())
        },
    )
}

fn handle_post_tool(
    input: PostToolUseInput,
    context: &RuntimeContext<'_>,
    state_dir: Option<&Path>,
) -> hookkit_core::Result<PostToolUseOutput> {
    let report = observe_post_tool(&input, context);
    let state_root = state_dir.map(StateRoot::new).unwrap_or_default();
    let store = FileActivityStore::ensure(context, state_root).map_err(activity_error)?;
    let invocation = String::from_utf8_lossy(context.raw().bytes());
    store
        .append_report(&invocation, &report)
        .map_err(activity_error)?;
    native_no_op(context.harness())
}

fn native_no_op(harness: &HarnessId) -> hookkit_core::Result<PostToolUseOutput> {
    match harness.as_str() {
        "claude-code" => Ok(PostToolUseOutput::Claude(
            hookkit_claude::protocol::PostToolUseOutput::no_op(),
        )),
        "codex" => Ok(PostToolUseOutput::Codex(
            hookkit_codex::protocol::PostToolUseOutput::no_op(),
        )),
        "gemini-cli" => Ok(PostToolUseOutput::Gemini(
            hookkit_gemini::protocol::AfterToolOutput::no_op(),
        )),
        _ => Err(std::io::Error::new(
            ErrorKind::Unsupported,
            format!("unsupported tracker harness {harness}"),
        )
        .into()),
    }
}

fn activity_error(error: hookkit_file_activity::FileActivityError) -> hookkit_core::HookkitError {
    std::io::Error::other(error).into()
}
