//! Codex CLI native hook input/output types.
//!
//! The implemented contracts follow the Codex snapshot named by
//! [`protocol::SNAPSHOT_ID`]. Harness-sent enum values outside that snapshot
//! parse into `Unknown` arms instead of failing, and output constructors refuse
//! shapes Codex would reject silently, such as a blank block reason, because a
//! rejected hook output fails open in Codex.
//!
//! Except on `PreModelRequest` and `PostModelRequest`, Codex never reads
//! stderr from a hook that exits 0; user-facing notices belong in
//! `systemMessage`. Every non-zero exit other than a blocking exit 2 with
//! non-blank stderr is a failed run that does not block, and on those events
//! Codex shows that run's stderr only for `PreCompact`, `PostCompact`, and
//! `SessionEnd`. A HookKit runtime error (exit 1) is therefore invisible to
//! the user on the other events.
//!
//! `PreModelRequest` and `PostModelRequest` are observation-only: Codex never
//! parses their stdout, every non-zero exit (2 included) is a plain failure,
//! and trimmed non-blank stderr becomes an error entry at every exit code,
//! including 0. A synchronous handler's entries are shown on every model
//! request attempt, so a HookKit runtime error on these events is visible to
//! the user each time; a background (`async: true`) handler's are never shown.
//! See [`catalog::PreModelRequestOutput`].
//!
//! `session_id` is the root session identity that every descendant thread
//! shares, so on a subagent thread it names the root session, not the
//! subagent; [`hookkit_core::NativeContext::session_id`] therefore groups a
//! subagent's activity under its root session. The subagent itself is named by
//! `agent_id` (and, on the model-request events, `thread_id`), which the
//! native inputs expose but [`hookkit_core::NativeContext`] does not carry.
//!
//! Since Codex 0.166.0 the `cwd` of the tool events (`PreToolUse`,
//! `PermissionRequest`, and `PostToolUse`) is the tool command's working
//! directory, which for a local Bash command is its resolved `workdir`, not a
//! project root. Their [`hookkit_core::NativeContext::workspace_roots`] is
//! therefore empty; [`protocol::AnyInput::command_cwd`] and the inputs' `cwd`
//! fields expose the directory. Codex still starts every tool hook in the
//! step's environment directory.
#![deny(missing_docs)]

pub mod catalog;
pub mod environment;
/// Implemented Codex native event contracts and harness-wide dispatch types.
pub mod protocol;
mod wire;

pub use environment::{CodexCommandEnvironment, CodexPluginEnvironment};
