use hookkit_common::{PostToolUseInput, PostToolUseOutput};
use hookkit_core::{HarnessId, RuntimeContext};
use hookkit_runtime::artifacts::{ArtifactKey, ArtifactManager};
use std::path::Path;
use std::process::Command;

#[derive(Debug)]
enum AutofixOutcome {
    Clean,
    AutoFixed {
        summary: String,
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
        Some("--gemini") => HarnessId::GEMINI_CLI,
        _ => {
            eprintln!("Usage: shared-posttool-autofix --claude|--codex|--gemini");
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
    ctx: &RuntimeContext<'_>,
) -> hookkit_core::Result<PostToolUseOutput> {
    let tool_name = match &post_tool {
        PostToolUseInput::Claude(input) => input.tool_name.as_str(),
        PostToolUseInput::Codex(input) => input.tool_name.as_str(),
        PostToolUseInput::Gemini(input) => input.tool_name.as_str(),
        PostToolUseInput::Antigravity(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Antigravity PostToolUse has no tool payload",
            )
            .into());
        }
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "unknown aligned PostToolUse arm",
            )
            .into());
        }
    };
    let is_file_tool = matches!(
        tool_name,
        "Write" | "Edit" | "Bash" | "run_shell_command" | "write_file" | "replace"
    );

    if !is_file_tool {
        return native_output(ctx.harness(), None, None);
    }

    let cwd = ctx
        .workspace_roots()
        .first()
        .ok_or_else(|| std::io::Error::other("missing workspace root"))?;
    let outcome = run_autofix_pipeline(test_outcome_override(&post_tool), cwd.as_str());

    match outcome {
        AutofixOutcome::Clean => native_output(ctx.harness(), None, None),
        AutofixOutcome::AutoFixed { summary } => native_output(
            ctx.harness(),
            None,
            Some(format!("[shared-posttool-autofix] {summary}")),
        ),
        AutofixOutcome::ManualActionRequired {
            summary,
            diagnostics,
        } => {
            let manager = ArtifactManager::in_temp_dir()?;
            let session = ctx
                .session_id()
                .map(ToString::to_string)
                .or_else(|| ctx.conversation_id().map(ToString::to_string))
                .unwrap_or_else(|| "unknown-session".into());
            let key = ArtifactKey::new(session, "autofix-diagnostics");
            let path = manager.write_text(&key, &diagnostics)?;
            let guidance = format!(
                "Manual fixes remain. Review diagnostics at {} and continue with targeted changes.",
                path.display()
            );
            native_output(
                ctx.harness(),
                Some(guidance),
                Some(format!(
                    "[shared-posttool-autofix] {summary}. Diagnostics: {}",
                    path.display()
                )),
            )
        }
    }
}

fn native_output(
    harness: &HarnessId,
    context: Option<String>,
    stderr: Option<String>,
) -> hookkit_core::Result<PostToolUseOutput> {
    match harness.as_str() {
        "claude-code" => {
            let output = context.map_or_else(
                hookkit_claude::protocol::PostToolUseOutput::no_op,
                hookkit_claude::protocol::PostToolUseOutput::with_context,
            );
            Ok(PostToolUseOutput::Claude(match stderr {
                Some(stderr) => output.with_protocol_stderr(stderr)?,
                None => output,
            }))
        }
        "codex" => {
            let output = context.map_or_else(
                hookkit_codex::protocol::PostToolUseOutput::no_op,
                hookkit_codex::protocol::PostToolUseOutput::with_context,
            );
            Ok(PostToolUseOutput::Codex(match stderr {
                Some(stderr) => output.with_protocol_stderr(stderr)?,
                None => output,
            }))
        }
        "gemini-cli" => {
            let output = context.map_or_else(
                hookkit_gemini::protocol::AfterToolOutput::no_op,
                hookkit_gemini::protocol::AfterToolOutput::with_context,
            );
            Ok(PostToolUseOutput::Gemini(match stderr {
                Some(stderr) => output.with_protocol_stderr(stderr)?,
                None => output,
            }))
        }
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!("unsupported harness {harness}"),
        )
        .into()),
    }
}

fn run_autofix_pipeline(test_mode: Option<&str>, cwd: &str) -> AutofixOutcome {
    if let Some(mode) = test_mode {
        return match mode {
            "clean" => AutofixOutcome::Clean,
            "autofixed" => AutofixOutcome::AutoFixed {
                summary: "Applied formatter/linter autofixes".to_string(),
            },
            "manual" => AutofixOutcome::ManualActionRequired {
                summary: "Formatter/linter reported remaining issues".to_string(),
                diagnostics: "manual issues remain (test override)".to_string(),
            },
            _ => AutofixOutcome::Clean,
        };
    }

    let root = Path::new(cwd);
    if !root.exists() {
        return AutofixOutcome::Clean;
    }

    let specs: Vec<(&'static str, &'static str, &'static [&'static str])> =
        if root.join("Cargo.toml").exists() {
            vec![
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
            ]
        } else {
            Vec::new()
        };

    if specs.is_empty() {
        return AutofixOutcome::Clean;
    }

    let mut logs: Vec<CommandLog> = Vec::new();
    let mut had_output = false;

    for (label, program, args) in specs {
        let log = run_command(label, program, args, cwd);
        if !log.stdout.trim().is_empty() || !log.stderr.trim().is_empty() {
            had_output = true;
        }

        let failed = log.error.is_some() || log.status.unwrap_or(1) != 0;
        logs.push(log);

        if failed {
            return AutofixOutcome::ManualActionRequired {
                summary: "Formatter/linter reported remaining issues".to_string(),
                diagnostics: format_diagnostics(&logs),
            };
        }
    }

    if had_output {
        AutofixOutcome::AutoFixed {
            summary: "Applied formatter/linter autofixes".to_string(),
        }
    } else {
        AutofixOutcome::Clean
    }
}

fn run_command(
    label: &'static str,
    program: &'static str,
    args: &'static [&'static str],
    cwd: &str,
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
    match input {
        PostToolUseInput::Claude(input) => input.tool_input.get("__hookkit_test_outcome"),
        PostToolUseInput::Codex(input) => input.tool_input.get("__hookkit_test_outcome"),
        PostToolUseInput::Gemini(input) => input.tool_input.get("__hookkit_test_outcome"),
        PostToolUseInput::Antigravity(_) => None,
        _ => None,
    }
    .and_then(serde_json::Value::as_str)
}

#[cfg(not(feature = "test-support"))]
fn test_outcome_override(_input: &PostToolUseInput) -> Option<&str> {
    None
}
