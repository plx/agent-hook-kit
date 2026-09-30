//! Executable conformance registry for every built-in native event adapter.
//!
//! Fixture locations are derived from each implementing [`EventSpec`]: its
//! harness, snapshot, and wire event name select the frozen snapshot event
//! directory through that snapshot's `snapshot.yaml` index. The snapshot must
//! be the one `contracts/registry.yaml` currently selects for the harness, so
//! a crate cannot be checked against stale fixtures after a snapshot switch.
#![deny(missing_docs)]

use base64::Engine as _;
use hookkit_core::{EventSpec, NativeEventDescriptor, ProcessEmission, RawInvocation};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

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
///
/// Every implemented contract must also have had all of its input fixtures
/// checked against the native parser; see [`verify_all_inputs`].
pub fn verified_descriptors() -> Result<Vec<NativeEventDescriptor>, String> {
    let descriptors = implementation_descriptors();
    let implemented: BTreeSet<_> = descriptors
        .iter()
        .map(|descriptor| descriptor.contract().as_str())
        .collect();
    let input_verified = verify_all_inputs()?;
    if input_verified != implemented {
        return Err(format!(
            "input fixtures were not verified for {:?}, or were verified for unregistered contracts {:?}",
            implemented.difference(&input_verified).collect::<Vec<_>>(),
            input_verified.difference(&implemented).collect::<Vec<_>>()
        ));
    }

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
    if let Some(case) = executed
        .iter()
        .find(|case| !implemented.contains(case.contract))
    {
        return Err(format!(
            "executed conformance case references unregistered contract {}",
            case.contract
        ));
    }
    Ok(descriptors)
}

/// Parses every input fixture of every implemented event with its native
/// parser and returns the contracts that were checked.
///
/// Every positive fixture (not only the `representative` one) must parse, so
/// a relaxed or optional field the harness may omit cannot silently become
/// required. Every negative fixture must be rejected, except negatives that
/// only close a harness-sent value set: an `enum`, or a `const` on any pointer
/// other than the `/hook_event_name` discriminator. Every native crate parses
/// harness-sent values it does not know into an `Unknown(String)` arm, so a
/// newer harness release cannot turn a hook into a fail-open parse error.
pub fn verify_all_inputs() -> Result<BTreeSet<&'static str>, String> {
    let mut verified = BTreeSet::new();
    macro_rules! verify {
        ($($event:ty),+ $(,)?) => {
            $(verified.insert(verify_inputs::<$event>()?);)+
        };
    }
    {
        use hookkit_claude::events::*;
        verify!(
            ConfigChange,
            CwdChanged,
            DirectoryAdded,
            Elicitation,
            ElicitationResult,
            FileChanged,
            InstructionsLoaded,
            MessageDisplay,
            Notification,
            PermissionDenied,
            PermissionRequest,
            PostCompact,
            PostModelSwitch,
            PostToolBatch,
            PostToolUse,
            PostToolUseFailure,
            PreCompact,
            PreModelSwitch,
            PreToolUse,
            SessionEnd,
            SessionStart,
            Setup,
            Stop,
            StopFailure,
            SubagentStart,
            SubagentStop,
            TaskCompleted,
            TaskCreated,
            TeammateIdle,
            UserPromptExpansion,
            UserPromptSubmit,
            WorktreeCreate,
            WorktreeRemove,
        );
    }
    {
        use hookkit_codex::catalog::*;
        use hookkit_codex::protocol::{PostToolUse, PreToolUse};
        verify!(
            Interrupt,
            PermissionRequest,
            PostCompact,
            PostToolUse,
            PreCompact,
            PreToolUse,
            SessionEnd,
            SessionStart,
            Stop,
            SubagentStart,
            SubagentStop,
            UserPromptSubmit,
        );
    }
    {
        use hookkit_antigravity::*;
        verify!(PreInvocation, PostInvocation, PreToolUse, PostToolUse, Stop);
    }
    Ok(verified)
}

/// Executes every declared process conformance case and returns their
/// identities.
///
/// Each case emits a typed command output and compares the exit code and
/// stderr byte for byte, and stdout byte for byte or, for JSON, as an equal
/// value with the same trailing-newline framing.
///
/// The function stops at the first emission or exact-byte mismatch and
/// returns a human-readable error suitable for the conformance CLI.
pub fn execute_all_cases() -> Result<Vec<ExecutedCase>, String> {
    let mut executed = execute_claude_cases()?;
    executed.extend(execute_codex_cases()?);
    executed.extend(execute_antigravity_cases()?);
    Ok(executed)
}

/// Converts a builder error into the conformance error type.
fn built<T>(result: hookkit_core::Result<T>) -> Result<T, String> {
    result.map_err(|error| error.to_string())
}

fn execute_claude_cases() -> Result<Vec<ExecutedCase>, String> {
    use hookkit_claude::events::*;
    let mut executed = Vec::new();
    macro_rules! case {
        ($event:ty, $case:literal, $output:expr) => {
            executed.push(verify_case::<$event>($case, $output)?);
        };
    }
    const BLOCKED: &str = "blocked by hook";
    const FAILED: &str = "hook failed";
    const OPEN_BRACE: &str = "{ context without a closing brace";

    case!(
        ConfigChange,
        "command-structured",
        ConfigChangeOutput::block("Configuration change rejected.")
    );
    case!(
        ConfigChange,
        "command-exit-2",
        ConfigChangeOutput::blocking_error(BLOCKED)
    );
    case!(
        ConfigChange,
        "command-exit-2-structured",
        built(
            ConfigChangeOutput::block("Configuration change rejected.")
                .into_blocking_error(BLOCKED)
        )?
    );
    case!(
        ConfigChange,
        "command-nonzero-unstructured",
        ConfigChangeOutput::nonblocking_error(FAILED)
    );
    case!(
        CwdChanged,
        "command-structured",
        built(
            CwdChangedOutput::system_message("Working directory changed.")
                .with_watch_paths(vec!["/repo/crate/.env".into()])
        )?
    );
    case!(
        CwdChanged,
        "command-nonzero-unstructured",
        CwdChangedOutput::nonblocking_error(FAILED)
    );
    case!(
        DirectoryAdded,
        "command-structured",
        DirectoryAddedOutput::system_message("Working directory added.")
    );
    case!(
        Elicitation,
        "command-structured",
        ElicitationOutput::accept(serde_json::Map::from_iter([(
            "name".into(),
            serde_json::json!("Ada"),
        )]))
    );
    case!(
        Elicitation,
        "command-decision-block",
        ElicitationOutput::block("Declined by policy.")
    );
    case!(
        Elicitation,
        "command-exit-2",
        ElicitationOutput::blocking_error(BLOCKED)
    );
    case!(
        Elicitation,
        "command-nonzero-unstructured",
        ElicitationOutput::nonblocking_error(FAILED)
    );
    case!(
        ElicitationResult,
        "command-structured",
        ElicitationResultOutput::decline()
    );
    case!(
        ElicitationResult,
        "command-decision-block",
        ElicitationResultOutput::block("Declined by policy.")
    );
    case!(
        ElicitationResult,
        "command-exit-2",
        ElicitationResultOutput::blocking_error(BLOCKED)
    );
    case!(
        ElicitationResult,
        "command-nonzero-unstructured",
        ElicitationResultOutput::nonblocking_error(FAILED)
    );
    case!(
        FileChanged,
        "command-structured",
        built(
            FileChangedOutput::system_message("Watched file changed.")
                .with_watch_paths(vec!["/repo/.env".into(), "/repo/.env.local".into()])
        )?
    );
    case!(
        FileChanged,
        "command-nonzero-unstructured",
        FileChangedOutput::nonblocking_error(FAILED)
    );
    // Claude Code discards InstructionsLoaded, Setup, SessionEnd, and
    // PostCompact JSON output, so the structured case is an empty object.
    case!(
        InstructionsLoaded,
        "command-structured",
        InstructionsLoadedOutput::no_op()
    );
    case!(
        MessageDisplay,
        "command-structured",
        MessageDisplayOutput::display("Here is the plan:")
    );
    // Notification honors only the terminal sequence.
    case!(
        Notification,
        "command-structured",
        NotificationOutput::terminal_sequence("\u{7}")
    );
    case!(
        PermissionDenied,
        "command-structured",
        PermissionDeniedOutput::retry(true)
    );
    // PermissionRequest ignores exit 2, so there is no blocking-error case.
    case!(
        PermissionRequest,
        "command-structured",
        built(PermissionRequestOutput::deny("Blocked by policy.").with_interrupt(false))?
    );
    case!(
        PermissionRequest,
        "command-nonzero-unstructured",
        PermissionRequestOutput::nonblocking_error(FAILED)
    );
    case!(
        PostCompact,
        "command-structured",
        PostCompactOutput::no_op()
    );
    case!(
        PostCompact,
        "command-nonzero",
        PostCompactOutput::nonblocking_error(FAILED)
    );
    case!(
        PostModelSwitch,
        "command-structured",
        PostModelSwitchOutput::with_context("On Opus, delegate implementation work to subagents.")
    );
    case!(
        PostModelSwitch,
        "command-text",
        PostModelSwitchOutput::text_context("Hook-provided context.")
    );
    case!(
        PostModelSwitch,
        "command-text-open-brace",
        PostModelSwitchOutput::text_context(OPEN_BRACE)
    );
    case!(
        PostModelSwitch,
        "command-nonzero-unstructured",
        PostModelSwitchOutput::nonblocking_error(FAILED)
    );
    case!(
        PostToolBatch,
        "command-structured",
        PostToolBatchOutput::with_context("Hook-provided context.")
    );
    case!(
        PostToolBatch,
        "command-exit-2",
        PostToolBatchOutput::blocking_error(BLOCKED)
    );
    case!(
        PostToolBatch,
        "command-exit-2-structured",
        built(
            PostToolBatchOutput::block("Stop before the next model call.")
                .into_blocking_error(BLOCKED)
        )?
    );
    case!(
        PostToolBatch,
        "command-nonzero-unstructured",
        PostToolBatchOutput::nonblocking_error(FAILED)
    );
    case!(
        PostToolUse,
        "command-structured",
        built(
            PostToolUseOutput::with_context("Generated files changed.")
                .with_updated_tool_output(serde_json::json!({"status": "redacted"}))
                .and_then(|output| output.with_block("Review result."))
        )?
    );
    case!(
        PostToolUse,
        "command-classifier-context",
        built(PostToolUseOutput::no_op().with_classifier_context(
            "This query ran against the staging database, not production.",
        ))?
    );
    case!(
        PostToolUse,
        "command-exit-2",
        PostToolUseOutput::feedback_error(BLOCKED)
    );
    case!(
        PostToolUse,
        "command-exit-2-structured",
        built(
            PostToolUseOutput::with_context("Generated files changed.")
                .into_feedback_error(BLOCKED)
        )?
    );
    case!(
        PostToolUse,
        "command-nonzero-unstructured",
        PostToolUseOutput::nonblocking_error(FAILED)
    );
    case!(
        PostToolUseFailure,
        "command-structured",
        PostToolUseFailureOutput::with_context("Hook-provided context.")
    );
    case!(
        PostToolUseFailure,
        "command-exit-2",
        PostToolUseFailureOutput::feedback_error(BLOCKED)
    );
    case!(
        PostToolUseFailure,
        "command-exit-2-structured",
        built(
            PostToolUseFailureOutput::with_context("Retry with --offline.")
                .into_feedback_error(BLOCKED)
        )?
    );
    case!(
        PostToolUseFailure,
        "command-nonzero-unstructured",
        PostToolUseFailureOutput::nonblocking_error(FAILED)
    );
    case!(
        PreCompact,
        "command-structured",
        PreCompactOutput::block("Save state first.")
    );
    case!(
        PreCompact,
        "command-exit-2",
        PreCompactOutput::blocking_error(BLOCKED)
    );
    case!(
        PreCompact,
        "command-exit-2-structured",
        built(PreCompactOutput::block("Save state first.").into_blocking_error(BLOCKED))?
    );
    case!(
        PreCompact,
        "command-nonzero-unstructured",
        PreCompactOutput::nonblocking_error(FAILED)
    );
    case!(
        PreModelSwitch,
        "command-structured",
        PreModelSwitchOutput::ask(
            "Switching now re-sends about 180k tokens to the new model. Continue?"
        )
    );
    case!(
        PreModelSwitch,
        "command-block",
        PreModelSwitchOutput::block("Opus 4.6 is retired for this project.")
    );
    case!(
        PreModelSwitch,
        "command-exit-2",
        PreModelSwitchOutput::blocking_error(BLOCKED)
    );
    case!(
        PreModelSwitch,
        "command-exit-2-structured",
        built(PreModelSwitchOutput::allow().into_blocking_error(BLOCKED))?
    );
    case!(
        PreModelSwitch,
        "command-nonzero-unstructured",
        PreModelSwitchOutput::nonblocking_error(FAILED)
    );
    case!(
        PreToolUse,
        "command-structured",
        built(
            PreToolUseOutput::ask("Review command.")
                .with_updated_input(serde_json::Map::from_iter([(
                    "command".into(),
                    serde_json::Value::String("cargo test".into()),
                )]))
                .and_then(|output| output.with_additional_context("Production environment."))
        )?
    );
    case!(
        PreToolUse,
        "command-exit-2",
        PreToolUseOutput::blocking_error(BLOCKED)
    );
    case!(
        PreToolUse,
        "command-exit-2-structured",
        built(PreToolUseOutput::allow().into_blocking_error(BLOCKED))?
    );
    case!(
        PreToolUse,
        "command-nonzero-unstructured",
        PreToolUseOutput::nonblocking_error(FAILED)
    );
    case!(SessionEnd, "command-structured", SessionEndOutput::no_op());
    case!(
        SessionEnd,
        "command-nonzero",
        SessionEndOutput::nonblocking_error(FAILED)
    );
    case!(
        SessionStart,
        "command-structured",
        built(
            SessionStartOutput::with_context("Read conventions.")
                .with_reload_skills(true)
                .and_then(|output| output.with_session_title("Review"))
                .and_then(|output| output.with_watch_paths(vec!["/repo/.env".into()]))
        )?
    );
    case!(
        SessionStart,
        "command-text",
        SessionStartOutput::text_context("Hook-provided context.")
    );
    case!(
        SessionStart,
        "command-text-open-brace",
        SessionStartOutput::text_context(OPEN_BRACE)
    );
    case!(
        SessionStart,
        "command-nonzero-unstructured",
        SessionStartOutput::nonblocking_error(FAILED)
    );
    case!(Setup, "command-structured", SetupOutput::no_op());
    case!(
        Stop,
        "command-structured",
        StopOutput::block_with_context("Run tests again.", "Focus on failures.")
    );
    case!(Stop, "command-exit-2", StopOutput::blocking_error(BLOCKED));
    case!(
        Stop,
        "command-exit-2-structured",
        built(StopOutput::block("Run tests again.").into_blocking_error(BLOCKED))?
    );
    case!(
        Stop,
        "command-nonzero-unstructured",
        StopOutput::nonblocking_error(FAILED)
    );
    case!(
        StopFailure,
        "command-structured",
        StopFailureOutput::no_op()
    );
    case!(
        StopFailure,
        "command-terminal-sequence",
        StopFailureOutput::terminal_sequence("\u{7}")
    );
    case!(
        SubagentStart,
        "command-structured",
        SubagentStartOutput::with_context("Hook-provided context.")
    );
    case!(
        SubagentStart,
        "command-nonzero-unstructured",
        SubagentStartOutput::nonblocking_error(FAILED)
    );
    case!(
        SubagentStop,
        "command-structured",
        built(
            SubagentStopOutput::with_context("Check edge cases.").with_block("Run another pass.")
        )?
    );
    case!(
        SubagentStop,
        "command-exit-2",
        SubagentStopOutput::blocking_error(BLOCKED)
    );
    case!(
        SubagentStop,
        "command-exit-2-structured",
        built(SubagentStopOutput::block("Run another pass.").into_blocking_error(BLOCKED))?
    );
    case!(
        SubagentStop,
        "command-nonzero-unstructured",
        SubagentStopOutput::nonblocking_error(FAILED)
    );
    case!(
        TaskCompleted,
        "command-structured",
        built(
            TaskCompletedOutput::no_op()
                .with_continue(false)
                .and_then(|output| output.with_stop_reason("Verification is incomplete."))
        )?
    );
    case!(
        TaskCompleted,
        "command-exit-2",
        TaskCompletedOutput::blocking_error(BLOCKED)
    );
    case!(
        TaskCompleted,
        "command-exit-2-structured",
        built(
            TaskCompletedOutput::no_op()
                .with_system_message("Tests are still failing.")
                .and_then(|output| output.into_blocking_error(BLOCKED))
        )?
    );
    case!(
        TaskCompleted,
        "command-nonzero-unstructured",
        TaskCompletedOutput::nonblocking_error(FAILED)
    );
    // TaskCreated discards `continue`; it blocks through `decision: "block"`.
    case!(
        TaskCreated,
        "command-structured",
        TaskCreatedOutput::block("Task needs an owner.")
    );
    case!(
        TaskCreated,
        "command-exit-2",
        TaskCreatedOutput::blocking_error(BLOCKED)
    );
    case!(
        TaskCreated,
        "command-exit-2-structured",
        built(TaskCreatedOutput::block("Task needs an owner.").into_blocking_error(BLOCKED))?
    );
    case!(
        TaskCreated,
        "command-nonzero-unstructured",
        TaskCreatedOutput::nonblocking_error(FAILED)
    );
    case!(
        TeammateIdle,
        "command-structured",
        built(
            TeammateIdleOutput::no_op()
                .with_continue(false)
                .and_then(|output| output.with_stop_reason("Continue reviewing."))
        )?
    );
    case!(
        TeammateIdle,
        "command-exit-2",
        TeammateIdleOutput::blocking_error(BLOCKED)
    );
    case!(
        TeammateIdle,
        "command-exit-2-structured",
        built(
            TeammateIdleOutput::no_op()
                .with_system_message("Teammate kept working.")
                .and_then(|output| output.into_blocking_error(BLOCKED))
        )?
    );
    case!(
        TeammateIdle,
        "command-nonzero-unstructured",
        TeammateIdleOutput::nonblocking_error(FAILED)
    );
    case!(
        UserPromptExpansion,
        "command-structured",
        UserPromptExpansionOutput::block_with_context("Unavailable.", "Use the team checklist.")
    );
    case!(
        UserPromptExpansion,
        "command-text",
        UserPromptExpansionOutput::text_context("Hook-provided context.")
    );
    case!(
        UserPromptExpansion,
        "command-text-open-brace",
        UserPromptExpansionOutput::text_context(OPEN_BRACE)
    );
    case!(
        UserPromptExpansion,
        "command-exit-2",
        UserPromptExpansionOutput::blocking_error(BLOCKED)
    );
    case!(
        UserPromptExpansion,
        "command-exit-2-structured",
        built(
            UserPromptExpansionOutput::block("Blocked by JSON reason.")
                .into_blocking_error(BLOCKED)
        )?
    );
    case!(
        UserPromptExpansion,
        "command-nonzero-unstructured",
        UserPromptExpansionOutput::nonblocking_error(FAILED)
    );
    case!(
        UserPromptSubmit,
        "command-structured",
        built(
            UserPromptSubmitOutput::block("Confirmation required.")
                .with_additional_context("Clarify scope.")
                .and_then(|output| output.with_session_title("Clarify"))
                .and_then(|output| output.with_suppress_original_prompt(true))
        )?
    );
    case!(
        UserPromptSubmit,
        "command-text",
        UserPromptSubmitOutput::text_context("Hook-provided context.")
    );
    case!(
        UserPromptSubmit,
        "command-text-open-brace",
        UserPromptSubmitOutput::text_context(OPEN_BRACE)
    );
    case!(
        UserPromptSubmit,
        "command-exit-2",
        UserPromptSubmitOutput::blocking_error(BLOCKED)
    );
    case!(
        UserPromptSubmit,
        "command-exit-2-structured",
        built(
            UserPromptSubmitOutput::block("Blocked by JSON reason.").into_blocking_error(BLOCKED)
        )?
    );
    case!(
        UserPromptSubmit,
        "command-nonzero-unstructured",
        UserPromptSubmitOutput::nonblocking_error(FAILED)
    );
    case!(
        WorktreeCreate,
        "command-created",
        built(WorktreeCreateOutput::path_with_newline(
            "/tmp/hookkit-worktree".into()
        ))?
    );
    case!(
        WorktreeCreate,
        "command-failed",
        built(WorktreeCreateOutput::failed("", 1))?
    );
    // WorktreeRemove has no JSON output: only the exit code matters.
    case!(
        WorktreeRemove,
        "command-removed",
        WorktreeRemoveOutput::removed()
    );
    case!(
        WorktreeRemove,
        "command-failed",
        built(WorktreeRemoveOutput::failed("worktree is still in use", 1))?
    );
    case!(
        WorktreeRemove,
        "command-exit-2",
        built(WorktreeRemoveOutput::failed("worktree is still in use", 2))?
    );
    Ok(executed)
}

fn execute_codex_cases() -> Result<Vec<ExecutedCase>, String> {
    use hookkit_codex::catalog::*;
    use hookkit_codex::protocol::{PostToolUse, PostToolUseOutput, PreToolUse, PreToolUseOutput};
    let mut executed = Vec::new();
    macro_rules! case {
        ($event:ty, $case:literal, $output:expr) => {
            executed.push(verify_case::<$event>($case, $output)?);
        };
    }
    const BLOCKED: &str = "blocked by hook";
    // Codex shows failure stderr only for PreCompact, PostCompact, and
    // SessionEnd, and prints it verbatim.
    const FAILED: &str = "hook failed\n";
    const DEVELOPER_CONTEXT: &str = "Hook-provided developer context.";

    // `no_op()` is empty stdout on every Codex event.
    case!(PreToolUse, "no-op", PreToolUseOutput::no_op());
    case!(PreToolUse, "deny-json", PreToolUseOutput::deny("blocked"));
    case!(
        PreToolUse,
        "deny-stderr",
        PreToolUseOutput::deny_stderr("blocked")
    );
    case!(
        PostToolUse,
        "structured",
        built(
            PostToolUseOutput::with_context("Generated files changed.")
                .with_block("Review the output.")
        )?
    );
    case!(PostToolUse, "no-op", PostToolUseOutput::no_op());
    case!(
        PostToolUse,
        "exit-2",
        PostToolUseOutput::blocking_error(BLOCKED)
    );
    case!(
        Interrupt,
        "structured",
        InterruptOutput::system_message("Saved the interrupted turn to the local audit log.")
    );
    case!(Interrupt, "no-op", InterruptOutput::no_op());
    case!(
        PermissionRequest,
        "structured",
        PermissionRequestOutput::deny("Blocked by policy.")
    );
    case!(
        PermissionRequest,
        "structured-allow",
        PermissionRequestOutput::allow()
    );
    case!(PermissionRequest, "no-op", PermissionRequestOutput::no_op());
    case!(
        PermissionRequest,
        "exit-2",
        PermissionRequestOutput::blocking_error(BLOCKED)
    );
    case!(
        PostCompact,
        "structured",
        built(PostCompactOutput::no_op().with_system_message("Compaction completed."))?
    );
    case!(PostCompact, "no-op", PostCompactOutput::no_op());
    case!(PostCompact, "failure", PostCompactOutput::failure(FAILED));
    case!(
        PreCompact,
        "structured",
        PreCompactOutput::stop("Save state before compacting.")
    );
    case!(PreCompact, "no-op", PreCompactOutput::no_op());
    case!(PreCompact, "failure", PreCompactOutput::failure(FAILED));
    case!(SessionEnd, "no-op", SessionEndOutput::no_op());
    case!(SessionEnd, "failure", SessionEndOutput::failure(FAILED));
    case!(
        SessionStart,
        "structured",
        SessionStartOutput::with_context("Load repository conventions.")
    );
    case!(SessionStart, "no-op", SessionStartOutput::no_op());
    case!(
        SessionStart,
        "text-context",
        SessionStartOutput::text_context(DEVELOPER_CONTEXT)
    );
    case!(
        Stop,
        "structured",
        StopOutput::block("Run the failing tests again.")
    );
    case!(Stop, "no-op", StopOutput::no_op());
    case!(Stop, "exit-2", StopOutput::blocking_error(BLOCKED));
    case!(
        SubagentStart,
        "structured",
        SubagentStartOutput::with_context("Review test conventions.")
    );
    case!(SubagentStart, "no-op", SubagentStartOutput::no_op());
    case!(
        SubagentStart,
        "text-context",
        SubagentStartOutput::text_context(DEVELOPER_CONTEXT)
    );
    case!(
        SubagentStop,
        "structured",
        SubagentStopOutput::block("Run another focused pass.")
    );
    case!(SubagentStop, "no-op", SubagentStopOutput::no_op());
    case!(
        SubagentStop,
        "exit-2",
        SubagentStopOutput::blocking_error(BLOCKED)
    );
    case!(
        UserPromptSubmit,
        "structured",
        UserPromptSubmitOutput::block_with_context(
            "Ask for confirmation.",
            "Clarify the reproduction."
        )
    );
    case!(
        UserPromptSubmit,
        "structured-context",
        UserPromptSubmitOutput::with_context(
            "Ask for a clearer reproduction before editing files."
        )
    );
    case!(
        UserPromptSubmit,
        "structured-block",
        UserPromptSubmitOutput::block("Ask for confirmation before doing that.")
    );
    case!(UserPromptSubmit, "no-op", UserPromptSubmitOutput::no_op());
    case!(
        UserPromptSubmit,
        "text-context",
        UserPromptSubmitOutput::text_context(DEVELOPER_CONTEXT)
    );
    case!(
        UserPromptSubmit,
        "exit-2",
        UserPromptSubmitOutput::blocking_error(BLOCKED)
    );
    Ok(executed)
}

fn execute_antigravity_cases() -> Result<Vec<ExecutedCase>, String> {
    use hookkit_antigravity::*;
    let mut executed = Vec::new();
    macro_rules! case {
        ($event:ty, $case:literal, $output:expr) => {
            executed.push(verify_case::<$event>($case, $output)?);
        };
    }
    case!(
        PreInvocation,
        "inject-reminder",
        PreInvocationOutput::inject(InjectStep::ephemeral_message("Remember to lint"))
    );
    case!(
        PostInvocation,
        "default",
        PostInvocationOutput::no_op().with_termination_behavior(TerminationBehavior::Default)
    );
    case!(
        PostInvocation,
        "force-continue",
        PostInvocationOutput::no_op().with_termination_behavior(TerminationBehavior::ForceContinue)
    );
    case!(
        PreToolUse,
        "ask",
        PreToolUseOutput::ask()
            .with_reason("Requires confirmation for test execution.")
            .with_permission_override("command(npm test)")
    );
    case!(PostToolUse, "no-op", PostToolUseOutput::no_op());
    case!(Stop, "continue", StopOutput::continue_with("Not done yet"));
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
    case: &'static str,
    output: E::CommandOutput,
) -> Result<ExecutedCase, String> {
    let fixture = event_fixtures::<E>()?;
    let emission = E::emit(output).map_err(|error| format!("{} {case}: {error}", E::CONTRACT))?;
    assert_emission::<E>(case, &emission, process_case(&fixture, case)?)?;
    Ok(ExecutedCase {
        contract: E::CONTRACT.as_str(),
        binding: "command",
        case,
    })
}

/// Checks every input fixture of `E` against its native parser and returns
/// the contract that was checked.
fn verify_inputs<E: EventSpec>() -> Result<&'static str, String> {
    let fixture = event_fixtures::<E>()?;
    let positives = fixture_list(&fixture, "positive", E::CONTRACT.as_str())?;
    for positive in positives {
        let id = fixture_id(positive, E::CONTRACT.as_str())?;
        let input = E::parse(&invocation(&positive["value"])?).map_err(|error| {
            format!(
                "{} positive input fixture {id} failed native parse: {error}",
                E::CONTRACT
            )
        })?;
        // Context extraction must succeed for every accepted payload.
        let _ = E::context(&input);
    }
    let negatives = fixture_list(&fixture, "negative", E::CONTRACT.as_str())?;
    for negative in negatives {
        let id = fixture_id(negative, E::CONTRACT.as_str())?;
        if closes_open_value_set(negative) {
            continue;
        }
        if E::parse(&invocation(&negative["value"])?).is_ok() {
            return Err(format!(
                "{} negative input fixture {id} was accepted by the native parser",
                E::CONTRACT
            ));
        }
    }
    Ok(E::CONTRACT.as_str())
}

/// Reports whether a negative fixture only closes a harness-sent value set.
///
/// The snapshot schemas close enumerations such as a session `source` or end
/// `reason`, but every native crate deliberately parses values it does not
/// know into an `Unknown(String)` arm, so a newer harness release cannot turn
/// a hook into a fail-open parse error. Such a negative (`enum`, or `const`
/// on any pointer other than the `/hook_event_name` discriminator) is
/// therefore not required to be rejected. Every other negative is.
fn closes_open_value_set(negative: &serde_yaml_ng::Value) -> bool {
    match negative["expected_keyword"].as_str() {
        Some("enum") => true,
        Some("const") => negative["expected_pointer"].as_str() != Some("/hook_event_name"),
        _ => false,
    }
}

#[derive(Deserialize)]
struct Registry {
    harnesses: BTreeMap<String, SelectedSnapshot>,
}

#[derive(Deserialize)]
struct SelectedSnapshot {
    current: String,
}

#[derive(Deserialize)]
struct SnapshotIndex {
    id: String,
    harness: String,
    events: Vec<SnapshotEvent>,
}

#[derive(Deserialize)]
struct SnapshotEvent {
    wire_name: String,
    path: String,
}

/// Loads the fixtures of the snapshot event that `E` implements.
///
/// The snapshot directory comes from `E::HARNESS` and `E::SNAPSHOT`, which
/// must be the registry-selected snapshot for the harness, and the event
/// directory from that snapshot's index. The event's `contract.yaml` must
/// name `E::CONTRACT`.
fn event_fixtures<E: EventSpec>() -> Result<serde_yaml_ng::Value, String> {
    let harness_id = E::HARNESS;
    let harness = harness_id.as_str();
    let snapshot = E::SNAPSHOT.as_str();
    let registry: Registry = read_yaml(&workspace_root().join("contracts/registry.yaml"))?;
    let selected = registry
        .harnesses
        .get(harness)
        .ok_or_else(|| format!("{}: harness {harness} is not in the registry", E::CONTRACT))?;
    if selected.current != snapshot {
        return Err(format!(
            "{} implements {harness}/{snapshot}, but the registry selects {harness}/{}",
            E::CONTRACT,
            selected.current
        ));
    }

    let snapshot_dir = workspace_root()
        .join("contracts/harnesses")
        .join(harness)
        .join("snapshots")
        .join(snapshot);
    let index: SnapshotIndex = read_yaml(&snapshot_dir.join("snapshot.yaml"))?;
    if index.id != snapshot || index.harness != harness {
        return Err(format!(
            "{}: snapshot index identity mismatch",
            snapshot_dir.display()
        ));
    }
    let event = index
        .events
        .iter()
        .find(|event| event.wire_name == E::EVENT.name())
        .ok_or_else(|| {
            format!(
                "{} names event {}, which {harness}/{snapshot} does not index",
                E::CONTRACT,
                E::EVENT.name()
            )
        })?;
    let event_dir = snapshot_dir.join(&event.path);

    let contract: serde_yaml_ng::Value = read_yaml(&event_dir.join("contract.yaml"))?;
    if contract["id"].as_str() != Some(E::CONTRACT.as_str()) {
        return Err(format!(
            "{}: contract id {:?} differs from the native contract {}",
            event_dir.display(),
            contract["id"].as_str(),
            E::CONTRACT
        ));
    }
    read_yaml(&event_dir.join("fixtures.yaml"))
}

fn read_yaml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_yaml_ng::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))
}

fn fixture_list<'a>(
    document: &'a serde_yaml_ng::Value,
    kind: &str,
    contract: &str,
) -> Result<&'a [serde_yaml_ng::Value], String> {
    document["input"][kind]
        .as_sequence()
        .map(Vec::as_slice)
        .filter(|fixtures| !fixtures.is_empty())
        .ok_or_else(|| format!("{contract} has no {kind} input fixtures"))
}

fn fixture_id<'a>(fixture: &'a serde_yaml_ng::Value, contract: &str) -> Result<&'a str, String> {
    fixture["id"]
        .as_str()
        .ok_or_else(|| format!("{contract} has an unnamed input fixture"))
}

fn invocation(value: &serde_yaml_ng::Value) -> Result<RawInvocation, String> {
    let json = serde_json::to_value(value).map_err(|error| error.to_string())?;
    RawInvocation::parse(serde_json::to_vec(&json).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())
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
    if case["binding"].as_str() != Some("command") {
        return Err(format!("process fixture {id} is not a command case"));
    }
    let decode = |field: &str| {
        base64::engine::general_purpose::STANDARD
            .decode(
                case[field]
                    .as_str()
                    .ok_or_else(|| format!("process fixture {id} is missing {field}"))?,
            )
            .map_err(|error| error.to_string())
    };
    let exit_code = case["exit_code"]
        .as_u64()
        .and_then(|code| u8::try_from(code).ok())
        .ok_or_else(|| format!("process fixture {id} has no valid exit_code"))?;
    Ok((
        decode("stdout_base64")?,
        decode("stderr_base64")?,
        exit_code,
    ))
}

fn assert_emission<E: EventSpec>(
    case: &str,
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
            "{} {case} emission mismatch: stdout={:?}/{:?}, stderr={:?}/{:?}, exit={}/{}",
            E::CONTRACT,
            String::from_utf8_lossy(actual.stdout()),
            String::from_utf8_lossy(&expected.0),
            String::from_utf8_lossy(actual.stderr()),
            String::from_utf8_lossy(&expected.1),
            actual.exit_code(),
            expected.2
        ));
    }
    Ok(())
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate lives under workspace/crates")
        .to_path_buf()
}
