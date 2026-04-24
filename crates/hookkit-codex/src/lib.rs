//! Codex CLI native hook input/output types.

pub mod input;
pub mod output;

pub use input::CodexHookInput;
pub use output::CodexHookOutput;

#[cfg(test)]
mod tests;
