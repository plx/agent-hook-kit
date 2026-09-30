//! Errors raised by Pkl evaluation and config discovery.

use std::path::PathBuf;
use thiserror::Error;

/// Error raised while staging, evaluating, or decoding Pkl configuration.
#[derive(Debug, Error)]
pub enum PklConfigError {
    #[error(
        "pkl is not installed or not on PATH (see https://pkl-lang.org/main/current/pkl-cli/index.html)"
    )]
    /// The `pkl` executable could not be found.
    PklNotFound,

    #[error("failed to execute pkl: {0}")]
    /// The `pkl` process could not be spawned or waited on.
    PklExec(String),

    #[error("pkl eval failed for {path}:\n{stderr}", path = path.display())]
    /// Pkl evaluated the source unsuccessfully.
    PklEvalFailed {
        /// Configuration file that failed. For discovered and explicit files
        /// this is the real source, not the deleted staging copy; in-memory
        /// sources report their staged path.
        path: PathBuf,
        /// Standard error emitted by Pkl, with staged paths rewritten to the
        /// real source files.
        stderr: String,
    },

    #[error("failed to decode pkl JSON output for {path}: {error}", path = path.display())]
    /// Pkl output was not valid for the requested Rust schema.
    JsonDecode {
        /// Configuration file whose evaluated output failed to decode.
        path: PathBuf,
        /// JSON or schema decoding diagnostic.
        error: String,
    },

    #[error("temporary pkl file IO failed for {path}: {error}", path = path.display())]
    /// Creating or populating the temporary staging tree failed.
    TempIo {
        /// Affected staging path.
        path: PathBuf,
        /// I/O diagnostic.
        error: String,
    },

    #[error("failed to read pkl file {path}: {error}", path = path.display())]
    /// Reading a source file for staging failed.
    ReadIo {
        /// Source path.
        path: PathBuf,
        /// I/O diagnostic.
        error: String,
    },

    #[error("builtin catalog validation failed:\n{0}")]
    /// One or more embedded builtin tool definitions are inconsistent.
    CatalogValidation(String),
}

/// Wraps the error as [`hookkit_core::HookkitError::Handler`], keeping its
/// type: callers can `downcast_ref::<PklConfigError>()` the source, and the
/// message is not reported as an I/O error.
impl From<PklConfigError> for hookkit_core::HookkitError {
    fn from(err: PklConfigError) -> Self {
        hookkit_core::HookkitError::handler(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_keeps_the_pkl_error_type_and_message() {
        let error = hookkit_core::HookkitError::from(PklConfigError::PklNotFound);
        let hookkit_core::HookkitError::Handler(source) = &error else {
            panic!("expected a handler error, got {error:?}");
        };
        assert!(matches!(
            source.downcast_ref::<PklConfigError>(),
            Some(PklConfigError::PklNotFound)
        ));
        assert!(error.to_string().contains("pkl is not installed"));
        assert!(!error.to_string().contains("I/O error"));
    }
}
