//! Claude Code native hook input/output types.

pub mod input;
pub mod output;
pub mod protocol;

pub use input::ClaudeHookInput;
pub use output::ClaudeHookOutput;

#[cfg(test)]
mod tests;
