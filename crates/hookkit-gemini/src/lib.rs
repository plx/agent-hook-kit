//! Gemini CLI native hook input/output types.
#![deny(missing_docs)]

pub mod catalog;
pub mod environment;
/// Implemented Gemini CLI native event contracts and harness-wide dispatch.
pub mod protocol;

pub use environment::GeminiCommandEnvironment;
