//! Codex CLI native hook input/output types.
//!
//! The implemented contracts follow the Codex snapshot named by
//! [`protocol::SNAPSHOT_ID`]. Harness-sent enum values outside that snapshot
//! parse into `Unknown` arms instead of failing, and output constructors refuse
//! shapes Codex would reject silently, such as a blank block reason, because a
//! rejected hook output fails open in Codex.
//!
//! Codex never reads stderr from a hook that exits 0; user-facing notices
//! belong in `systemMessage`. Every non-zero exit other than a blocking exit 2
//! with non-blank stderr is a failed run that does not block, and Codex shows
//! that run's stderr only for `PreCompact`, `PostCompact`, and `SessionEnd`.
//! A HookKit runtime error (exit 1) is therefore invisible to the user on the
//! other events.
#![deny(missing_docs)]

pub mod catalog;
pub mod environment;
/// Implemented Codex native event contracts and harness-wide dispatch types.
pub mod protocol;
mod wire;

pub use environment::{CodexCommandEnvironment, CodexPluginEnvironment};
