//! One aligned `PostToolUse` handler that runs a formatter/linter autofix pass
//! after Claude Code or Codex edits files.
//!
//! The two audiences get different channels:
//!
//! - The user sees a one-line status through the top-level `systemMessage`.
//!   Stderr from a hook that exits 0 is not a user channel: Claude Code writes
//!   it only to its debug log and Codex discards it.
//! - The agent gets `additionalContext`: which files the autofix rewrote (so it
//!   re-reads them before editing again), or a pointer to the full diagnostics
//!   when manual fixes remain.
//!
//! The pass runs in the Cargo workspace that holds the edited file (for a
//! shell call, the agent's working directory), never blindly in the project
//! root: after Claude Code enters a git worktree, `CLAUDE_PROJECT_DIR` still
//! names the main checkout, which the agent is not editing.
//!
//! Whether the pass changed anything is decided from content digests of the
//! workspace's Rust sources taken before and after, not from whether a tool
//! printed anything: `cargo clippy` always prints progress to stderr.

use hookkit_common::{PostToolUseCommandEnvironment, PostToolUseInput, PostToolUseOutput};
use hookkit_core::{BuiltinHarness, HarnessId, RuntimeContext};
use hookkit_runtime::artifacts::{ArtifactKey, ArtifactManager};
use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

const STATUS_PREFIX: &str = "[shared-posttool-autofix]";

/// Directories never scanned for Rust sources.
const SKIPPED_DIRECTORIES: &[&str] = &["target", "node_modules"];

#[derive(Debug, PartialEq, Eq)]
enum AutofixOutcome {
    Clean,
    AutoFixed {
        changed: Vec<PathBuf>,
    },
    ManualActionRequired {
        summary: String,
        diagnostics: String,
    },
}

#[derive(Debug)]
struct CommandLog {
    label: &'static str,
    command: String,
    status: Option<i32>,
    stdout: String,
    stderr: String,
    error: Option<String>,
}

fn main() -> std::process::ExitCode {
    let harness = match std::env::args().nth(1).as_deref() {
        Some("--claude") => HarnessId::CLAUDE_CODE,
        Some("--codex") => HarnessId::CODEX,
        _ => {
            eprintln!("Usage: shared-posttool-autofix --claude|--codex");
            return std::process::ExitCode::from(1);
        }
    };

    hookkit_runtime::aligned::run_aligned_event::<hookkit_runtime::aligned::PostToolUse, _>(
        harness,
        handle_post_tool,
    )
}

fn handle_post_tool(
    post_tool: PostToolUseInput,
    environment: &PostToolUseCommandEnvironment,
    ctx: &RuntimeContext<'_>,
) -> hookkit_core::Result<PostToolUseOutput> {
    let edits_files = post_tool
        .tool_name()
        .is_some_and(|tool| edits_files(ctx.harness(), tool));
    if !edits_files {
        return native_output(ctx.harness(), None, None);
    }

    let outcome = match test_outcome_override(&post_tool) {
        Some(mode) => test_outcome(mode),
        None => match cargo_root(&post_tool, environment) {
            Some(root) => run_autofix_pipeline(&root),
            // No Cargo workspace encloses the edit: nothing to format.
            None => AutofixOutcome::Clean,
        },
    };

    match outcome {
        AutofixOutcome::Clean => native_output(ctx.harness(), None, None),
        AutofixOutcome::AutoFixed { changed } => {
            let files = changed
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            native_output(
                ctx.harness(),
                Some(format!(
                    "Formatter/linter autofixes rewrote {} file(s) after this edit: {files}. \
                     Re-read them before editing them again.",
                    changed.len()
                )),
                Some(format!(
                    "{STATUS_PREFIX} Applied formatter/linter autofixes to {} file(s).",
                    changed.len()
                )),
            )
        }
        AutofixOutcome::ManualActionRequired {
            summary,
            diagnostics,
        } => {
            let path = ArtifactManager::in_temp_dir()?
                .write_text(&artifact_key(&post_tool, ctx), &diagnostics)?;
            let guidance = format!(
                "Manual fixes remain. Review diagnostics at {} and continue with targeted changes.",
                path.display()
            );
            native_output(
                ctx.harness(),
                Some(guidance),
                Some(format!(
                    "{STATUS_PREFIX} {summary}. Diagnostics: {}",
                    path.display()
                )),
            )
        }
    }
}

/// Native tools that can change files on each harness.
///
/// Codex edits files only through `apply_patch` (`Write` and `Edit` are
/// matcher aliases, never the reported tool name) and its shell.
fn edits_files(harness: &HarnessId, tool_name: &str) -> bool {
    match BuiltinHarness::from_id(harness) {
        Some(BuiltinHarness::ClaudeCode) => matches!(
            tool_name,
            "Write" | "Edit" | "MultiEdit" | "NotebookEdit" | "Bash"
        ),
        Some(BuiltinHarness::Codex) => matches!(tool_name, "apply_patch" | "Bash"),
        _ => false,
    }
}

/// The Cargo workspace the autofix pass runs in, found from the first of:
/// the edited file, the agent's working directory, and the stable project
/// root, that lies in one.
///
/// Claude Code reports edits by absolute `file_path` (`notebook_path` for
/// `NotebookEdit`), and its `cwd` follows the agent into a worktree, while
/// `CLAUDE_PROJECT_DIR` stays at the main checkout. Starting from the edit
/// formats the checkout the agent is changing; the project root is only the
/// last resort, for a shell call made after `cd` out of the project.
fn cargo_root(
    post_tool: &PostToolUseInput,
    environment: &PostToolUseCommandEnvironment,
) -> Option<PathBuf> {
    let cwd = post_tool.cwd().map(|cwd| cwd.as_std_path());
    let edited = post_tool
        .tool_input()
        .and_then(|input| {
            input
                .get("file_path")
                .or_else(|| input.get("notebook_path"))
        })
        .and_then(serde_json::Value::as_str)
        .map(|path| match cwd {
            Some(cwd) => cwd.join(path),
            None => PathBuf::from(path),
        });
    let project_roots = post_tool.project_roots(environment);
    edited
        .as_deref()
        .and_then(Path::parent)
        .into_iter()
        .chain(cwd)
        .chain(project_roots.iter().map(|root| root.as_std_path()))
        .find_map(cargo_workspace_root)
}

/// The Cargo workspace enclosing `directory`: the nearest ancestor whose
/// `Cargo.toml` declares `[workspace]`, else the nearest package, never
/// looking past the checkout (a directory holding `.git`, which in a linked
/// worktree is a file) that contains `directory`.
fn cargo_workspace_root(directory: &Path) -> Option<PathBuf> {
    let mut package = None;
    for ancestor in directory.ancestors() {
        let manifest = ancestor.join("Cargo.toml");
        if manifest.is_file() {
            let declares_workspace = std::fs::read_to_string(&manifest)
                .is_ok_and(|text| text.lines().any(|line| line.trim() == "[workspace]"));
            if declares_workspace {
                return Some(ancestor.to_path_buf());
            }
            package.get_or_insert_with(|| ancestor.to_path_buf());
        }
        if ancestor.join(".git").exists() {
            break;
        }
    }
    package
}

/// Keys the diagnostics artifact by session and tool call, so concurrent
/// PostToolUse hooks in one session never overwrite each other's report.
fn artifact_key(post_tool: &PostToolUseInput, ctx: &RuntimeContext<'_>) -> ArtifactKey {
    let session = ctx
        .session_id()
        .map(ToString::to_string)
        .or_else(|| ctx.conversation_id().map(ToString::to_string))
        .unwrap_or_else(|| "unknown-session".into());
    let key = ArtifactKey::new(session, "autofix-diagnostics");
    match post_tool.tool_call_id() {
        Some(tool_call_id) => key.with_tool_use(tool_call_id),
        None => key,
    }
}

/// Lowers agent context and a user status line to the native channels:
/// `additionalContext` for the agent and `systemMessage` for the user.
fn native_output(
    harness: &HarnessId,
    context: Option<String>,
    user_status: Option<String>,
) -> hookkit_core::Result<PostToolUseOutput> {
    match BuiltinHarness::from_id(harness) {
        Some(BuiltinHarness::ClaudeCode) => {
            let output = context.map_or_else(
                hookkit_claude::protocol::PostToolUseOutput::no_op,
                hookkit_claude::protocol::PostToolUseOutput::with_context,
            );
            Ok(PostToolUseOutput::Claude(match user_status {
                Some(status) => output.with_system_message(status)?,
                None => output,
            }))
        }
        Some(BuiltinHarness::Codex) => {
            let output = context.map_or_else(
                hookkit_codex::protocol::PostToolUseOutput::no_op,
                hookkit_codex::protocol::PostToolUseOutput::with_context,
            );
            Ok(PostToolUseOutput::Codex(match user_status {
                Some(status) => output.with_system_message(status)?,
                None => output,
            }))
        }
        _ => Err(hookkit_core::HookkitError::UnsupportedHarness {
            harness: harness.clone(),
            message: "shared-posttool-autofix supports claude-code and codex".into(),
        }),
    }
}

/// The outcome a `test-support` build reports instead of running Cargo.
fn test_outcome(mode: &str) -> AutofixOutcome {
    match mode {
        "autofixed" => AutofixOutcome::AutoFixed {
            changed: vec![PathBuf::from("src/lib.rs")],
        },
        "manual" => AutofixOutcome::ManualActionRequired {
            summary: "Formatter/linter reported remaining issues".to_string(),
            diagnostics: "manual issues remain (test override)".to_string(),
        },
        _ => AutofixOutcome::Clean,
    }
}

fn run_autofix_pipeline(root: &Path) -> AutofixOutcome {
    if !root.join("Cargo.toml").is_file() {
        return AutofixOutcome::Clean;
    }
    let specs: [(&'static str, &'static str, &'static [&'static str]); 2] = [
        ("formatter", "cargo", &["fmt", "--all"]),
        (
            "linter-autofix",
            "cargo",
            &[
                "clippy",
                "--all-targets",
                "--fix",
                "--allow-dirty",
                "--allow-staged",
            ],
        ),
    ];

    let before = source_digests(root);
    let mut logs: Vec<CommandLog> = Vec::new();
    for (label, program, args) in specs {
        let log = run_command(label, program, args, root);
        let failed = log.error.is_some() || log.status != Some(0);
        logs.push(log);
        if failed {
            return AutofixOutcome::ManualActionRequired {
                summary: "Formatter/linter reported remaining issues".to_string(),
                diagnostics: format_diagnostics(&logs),
            };
        }
    }
    classify(&before, &source_digests(root))
}

/// Compares two digest snapshots: any added, removed, or rewritten file means
/// the autofix pass changed the workspace.
fn classify(before: &BTreeMap<PathBuf, u64>, after: &BTreeMap<PathBuf, u64>) -> AutofixOutcome {
    let mut changed: Vec<PathBuf> = after
        .iter()
        .filter(|(path, digest)| before.get(*path) != Some(digest))
        .map(|(path, _)| path.clone())
        .chain(
            before
                .keys()
                .filter(|path| !after.contains_key(*path))
                .cloned(),
        )
        .collect();
    changed.sort();
    if changed.is_empty() {
        AutofixOutcome::Clean
    } else {
        AutofixOutcome::AutoFixed { changed }
    }
}

/// Content digests of every Rust source below `root`, skipping build output
/// and hidden directories. Symbolic links are not followed.
fn source_digests(root: &Path) -> BTreeMap<PathBuf, u64> {
    let mut digests = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if kind.is_dir() {
                if !name.starts_with('.') && !SKIPPED_DIRECTORIES.contains(&name.as_ref()) {
                    pending.push(path);
                }
            } else if kind.is_file() && name.ends_with(".rs") {
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                let mut hasher = DefaultHasher::new();
                hasher.write(&bytes);
                digests.insert(path, hasher.finish());
            }
        }
    }
    digests
}

fn run_command(
    label: &'static str,
    program: &'static str,
    args: &'static [&'static str],
    cwd: &Path,
) -> CommandLog {
    match Command::new(program).args(args).current_dir(cwd).output() {
        Ok(output) => CommandLog {
            label,
            command: format!("{program} {}", args.join(" ")),
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            error: None,
        },
        Err(e) => CommandLog {
            label,
            command: format!("{program} {}", args.join(" ")),
            status: None,
            stdout: String::new(),
            stderr: String::new(),
            error: Some(e.to_string()),
        },
    }
}

fn format_diagnostics(logs: &[CommandLog]) -> String {
    let mut out = String::new();
    for log in logs {
        out.push_str(&format!(
            "[{label}] command: {command}\nstatus: {status:?}\n",
            label = log.label,
            command = log.command,
            status = log.status
        ));
        if let Some(err) = &log.error {
            out.push_str(&format!("error: {err}\n"));
        }
        if !log.stdout.trim().is_empty() {
            out.push_str("stdout:\n");
            out.push_str(&log.stdout);
            out.push('\n');
        }
        if !log.stderr.trim().is_empty() {
            out.push_str("stderr:\n");
            out.push_str(&log.stderr);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

#[cfg(feature = "test-support")]
fn test_outcome_override(input: &PostToolUseInput) -> Option<&str> {
    input
        .tool_input()?
        .get("__hookkit_test_outcome")
        .and_then(serde_json::Value::as_str)
}

#[cfg(not(feature = "test-support"))]
fn test_outcome_override(_input: &PostToolUseInput) -> Option<&str> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use hookkit_core::EventSpec;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

    fn stdout_json(output: PostToolUseOutput) -> serde_json::Value {
        let emission = match output {
            PostToolUseOutput::Claude(output) => {
                hookkit_claude::protocol::PostToolUse::emit(output).unwrap()
            }
            PostToolUseOutput::Codex(output) => {
                hookkit_codex::protocol::PostToolUse::emit(output).unwrap()
            }
            other => panic!("unexpected arm {other:?}"),
        };
        assert_eq!(emission.exit_code(), 0);
        assert!(emission.stderr().is_empty(), "exit-0 stderr reaches no one");
        serde_json::from_slice(emission.stdout()).unwrap()
    }

    #[test]
    fn each_harness_edit_tools_trigger_the_pipeline() {
        for tool in ["Write", "Edit", "MultiEdit", "NotebookEdit", "Bash"] {
            assert!(edits_files(&HarnessId::CLAUDE_CODE, tool), "{tool}");
        }
        for tool in ["apply_patch", "Bash"] {
            assert!(edits_files(&HarnessId::CODEX, tool), "{tool}");
        }
        for (harness, tool) in [
            (HarnessId::CLAUDE_CODE, "Read"),
            (HarnessId::CLAUDE_CODE, "apply_patch"),
            (HarnessId::CODEX, "Write"),
            (HarnessId::CODEX, "write_file"),
            (HarnessId::CODEX, "replace"),
            (HarnessId::ANTIGRAVITY, "write_to_file"),
        ] {
            assert!(!edits_files(&harness, tool), "{harness} {tool}");
        }
    }

    #[test]
    fn user_status_uses_system_message_and_guidance_uses_additional_context() {
        for harness in [HarnessId::CLAUDE_CODE, HarnessId::CODEX] {
            let output = stdout_json(
                native_output(
                    &harness,
                    Some("Manual fixes remain.".into()),
                    Some("[shared-posttool-autofix] Diagnostics: /tmp/d.txt".into()),
                )
                .unwrap(),
            );
            assert_eq!(
                output["systemMessage"], "[shared-posttool-autofix] Diagnostics: /tmp/d.txt",
                "{harness}"
            );
            assert_eq!(
                output["hookSpecificOutput"]["additionalContext"], "Manual fixes remain.",
                "{harness}"
            );
        }
    }

    fn claude_post_tool(
        cwd: &Path,
        project: &Path,
        tool_name: &str,
        tool_input: serde_json::Value,
    ) -> (PostToolUseInput, PostToolUseCommandEnvironment) {
        let input = PostToolUseInput::Claude(
            serde_json::from_value(serde_json::json!({
                "session_id": "session",
                "transcript_path": "/tmp/transcript.jsonl",
                "cwd": cwd,
                "permission_mode": "default",
                "hook_event_name": "PostToolUse",
                "tool_name": tool_name,
                "tool_input": tool_input,
                "tool_response": {},
                "tool_use_id": "toolu_1"
            }))
            .unwrap(),
        );
        let environment = PostToolUseCommandEnvironment::Claude(
            <hookkit_claude::ClaudeCommandEnvironment as hookkit_core::CommandEnvironmentSpec>::from_variables(
                &hookkit_core::EventId::builtin(HarnessId::CLAUDE_CODE, "PostToolUse"),
                &hookkit_core::EnvironmentVariables::from_pairs([
                    ("CLAUDECODE", "1"),
                    ("CLAUDE_CODE_CHILD_SESSION", "1"),
                    ("CLAUDE_CODE_SESSION_ID", "session"),
                    ("CLAUDE_PROJECT_DIR", project.to_str().unwrap()),
                ]),
            )
            .unwrap(),
        );
        (input, environment)
    }

    #[test]
    fn the_pipeline_runs_in_the_worktree_the_agent_edits() {
        let directory = std::env::temp_dir().join(format!(
            "hookkit-autofix-worktree-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        let project = directory.join("project");
        let worktree = project.join(".claude/worktrees/w");
        std::fs::create_dir_all(project.join(".git")).unwrap();
        std::fs::create_dir_all(project.join("src")).unwrap();
        std::fs::create_dir_all(worktree.join("crates/core/src")).unwrap();
        std::fs::write(project.join("Cargo.toml"), "[package]\nname = \"p\"\n").unwrap();
        std::fs::write(worktree.join(".git"), "gitdir: ../../../.git/worktrees/w\n").unwrap();
        std::fs::write(
            worktree.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/core\"]\n",
        )
        .unwrap();
        std::fs::write(
            worktree.join("crates/core/Cargo.toml"),
            "[package]\nname = \"core\"\n",
        )
        .unwrap();
        let edited = worktree.join("crates/core/src/lib.rs");
        // A checkout of its own, so the upward search stops there wherever
        // the temporary directory lives.
        let outside = directory.join("elsewhere");
        std::fs::create_dir_all(outside.join(".git")).unwrap();

        // CLAUDE_PROJECT_DIR stays at the main checkout after Claude enters
        // the worktree; the edit and `cwd` are in the worktree.
        let (input, environment) = claude_post_tool(
            &worktree,
            &project,
            "Write",
            serde_json::json!({"file_path": edited.to_str().unwrap(), "content": ""}),
        );
        assert_eq!(cargo_root(&input, &environment), Some(worktree.clone()));

        // A shell edit has no path: the agent's directory decides, and a
        // member crate resolves to its workspace.
        let (input, environment) = claude_post_tool(
            &worktree.join("crates/core/src"),
            &project,
            "Bash",
            serde_json::json!({"command": "sed -i s/a/b/ lib.rs"}),
        );
        assert_eq!(cargo_root(&input, &environment), Some(worktree.clone()));

        // After `cd` out of every project, the project root is the fallback.
        let (input, environment) = claude_post_tool(
            &outside,
            &project,
            "Bash",
            serde_json::json!({"command": "touch x"}),
        );
        assert_eq!(cargo_root(&input, &environment), Some(project.clone()));

        // An edit outside any Cargo workspace, with no project to fall back
        // on, formats nothing.
        let (input, environment) = claude_post_tool(
            &outside,
            &outside,
            "Write",
            serde_json::json!({"file_path": outside.join("notes.md").to_str().unwrap()}),
        );
        assert_eq!(cargo_root(&input, &environment), None);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn autofix_classification_follows_file_contents_not_tool_output() {
        let root = std::env::temp_dir().join(format!(
            "hookkit-autofix-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "fn  main() {}\n").unwrap();
        std::fs::write(root.join("target/debug/build.rs"), "// output\n").unwrap();

        let before = source_digests(&root);
        assert_eq!(before.len(), 1, "build output is not scanned");
        // Nothing changed: clean, however noisy the tools were.
        assert_eq!(
            classify(&before, &source_digests(&root)),
            AutofixOutcome::Clean
        );

        std::fs::write(root.join("src/lib.rs"), "fn main() {}\n").unwrap();
        assert_eq!(
            classify(&before, &source_digests(&root)),
            AutofixOutcome::AutoFixed {
                changed: vec![root.join("src/lib.rs")]
            }
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
