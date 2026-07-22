//! Codex CLI native hook input/output types.
#![deny(missing_docs)]

pub mod catalog;
pub mod environment;
/// Implemented Codex native event contracts and harness-wide dispatch types.
pub mod protocol;

pub use environment::{CodexCommandEnvironment, CodexPluginEnvironment};
