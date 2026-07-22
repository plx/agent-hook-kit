//! Claude Code native hook input/output types.
#![deny(missing_docs)]

pub mod catalog;
pub mod environment;
pub mod protocol;

pub use environment::{
    ClaudeCommandEnvironment, ClaudeEffort, ClaudeExecutionLocation, ClaudePluginEnvironment,
    ClaudePluginOptions,
};
