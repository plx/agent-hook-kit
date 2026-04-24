//! Gemini CLI native hook input/output types.

pub mod input;
pub mod output;

pub use input::GeminiHookInput;
pub use output::GeminiHookOutput;

#[cfg(test)]
mod tests;
