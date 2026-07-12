//! Contract-safe I/O and execution plumbing for hookkit executables.

pub mod aligned;
pub mod artifacts;
pub mod resolution;
pub mod selected;
pub mod typed;

pub use hookkit_core::RuntimeContext;
pub use selected::{
    BuiltinInput, BuiltinOutput, dispatch_builtin_harness, execute_builtin_harness,
    execute_harness, execute_harness_with_diagnostics, run_harness,
};
pub use typed::{
    execute_typed, execute_typed_with_diagnostics, run_event, run_event_with_diagnostics, run_typed,
};
