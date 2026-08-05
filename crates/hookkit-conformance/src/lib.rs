//! Executable conformance registry for every built-in native event adapter.
#![deny(missing_docs)]

use base64::Engine as _;
use hookkit_core::{EventSpec, NativeEventDescriptor, ProcessEmission, RawInvocation};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
/// One conformance fixture that was executed successfully.
pub struct ExecutedCase {
    /// Stable native event/binding contract identifier.
    pub contract: &'static str,
    /// Native handler mechanism exercised by the fixture.
    pub binding: &'static str,
    /// Fixture identifier within the contract.
    pub case: &'static str,
}

/// Execute every case advertised by the implementation registry and reject any
/// declared/executed mismatch before the registry can be generated.
pub fn verified_descriptors() -> Result<Vec<NativeEventDescriptor>, String> {
    let descriptors = implementation_descriptors();
    verify_all_negative_inputs()?;
    let executed = execute_all_cases()?;
    let executed_by_contract = executed.iter().fold(
        BTreeMap::<&str, BTreeSet<&str>>::new(),
        |mut grouped, case| {
            grouped.entry(case.contract).or_default().insert(case.case);
            grouped
        },
    );

    for descriptor in &descriptors {
        let declared: BTreeSet<_> = descriptor.conformance_cases().iter().copied().collect();
        let actual = executed_by_contract
            .get(descriptor.contract().as_str())
            .cloned()
            .unwrap_or_default();
        if declared != actual {
            return Err(format!(
                "declared/executed conformance mismatch for {}: declared={declared:?}, executed={actual:?}",
                descriptor.contract()
            ));
        }
    }
    let known: BTreeSet<_> = descriptors
        .iter()
        .map(|descriptor| descriptor.contract().as_str())
        .collect();
    if let Some(case) = executed.iter().find(|case| !known.contains(case.contract)) {
        return Err(format!(
            "executed conformance case references unregistered contract {}",
            case.contract
        ));
    }
    Ok(descriptors)
}

fn verify_all_negative_inputs() -> Result<(), String> {
    verify_claude_catalog_negative_inputs()?;
    verify_codex_catalog_negative_inputs()?;
    verify_negative_inputs::<hookkit_claude::protocol::SessionStart>(
        "claude-code",
        "docs-2026-08-05-r1",
        "session-start",
    )?;
    verify_negative_inputs::<hookkit_claude::protocol::PostToolUse>(
        "claude-code",
        "docs-2026-08-05-r1",
        "post-tool-use",
    )?;
    verify_negative_inputs::<hookkit_claude::protocol::WorktreeCreate>(
        "claude-code",
        "docs-2026-08-05-r1",
        "worktree-create",
    )?;
    verify_negative_inputs::<hookkit_codex::protocol::PreToolUse>(
        "codex",
        "commit-9e552e9-r2",
        "pre-tool-use",
    )?;
    verify_negative_inputs::<hookkit_codex::protocol::PostToolUse>(
        "codex",
        "commit-9e552e9-r2",
        "post-tool-use",
    )?;
    verify_negative_inputs::<hookkit_antigravity::PreInvocation>(
        "antigravity",
        "docs-2026-08-04-r1",
        "pre-invocation",
    )?;
    verify_negative_inputs::<hookkit_antigravity::PostInvocation>(
        "antigravity",
        "docs-2026-08-04-r1",
        "post-invocation",
    )?;
    verify_negative_inputs::<hookkit_antigravity::PreToolUse>(
        "antigravity",
        "docs-2026-08-04-r1",
        "pre-tool-use",
    )?;
    verify_negative_inputs::<hookkit_antigravity::PostToolUse>(
        "antigravity",
        "docs-2026-08-04-r1",
        "post-tool-use",
    )?;
    verify_negative_inputs::<hookkit_antigravity::Stop>(
        "antigravity",
        "docs-2026-08-04-r1",
        "stop",
    )?;
    Ok(())
}

/// Executes every positive conformance fixture and returns their identities.
///
/// This covers the shared-envelope `catalog` events (via
/// `execute_catalog_cases`), the contract-first `protocol` events for Claude
/// Claude Code, Codex, and the native Antigravity events.
///
/// The function stops at the first parse, emission, or exact-byte mismatch and
/// returns a human-readable error suitable for the conformance CLI.
pub fn execute_all_cases() -> Result<Vec<ExecutedCase>, String> {
    let mut executed = execute_catalog_cases()?;

    executed.push(verify_case::<hookkit_claude::protocol::SessionStart>(
        "claude-code",
        "docs-2026-08-05-r1",
        "session-start",
        "command-structured",
        hookkit_claude::protocol::SessionStartOutput::structured(
            Some("Read conventions.".into()),
            Some(true),
            Some("Review".into()),
            vec!["/repo/.env".into()],
        )
        .map_err(|error| error.to_string())?,
    )?);
    executed.push(verify_case::<hookkit_claude::protocol::SessionStart>(
        "claude-code",
        "docs-2026-08-05-r1",
        "session-start",
        "command-text",
        hookkit_claude::protocol::SessionStartOutput::text_context("Hook-provided context."),
    )?);
    executed.push(verify_case::<hookkit_claude::protocol::PostToolUse>(
        "claude-code",
        "docs-2026-08-05-r1",
        "post-tool-use",
        "command-structured",
        hookkit_claude::protocol::PostToolUseOutput::with_context("Generated files changed.")
            .with_block("Review result.")
            .map_err(|error| error.to_string())?
            .with_updated_tool_output(serde_json::json!({"status":"redacted"}))
            .map_err(|error| error.to_string())?,
    )?);
    executed.push(verify_case::<hookkit_claude::protocol::PostToolUse>(
        "claude-code",
        "docs-2026-08-05-r1",
        "post-tool-use",
        "command-exit-2",
        hookkit_claude::protocol::PostToolUseOutput::feedback_error("blocked by hook"),
    )?);
    executed.push(verify_case::<hookkit_claude::protocol::WorktreeCreate>(
        "claude-code",
        "docs-2026-08-05-r1",
        "worktree-create",
        "command-created",
        hookkit_claude::protocol::WorktreeCreateOutput::path_with_newline(
            "/tmp/hookkit-worktree".into(),
        )
        .map_err(|error| error.to_string())?,
    )?);
    executed.push(verify_case::<hookkit_claude::protocol::WorktreeCreate>(
        "claude-code",
        "docs-2026-08-05-r1",
        "worktree-create",
        "command-failed",
        hookkit_claude::protocol::WorktreeCreateOutput::failed("", 1)
            .map_err(|error| error.to_string())?,
    )?);

    executed.push(verify_case::<hookkit_codex::protocol::PreToolUse>(
        "codex",
        "commit-9e552e9-r2",
        "pre-tool-use",
        "no-op",
        hookkit_codex::protocol::PreToolUseOutput::no_op(),
    )?);
    executed.push(verify_case::<hookkit_codex::protocol::PreToolUse>(
        "codex",
        "commit-9e552e9-r2",
        "pre-tool-use",
        "deny-json",
        hookkit_codex::protocol::PreToolUseOutput::deny(Some("blocked".into())),
    )?);
    executed.push(verify_case::<hookkit_codex::protocol::PreToolUse>(
        "codex",
        "commit-9e552e9-r2",
        "pre-tool-use",
        "deny-stderr",
        hookkit_codex::protocol::PreToolUseOutput::deny_stderr("blocked"),
    )?);
    executed.push(verify_case::<hookkit_codex::protocol::PostToolUse>(
        "codex",
        "commit-9e552e9-r2",
        "post-tool-use",
        "structured",
        hookkit_codex::protocol::PostToolUseOutput::with_context("Generated files changed.")
            .with_block("Review the output.")
            .map_err(|error| error.to_string())?,
    )?);
    executed.push(verify_case::<hookkit_codex::protocol::PostToolUse>(
        "codex",
        "commit-9e552e9-r2",
        "post-tool-use",
        "exit-2",
        hookkit_codex::protocol::PostToolUseOutput::blocking_error("blocked by hook"),
    )?);

    executed.push(verify_case::<hookkit_antigravity::PreInvocation>(
        "antigravity",
        "docs-2026-08-04-r1",
        "pre-invocation",
        "inject-reminder",
        hookkit_antigravity::PreInvocationOutput::inject(
            hookkit_antigravity::InjectStep::EphemeralMessage {
                ephemeral_message: "Remember to lint".into(),
            },
        ),
    )?);
    executed.push(verify_case::<hookkit_antigravity::PostInvocation>(
        "antigravity",
        "docs-2026-08-04-r1",
        "post-invocation",
        "default",
        hookkit_antigravity::PostInvocationOutput {
            inject_steps: Vec::new(),
            termination_behavior: Some(hookkit_antigravity::TerminationBehavior::Default),
        },
    )?);
    executed.push(verify_case::<hookkit_antigravity::PostInvocation>(
        "antigravity",
        "docs-2026-08-04-r1",
        "post-invocation",
        "force-continue",
        hookkit_antigravity::PostInvocationOutput {
            inject_steps: Vec::new(),
            termination_behavior: Some(hookkit_antigravity::TerminationBehavior::ForceContinue),
        },
    )?);
    executed.push(verify_case::<hookkit_antigravity::PreToolUse>(
        "antigravity",
        "docs-2026-08-04-r1",
        "pre-tool-use",
        "ask",
        hookkit_antigravity::PreToolUseOutput {
            decision: hookkit_antigravity::ToolDecision::Ask,
            reason: Some("Requires confirmation.".into()),
            permission_overrides: vec!["command(npm test)".into()],
        },
    )?);
    executed.push(verify_case::<hookkit_antigravity::PostToolUse>(
        "antigravity",
        "docs-2026-08-04-r1",
        "post-tool-use",
        "no-op",
        hookkit_antigravity::PostToolUseOutput::default(),
    )?);
    executed.push(verify_case::<hookkit_antigravity::Stop>(
        "antigravity",
        "docs-2026-08-04-r1",
        "stop",
        "continue",
        hookkit_antigravity::StopOutput {
            decision: "continue".into(),
            reason: Some("Not done yet".into()),
        },
    )?);

    Ok(executed)
}

fn verify_claude_catalog_negative_inputs() -> Result<(), String> {
    macro_rules! verify {
        ($event:ident, $path:literal) => {
            verify_negative_inputs::<hookkit_claude::catalog::$event>(
                "claude-code",
                "docs-2026-08-05-r1",
                $path,
            )?;
        };
    }
    verify!(ConfigChange, "config-change");
    verify!(CwdChanged, "cwd-changed");
    verify!(DirectoryAdded, "directory-added");
    verify!(Elicitation, "elicitation");
    verify!(ElicitationResult, "elicitation-result");
    verify!(FileChanged, "file-changed");
    verify!(InstructionsLoaded, "instructions-loaded");
    verify!(MessageDisplay, "message-display");
    verify!(Notification, "notification");
    verify!(PermissionDenied, "permission-denied");
    verify!(PermissionRequest, "permission-request");
    verify!(PostCompact, "post-compact");
    verify!(PostToolBatch, "post-tool-batch");
    verify!(PostToolUseFailure, "post-tool-use-failure");
    verify!(PreCompact, "pre-compact");
    verify!(PreToolUse, "pre-tool-use");
    verify!(SessionEnd, "session-end");
    verify!(Setup, "setup");
    verify!(Stop, "stop");
    verify!(StopFailure, "stop-failure");
    verify!(SubagentStart, "subagent-start");
    verify!(SubagentStop, "subagent-stop");
    verify!(TaskCompleted, "task-completed");
    verify!(TaskCreated, "task-created");
    verify!(TeammateIdle, "teammate-idle");
    verify!(UserPromptExpansion, "user-prompt-expansion");
    verify!(UserPromptSubmit, "user-prompt-submit");
    verify!(WorktreeRemove, "worktree-remove");
    Ok(())
}

fn verify_codex_catalog_negative_inputs() -> Result<(), String> {
    macro_rules! verify {
        ($event:ident, $path:literal) => {
            verify_negative_inputs::<hookkit_codex::catalog::$event>(
                "codex",
                "commit-9e552e9-r2",
                $path,
            )?;
        };
    }
    verify!(PermissionRequest, "permission-request");
    verify!(PostCompact, "post-compact");
    verify!(PreCompact, "pre-compact");
    verify!(SessionStart, "session-start");
    verify!(Stop, "stop");
    verify!(SubagentStart, "subagent-start");
    verify!(SubagentStop, "subagent-stop");
    verify!(UserPromptSubmit, "user-prompt-submit");
    Ok(())
}

fn execute_catalog_cases() -> Result<Vec<ExecutedCase>, String> {
    let mut executed = Vec::new();
    macro_rules! claude_case {
        ($event:ident, $path:literal, $case:literal, $output:expr) => {
            executed.push(verify_case::<hookkit_claude::catalog::$event>(
                "claude-code",
                "docs-2026-08-05-r1",
                $path,
                $case,
                $output,
            )?);
        };
    }
    macro_rules! codex_case {
        ($event:ident, $path:literal, $case:literal, $output:expr) => {
            executed.push(verify_case::<hookkit_codex::catalog::$event>(
                "codex",
                "commit-9e552e9-r2",
                $path,
                $case,
                $output,
            )?);
        };
    }
    claude_case!(
        ConfigChange,
        "config-change",
        "command-structured",
        hookkit_claude::catalog::ConfigChangeOutput::block("Configuration change rejected.")
    );
    claude_case!(
        ConfigChange,
        "config-change",
        "command-exit-2",
        hookkit_claude::catalog::ConfigChangeOutput::blocking_error("blocked by hook")
    );
    claude_case!(
        CwdChanged,
        "cwd-changed",
        "command-structured",
        hookkit_claude::catalog::CwdChangedOutput::with_system_message(
            "Working directory changed.",
        )
        .with_watch_paths(vec!["/repo/crate/.env".into()])
        .map_err(|error| error.to_string())?
    );
    claude_case!(
        DirectoryAdded,
        "directory-added",
        "command-structured",
        hookkit_claude::catalog::DirectoryAddedOutput::with_system_message(
            "Working directory added.",
        )
    );
    claude_case!(
        Elicitation,
        "elicitation",
        "command-structured",
        hookkit_claude::catalog::ElicitationOutput::accept(serde_json::Map::from_iter([(
            "name".into(),
            serde_json::json!("Ada"),
        )]))
    );
    claude_case!(
        Elicitation,
        "elicitation",
        "command-exit-2",
        hookkit_claude::catalog::ElicitationOutput::blocking_error("blocked by hook")
    );
    claude_case!(
        ElicitationResult,
        "elicitation-result",
        "command-structured",
        hookkit_claude::catalog::ElicitationResultOutput::decline()
    );
    claude_case!(
        ElicitationResult,
        "elicitation-result",
        "command-exit-2",
        hookkit_claude::catalog::ElicitationResultOutput::blocking_error("blocked by hook")
    );
    claude_case!(
        FileChanged,
        "file-changed",
        "command-structured",
        hookkit_claude::catalog::FileChangedOutput::with_system_message("Watched file changed.")
            .with_watch_paths(vec!["/repo/.env".into(), "/repo/.env.local".into()])
            .map_err(|error| error.to_string())?
    );
    claude_case!(
        InstructionsLoaded,
        "instructions-loaded",
        "command-structured",
        hookkit_claude::catalog::InstructionsLoadedOutput::no_op()
    );
    claude_case!(
        MessageDisplay,
        "message-display",
        "command-structured",
        hookkit_claude::catalog::MessageDisplayOutput::display("Here is the plan:")
    );
    claude_case!(
        Notification,
        "notification",
        "command-structured",
        hookkit_claude::catalog::NotificationOutput::with_system_message(
            "Permission notification emitted.",
        )
        .with_continue(true)
        .map_err(|error| error.to_string())?
        .with_suppress_output(true)
        .map_err(|error| error.to_string())?
        .with_terminal_sequence("\u{7}")
        .map_err(|error| error.to_string())?
    );
    claude_case!(
        PermissionDenied,
        "permission-denied",
        "command-structured",
        hookkit_claude::catalog::PermissionDeniedOutput::retry(true)
    );
    claude_case!(
        PermissionRequest,
        "permission-request",
        "command-structured",
        hookkit_claude::catalog::PermissionRequestOutput::decide(
            hookkit_claude::catalog::PermissionRequestBehavior::Deny,
            Some("Blocked by policy.".into()),
            Some(false),
        )
    );
    claude_case!(
        PermissionRequest,
        "permission-request",
        "command-exit-2",
        hookkit_claude::catalog::PermissionRequestOutput::blocking_error("blocked by hook")
    );
    claude_case!(
        PostCompact,
        "post-compact",
        "command-structured",
        hookkit_claude::catalog::PostCompactOutput::with_system_message("Compaction complete.")
    );
    claude_case!(
        PostToolBatch,
        "post-tool-batch",
        "command-structured",
        hookkit_claude::catalog::PostToolBatchOutput::with_context("Hook-provided context.")
    );
    claude_case!(
        PostToolBatch,
        "post-tool-batch",
        "command-exit-2",
        hookkit_claude::catalog::PostToolBatchOutput::blocking_error("blocked by hook")
    );
    claude_case!(
        PostToolUseFailure,
        "post-tool-use-failure",
        "command-structured",
        hookkit_claude::catalog::PostToolUseFailureOutput::with_context("Hook-provided context.",)
    );
    claude_case!(
        PostToolUseFailure,
        "post-tool-use-failure",
        "command-exit-2",
        hookkit_claude::catalog::PostToolUseFailureOutput::feedback_error("blocked by hook")
    );
    claude_case!(
        PreCompact,
        "pre-compact",
        "command-structured",
        hookkit_claude::catalog::PreCompactOutput::block("Save state first.")
    );
    claude_case!(
        PreCompact,
        "pre-compact",
        "command-exit-2",
        hookkit_claude::catalog::PreCompactOutput::blocking_error("blocked by hook")
    );
    let updated_input = serde_json::Map::from_iter([(
        "command".into(),
        serde_json::Value::String("cargo test".into()),
    )]);
    claude_case!(
        PreToolUse,
        "pre-tool-use",
        "command-structured",
        hookkit_claude::catalog::PreToolUseOutput::decide(
            hookkit_claude::catalog::PreToolPermissionDecision::Ask,
            Some("Review command.".into()),
            Some(updated_input),
            Some("Production environment.".into()),
        )
    );
    claude_case!(
        PreToolUse,
        "pre-tool-use",
        "command-exit-2",
        hookkit_claude::catalog::PreToolUseOutput::blocking_error("blocked by hook")
    );
    claude_case!(
        SessionEnd,
        "session-end",
        "command-structured",
        hookkit_claude::catalog::SessionEndOutput::with_system_message("Session ended.")
    );
    claude_case!(
        Setup,
        "setup",
        "command-structured",
        hookkit_claude::catalog::SetupOutput::with_context("Hook-provided context.")
    );
    claude_case!(
        StopFailure,
        "stop-failure",
        "command-structured",
        hookkit_claude::catalog::StopFailureOutput::no_op()
    );
    claude_case!(
        Stop,
        "stop",
        "command-structured",
        hookkit_claude::catalog::StopOutput::block_with_context(
            "Run tests again.",
            "Focus on failures.",
        )
    );
    claude_case!(
        Stop,
        "stop",
        "command-exit-2",
        hookkit_claude::catalog::StopOutput::blocking_error("blocked by hook")
    );
    claude_case!(
        SubagentStart,
        "subagent-start",
        "command-structured",
        hookkit_claude::catalog::SubagentStartOutput::with_context("Hook-provided context.")
    );
    claude_case!(
        SubagentStop,
        "subagent-stop",
        "command-structured",
        hookkit_claude::catalog::SubagentStopOutput::block_with_context(
            "Run another pass.",
            "Check edge cases.",
        )
    );
    claude_case!(
        SubagentStop,
        "subagent-stop",
        "command-exit-2",
        hookkit_claude::catalog::SubagentStopOutput::blocking_error("blocked by hook")
    );
    claude_case!(
        TaskCompleted,
        "task-completed",
        "command-structured",
        hookkit_claude::catalog::TaskCompletedOutput::no_op()
            .with_continue(false)
            .map_err(|error| error.to_string())?
            .with_stop_reason("Verification is incomplete.")
            .map_err(|error| error.to_string())?
    );
    claude_case!(
        TaskCompleted,
        "task-completed",
        "command-exit-2",
        hookkit_claude::catalog::TaskCompletedOutput::blocking_error("blocked by hook")
    );
    claude_case!(
        TaskCreated,
        "task-created",
        "command-structured",
        hookkit_claude::catalog::TaskCreatedOutput::no_op()
            .with_continue(false)
            .map_err(|error| error.to_string())?
            .with_stop_reason("Task needs an owner.")
            .map_err(|error| error.to_string())?
    );
    claude_case!(
        TaskCreated,
        "task-created",
        "command-exit-2",
        hookkit_claude::catalog::TaskCreatedOutput::blocking_error("blocked by hook")
    );
    claude_case!(
        TeammateIdle,
        "teammate-idle",
        "command-structured",
        hookkit_claude::catalog::TeammateIdleOutput::no_op()
            .with_continue(false)
            .map_err(|error| error.to_string())?
            .with_stop_reason("Continue reviewing.")
            .map_err(|error| error.to_string())?
    );
    claude_case!(
        TeammateIdle,
        "teammate-idle",
        "command-exit-2",
        hookkit_claude::catalog::TeammateIdleOutput::blocking_error("blocked by hook")
    );
    claude_case!(
        UserPromptExpansion,
        "user-prompt-expansion",
        "command-structured",
        hookkit_claude::catalog::UserPromptExpansionOutput::block_with_context(
            "Unavailable.",
            "Use the team checklist.",
        )
    );
    claude_case!(
        UserPromptExpansion,
        "user-prompt-expansion",
        "command-text",
        hookkit_claude::catalog::UserPromptExpansionOutput::text_context("Hook-provided context.",)
    );
    claude_case!(
        UserPromptExpansion,
        "user-prompt-expansion",
        "command-exit-2",
        hookkit_claude::catalog::UserPromptExpansionOutput::blocking_error("blocked by hook")
    );
    claude_case!(
        UserPromptSubmit,
        "user-prompt-submit",
        "command-structured",
        hookkit_claude::catalog::UserPromptSubmitOutput::block_with_context(
            "Confirmation required.",
            "Clarify scope.",
            Some("Clarify".into()),
            Some(true),
        )
    );
    claude_case!(
        UserPromptSubmit,
        "user-prompt-submit",
        "command-text",
        hookkit_claude::catalog::UserPromptSubmitOutput::text_context("Hook-provided context.")
    );
    claude_case!(
        UserPromptSubmit,
        "user-prompt-submit",
        "command-exit-2",
        hookkit_claude::catalog::UserPromptSubmitOutput::blocking_error("blocked by hook")
    );
    claude_case!(
        WorktreeRemove,
        "worktree-remove",
        "command-structured",
        hookkit_claude::catalog::WorktreeRemoveOutput::no_op()
    );

    codex_case!(
        PermissionRequest,
        "permission-request",
        "structured",
        hookkit_codex::catalog::PermissionRequestOutput::deny("Blocked by policy.")
    );
    codex_case!(
        PermissionRequest,
        "permission-request",
        "exit-2",
        hookkit_codex::catalog::PermissionRequestOutput::blocking_error("blocked by hook")
    );
    codex_case!(
        PostCompact,
        "post-compact",
        "structured",
        hookkit_codex::catalog::PostCompactOutput::no_op()
            .with_system_message("Compaction completed.")
            .map_err(|error| error.to_string())?
    );
    codex_case!(
        PreCompact,
        "pre-compact",
        "structured",
        hookkit_codex::catalog::PreCompactOutput::stop("Save state before compacting.")
    );
    codex_case!(
        PreCompact,
        "pre-compact",
        "exit-2",
        hookkit_codex::catalog::PreCompactOutput::blocking_error("blocked by hook")
    );
    codex_case!(
        SessionStart,
        "session-start",
        "structured",
        hookkit_codex::catalog::SessionStartOutput::with_context("Load repository conventions.")
    );
    codex_case!(
        SessionStart,
        "session-start",
        "text-context",
        hookkit_codex::catalog::SessionStartOutput::text_context(
            "Hook-provided developer context.",
        )
    );
    codex_case!(
        Stop,
        "stop",
        "structured",
        hookkit_codex::catalog::StopOutput::block("Run the failing tests again.")
    );
    codex_case!(
        Stop,
        "stop",
        "exit-2",
        hookkit_codex::catalog::StopOutput::blocking_error("blocked by hook")
    );
    codex_case!(
        SubagentStart,
        "subagent-start",
        "structured",
        hookkit_codex::catalog::SubagentStartOutput::with_context("Review test conventions.")
    );
    codex_case!(
        SubagentStart,
        "subagent-start",
        "text-context",
        hookkit_codex::catalog::SubagentStartOutput::text_context(
            "Hook-provided developer context.",
        )
    );
    codex_case!(
        SubagentStop,
        "subagent-stop",
        "structured",
        hookkit_codex::catalog::SubagentStopOutput::block("Run another focused pass.")
    );
    codex_case!(
        SubagentStop,
        "subagent-stop",
        "exit-2",
        hookkit_codex::catalog::SubagentStopOutput::blocking_error("blocked by hook")
    );
    codex_case!(
        UserPromptSubmit,
        "user-prompt-submit",
        "structured",
        hookkit_codex::catalog::UserPromptSubmitOutput::block_with_context(
            "Ask for confirmation.",
            "Clarify the reproduction.",
        )
    );
    codex_case!(
        UserPromptSubmit,
        "user-prompt-submit",
        "text-context",
        hookkit_codex::catalog::UserPromptSubmitOutput::text_context(
            "Hook-provided developer context.",
        )
    );
    codex_case!(
        UserPromptSubmit,
        "user-prompt-submit",
        "exit-2",
        hookkit_codex::catalog::UserPromptSubmitOutput::blocking_error("blocked by hook")
    );

    Ok(executed)
}

fn implementation_descriptors() -> Vec<NativeEventDescriptor> {
    let mut descriptors = Vec::new();
    descriptors.extend(hookkit_claude::protocol::events());
    descriptors.extend(hookkit_codex::protocol::events());
    descriptors.extend(hookkit_antigravity::events());
    descriptors
}

fn verify_case<E: EventSpec>(
    harness: &str,
    snapshot: &str,
    event: &str,
    case: &'static str,
    output: E::CommandOutput,
) -> Result<ExecutedCase, String> {
    let fixture = fixture(harness, snapshot, event)?;
    let representative = positive_value(&fixture, "representative")?;
    let raw = RawInvocation::parse(
        serde_json::to_vec(&representative).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    E::parse(&raw).map_err(|error| {
        format!(
            "{} representative input failed native parse: {error}",
            E::CONTRACT
        )
    })?;
    let emission = E::emit(output).map_err(|error| error.to_string())?;
    assert_emission::<E>(&emission, process_case(&fixture, case)?)?;
    Ok(ExecutedCase {
        contract: E::CONTRACT.as_str(),
        binding: "command",
        case,
    })
}

fn verify_negative_inputs<E: EventSpec>(
    harness: &str,
    snapshot: &str,
    event: &str,
) -> Result<(), String> {
    let fixture = fixture(harness, snapshot, event)?;
    let negatives = fixture["input"]["negative"]
        .as_sequence()
        .ok_or_else(|| format!("{} has no negative input fixtures", E::CONTRACT))?;
    for negative in negatives {
        let id = negative["id"]
            .as_str()
            .ok_or_else(|| format!("{} has an unnamed negative fixture", E::CONTRACT))?;
        let value =
            serde_json::to_value(negative["value"].clone()).map_err(|error| error.to_string())?;
        let raw =
            RawInvocation::parse(serde_json::to_vec(&value).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
        if E::parse(&raw).is_ok() {
            return Err(format!(
                "{} negative input fixture {id} was accepted by the native parser",
                E::CONTRACT
            ));
        }
    }
    Ok(())
}

fn fixture(harness: &str, snapshot: &str, event: &str) -> Result<serde_yaml_ng::Value, String> {
    let path = workspace_root()
        .join("contracts/harnesses")
        .join(harness)
        .join("snapshots")
        .join(snapshot)
        .join("events")
        .join(event)
        .join("fixtures.yaml");
    serde_yaml_ng::from_slice(&std::fs::read(&path).map_err(|error| error.to_string())?)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn positive_value(document: &serde_yaml_ng::Value, id: &str) -> Result<serde_json::Value, String> {
    let fixtures = document["input"]["positive"]
        .as_sequence()
        .ok_or_else(|| "missing positive input fixtures".to_string())?;
    let value = fixtures
        .iter()
        .find(|fixture| fixture["id"].as_str() == Some(id))
        .ok_or_else(|| format!("missing positive fixture {id}"))?["value"]
        .clone();
    serde_json::to_value(value).map_err(|error| error.to_string())
}

fn process_case(
    document: &serde_yaml_ng::Value,
    id: &str,
) -> Result<(Vec<u8>, Vec<u8>, u8), String> {
    let cases = document["process"]
        .as_sequence()
        .ok_or_else(|| "missing process fixtures".to_string())?;
    let case = cases
        .iter()
        .find(|case| case["id"].as_str() == Some(id))
        .ok_or_else(|| format!("missing process fixture {id}"))?;
    let decode = |field: &str| {
        base64::engine::general_purpose::STANDARD
            .decode(
                case[field]
                    .as_str()
                    .ok_or_else(|| format!("missing {field}"))?,
            )
            .map_err(|error| error.to_string())
    };
    Ok((
        decode("stdout_base64")?,
        decode("stderr_base64")?,
        case["exit_code"]
            .as_u64()
            .ok_or_else(|| "missing exit_code".to_string())? as u8,
    ))
}

fn assert_emission<E: EventSpec>(
    actual: &ProcessEmission,
    expected: (Vec<u8>, Vec<u8>, u8),
) -> Result<(), String> {
    // JSON object member order is not protocol-significant. Its value is
    // checked structurally, while empty/text/opaque bytes and JSON framing are
    // checked exactly as required by the catalog contract.
    let stdout_matches = match (
        serde_json::from_slice::<serde_json::Value>(actual.stdout()),
        serde_json::from_slice::<serde_json::Value>(&expected.0),
    ) {
        (Ok(actual_json), Ok(expected_json)) => {
            actual_json == expected_json
                && actual.stdout().ends_with(b"\n") == expected.0.ends_with(b"\n")
        }
        _ => actual.stdout() == expected.0,
    };
    if actual.contract() != E::CONTRACT
        || actual.binding() != hookkit_core::HandlerKind::Command
        || !stdout_matches
        || actual.stderr() != expected.1
        || actual.exit_code() != expected.2
    {
        return Err(format!(
            "{} emission mismatch: stdout={:?}/{:?}, stderr={:?}/{:?}, exit={}/{}",
            E::CONTRACT,
            actual.stdout(),
            expected.0,
            actual.stderr(),
            expected.1,
            actual.exit_code(),
            expected.2
        ));
    }
    Ok(())
}

fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate lives under workspace/crates")
        .to_path_buf()
}
