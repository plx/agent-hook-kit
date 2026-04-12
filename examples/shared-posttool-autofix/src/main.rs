use hookkit_common::input::{CommonHookInput, CommonPostToolUseInput};
use hookkit_common::output::{CommonHookOutput, CommonPostToolUseOutput};
use hookkit_core::Harness;
use hookkit_runtime::artifacts::{ArtifactKey, ArtifactManager};
use hookkit_runtime::RuntimeContext;
use std::path::Path;
use std::process::Command;

#[derive(Debug)]
enum AutofixOutcome {
    Clean,
    AutoFixed { summary: String },
    ManualActionRequired { summary: String, diagnostics: String },
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
        Some("--claude") => Harness::Claude,
        Some("--codex") => Harness::Codex,
        Some("--gemini") => Harness::Gemini,
        _ => {
            eprintln!("Usage: shared-posttool-autofix --claude|--codex|--gemini");
            return std::process::ExitCode::from(1);
        }
    };

    hookkit_runtime::run_common(harness, handle)
}

fn handle(input: CommonHookInput, ctx: &RuntimeContext) -> hookkit_core::Result<CommonHookOutput> {
    match input {
        CommonHookInput::PostToolUse(post_tool) => handle_post_tool(post_tool, ctx),
        _ => Ok(CommonHookOutput::empty()),
    }
}

fn handle_post_tool(
    post_tool: CommonPostToolUseInput,
    ctx: &RuntimeContext,
) -> hookkit_core::Result<CommonHookOutput> {
    let tool_name = post_tool.tool_name().unwrap_or("");
    let is_file_tool = matches!(tool_name, "Write" | "Edit" | "shell" | "Bash");

    if !is_file_tool {
        return Ok(CommonHookOutput::empty());
    }

    let outcome = run_autofix_pipeline(post_tool.raw_tool_input(), ctx.cwd.as_str());

    match outcome {
        AutofixOutcome::Clean => Ok(CommonHookOutput::empty()),
        AutofixOutcome::AutoFixed { summary } => {
            // User-facing concise status should go to stderr, not model context.
            eprintln!("[shared-posttool-autofix] {summary}");
            Ok(CommonHookOutput::empty())
        }
        AutofixOutcome::ManualActionRequired {
            summary,
            diagnostics,
        } => {
            let manager = ArtifactManager::in_temp_dir()?;
            let key = ArtifactKey::new(post_tool.session_id(), "autofix-diagnostics");
            let path = manager.write_text(&key, &diagnostics)?;

            // User-facing concise status plus artifact location.
            eprintln!(
                "[shared-posttool-autofix] {summary}. Diagnostics: {}",
                path.display()
            );

            // Codex has limited post-tool output support; keep model output quiet there.
            if matches!(ctx.harness, Harness::Codex) {
                return Ok(CommonHookOutput::empty());
            }

            let guidance = format!(
                "Manual fixes remain. Review diagnostics at {} and continue with targeted changes.",
                path.display()
            );
            Ok(CommonHookOutput::PostToolUse(
                CommonPostToolUseOutput::new().with_agent_feedback(guidance),
            ))
        }
    }
}

fn run_autofix_pipeline(tool_input: Option<&serde_json::Value>, cwd: &str) -> AutofixOutcome {
    if let Some(mode) = test_outcome_override(tool_input) {
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

    let specs: Vec<(&'static str, &'static str, &'static [&'static str])> = if root
        .join("Cargo.toml")
        .exists()
    {
        vec![
            ("formatter", "cargo", &["fmt", "--all"]),
            (
                "linter-autofix",
                "cargo",
                &["clippy", "--all-targets", "--fix", "--allow-dirty", "--allow-staged"],
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

fn test_outcome_override(tool_input: Option<&serde_json::Value>) -> Option<&str> {
    tool_input
        .and_then(|v| v.get("__hookkit_test_outcome"))
        .and_then(|v| v.as_str())
}
