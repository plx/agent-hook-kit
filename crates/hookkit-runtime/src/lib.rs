//! Contract-safe I/O and execution plumbing for hookkit executables.
#![deny(missing_docs)]
#![warn(missing_debug_implementations)]

// The aligned family markers are uninhabited enums without `Debug`.
#[allow(missing_debug_implementations)]
pub mod aligned;
pub mod artifacts;
/// Capture of declared command-hook process environment variables.
pub mod environment;
pub mod failure;
/// Stderr diagnostics shared by the stdin/stdout runtime adapters.
mod report;
/// Event identification, best-effort detection, and executable resolution.
pub mod resolution;
/// Compile-time and runtime selected-harness execution.
pub mod selected;
/// Exact typed-event execution and process I/O adapters.
pub mod typed;

pub use failure::{FailurePolicy, RunOptions};
pub use hookkit_core::RuntimeContext;
pub use selected::{
    BuiltinCommandEnvironment, BuiltinInput, BuiltinOutput, dispatch_builtin_harness,
    dispatch_builtin_harness_with_options, execute_builtin_harness,
    execute_builtin_harness_with_diagnostics, execute_harness, execute_harness_with_diagnostics,
    run_harness, run_harness_with_options,
};
pub use typed::{
    execute_typed, execute_typed_with_diagnostics, run_event, run_event_with_diagnostics,
    run_event_with_options, run_typed,
};
