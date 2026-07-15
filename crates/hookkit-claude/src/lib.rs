//! Claude Code native hook input/output types.

pub mod catalog;
pub mod environment;
pub mod protocol;

pub use environment::{
    ClaudeCommandEnvironment, ClaudeEffort, ClaudeExecutionLocation, ClaudePluginEnvironment,
    ClaudePluginOptions,
};
