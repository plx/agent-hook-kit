//! Claude Code native hook input/output types.
//!
//! The crate implements the Claude Code contract snapshot named by
//! [`protocol::SNAPSHOT_ID`] (Claude Code 2.1.332, Agent SDK 0.3.332). Every implemented event's [`hookkit_core::EventSpec`]
//! is re-exported from [`events`], whichever module defines it:
//!
//! - [`protocol`] holds the events with dedicated typed inputs
//!   (`SessionStart`, `PostToolUse`, `WorktreeCreate`), the harness adapter,
//!   and the forward-compatible input value types;
//! - [`model_switch`] holds `PreModelSwitch` and `PostModelSwitch`;
//! - [`catalog`] holds the events that share [`catalog::CatalogInput`], which
//!   exposes typed accessors for the high-traffic fields, including
//!   `PreSkillShellCommand` (Claude Code v2.1.332 or later).
//!
//! Harness-sent enum values that the snapshot does not document parse as an
//! `Unknown(String)` arm instead of failing, so a newer Claude Code release
//! cannot silently turn a hook into a fail-open parse error. For the same
//! reason a field the Agent SDK types optional (`UserPromptExpansion`
//! `command_source`) or that Claude Code has deprecated for removal
//! (`TeammateIdle` `team_name`) is not required. A payload that violates its
//! event's shape is reported as `HookkitError::InvalidInputForHint`.
//!
//! Unknown fields, including those of nested objects, are retained, so an
//! input re-serializes to a JSON value equal to the one it was parsed from;
//! the one exception is an explicit `null` in a typed optional field, which
//! is omitted.
//!
//! # Output builder conventions
//!
//! - Constructors are associated functions that name the response they
//!   build: `no_op`, `block`, `allow`, `deny`, `system_message`, and so on.
//!   The long-standing `with_context` constructors are the one exception.
//! - Methods named `with_*` take `self`, add one field, and return
//!   `hookkit_core::Result<Self>`. They fail when the response has already
//!   become a stderr or text outcome, or when Claude Code would ignore the
//!   field for the decision already chosen.
//! - `blocking_error` exits 2 with required stderr where exit 2 blocks,
//!   `feedback_error` exits 2 where the tool already ran and stderr is only
//!   feedback for Claude, and `nonblocking_error` exits 1 with stderr that
//!   Claude Code shows the user as a hook error notice. Events where Claude
//!   Code ignores or only logs failures have no error constructor.
//! - `nonblocking_error` and every other failure (a hook that cannot start,
//!   times out, or prints invalid JSON) is non-blocking only while the hook's
//!   `onFailure` setting is unset or `continue`. With `"onFailure": "block"`
//!   (Claude Code v2.1.295 or later) a failure does what exit 2 does on the
//!   event, and on `PermissionRequest` it denies; `Stop`, `SubagentStop`,
//!   `TaskCompleted`, and `TeammateIdle` ignore the setting. On Windows
//!   without Git Bash a shell-form `PreToolUse`, `PermissionRequest`, or
//!   `PreSkillShellCommand` command hook's failure blocks even with the
//!   setting unset. Each `nonblocking_error` says what applies to its event.
//! - Claude Code reads JSON stdout on every exit code, so
//!   `into_blocking_error` and `into_feedback_error` keep a structured
//!   response on stdout while exiting 2; a blocking hook then still blocks if
//!   its JSON is rejected. Their message may be empty when the JSON makes a
//!   blocking decision, whose reason Claude Code then shows; otherwise an
//!   empty message is rejected when the response is built. A JSON response
//!   whose exit code is neither 0 nor 2 counts as a failure under
//!   `"onFailure": "block"`, so typed constructors never emit one. On `Stop`
//!   and `SubagentStop`, where `continue: false` takes precedence over exit 2,
//!   `into_blocking_error` rejects a response that sets it.
//! - Every builder describes a synchronous command hook. A hook registered
//!   with `async` or `asyncRewake` cannot block or control Claude: decision
//!   fields, `continue`, and `terminalSequence` have no effect, and
//!   `additionalContext` and `systemMessage` reach Claude on the next turn
//!   instead of the user.
//! - Claude Code caps `additionalContext`, `systemMessage`,
//!   `initialUserMessage`, and plain stdout at 10,000 characters per hook
//!   (longer text is saved to a file and replaced by its path and a short
//!   preview; `PreCompact` instructions are cut instead) and, since v2.1.292,
//!   escapes `<system-reminder>` tags. Builders emit text unchanged.
//! - `no_op` prints `{}`, the structured response with no fields, except on
//!   `PreCompact`, where Claude Code would add that text to the compaction
//!   instructions, so it prints nothing.
//! - `text_context` writes plain text verbatim, except that text Claude Code
//!   would parse as JSON (trimmed text starting with `{` and ending with
//!   `}`) is sent as structured `additionalContext` so it is never dropped.
//! - Superseded APIs stay as `#[deprecated]` shims whose notes name the
//!   replacement: positional `decide`/`structured` constructors, the
//!   constructor-style `with_system_message`, builders for fields Claude Code
//!   discards on the event (such as `suppressOutput` everywhere, or
//!   `systemMessage` on `Notification`), `SetupOutput::with_context` (now
//!   `{}`, since Setup output is discarded), and
//!   `PermissionRequestOutput::blocking_error` (now also prints the deny
//!   decision, because exit 2 alone no longer denies). `PreModelSwitch` and
//!   `PostModelSwitch` have no deprecated shims.
//! - A few signatures changed in place, without a shim, because the old and
//!   new forms share a name: `SessionStartOutput::with_initial_user_message`
//!   is now a `self` builder (use `no_op().with_initial_user_message(..)`
//!   where it was a constructor), `PermissionRequestOutput::with_updated_input`
//!   takes a JSON object (`Map`) instead of a `Value`, and
//!   `SessionStartOutput` and `PostToolUseOutput` are opaque types built only
//!   through their constructors.
//! - The emitted JSON satisfies the snapshot's output schema, including
//!   `UserPromptExpansionOutput::with_suppress_original_prompt`, a field the
//!   Agent SDK types for that event and the snapshot accepts although the
//!   hooks reference does not document it there.
#![deny(missing_docs)]

pub mod catalog;
pub mod environment;
pub mod events;
pub mod model_switch;
pub mod protocol;
mod values;

/// The Claude Code contract snapshot this crate implements, at the
/// crate root like `hookkit_antigravity::SNAPSHOT`; the same value as
/// [`protocol::SNAPSHOT_ID`].
pub use protocol::SNAPSHOT_ID as SNAPSHOT;

pub use environment::{
    ClaudeCommandEnvironment, ClaudeEffort, ClaudeExecutionLocation, ClaudeMessagingEnvironment,
    ClaudeMessagingToken, ClaudePluginEnvironment, ClaudePluginOptions,
};
