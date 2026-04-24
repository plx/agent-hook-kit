//! Cross-harness wrapper enums and semantic helpers.
//!
//! This crate provides common wrapper enums that align semantically
//! equivalent events across Claude, Codex, and Gemini, while preserving
//! lossless access to the underlying native types.

pub mod input;
pub mod message;
pub mod output;

pub use input::CommonHookInput;
pub use message::{NoticeLevel, UserNotice};
pub use output::CommonHookOutput;

#[cfg(test)]
mod tests;
