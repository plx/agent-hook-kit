//! Generates the Codex event contracts of every snapshot in [`SNAPSHOTS`] from
//! the vendored upstream schemas.
//!
//! `PreToolUse` is hand-authored in each snapshot and is intentionally not
//! seeded here, as are each snapshot's `snapshot.yaml` and `sources.yaml`.
//! Frozen snapshots are immutable, so regenerating must reproduce each of them
//! byte for byte: a seed correction gets a new [`Snapshot`] revision and keeps
//! the text of the revisions before it. A draft snapshot is regenerated freely
//! until `cargo xtask contracts freeze` freezes it.

use base64::Engine as _;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Interpretations of the pinned upstream revisions, oldest first. A later
/// variant inherits every seed rule of the earlier ones unless a seed says
/// otherwise.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Snapshot {
    /// `rust-v0.159.2` (frozen).
    R1,
    /// Corrects uncertainty text that misstated the pinned source (frozen).
    R2,
    /// `rust-v0.172.0` (draft): adds the observation-only model-request
    /// events and the tool-event `cwd`, environment-scoped hook, and plugin
    /// trust drift, and corrects texts that the earlier revisions misstated.
    Commit3d86e43R1,
}

const SNAPSHOTS: [Snapshot; 3] = [Snapshot::R1, Snapshot::R2, Snapshot::Commit3d86e43R1];

impl Snapshot {
    fn id(self) -> &'static str {
        match self {
            Self::R1 => "commit-ff6aec9-r1",
            Self::R2 => "commit-ff6aec9-r2",
            Self::Commit3d86e43R1 => "commit-3d86e43-r1",
        }
    }

    /// The openai/codex revision whose vendored generated schemas, under
    /// `contracts/vendor/codex/<revision>/generated`, the snapshot derives from.
    fn revision(self) -> &'static str {
        match self {
            Self::R1 | Self::R2 => "ff6aec96948b70d94983af2641a6b67c94faeff5",
            Self::Commit3d86e43R1 => "3d86e43b075adeba2193ec5c22ac92f55afd82e6",
        }
    }
}

/// Stderr written by the synthesized non-zero-exit failure fixtures.
const FAILURE_STDERR: &[u8] = b"hook failed\n";
/// Stderr that Codex trims to nothing, so exit 2 cannot block with it.
const BLANK_STDERR: &[u8] = b" \n";

const COMPATIBILITY_FIELDS: &str = "The official generated schemas include parsed compatibility fields; runtime support restrictions from the hooks reference and pinned source remain controlling.";
const EXIT_ZERO_STDERR: &str = "Codex never reads stderr from a command that exits 0, so exit-0 outcomes model stderr as ignored rather than as diagnostics.";
const ASYNC_HANDLERS: &str = "Command handlers with `async: true` (Codex >= 0.148.0) run in the background: Codex ignores every control effect (decision, permissionDecision, updatedInput, PermissionRequest decisions, continue:false, stopReason, and exit 2), treats exit 2 as a plain failure, and delivers only additionalContext and systemMessage at the next safe point. The stdin payload carries no execution-mode marker, so these outcomes describe synchronous handlers.";
const HANDLER_KINDS: &str = "Codex >= 0.148.0 also runs `mcp_tool` handlers, always synchronously, and parses a successful tool result like exit-0 stdout; `prompt` and `agent` handlers are parsed but skipped. Only the command binding is modeled.";
const PROCESS_LIFECYCLE: &str = "Since Codex 0.149.0 command hooks receive a replay of the environment Codex captured when the session hook registry was created, not the live environment; since 0.155.0 they start in a new session without a controlling terminal and their timeout also covers stdin delivery. On timeout, I/O failure, or turn abort Codex SIGKILLs the hook's whole process group.";
const CLOUD_ORCHESTRATION: &str = "Codex does not dispatch command hooks for Work threads that use cloud orchestration, even when tools execute locally; only admin-managed remote mcp_tool hooks run there.";
const TEXT_CONTEXT: &str = "Codex trims plain-text stdout before using it as context, and treats stdout whose first non-whitespace character is `{` or `[` as JSON: such text fails the run when it is not a valid output object, and a valid object such as `{}` is consumed as structured output without context.";
const BLOCKING_EXIT_2: &str = "Exit 2 selects the exit-2 outcome only when stderr is non-empty after trimming; exit 2 with empty or whitespace-only stderr is a failed run (failure outcome) and has no blocking effect.";
const BLOCK_REASON: &str = "`decision: block` requires a `reason` that is non-empty after trimming unless `continue: false` takes precedence; otherwise Codex fails the synchronous run without blocking. The output schema adds this runtime rule, which the generated schema does not express.";
const TOOL_HOOK_SCOPE: &str = "Since Codex 0.157.0 `cwd` and the process working directory are the step's local-environment cwd when one is selected (falling back to the turn cwd), so they can differ from the cwd reported by turn-scoped events; the hooks reference still says the session cwd. Since 0.156.0 `model` and `permission_mode` come from the step's captured settings (PermissionRequest: the review context) and can change within a turn.";
const SILENT_FAILURE: &str = "For non-zero exits that do not select a blocking outcome Codex records only `hook exited with code N`, discards stderr, and continues (fail-open); a process that dies without an exit status is also a failed run.";
const VISIBLE_FAILURE: &str = "For non-zero exits Codex marks the run failed and reports the trimmed stderr (or `hook exited with code N` when stderr is blank) as the failure message; exit 2 has no special meaning and the operation continues.";

// Texts of `commit-3d86e43-r1` (rust-v0.172.0) and later. The constants above
// keep the exact wording of the frozen revisions.

const EXIT_ZERO_STDERR_SCOPED: &str = "Codex never reads stderr from a command that exits 0 for this event (only PreModelRequest and PostModelRequest surface exit-0 stderr), so exit-0 outcomes model stderr as ignored rather than as diagnostics.";
const HANDLER_KINDS_SCOPED: &str = "Codex >= 0.148.0 also runs `mcp_tool` handlers, synchronously unless they are executor-scoped (those always run in the background and cannot apply control effects): Codex joins the tool result's text content blocks with newlines and parses them like exit-0 stdout, `isError: true` fails the run, and `structuredContent` is ignored. Since 0.164.0 a hook call waits for its MCP server and may start a dormant one within the hook timeout; since 0.171.0 bundled App hooks must target the host Apps server, and the connector and current policy must allow automatic execution. `prompt` and `agent` handlers are parsed but skipped. Only the command binding is modeled.";
const PROCESS_LIFECYCLE_LOCAL: &str = "Since Codex 0.149.0 locally executed command hooks receive a replay of the environment Codex captured when the session hook registry was created, not the live environment; since 0.155.0 they start in a new session without a controlling terminal and their timeout also covers stdin delivery. On timeout, I/O failure, or turn abort Codex SIGKILLs the hook's whole process group.";
const CLOUD_ORCHESTRATION_GLOBAL: &str = "Codex does not dispatch command hooks for Work threads that use cloud orchestration, even when tools execute locally; only admin-managed Global `mcp_tool` hooks run there, never environment-scoped hooks or SessionEnd MCP hooks.";
const ENVIRONMENT_SCOPED: &str = "Since Codex 0.165.0 managed hooks declared by an execution environment's requirements (environment-scoped hooks) also run for this event. Codex rewrites their stdin without marking the scope: `cwd` becomes the environment's cwd in its native form, and `transcript_path` (and `agent_transcript_path`, where present) becomes null even when the session has a transcript. They run on the environment's executor through the exec server, in the environment's shell (on Windows only PowerShell, which pipes the stdin line to the command, with its trailing newline and encoding unverified; cmd is rejected), under its `shell_environment_policy` without an environment replay, with the exec server's credential scrub and no plugin variables; stdout and stderr are each silently capped at 1 MiB, and one deadline covers launch, stdin, and output.";
const PLUGIN_TRUST: &str = "Since Codex 0.164.0 a non-builtin plugin hook with a saved approval re-hashes its whole plugin bundle before every run (synchronous, background, or `mcp_tool`) unless `--dangerously-bypass-hook-trust` is set, and the scan counts against the hook timeout. A changed declaration or bundle content, a symlink or special entry (including a symlinked plugin root), a file that changes during the scan, or a scan timeout records a failed run with an error entry and no exit code, without launching the hook, and the operation continues (fail-open). The hooks reference says changed hooks are skipped until trusted, but at runtime they fail on every match.";
const MATCHER_RULE: &str = "A `matcher` that is omitted, `\"\"`, or `\"*\"` matches every occurrence; one made only of ASCII letters, digits, `_`, and `|` matches exactly one of its `|`-separated alternatives (`Bash` does not match `BashOutput`); anything else is an unanchored regular expression. An invalid regular expression skips its matcher group, which is a load failure for required managed hooks.";
const MATCHER_IGNORED: &str = "Codex ignores any configured `matcher` for this event.";
const PERMISSION_MODE_VALUES: &str = "The pinned source derives `permission_mode` only from the approval policy and sends only `default` or `bypassPermissions` (approval policy `never`); `acceptEdits`, `plan`, and `dontAsk` never occur, although the hooks reference and the generated schema list them, so the schema keeps the five-value superset and the value cannot reveal plan mode.";
const ROOT_SESSION: &str = "`session_id` is the root session identity that every descendant thread shares (the hooks reference says the parent session id), so on a subagent thread it names the root session, not the subagent; `agent_id` identifies a thread-spawned subagent.";
const SPILL_CONTEXT: &str = "Model-visible `additionalContext` above the handler's `additionalContextLimit` (default about 2,500 tokens; 0 disables spilling) is saved to `<temp_dir>/hook_outputs/<thread_id>/<uuid>.txt` and replaced by a head-and-tail preview with that path, or by a plain truncation when the file cannot be written. The directory is keyed by the hook registry's thread id, not the `<session_id>` the hooks reference names, so subagent output spills separately.";
const SPILL_CONTINUATION: &str = "A block reason that becomes a continuation prompt spills like `additionalContext` (saved to `<temp_dir>/hook_outputs/<thread_id>/<uuid>.txt` behind a head-and-tail preview) above the default limit of about 2,500 tokens; `additionalContextLimit` does not apply to this event, and Codex warns and ignores it.";
const TOOL_ENVIRONMENTS: &str = "A tool call bound to execution environments runs only those environments' environment-scoped hooks, while an environment-agnostic call (one that may use every environment, or an MCP call without an owning environment) runs the tool hooks of every attached environment.";

// Observation-only model-request events (`commit-3d86e43-r1` and later).

const OBSERVER_STDOUT: &str = "Observation-only: Codex never parses stdout, at any exit code. Plain text, invalid JSON, and JSON that looks like a control directive, such as `{\"continue\":false,\"decision\":\"block\"}`, are only debug-logged; no output can block, veto, or change the model request or its context.";
const OBSERVER_STDERR: &str = "Unlike every other Codex event, trimmed non-blank stderr becomes an error entry at every exit code, including 0: an exit-0 run is completed but still carries its stderr, so exit-0 outcomes model stderr as diagnostics. A synchronous handler's entries appear in its hook-completed notification, so its stderr is shown on every model request attempt; background (`async: true`) runs never produce hook notifications, so their stderr is not shown.";
const OBSERVER_FAILURE: &str = "Every non-zero exit, exit 2 included, and a process that dies without an exit status is a failed run with no other effect; Codex adds no `hook exited with code N` text, so a failed run with blank stderr has no output entries. A timeout, spawn error, or plugin trust error adds its own error entry.";
const OBSERVER_DISPATCH: &str = "Codex ignores any configured `matcher` for this event, and ignores `additionalContextLimit` with a load warning. `async: true` is honored: a background run delays nothing, and Codex keeps its result out of hook notifications (it reaches only metrics and analytics). The timeout defaults to 600 seconds, with a minimum of 1 and no clamp.";
const OBSERVER_HANDLER_KINDS: &str = "Codex also runs `mcp_tool` handlers, synchronously unless they are executor-scoped (those always run in the background), and ignores the tool result's text like command stdout; `isError: true` fails the run. Since 0.164.0 a hook call waits for its MCP server and may start a dormant one within the hook timeout; since 0.171.0 bundled App hooks must target the host Apps server, and the connector and current policy must allow automatic execution. `prompt` and `agent` handlers are parsed but skipped. Only the command binding is modeled.";
const OBSERVER_PAIRING: &str = "Each call that opens a model response stream runs one PreModelRequest and PostModelRequest pair; transport recovery within that call does not start another pair, and no pair starts once the rollout budget is exhausted. `request_id` (a UUID) is shared by the retries and model fallbacks of one generation, and `attempt_number` counts its opens from 1, although the generated schema allows 0. The pair's lifecycle id is not serialized, so hooks can correlate the two events only by (`request_id`, `attempt_number`). PostModelRequest runs without a PreModelRequest when only post handlers are configured.";
const OBSERVER_KIND: &str = "`kind` is `sampling` for turn requests and `foreground_compaction` for compaction requests; `background_compaction` comes only from the under-development `AsyncCompaction` feature (off by default), whose requests can run before PreCompact, possibly for a discarded compaction candidate. `kind` arrived in 0.171.0 together with compaction dispatch and mirrors an internal enum, so consumers should accept unknown kinds.";
const OBSERVER_FIELDS: &str = "The payload carries no `permission_mode`. `cwd` is the step's local-environment cwd, falling back to the turn cwd. `thread_id` names the issuing thread, and `agent_id` and `agent_type` (which falls back to the default role name) are present only for thread-spawned subagents, where `agent_id` equals `thread_id`. `kind`, `request_id`, `attempt_number`, `thread_id`, and PostModelRequest `response_id` were added in 0.171.0; the hooks reference documents only the earlier field set, and the pinned generated schema controls.";
const OBSERVER_MEMORY: &str = "Memory-consolidation turns run only managed-source and executor-scoped model-request handlers; user, project, session-flag, plugin, and environment-scoped hooks do not run there.";
const OBSERVER_OPEN_SCHEMA: &str = "Unlike every other Codex input schema, the generated schema declares no `additionalProperties: false`, because the input struct flattens its event enum without `deny_unknown_fields`; the serialized key set is nonetheless closed. This contract derives one event's schema from the shared file's matching `oneOf` branch and keeps the vendored openness and the `attempt_number` minimum of 0.";

struct Seed {
    file_key: &'static str,
    wire_name: &'static str,
    rust_key: &'static str,
    category: &'static str,
    /// Effect of a synchronous exit 2 with non-blank stderr, when Codex gives exit 2 one.
    block_effect: Option<&'static str>,
    /// Plain-text exit-0 stdout becomes model context.
    text_context: bool,
    /// Representative structured stdout; `None` when Codex ignores exit-0 output.
    structured: Option<Value>,
    /// Codex reports trimmed stderr as the failure message for non-zero exits.
    failure_stderr_visible: bool,
    /// Structured `decision: block` requires a reason that is non-empty after trimming.
    block_reason_required: bool,
    /// Pinned core source controls part of this event's input semantics.
    core_semantics: bool,
    /// Vendored input schema that several events share and that is split on
    /// its `hook_event_name` `oneOf` branches; `None` reads the event's own
    /// `<file_key>.command.input.schema.json`.
    shared_input: Option<&'static str>,
    /// Observation-only event: Codex never parses stdout and reports trimmed
    /// stderr as an error entry at every exit code, including 0.
    observer: bool,
    uncertainties: Vec<&'static str>,
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask parent")
        .to_path_buf();
    for snapshot in SNAPSHOTS {
        for seed in seeds(snapshot) {
            generate(&root, snapshot, seed);
        }
    }
}

fn seeds(snapshot: Snapshot) -> Vec<Seed> {
    let r2 = snapshot >= Snapshot::R2;
    let v172 = snapshot >= Snapshot::Commit3d86e43R1;
    let mut seeds = vec![
        Seed {
            file_key: "session-start",
            wire_name: "SessionStart",
            rust_key: "session_start",
            category: "session",
            block_effect: None,
            text_context: true,
            structured: Some(
                json!({"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"Load repository conventions."}}),
            ),
            failure_stderr_visible: false,
            block_reason_required: false,
            core_semantics: true,
            shared_input: None,
            observer: false,
            uncertainties: if v172 {
                vec![
                    "Codex >= 0.155.0 reports `fork` for threads forked from a parent thread (also the matcher input) and `resume` instead of `startup` for thread/resume with supplied history.",
                    TEXT_CONTEXT,
                    "`continue: false` from a synchronous handler ends the turn without another model request.",
                    "After Codex compacts a root session, including automatic compaction in the middle of a turn, `compact`-matching SessionStart hooks run before the next model request and their context reaches the immediate continuation; subagents never run SessionStart for compaction.",
                    SPILL_CONTEXT,
                ]
            } else {
                vec![
                    "Codex >= 0.155.0 reports `fork` for threads forked from a parent thread (also the matcher input) and `resume` instead of `startup` for thread/resume with supplied history. The hooks reference still lists only startup, resume, clear, and compact; the pinned release schema and core source control.",
                    TEXT_CONTEXT,
                    "`continue: false` from a synchronous handler ends the turn without another model request.",
                ]
            },
        },
        Seed {
            file_key: "session-end",
            wire_name: "SessionEnd",
            rust_key: "session_end",
            category: "session",
            block_effect: None,
            text_context: false,
            structured: None,
            failure_stderr_visible: true,
            block_reason_required: false,
            core_semantics: false,
            shared_input: None,
            observer: false,
            uncertainties: {
                let mut uncertainties = vec![
                    "SessionEnd is advisory: Codex ignores stdout at exit 0, never runs it for subagents, and `reason` is always `other`.",
                    "SessionEnd handlers always run synchronously even when `async` is true, default to a one-second timeout clamped to one through three seconds, and reject `mcp_tool` handlers; `prompt` and `agent` handlers are parsed but skipped.",
                ];
                if v172 {
                    uncertainties.push("SessionEnd runs for the main thread when an open conversation is archived or deleted, when Codex shuts down normally, and when the thread is unloaded after no client has been subscribed to it for `thread_unload_delay_secs` (60 seconds by default in the pinned core source, configurable per configuration or thread; the hooks reference says 30 minutes). Unsubscribing or switching clients does not run it immediately, and it never runs when the process is killed, so cleanup keyed on SessionEnd must tolerate late or missing delivery.");
                }
                uncertainties
            },
        },
        Seed {
            file_key: "subagent-start",
            wire_name: "SubagentStart",
            rust_key: "subagent_start",
            category: "subagent",
            block_effect: None,
            text_context: true,
            structured: Some(
                json!({"hookSpecificOutput":{"hookEventName":"SubagentStart","additionalContext":"Review test conventions."}}),
            ),
            failure_stderr_visible: false,
            block_reason_required: false,
            core_semantics: false,
            shared_input: None,
            observer: false,
            uncertainties: {
                let mut uncertainties = vec![
                    TEXT_CONTEXT,
                    "`continue: false` is parsed for compatibility but does not stop the subagent from starting.",
                ];
                if v172 {
                    uncertainties.push(SPILL_CONTEXT);
                }
                uncertainties
            },
        },
        Seed {
            file_key: "permission-request",
            wire_name: "PermissionRequest",
            rust_key: "permission_request",
            category: "tool",
            block_effect: Some("deny"),
            text_context: false,
            structured: Some(
                json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Blocked by policy."}}}),
            ),
            failure_stderr_visible: false,
            block_reason_required: false,
            core_semantics: true,
            shared_input: None,
            observer: false,
            uncertainties: if v172 {
                vec![
                    "Since Codex 0.166.0 stdin `cwd` is the action's local working directory, in native path form, for approvals of local Bash commands, of `write_stdin` (the exec session's cwd), and of network access (the triggering command's cwd, else the action's cwd); `apply_patch`, MCP, `request_permissions`, and actions in a non-local environment report the step's local-environment cwd (falling back to the turn cwd). The hook process always starts in that step-local cwd, so stdin `cwd` and the process working directory can differ; the hooks reference still says the session cwd. Since 0.156.0 `model` and `permission_mode` come from the review context and can change within a turn.",
                    "Canonical `tool_name` values include `Bash` (local commands and network-access approvals), `apply_patch` (which also matches the `Edit` and `Write` matcher aliases), `write_stdin`, `request_permissions`, and MCP tool names; stdin keeps the canonical name. A `write_stdin` approval's `tool_input.cwd` is a `file://` URI string, not a native path. Since 0.163.0 per-subcommand zsh-fork execve escalations no longer produce PermissionRequest.",
                    BLOCKING_EXIT_2,
                    "A structured deny whose message is missing, empty, or whitespace-only is still a denial; Codex substitutes `PermissionRequest hook denied approval`.",
                    "The hooks reference calls `updatedInput`, `updatedPermissions`, and `interrupt` reserved fields that fail closed, but the pinned source treats them like `continue: false`, `stopReason`, and `suppressOutput`: any of them (`interrupt` only when `true`) fails the run and discards that handler's whole decision, allow or deny alike. Unless another matching hook decides, Codex falls back to the normal approval flow (Guardian review or the user prompt), so such an allow is not auto-approved and such a deny, for example a Claude Code style deny with `interrupt: true`, is not enforced.",
                    "Since Codex 0.165.0 a `decision.behavior: allow` from an environment-scoped managed hook (command or `mcp_tool`) has no effect: the run completes without a decision, and the request falls through to other hooks, Guardian review, or the user. A structured deny and exit 2 with non-blank stderr still deny. Stdin carries no marker of the scope, and an environment-scoped PreToolUse can still allow and rewrite.",
                ]
            } else if r2 {
                vec![
                    TOOL_HOOK_SCOPE,
                    BLOCKING_EXIT_2,
                    "A structured deny whose message is missing, empty, or whitespace-only is still a denial; Codex substitutes `PermissionRequest hook denied approval`.",
                    "The hooks reference calls `updatedInput`, `updatedPermissions`, and `interrupt` reserved fields that fail closed, but the pinned source treats them like `continue: false`, `stopReason`, and `suppressOutput`: any of them (`interrupt` only when `true`) fails the run and discards that handler's whole decision, allow or deny alike. Unless another matching hook decides, Codex falls back to the normal approval flow (Guardian review or the user prompt), so such an allow is not auto-approved and such a deny, for example a Claude Code style deny with `interrupt: true`, is not enforced.",
                ]
            } else {
                vec![
                    TOOL_HOOK_SCOPE,
                    BLOCKING_EXIT_2,
                    "A structured deny whose message is missing, empty, or whitespace-only is still a denial; Codex substitutes `PermissionRequest hook denied approval`. `updatedInput`, `updatedPermissions`, and `interrupt: true` fail closed, and `continue: false`, `stopReason`, and `suppressOutput` fail the run.",
                ]
            },
        },
        Seed {
            file_key: "post-tool-use",
            wire_name: "PostToolUse",
            rust_key: "post_tool_use",
            category: "tool",
            block_effect: Some("replace-result"),
            text_context: false,
            structured: Some(
                json!({"decision":"block","reason":"Review the output.","hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"Generated files changed."}}),
            ),
            failure_stderr_visible: false,
            block_reason_required: true,
            core_semantics: true,
            shared_input: None,
            observer: false,
            uncertainties: if v172 {
                vec![
                    "Since Codex 0.166.0 stdin `cwd` for unified-exec (`Bash`) output is the original command's local execution cwd, in native path form, even when a later `write_stdin` poll or a later turn delivers the completion; `apply_patch`, MCP, other function tools, and remote-environment commands report the step's local-environment cwd (falling back to the turn cwd). The hook process always starts in that step-local cwd, so stdin `cwd` and the process working directory can differ; the hooks reference still says the session cwd. Since 0.156.0 `model` and `permission_mode` come from the step's captured settings and can change within a turn.",
                    BLOCKING_EXIT_2,
                    BLOCK_REASON,
                    "`reason` without `decision: block` fails the run, and `suppressOutput` and `updatedMCPToolOutput` are parsed but fail the run, unless `continue` is false.",
                    "A synchronous `continue: false` does not end the turn. It marks the run stopped, takes precedence over `decision: block`, the reason rules, and the unsupported fields, and replaces the model-visible tool result with the trimmed `reason` when one is present, otherwise with `stopReason` (or `PostToolUse hook stopped execution`); the model then continues from that text. A code-mode tool promise resolves with the original tool result: the hook feedback replaces only the top-level model-visible output.",
                    "In code mode, `decision: block` or exit 2 with non-blank stderr rejects the nested tool promise with the hook reason after the tool ran.",
                    "Canonical `tool_name` values: shell commands and unified exec report `Bash`; `apply_patch` also matches the `Edit` and `Write` matcher aliases and `spawn_agent` the `Agent` alias, but stdin keeps the canonical name. A later `write_stdin` poll, possibly in another turn, can deliver the original command's PostToolUse. Hosted tools such as WebSearch never run tool hooks.",
                    TOOL_ENVIRONMENTS,
                    SPILL_CONTEXT,
                    "Tool feedback that replaces the tool result always spills at the default limit, whatever `additionalContextLimit` says.",
                ]
            } else if r2 {
                vec![
                    TOOL_HOOK_SCOPE,
                    BLOCKING_EXIT_2,
                    BLOCK_REASON,
                    "`reason` without `decision: block` fails the run, and `suppressOutput` and `updatedMCPToolOutput` are parsed but fail the run, unless `continue` is false.",
                    "A synchronous `continue: false` does not end the turn. It marks the run stopped, takes precedence over `decision: block`, the reason rules, and the unsupported fields, and replaces the model-visible tool result with the trimmed `reason` when one is present, otherwise with `stopReason` (or `PostToolUse hook stopped execution`); the model then continues from that text, and a code-mode tool promise resolves instead of rejecting.",
                ]
            } else {
                vec![
                    TOOL_HOOK_SCOPE,
                    BLOCKING_EXIT_2,
                    BLOCK_REASON,
                    "`reason` without `decision: block` fails the run unless `continue` is false; `suppressOutput` and `updatedMCPToolOutput` are parsed but fail the run.",
                ]
            },
        },
        Seed {
            file_key: "pre-compact",
            wire_name: "PreCompact",
            rust_key: "pre_compact",
            category: "compaction",
            block_effect: None,
            text_context: false,
            structured: Some(
                json!({"continue":false,"stopReason":"Save state before compacting."}),
            ),
            failure_stderr_visible: true,
            block_reason_required: false,
            core_semantics: false,
            shared_input: None,
            observer: false,
            uncertainties: if r2 {
                vec![
                    "Plain text on stdout is ignored at exit 0. A synchronous `continue: false` stops before compacting and aborts the active turn, whether the compaction is manual or automatic; Codex reports the turn as interrupted, which on the main thread also dispatches Interrupt.",
                ]
            } else {
                vec![
                    "Plain text on stdout is ignored at exit 0. A synchronous `continue: false` stops before compacting; on a manual compact this aborts the turn as interrupted, which also dispatches Interrupt.",
                ]
            },
        },
        Seed {
            file_key: "post-compact",
            wire_name: "PostCompact",
            rust_key: "post_compact",
            category: "compaction",
            block_effect: None,
            text_context: false,
            structured: Some(json!({"systemMessage":"Compaction completed."})),
            failure_stderr_visible: true,
            block_reason_required: false,
            core_semantics: false,
            shared_input: None,
            observer: false,
            uncertainties: if r2 {
                vec![
                    "Plain text on stdout is ignored at exit 0. A synchronous `continue: false` takes effect after compaction has completed, so the compacted history is kept, and aborts the active turn, whether the compaction is manual or automatic; Codex reports the turn as interrupted, which on the main thread also dispatches Interrupt.",
                ]
            } else {
                vec!["Plain text on stdout is ignored at exit 0."]
            },
        },
        Seed {
            file_key: "user-prompt-submit",
            wire_name: "UserPromptSubmit",
            rust_key: "user_prompt_submit",
            category: "prompt",
            block_effect: Some("deny"),
            text_context: true,
            structured: Some(
                json!({"decision":"block","reason":"Ask for confirmation.","hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"Clarify the reproduction."}}),
            ),
            failure_stderr_visible: false,
            block_reason_required: true,
            core_semantics: false,
            shared_input: None,
            observer: false,
            uncertainties: if v172 {
                vec![
                    TEXT_CONTEXT,
                    BLOCKING_EXIT_2,
                    BLOCK_REASON,
                    MATCHER_IGNORED,
                    SPILL_CONTEXT,
                ]
            } else {
                vec![TEXT_CONTEXT, BLOCKING_EXIT_2, BLOCK_REASON]
            },
        },
        Seed {
            file_key: "subagent-stop",
            wire_name: "SubagentStop",
            rust_key: "subagent_stop",
            category: "subagent",
            block_effect: Some("continue"),
            text_context: false,
            structured: Some(json!({"decision":"block","reason":"Run another focused pass."})),
            failure_stderr_visible: false,
            block_reason_required: true,
            core_semantics: false,
            shared_input: None,
            observer: false,
            uncertainties: {
                let mut uncertainties = vec![
                    "Synchronous handlers must write JSON or nothing at exit 0; plain text fails the run.",
                    BLOCKING_EXIT_2,
                    BLOCK_REASON,
                ];
                if v172 {
                    uncertainties.push(SPILL_CONTINUATION);
                }
                uncertainties
            },
        },
        Seed {
            file_key: "stop",
            wire_name: "Stop",
            rust_key: "stop",
            category: "turn",
            block_effect: Some("continue"),
            text_context: false,
            structured: Some(json!({"decision":"block","reason":"Run the failing tests again."})),
            failure_stderr_visible: false,
            block_reason_required: true,
            core_semantics: true,
            shared_input: None,
            observer: false,
            uncertainties: if v172 {
                vec![
                    "Synchronous handlers must write JSON or nothing at exit 0; plain text fails the run.",
                    BLOCKING_EXIT_2,
                    BLOCK_REASON,
                    "Since Codex 0.150.0 managed-source Stop hooks (system, MDM, cloud, and legacy managed configuration, and since 0.165.0 environment-scoped managed hooks) also fire for internal memory-consolidation turns with an ordinary Stop payload; user, project, session-flag, and plugin Stop hooks do not, and a block there ends the memory worker with an error instead of continuing.",
                    "The pinned core source dispatches Interrupt, not Stop, from the path that aborts an interrupted turn; this has not been live-observed.",
                    MATCHER_IGNORED,
                    SPILL_CONTINUATION,
                ]
            } else {
                vec![
                    "Synchronous handlers must write JSON or nothing at exit 0; plain text fails the run.",
                    BLOCKING_EXIT_2,
                    BLOCK_REASON,
                    "Since Codex 0.150.0 managed-source Stop hooks (system, MDM, cloud, and legacy managed configuration) also fire for internal memory-consolidation turns with an ordinary Stop payload; user, project, session-flag, and plugin Stop hooks do not, and a block there ends the memory worker with an error instead of continuing.",
                    "The pinned core source dispatches Interrupt, not Stop, from the path that aborts an interrupted turn; this has not been live-observed.",
                ]
            },
        },
        Seed {
            file_key: "interrupt",
            wire_name: "Interrupt",
            rust_key: "interrupt",
            category: "turn",
            block_effect: None,
            text_context: false,
            structured: Some(
                json!({"systemMessage":"Saved the interrupted turn to the local audit log."}),
            ),
            failure_stderr_visible: false,
            block_reason_required: false,
            core_semantics: true,
            shared_input: None,
            observer: false,
            uncertainties: vec![
                "Interrupt (Codex >= 0.150.0) runs for an active main-thread turn aborted as interrupted, after Codex flushes the transcript and before it reports the aborted turn. It never runs for subagents or idle threads, ignores any configured matcher, and carries no agent_id or agent_type.",
                if r2 {
                    "The hooks reference describes user interrupts only; the pinned core source and tests also dispatch Interrupt for self-aborts reported as interrupted, such as a turn aborted by a PreCompact or PostCompact `continue: false`."
                } else {
                    "The hooks reference describes user interrupts only; the pinned core source and tests also dispatch Interrupt for self-aborts reported as interrupted, such as a manual compact stopped by a PreCompact `continue: false`."
                },
                "Output cannot prevent the interruption or restart the turn. Codex accepts empty stdout or a JSON object whose only member is `systemMessage`, which surfaces as a warning; plain text, any other member (including continue, stopReason, suppressOutput, and decision), and every non-zero exit (including 2) fail the run.",
                "The runtime deserializes `systemMessage` as an optional string and therefore also accepts null, but the generated schema declares a string; emit a string or omit the member.",
                if v172 {
                    "Command and `mcp_tool` handlers default to a one-second timeout clamped to one through three seconds, including when `async` is true. This corrects commit-ff6aec9-r2, which named only command handlers although the clamp applies to both at that revision too; the hooks reference's MCP field table still gives the 600-second default."
                } else {
                    "Command handlers default to a one-second timeout clamped to one through three seconds, including when `async` is true."
                },
            ]
            .into_iter()
            .chain(v172.then_some(
                "Environment-scoped managed hooks of the interrupted turn's last-known step environments run as well; none run when the turn has no last-known step context.",
            ))
            .collect(),
        },
    ];
    if v172 {
        seeds.push(Seed {
            file_key: "pre-model-request",
            wire_name: "PreModelRequest",
            rust_key: "pre_model_request",
            category: "model",
            block_effect: None,
            text_context: false,
            structured: None,
            failure_stderr_visible: true,
            block_reason_required: false,
            core_semantics: true,
            shared_input: Some("model-request"),
            observer: true,
            uncertainties: model_request_uncertainties(vec![
                "PreModelRequest (Codex >= 0.164.0) runs before each attempt to open a model response stream for a turn request, and since 0.171.0 also for a compaction request, on the main thread and on subagent threads.",
                OBSERVER_STDOUT,
                "A synchronous PreModelRequest handler delays the request until it finishes, for up to its timeout.",
            ]),
        });
        seeds.push(Seed {
            file_key: "post-model-request",
            wire_name: "PostModelRequest",
            rust_key: "post_model_request",
            category: "model",
            block_effect: None,
            text_context: false,
            structured: None,
            failure_stderr_visible: true,
            block_reason_required: false,
            core_semantics: true,
            shared_input: Some("model-request"),
            observer: true,
            uncertainties: model_request_uncertainties(vec![
                "PostModelRequest (Codex >= 0.164.0) reports the outcome of each attempt to open a model response stream for a turn request, and since 0.171.0 also for a compaction request, as `completed`, `failed`, or `cancelled`.",
                OBSERVER_STDOUT,
                "Status rules from the pinned source, which the schema does not encode: `completed` has a null `error` and the response's `response_id`, with `token_usage` the provider usage or null; `failed` carries the error's display text in `error` (for example `stream disconnected before completion: stream closed before response.completed` when the stream ends early), and only an incomplete response (`Incomplete response returned, reason: ...`) can also carry `response_id` and `token_usage` (each null when the provider reported none), which are null for every other failure; `cancelled` (a turn abort, an interrupt, or an attempt dropped before it settled) has all three null. `token_usage` always includes `cache_write_input_tokens` although the schema marks it optional.",
                "A synchronous PostModelRequest handler holds back the attempt's terminal event (completed or error) and, under `response_process_mode` `AfterCompleteResponse`, its buffered items until it finishes; in the default mode streaming output and tool dispatch proceed. When the turn stops waiting, unfinished synchronous runs are marked stopped (`Hook cancelled because the turn was interrupted`). An attempt dropped before it settled schedules its `cancelled` PostModelRequest on the background executor even for synchronous handlers; that run shares the eight-hook background limit and is aborted at shutdown, so it may never happen.",
                "A synchronous, non-executor `mcp_tool` handler whose run completes may price a billable attempt (`completed`, or `failed` with `token_usage`) by returning `structuredContent` `{\"extra_codex_rollout_budget_units\": <finite number >= 0>}` (default 0); a non-object, a wrong type, or a negative or non-finite value is an invalid price that makes usage posting fail, while sibling prices are kept. Command handlers can never price, so stdout such as `{\"extra_codex_rollout_budget_units\":3}` is ignored. The hooks reference does not document pricing.",
            ]),
        });
    }
    seeds
}

/// The event-specific `leading` uncertainties of a model-request event,
/// followed by those both model-request events share.
fn model_request_uncertainties(mut leading: Vec<&'static str>) -> Vec<&'static str> {
    leading.extend([
        OBSERVER_PAIRING,
        OBSERVER_KIND,
        OBSERVER_FIELDS,
        OBSERVER_MEMORY,
        OBSERVER_OPEN_SCHEMA,
    ]);
    leading
}

/// Events whose `matcher` Codex applies (`HOOK_EVENT_NAMES_WITH_MATCHERS`).
fn honors_matcher(seed: &Seed) -> bool {
    matches!(
        seed.wire_name,
        "PreToolUse"
            | "PermissionRequest"
            | "PostToolUse"
            | "PreCompact"
            | "PostCompact"
            | "SessionStart"
            | "SessionEnd"
            | "SubagentStart"
            | "SubagentStop"
    )
}

/// Events that can fire on a subagent thread, where `session_id` names the
/// root session.
fn fires_on_subagent_threads(seed: &Seed) -> bool {
    matches!(
        seed.wire_name,
        "PreToolUse"
            | "PermissionRequest"
            | "PostToolUse"
            | "PreCompact"
            | "PostCompact"
            | "UserPromptSubmit"
            | "SubagentStart"
            | "SubagentStop"
            | "PreModelRequest"
            | "PostModelRequest"
    )
}

fn generate(root: &Path, snapshot: Snapshot, seed: Seed) {
    let vendor = root
        .join("contracts/vendor/codex")
        .join(snapshot.revision())
        .join("generated");
    let event_dir = root
        .join("contracts/harnesses/codex/snapshots")
        .join(snapshot.id())
        .join("events")
        .join(seed.file_key);
    fs::create_dir_all(&event_dir).expect("create Codex event directory");

    let mut input = match seed.shared_input {
        Some(shared) => split_event_branch(
            read_json(&vendor.join(format!("{shared}.command.input.schema.json"))),
            seed.wire_name,
        ),
        None => read_json(&vendor.join(format!("{}.command.input.schema.json", seed.file_key))),
    };
    normalize_schema(&mut input, snapshot, &seed, "input");
    write_json(&event_dir.join("input.schema.json"), &input);

    if seed.structured.is_some() {
        let mut output =
            read_json(&vendor.join(format!("{}.command.output.schema.json", seed.file_key)));
        normalize_schema(&mut output, snapshot, &seed, "command-output");
        if seed.block_reason_required {
            require_block_reason(&mut output);
        }
        write_json(&event_dir.join("output.command.schema.json"), &output);
    }

    let contract = contract(snapshot, &seed, &input);
    write_yaml(&event_dir.join("contract.yaml"), &contract);
    let fixtures = if seed.observer {
        model_request_fixtures(&seed, &input)
    } else {
        fixtures(snapshot, &seed, &input)
    };
    write_yaml(&event_dir.join("fixtures.yaml"), &fixtures);
}

/// Derives one event's input schema from a generated schema that several
/// events share: its root holds their common members, and a root `oneOf`
/// keyed on `hook_event_name` holds each event's own members. The event's
/// branch is merged into the root, `hook_event_name` becomes a required
/// constant like in every other generated input schema, and the `oneOf` is
/// dropped. Everything else, including the absence of
/// `additionalProperties: false`, stays as vendored.
fn split_event_branch(mut schema: Value, wire_name: &str) -> Value {
    let object = schema.as_object_mut().expect("official schema object");
    let Some(Value::Array(branches)) = object.remove("oneOf") else {
        panic!("a shared input schema needs a root oneOf");
    };
    let mut matching = branches
        .into_iter()
        .filter(|branch| branch["properties"]["hook_event_name"]["enum"] == json!([wire_name]));
    let branch = matching
        .next()
        .unwrap_or_else(|| panic!("no oneOf branch for {wire_name}"));
    assert!(
        matching.next().is_none(),
        "several oneOf branches for {wire_name}"
    );
    assert_eq!(
        branch.get("type"),
        object.get("type"),
        "{wire_name} branch type"
    );
    let properties = object
        .get_mut("properties")
        .and_then(Value::as_object_mut)
        .expect("root properties");
    for (name, property) in branch["properties"].as_object().expect("branch properties") {
        let property = if name == "hook_event_name" {
            json!({"const": wire_name, "type": "string"})
        } else {
            property.clone()
        };
        assert!(
            properties.insert(name.clone(), property).is_none(),
            "{wire_name} branch member {name} shadows a shared member"
        );
    }
    let required: BTreeSet<&str> = [&object["required"], &branch["required"]]
        .into_iter()
        .flat_map(|required| required.as_array().expect("required array"))
        .map(|name| name.as_str().expect("required name"))
        .collect();
    let required = json!(required);
    object.insert("required".to_string(), required);
    schema
}

fn normalize_schema(schema: &mut Value, snapshot: Snapshot, seed: &Seed, suffix: &str) {
    let object = schema.as_object_mut().expect("official schema object");
    object.insert(
        "$schema".to_string(),
        Value::String("https://json-schema.org/draft/2020-12/schema".to_string()),
    );
    object.insert(
        "$id".to_string(),
        Value::String(format!(
            "urn:agent-hook-kit:contracts:codex:{}:{}:{suffix}",
            snapshot.id(),
            seed.file_key
        )),
    );
}

/// Narrows a generated output schema to the synchronous runtime rule that a
/// structured block needs a reason that is non-empty after trimming.
fn require_block_reason(schema: &mut Value) {
    let object = schema.as_object_mut().expect("official schema object");
    assert!(
        object["properties"].get("decision").is_some() && !object.contains_key("allOf"),
        "block-reason narrowing needs a root decision property and no existing allOf"
    );
    object.insert(
        "allOf".to_string(),
        // `continue: false` takes precedence over the block, so only a
        // continuing block needs a reason.
        json!([{
            "if": {"properties": {"decision": {"const": "block"}, "continue": {"const": true}}, "required": ["decision"]},
            "then": {"properties": {"reason": {"type": "string", "pattern": "\\S"}}, "required": ["reason"]}
        }]),
    );
}

fn is_session_end(seed: &Seed) -> bool {
    seed.wire_name == "SessionEnd"
}

fn ignored_stdout() -> Value {
    json!({"presence":"optional","role":"ignored","content_kind":"opaque"})
}

fn ignored_stderr() -> Value {
    json!({"presence":"optional","role":"ignored","content_kind":"opaque"})
}

fn diagnostic_stderr() -> Value {
    json!({"presence":"optional","role":"diagnostics","content_kind":"text","encoding":"utf-8"})
}

fn contract(snapshot: Snapshot, seed: &Seed, input_schema: &Value) -> Value {
    let v172 = snapshot >= Snapshot::Commit3d86e43R1;
    let mut outcomes = Vec::new();
    if seed.structured.is_some() {
        outcomes.push(json!({
            "id":"structured",
            "effect":"event-specific-control",
            "exit":{"exact":0},
            "stdout":{"presence":"required","role":"protocol-value","content_kind":"json"},
            "stderr":ignored_stderr(),
            "output_schema":"command-response",
            "sources":["codex-hooks-reference","codex-generated-schemas","codex-hooks-source"],
            "assurance":{"confidence":"high","verification":"source-reviewed"}
        }));
    }
    outcomes.push(json!({
        "id":"no-op",
        "effect":"none",
        "exit":{"exact":0},
        "stdout": if seed.structured.is_some() {
            json!({"presence":"optional","role":"none","content_kind":"empty"})
        } else {
            ignored_stdout()
        },
        "stderr": if seed.observer {
            diagnostic_stderr()
        } else {
            ignored_stderr()
        },
        "sources":["codex-hooks-reference","codex-hooks-source"],
        "assurance":{"confidence":"high","verification":"source-reviewed"}
    }));
    if seed.text_context {
        outcomes.push(json!({
            "id":"text-context",
            "effect":"provide-context",
            "exit":{"exact":0},
            "stdout":{"presence":"required","role":"agent-context","content_kind":"text","encoding":"utf-8","trailing_newline":"allowed"},
            "stderr":ignored_stderr(),
            "sources":["codex-hooks-reference","codex-hooks-source"],
            "assurance":{"confidence":"high","verification":"source-reviewed"}
        }));
    }
    if let Some(effect) = seed.block_effect {
        outcomes.push(json!({
            "id":"exit-2",
            "effect":effect,
            "exit":{"exact":2},
            "stdout":ignored_stdout(),
            "stderr":{"presence":"required","role":"agent-context","content_kind":"text","encoding":"utf-8"},
            "sources":["codex-hooks-reference","codex-hooks-source"],
            "assurance":{"confidence":"high","verification":"source-reviewed"}
        }));
    }
    outcomes.push(json!({
        "id":"failure",
        "effect":"nonblocking-error",
        "exit":{"range":{"min":1,"max":255}},
        "stdout":ignored_stdout(),
        "stderr": if seed.failure_stderr_visible {
            diagnostic_stderr()
        } else {
            ignored_stderr()
        },
        "sources":["codex-hooks-reference","codex-hooks-source"],
        "assurance":{"confidence":"high","verification":"source-reviewed"}
    }));

    let mut input_sources = vec!["codex-hooks-reference", "codex-generated-schemas"];
    if seed.shared_input.is_some() {
        // The oneOf split follows the event enum in the hooks source.
        input_sources.push("codex-hooks-source");
    }
    if seed.core_semantics {
        input_sources.push("codex-core-source");
    }
    let mut output_sources = vec!["codex-hooks-reference", "codex-generated-schemas"];
    if seed.block_reason_required {
        output_sources.push("codex-hooks-source");
    }

    let mut uncertainties: Vec<&str> = seed.uncertainties.clone();
    uncertainties.push(COMPATIBILITY_FIELDS);
    uncertainties.push(if seed.observer {
        OBSERVER_STDERR
    } else if v172 {
        EXIT_ZERO_STDERR_SCOPED
    } else {
        EXIT_ZERO_STDERR
    });
    uncertainties.push(if seed.observer {
        OBSERVER_FAILURE
    } else if seed.failure_stderr_visible {
        VISIBLE_FAILURE
    } else {
        SILENT_FAILURE
    });
    if seed.observer {
        uncertainties.push(OBSERVER_DISPATCH);
        uncertainties.push(OBSERVER_HANDLER_KINDS);
    } else if !is_session_end(seed) {
        uncertainties.push(ASYNC_HANDLERS);
        uncertainties.push(if v172 {
            HANDLER_KINDS_SCOPED
        } else {
            HANDLER_KINDS
        });
    }
    if v172 {
        uncertainties.push(PROCESS_LIFECYCLE_LOCAL);
        uncertainties.push(CLOUD_ORCHESTRATION_GLOBAL);
        uncertainties.push(ENVIRONMENT_SCOPED);
        uncertainties.push(PLUGIN_TRUST);
        if honors_matcher(seed) {
            uncertainties.push(MATCHER_RULE);
        }
        if input_schema["properties"].get("permission_mode").is_some() {
            uncertainties.push(PERMISSION_MODE_VALUES);
        }
        if fires_on_subagent_threads(seed) {
            uncertainties.push(ROOT_SESSION);
        }
    } else {
        uncertainties.push(PROCESS_LIFECYCLE);
        uncertainties.push(CLOUD_ORCHESTRATION);
    }

    json!({
        "format_version":1,
        "id":format!("codex/{}/{}",snapshot.id(),seed.wire_name),
        "harness":"codex",
        "snapshot":snapshot.id(),
        "event":{
            "wire_name":seed.wire_name,
            "rust_key":seed.rust_key,
            "category":seed.category,
            "identification":{
                "inferability":"definitive",
                "discriminator":{"json_pointer":"/hook_event_name","const":seed.wire_name}
            }
        },
        "schemas":{
            "input":{"file":"input.schema.json","origin":"derived","sources":input_sources,"assurance":{"confidence":"high","verification":"source-reviewed"}},
            "outputs": if seed.structured.is_some() {
                vec![json!({"id":"command-response","file":"output.command.schema.json","origin":"derived","sources":output_sources,"assurance":{"confidence":"high","verification":"source-reviewed"}})]
            } else {
                Vec::<Value>::new()
            }
        },
        "bindings":{
            "command":{
                "kind":"process",
                "request":{"channel":"stdin","framing":"single-document-at-eof","content_kind":"json"},
                "outcomes":outcomes
            }
        },
        "handler_kinds": if is_session_end(seed) { json!(["command"]) } else { json!(["command","mcp_tool"]) },
        "fixtures":"fixtures.yaml",
        "uncertainties":uncertainties
    })
}

fn fixtures(snapshot: Snapshot, seed: &Seed, input_schema: &Value) -> Value {
    let v172 = snapshot >= Snapshot::Commit3d86e43R1;
    let minimal = example_input(input_schema);
    let mut representative = minimal.clone();
    let representative_object = representative.as_object_mut().expect("input object");
    let properties = input_schema["properties"].as_object().expect("properties");
    if properties.contains_key("agent_id") {
        representative_object.insert("agent_id".to_string(), json!("agent-1"));
    }
    if properties.contains_key("agent_type") {
        representative_object.insert("agent_type".to_string(), json!("reviewer"));
    }
    if representative_object.get("transcript_path") == Some(&Value::Null) {
        representative_object.insert(
            "transcript_path".to_string(),
            json!("/tmp/transcript.jsonl"),
        );
    }

    let mut positive = vec![
        json!({"id":"minimal","origin":"synthesized","sources":["codex-generated-schemas"],"value":minimal}),
        json!({"id":"representative","origin":"synthesized","sources":["codex-hooks-reference","codex-generated-schemas"],"value":representative}),
    ];
    if seed.wire_name == "SessionStart" {
        let mut fork = representative.clone();
        fork["source"] = json!("fork");
        positive.push(json!({
            "id":"fork-source",
            "origin":"regression",
            "sources":["codex-generated-schemas","codex-core-source"],
            "value":fork
        }));
    }
    if v172 {
        positive.extend(cwd_and_scope_positives(seed, &minimal, &representative));
    }

    let mut wrong = minimal.clone();
    wrong["hook_event_name"] = json!("WrongEvent");
    let required = input_schema["required"].as_array().expect("required array");
    let common = [
        "cwd",
        "hook_event_name",
        "model",
        "permission_mode",
        "session_id",
        "transcript_path",
        "turn_id",
    ];
    let missing = if seed.wire_name == "Interrupt" {
        // Every Interrupt field is shared; the interrupted turn id is the
        // event's reason to exist.
        "turn_id"
    } else {
        required
            .iter()
            .filter_map(Value::as_str)
            .find(|field| !common.contains(field))
            .unwrap_or_else(|| required[0].as_str().expect("required name"))
    };
    let mut missing_value = minimal.clone();
    missing_value
        .as_object_mut()
        .expect("input object")
        .remove(missing);
    let mut negative = vec![
        json!({"id":"wrong-discriminator","origin":"regression","sources":["codex-generated-schemas"],"value":wrong,"expected_pointer":"/hook_event_name","expected_keyword":"const"}),
        json!({"id":format!("missing-{missing}"),"origin":"synthesized","sources":["codex-generated-schemas"],"value":missing_value,"expected_pointer":"","expected_keyword":"required"}),
    ];
    if seed.wire_name == "SessionEnd" {
        let mut invalid_reason = minimal.clone();
        invalid_reason["reason"] = json!("exit");
        negative.push(json!({
            "id":"invalid-reason",
            "origin":"regression",
            "sources":["codex-hooks-reference","codex-generated-schemas"],
            "value":invalid_reason,
            "expected_pointer":"/reason",
            "expected_keyword":"const"
        }));
    }
    if seed.wire_name == "SessionStart" {
        let mut unknown_source = minimal.clone();
        unknown_source["source"] = json!("restore");
        negative.push(json!({
            "id":"unknown-source",
            "origin":"regression",
            "sources":["codex-generated-schemas"],
            "value":unknown_source,
            "expected_pointer":"/source",
            "expected_keyword":"enum"
        }));
    }

    let mut output = Vec::new();
    let mut output_negative = Vec::new();
    let mut process = Vec::new();
    if let Some(structured) = &seed.structured {
        let structured_bytes = serde_json::to_vec(structured).expect("serialize output");
        let (origin, sources) = if seed.wire_name == "Interrupt" {
            // Verbatim example from the hooks reference.
            ("official", json!(["codex-hooks-reference"]))
        } else {
            (
                "synthesized",
                json!(["codex-hooks-reference", "codex-generated-schemas"]),
            )
        };
        output.push(json!({
            "id":"structured",
            "schema":"command-response",
            "origin":origin,
            "sources":sources,
            "value":structured
        }));
        process.push(process_case(
            "structured",
            "structured",
            0,
            &structured_bytes,
            b"",
        ));
        for (id, value) in documented_outputs(seed) {
            let bytes = serde_json::to_vec(&value).expect("serialize output");
            output.push(json!({
                "id":id,
                "schema":"command-response",
                "origin":"official",
                "sources":["codex-hooks-reference"],
                "value":value
            }));
            process.push(process_case(
                &format!("structured-{id}"),
                "structured",
                0,
                &bytes,
                b"",
            ));
        }
    }
    process.push(process_case("no-op", "no-op", 0, b"", b""));
    if seed.text_context {
        process.push(process_case(
            "text-context",
            "text-context",
            0,
            b"Hook-provided developer context.",
            b"",
        ));
    }
    if seed.block_effect.is_some() {
        process.push(process_case("exit-2", "exit-2", 2, b"", b"blocked by hook"));
        process.push(process_case(
            "exit-2-blank-stderr",
            "failure",
            2,
            b"",
            BLANK_STDERR,
        ));
    } else if seed.structured.is_some() {
        process.push(process_case(
            "exit-2-failure",
            "failure",
            2,
            b"",
            FAILURE_STDERR,
        ));
    }
    process.push(process_case("failure", "failure", 1, b"", FAILURE_STDERR));

    if seed.block_reason_required {
        output_negative.push(json!({
            "id":"block-without-reason",
            "schema":"command-response",
            "origin":"regression",
            "sources":["codex-hooks-source"],
            "value":{"decision":"block"},
            "expected_pointer":"",
            "expected_keyword":"required"
        }));
        output_negative.push(json!({
            "id":"block-blank-reason",
            "schema":"command-response",
            "origin":"regression",
            "sources":["codex-hooks-source"],
            "value":{"decision":"block","reason":" \t"},
            "expected_pointer":"/reason",
            "expected_keyword":"pattern"
        }));
    }
    if seed.wire_name == "Interrupt" {
        output_negative.push(json!({
            "id":"universal-continue",
            "schema":"command-response",
            "origin":"regression",
            "sources":["codex-generated-schemas","codex-hooks-source"],
            "value":{"continue":true},
            "expected_pointer":"",
            "expected_keyword":"additionalProperties"
        }));
        output_negative.push(json!({
            "id":"decision-block",
            "schema":"command-response",
            "origin":"regression",
            "sources":["codex-generated-schemas","codex-hooks-source"],
            "value":{"decision":"block","reason":"Keep going."},
            "expected_pointer":"",
            "expected_keyword":"additionalProperties"
        }));
    }

    let mut fixtures = json!({
        "format_version":1,
        "input":{
            "positive":positive,
            "negative":negative
        },
        "output":output,
        "process":process
    });
    if !output_negative.is_empty() {
        fixtures["output_negative"] = Value::Array(output_negative);
    }
    fixtures
}

/// Positive inputs of `commit-3d86e43-r1` and later that pin input semantics
/// the schema cannot express: compaction dispatch, a tool event's local
/// execution cwd (Codex >= 0.166.0), and environment-scoped rewriting
/// (Codex >= 0.165.0).
fn cwd_and_scope_positives(seed: &Seed, minimal: &Value, representative: &Value) -> Vec<Value> {
    let mut positive = Vec::new();
    match seed.wire_name {
        "SessionStart" => {
            let mut compact = minimal.clone();
            compact["source"] = json!("compact");
            positive.push(json!({
                "id":"compact-source",
                "origin":"synthesized",
                "sources":["codex-hooks-reference","codex-generated-schemas","codex-core-source"],
                "value":compact
            }));
        }
        "PermissionRequest" => {
            let mut write_stdin = minimal.clone();
            write_stdin["cwd"] = json!("/repo/nested");
            write_stdin["tool_name"] = json!("write_stdin");
            write_stdin["tool_input"] = json!({
                "session_id":7,
                "chars":"y\n",
                "parent_call_id":"call-1",
                "approval_id":"approval-1",
                "environment_id":"local",
                "cwd":"file:///repo/nested",
                "tty":true,
                "sandbox_permissions":"use_default",
                "additional_permissions":null
            });
            positive.push(json!({
                "id":"write-stdin-approval",
                "origin":"synthesized",
                "sources":["codex-generated-schemas","codex-hooks-source","codex-core-source"],
                "value":write_stdin
            }));
        }
        "PostToolUse" => {
            let mut delayed = minimal.clone();
            delayed["cwd"] = json!("/repo/nested");
            delayed["tool_input"] = json!({"command":"cat cwd-marker.txt"});
            delayed["tool_response"] = json!("nested:cwd-marker.txt");
            positive.push(json!({
                "id":"delayed-completion-cwd",
                "origin":"synthesized",
                "sources":["codex-generated-schemas","codex-hooks-source","codex-core-source"],
                "value":delayed
            }));
        }
        "SubagentStop" => {
            let mut scoped = representative.clone();
            scoped["cwd"] = json!("C:\\work\\repo");
            scoped["transcript_path"] = Value::Null;
            scoped["agent_transcript_path"] = Value::Null;
            positive.push(json!({
                "id":"environment-scoped",
                "origin":"synthesized",
                "sources":["codex-generated-schemas","codex-hooks-source"],
                "value":scoped
            }));
        }
        _ => {}
    }
    positive
}

/// Fixtures of an observation-only model-request event. The two events
/// share one generated schema, so positives also pin each status's value
/// rules from the pinned source, and process cases pin that stdout is never
/// parsed and that stderr is visible at every exit code.
fn model_request_fixtures(seed: &Seed, input_schema: &Value) -> Value {
    let post = seed.wire_name == "PostModelRequest";
    let mut minimal = example_input(input_schema);
    if post {
        minimal["status"] = json!("cancelled");
    }
    let mut subagent = minimal.clone();
    subagent["transcript_path"] = json!("/tmp/transcript.jsonl");
    subagent["thread_id"] = json!("thread-2");
    subagent["agent_id"] = json!("thread-2");
    subagent["agent_type"] = json!("reviewer");
    let token_usage = json!({
        "input_tokens":1200,
        "cached_input_tokens":800,
        "cache_write_input_tokens":0,
        "output_tokens":150,
        "reasoning_output_tokens":64,
        "total_tokens":1350
    });

    let positive_fixture = |id: &str, sources: Value, value: Value| json!({"id":id,"origin":"synthesized","sources":sources,"value":value});
    let mut positive = vec![positive_fixture(
        "minimal",
        json!(["codex-generated-schemas", "codex-hooks-source"]),
        minimal.clone(),
    )];
    if post {
        let mut completed = subagent.clone();
        completed["status"] = json!("completed");
        completed["response_id"] = json!("resp_123");
        completed["token_usage"] = token_usage.clone();
        positive.push(positive_fixture(
            "representative",
            json!([
                "codex-hooks-reference",
                "codex-generated-schemas",
                "codex-hooks-source"
            ]),
            completed,
        ));
        let mut without_usage = minimal.clone();
        without_usage["status"] = json!("completed");
        without_usage["response_id"] = json!("resp_789");
        positive.push(positive_fixture(
            "completed-without-usage",
            json!(["codex-generated-schemas", "codex-hooks-source"]),
            without_usage,
        ));
        let mut incomplete = minimal.clone();
        incomplete["kind"] = json!("foreground_compaction");
        incomplete["attempt_number"] = json!(2);
        incomplete["status"] = json!("failed");
        incomplete["error"] = json!("Incomplete response returned, reason: max_output_tokens");
        incomplete["response_id"] = json!("resp_456");
        let mut legacy_usage = token_usage.clone();
        legacy_usage
            .as_object_mut()
            .expect("token usage")
            .remove("cache_write_input_tokens");
        incomplete["token_usage"] = legacy_usage;
        positive.push(positive_fixture(
            "failed-incomplete",
            json!(["codex-generated-schemas", "codex-hooks-source"]),
            incomplete,
        ));
        let mut transport = minimal.clone();
        transport["status"] = json!("failed");
        transport["error"] =
            json!("stream disconnected before completion: stream closed before response.completed");
        positive.push(positive_fixture(
            "failed-transport",
            json!([
                "codex-generated-schemas",
                "codex-hooks-source",
                "codex-core-source"
            ]),
            transport,
        ));
    } else {
        let mut compaction_retry = subagent.clone();
        compaction_retry["kind"] = json!("foreground_compaction");
        compaction_retry["attempt_number"] = json!(2);
        positive.push(positive_fixture(
            "representative",
            json!([
                "codex-hooks-reference",
                "codex-generated-schemas",
                "codex-hooks-source"
            ]),
            compaction_retry,
        ));
        let mut background = minimal.clone();
        background["kind"] = json!("background_compaction");
        positive.push(positive_fixture(
            "background-compaction",
            json!(["codex-generated-schemas", "codex-core-source"]),
            background,
        ));
    }

    let negative_fixture = |id: &str, origin: &str, value: Value, pointer: &str, keyword: &str| {
        json!({
            "id":id,
            "origin":origin,
            "sources":["codex-generated-schemas"],
            "value":value,
            "expected_pointer":pointer,
            "expected_keyword":keyword
        })
    };
    let with = |member: &str, value: Value| {
        let mut changed = minimal.clone();
        changed[member] = value;
        changed
    };
    let without = |member: &str| {
        let mut changed = minimal.clone();
        changed
            .as_object_mut()
            .expect("input object")
            .remove(member);
        changed
    };
    let mut negative = vec![negative_fixture(
        "wrong-discriminator",
        "regression",
        with("hook_event_name", json!("WrongEvent")),
        "/hook_event_name",
        "const",
    )];
    if post {
        let mut missing_total = token_usage.clone();
        missing_total
            .as_object_mut()
            .expect("token usage")
            .remove("total_tokens");
        negative.extend([
            negative_fixture(
                "missing-response_id",
                "synthesized",
                without("response_id"),
                "",
                "required",
            ),
            negative_fixture(
                "missing-token_usage",
                "synthesized",
                without("token_usage"),
                "",
                "required",
            ),
            negative_fixture(
                "unknown-status",
                "synthesized",
                with("status", json!("timeout")),
                "/status",
                "enum",
            ),
            negative_fixture(
                "error-number",
                "synthesized",
                with("error", json!(5)),
                "/error",
                "type",
            ),
            negative_fixture(
                "token_usage-missing-total",
                "synthesized",
                with("token_usage", missing_total),
                "/token_usage",
                "anyOf",
            ),
        ]);
    } else {
        negative.extend([
            negative_fixture(
                "missing-request_id",
                "synthesized",
                without("request_id"),
                "",
                "required",
            ),
            negative_fixture(
                "missing-attempt_number",
                "synthesized",
                without("attempt_number"),
                "",
                "required",
            ),
            negative_fixture(
                "attempt_number-string",
                "synthesized",
                with("attempt_number", json!("1")),
                "/attempt_number",
                "type",
            ),
            negative_fixture(
                "unknown-kind",
                "synthesized",
                with("kind", json!("warmup")),
                "/kind",
                "enum",
            ),
            negative_fixture(
                "cwd-number",
                "synthesized",
                with("cwd", json!(1)),
                "/cwd",
                "type",
            ),
        ]);
    }

    let mut process = vec![
        process_case("no-op", "no-op", 0, b"", b""),
        process_case(
            "exit-0-control-json-ignored",
            "no-op",
            0,
            br#"{"continue":false,"decision":"block","reason":"ignored-observer-control"}"#,
            b"",
        ),
        process_case("exit-0-invalid-json-ignored", "no-op", 0, b"{not json", b""),
        process_case(
            "exit-0-stderr-diagnostics",
            "no-op",
            0,
            b"",
            b"slow log sink\n",
        ),
    ];
    if post {
        process.push(process_case(
            "exit-0-price-ignored",
            "no-op",
            0,
            br#"{"extra_codex_rollout_budget_units":3}"#,
            b"",
        ));
    }
    process.extend([
        process_case("exit-2-failure", "failure", 2, b"", b"block this request\n"),
        process_case("exit-1-blank-stderr", "failure", 1, b"", BLANK_STDERR),
        process_case("failure", "failure", 1, b"", FAILURE_STDERR),
    ]);

    json!({
        "format_version":1,
        "input":{
            "positive":positive,
            "negative":negative
        },
        "output":[],
        "process":process
    })
}

/// Verbatim hooks-reference examples of structured shapes that the
/// representative `structured` fixture does not already cover.
fn documented_outputs(seed: &Seed) -> Vec<(&'static str, Value)> {
    match seed.wire_name {
        "PermissionRequest" => vec![(
            "allow",
            json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}),
        )],
        "UserPromptSubmit" => vec![
            (
                "context",
                json!({"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"Ask for a clearer reproduction before editing files."}}),
            ),
            (
                "block",
                json!({"decision":"block","reason":"Ask for confirmation before doing that."}),
            ),
        ],
        _ => Vec::new(),
    }
}

fn process_case(id: &str, outcome: &str, exit_code: i32, stdout: &[u8], stderr: &[u8]) -> Value {
    json!({
        "id":id,
        "binding":"command",
        "outcome":outcome,
        "exit_code":exit_code,
        "stdout_base64":base64::engine::general_purpose::STANDARD.encode(stdout),
        "stdout_sha256":sha256(stdout),
        "stderr_base64":base64::engine::general_purpose::STANDARD.encode(stderr),
        "stderr_sha256":sha256(stderr)
    })
}

fn example_input(schema: &Value) -> Value {
    let properties = schema["properties"].as_object().expect("properties");
    let definitions = schema
        .get("definitions")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut value = Map::new();
    for name in schema["required"].as_array().expect("required") {
        let name = name.as_str().expect("required string");
        value.insert(
            name.to_string(),
            example_value(name, &properties[name], &definitions),
        );
    }
    Value::Object(value)
}

fn example_value(name: &str, schema: &Value, definitions: &Map<String, Value>) -> Value {
    if let Some(value) = schema.get("const") {
        return value.clone();
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return values.first().cloned().unwrap_or(Value::Null);
    }
    if let Some(key) = schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| reference.strip_prefix("#/definitions/"))
    {
        return example_value(name, &definitions[key], definitions);
    }
    if let Some(alternatives) = schema.get("anyOf").and_then(Value::as_array) {
        // A nullable alternative is the minimal value.
        if alternatives.contains(&json!({"type": "null"})) {
            return Value::Null;
        }
        return example_value(name, &alternatives[0], definitions);
    }
    if let Some(first) = schema
        .get("allOf")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
    {
        return example_value(name, first, definitions);
    }
    match schema.get("type") {
        Some(Value::String(kind)) if kind == "string" => json!(match name {
            "cwd" => "/repo",
            "model" => "gpt-test",
            "session_id" => "session-1",
            "turn_id" => "turn-1",
            "tool_name" => "Bash",
            "tool_use_id" => "call-1",
            "prompt" => "Run the tests",
            "source" => "startup",
            "trigger" => "manual",
            "agent_id" => "agent-1",
            "agent_type" => "reviewer",
            "thread_id" => "thread-1",
            "request_id" => "5f0c6c1e-1d1f-4c55-9a51-0d0d4f0b7a10",
            _ if name.ends_with("path") => "/tmp/transcript.jsonl",
            _ => "value",
        }),
        Some(Value::Array(types)) if types.iter().any(|kind| kind == "null") => Value::Null,
        Some(Value::String(kind)) if kind == "boolean" => Value::Bool(false),
        // Codex numbers model-request attempts from 1.
        Some(Value::String(kind)) if kind == "integer" => json!(u8::from(name == "attempt_number")),
        Some(Value::String(kind)) if kind == "array" => json!([]),
        Some(Value::String(kind)) if kind == "object" => json!({}),
        Some(Value::Bool(true)) | None => json!({}),
        _ => Value::Null,
    }
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(
        &fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display())),
    )
    .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn write_json(path: &Path, value: &Value) {
    let mut bytes = serde_json::to_vec_pretty(value).expect("serialize JSON");
    bytes.push(b'\n');
    fs::write(path, bytes).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
}

fn write_yaml(path: &Path, value: &Value) {
    let yaml = serde_yaml_ng::to_string(value).expect("serialize YAML");
    fs::write(path, yaml).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vendored(snapshot: Snapshot, file: &str) -> Value {
        let generated = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask parent")
            .join("contracts/vendor/codex")
            .join(snapshot.revision())
            .join("generated");
        read_json(&generated.join(file))
    }

    fn names(values: &Value) -> BTreeSet<String> {
        values
            .as_array()
            .expect("array")
            .iter()
            .map(|value| value.as_str().expect("string").to_owned())
            .collect()
    }

    #[test]
    fn splitting_the_shared_model_request_schema_merges_one_event_branch() {
        let shared = vendored(
            Snapshot::Commit3d86e43R1,
            "model-request.command.input.schema.json",
        );
        let shared_required = names(&shared["required"]);
        let shared_members: BTreeSet<_> = shared["properties"]
            .as_object()
            .expect("properties")
            .keys()
            .cloned()
            .collect();
        for (wire_name, own) in [
            ("PreModelRequest", &[][..]),
            (
                "PostModelRequest",
                &["error", "response_id", "status", "token_usage"][..],
            ),
        ] {
            let split = split_event_branch(shared.clone(), wire_name);
            assert!(split.get("oneOf").is_none());
            // The vendored openness and numbering are kept, not narrowed.
            assert!(split.get("additionalProperties").is_none());
            assert_eq!(split["properties"]["attempt_number"]["minimum"], json!(0.0));
            assert_eq!(
                split["properties"]["hook_event_name"],
                json!({"const": wire_name, "type": "string"})
            );
            let mut expected = shared_required.clone();
            expected.insert("hook_event_name".to_owned());
            expected.extend(own.iter().map(|name| (*name).to_owned()));
            assert_eq!(names(&split["required"]), expected, "{wire_name}");
            let members: BTreeSet<_> = split["properties"]
                .as_object()
                .expect("properties")
                .keys()
                .cloned()
                .collect();
            let mut expected_members = shared_members.clone();
            expected_members.insert("hook_event_name".to_owned());
            expected_members.extend(own.iter().map(|name| (*name).to_owned()));
            assert_eq!(members, expected_members, "{wire_name}");
        }
    }

    #[test]
    fn model_request_events_exist_only_from_rust_v0_172_and_are_observation_only() {
        for snapshot in SNAPSHOTS {
            let observers: Vec<_> = seeds(snapshot)
                .into_iter()
                .filter(|seed| seed.observer)
                .collect();
            if snapshot < Snapshot::Commit3d86e43R1 {
                assert!(observers.is_empty(), "{}", snapshot.id());
                continue;
            }
            assert_eq!(observers.len(), 2);
            for seed in observers {
                assert!(seed.structured.is_none() && seed.block_effect.is_none());
                let input = split_event_branch(
                    vendored(snapshot, "model-request.command.input.schema.json"),
                    seed.wire_name,
                );
                let contract = contract(snapshot, &seed, &input);
                assert_eq!(contract["schemas"]["outputs"], json!([]));
                assert_eq!(contract["handler_kinds"], json!(["command", "mcp_tool"]));
                let outcomes = contract["bindings"]["command"]["outcomes"]
                    .as_array()
                    .expect("outcomes");
                let ids: Vec<_> = outcomes.iter().map(|outcome| &outcome["id"]).collect();
                assert_eq!(ids, [&json!("no-op"), &json!("failure")]);
                for outcome in outcomes {
                    assert_eq!(outcome["stdout"], ignored_stdout());
                    assert_eq!(outcome["stderr"], diagnostic_stderr());
                }
            }
        }
    }
}
