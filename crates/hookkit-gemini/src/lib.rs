//! Gemini CLI native hook input/output types.

pub mod input;
pub mod output;
pub mod protocol;

pub use input::GeminiHookInput;
pub use output::GeminiHookOutput;

#[cfg(test)]
mod tests;
