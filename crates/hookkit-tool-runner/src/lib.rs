//! Reusable runners for immediate post-tool and session-batched completion hooks.
#![deny(missing_docs)]
//!
//! The runner reads a harness-native post-tool-use event from stdin, loads the
//! Pkl-driven tool catalog through [`hookkit_pkl_config`], runs each tool in
//! the configured order, and lowers a unified result to the selected harness.
//! The completion runner consumes exact snapshots from
//! [`hookkit_session_state`] and commits detailed run bundles before deciding
//! whether a turn may stop.
//!
//! Module layout:
//!
//! - `cli` parses the four bundled binaries' arguments;
//! - `convert` resolves the Pkl schema into crate-private `spec` types;
//! - `exec` partitions files into jobs and runs bounded subprocesses;
//! - `snapshot` attributes writes to one command by content digest;
//! - `post_tool` is the immediate PostToolUse runner and its native lowering;
//! - `turn_completion`, `stop_guard`, and `deferred` implement the Stop runner.

mod cli;
mod convert;
mod deferred;
mod exec;
mod post_tool;
mod snapshot;
mod spec;
mod stop_guard;
mod turn_completion;
mod util;

pub use cli::{
    Cli, CliError, FileActivityCli, SessionStartCli, TurnCompletionCli, parse_args,
    parse_args_from, parse_file_activity_args, parse_file_activity_args_from,
    parse_session_start_args, parse_session_start_args_from, parse_turn_completion_args,
    parse_turn_completion_args_from,
};
pub use deferred::{
    ArtifactClassification, CheckOutcome, CommandPhase, CoverageGap, DeferredRunResult,
    FileAssessment, FileResult, FileStatus, OperationalProblem, RunArtifact, ToolReport,
    ToolReportRef, UnavailableTool,
};

use hookkit_common::{PostToolUseCommandEnvironment, PostToolUseOutput};
use hookkit_core::{HarnessId, RuntimeContext};
use hookkit_file_activity::{FileActivityStore, observe_post_tool as observe_file_activity};
use hookkit_session_state::{SessionState, StateRoot};
use std::path::{Path, PathBuf};
use util::{activity_error, invalid_data, sha256_hex, state_error};

/// Run the full post-tool-use hook from parsed CLI args.
pub fn run_runner(cli: Cli) -> std::process::ExitCode {
    hookkit_runtime::aligned::run_aligned_event::<hookkit_runtime::aligned::PostToolUse, _>(
        cli.harness,
        move |input, environment, ctx| {
            post_tool::run_post_tool_input(input, environment, ctx, cli.config_path.as_deref())
        },
    )
}

/// Run the bundled quiet PostToolUse file-activity observer.
pub fn run_file_activity_observer(cli: FileActivityCli) -> std::process::ExitCode {
    hookkit_runtime::aligned::run_aligned_event::<hookkit_runtime::aligned::PostToolUse, _>(
        cli.harness,
        move |input, environment, ctx| {
            let report = observe_file_activity(&input, ctx);
            let anchor = post_tool_project_root(environment, ctx);
            let state_root = resolve_state_root(cli.state_dir.as_deref(), anchor.as_deref());
            let store = FileActivityStore::ensure(ctx, state_root).map_err(activity_error)?;
            store
                .append_report(&observation_key(ctx), &report)
                .map_err(activity_error)?;
            post_tool_no_op(ctx.harness())
        },
    )
}

/// Compact, repeatable journal key for one PostToolUse observation.
///
/// The raw hook input can carry whole file contents and command output, and
/// the pending journal persists keys verbatim in every record, so only the
/// tool-call id and a digest of the input are used.
fn observation_key(ctx: &RuntimeContext<'_>) -> String {
    format!(
        "post-tool-use\0{}\0{}",
        ctx.tool_call_id()
            .map(ToString::to_string)
            .unwrap_or_default(),
        sha256_hex(ctx.raw().bytes())
    )
}

fn post_tool_no_op(harness: &HarnessId) -> hookkit_core::Result<PostToolUseOutput> {
    match harness.as_str() {
        "claude-code" => Ok(PostToolUseOutput::Claude(
            hookkit_claude::protocol::PostToolUseOutput::no_op(),
        )),
        "codex" => Ok(PostToolUseOutput::Codex(
            hookkit_codex::protocol::PostToolUseOutput::no_op(),
        )),
        "antigravity" => Ok(PostToolUseOutput::Antigravity(
            hookkit_antigravity::PostToolUseOutput::default(),
        )),
        _ => Err(invalid_data(format!(
            "file-activity observer does not support {harness}"
        ))),
    }
}

/// Run the stop-time batch hook from parsed CLI args.
pub fn run_turn_completion_runner(cli: TurnCompletionCli) -> std::process::ExitCode {
    hookkit_runtime::aligned::run_aligned_event::<hookkit_runtime::aligned::TurnCompletion, _>(
        cli.harness,
        move |input, environment, ctx| {
            turn_completion::run_turn_completion_input(
                input,
                environment,
                ctx,
                cli.config_path.as_deref(),
                cli.state_dir.as_deref(),
            )
        },
    )
}

/// Run the small, library-owned lifecycle observer used when precise native
/// session-start timing is desired even before another stateful hook runs.
pub fn run_session_start_observer(cli: SessionStartCli) -> std::process::ExitCode {
    let state_dir = cli.state_dir;
    match cli.harness.as_str() {
        "claude-code" => hookkit_runtime::typed::run_typed::<
            hookkit_claude::protocol::SessionStart,
            _,
        >(move |_, environment, ctx| {
            let anchor = PathBuf::from(environment.project_dir.as_str());
            ensure_session_metadata(ctx, state_dir.as_deref(), Some(&anchor))?;
            Ok(hookkit_claude::protocol::SessionStartOutput::no_op())
        }),
        "codex" => hookkit_runtime::typed::run_typed::<hookkit_codex::catalog::SessionStart, _>(
            move |_, _, ctx| {
                let anchor = ctx
                    .workspace_roots()
                    .first()
                    .map(|root| PathBuf::from(root.as_str()));
                ensure_session_metadata(ctx, state_dir.as_deref(), anchor.as_deref())?;
                Ok(hookkit_codex::catalog::SessionStartOutput::no_op())
            },
        ),
        harness => {
            eprintln!(
                "session-start-state-agent-hook: {harness} has no native SessionStart event; use --claude or --codex"
            );
            std::process::ExitCode::from(1)
        }
    }
}

fn ensure_session_metadata(
    ctx: &RuntimeContext<'_>,
    state_dir: Option<&Path>,
    anchor: Option<&Path>,
) -> hookkit_core::Result<()> {
    SessionState::ensure(ctx, resolve_state_root(state_dir, anchor)).map_err(state_error)?;
    Ok(())
}

/// Stable project root for a PostToolUse invocation: `CLAUDE_PROJECT_DIR` on
/// Claude Code, otherwise the first workspace root from the native input.
fn post_tool_project_root(
    environment: &PostToolUseCommandEnvironment,
    ctx: &RuntimeContext<'_>,
) -> Option<PathBuf> {
    if let PostToolUseCommandEnvironment::Claude(environment) = environment {
        return Some(PathBuf::from(environment.project_dir.as_str()));
    }
    ctx.workspace_roots()
        .first()
        .map(|root| PathBuf::from(root.as_str()))
}

/// Resolve a `--state-dir` value into a state root.
///
/// Relative paths are anchored at the harness project root rather than the
/// hook process's current directory: Claude Code runs hooks in the agent's
/// current directory, which follows `cd` in its Bash tool, so a cwd-relative
/// state directory would split one session's state across directories. Without
/// an anchor the path is left to the process's current directory. `None`
/// selects the default temporary state root.
pub(crate) fn resolve_state_root(state_dir: Option<&Path>, anchor: Option<&Path>) -> StateRoot {
    match state_dir {
        None => StateRoot::default(),
        Some(path) if path.is_absolute() => StateRoot::new(path),
        Some(path) => match anchor.filter(|anchor| anchor.is_absolute()) {
            Some(anchor) => StateRoot::new(anchor.join(path)),
            None => StateRoot::new(path),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_state_dirs_resolve_against_the_project_root_not_the_cwd() {
        let project = Path::new("/work/project");
        assert_eq!(
            resolve_state_root(Some(Path::new(".context/state")), Some(project)).path(),
            Path::new("/work/project/.context/state")
        );
        assert_eq!(
            resolve_state_root(Some(Path::new("/abs/state")), Some(project)).path(),
            Path::new("/abs/state")
        );
        assert_eq!(
            resolve_state_root(Some(Path::new("rel")), None).path(),
            Path::new("rel")
        );
        assert_eq!(
            resolve_state_root(None, Some(project)).path(),
            StateRoot::default().path()
        );
    }
}
