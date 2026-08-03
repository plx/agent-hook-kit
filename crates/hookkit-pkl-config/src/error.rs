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
        /// Evaluated staged source path.
        path: PathBuf,
        /// Standard error emitted by Pkl.
        stderr: String,
    },

    #[error("failed to decode pkl JSON output for {path}: {error}", path = path.display())]
    /// Pkl output was not valid for the requested Rust schema.
    JsonDecode {
        /// Evaluated staged source path.
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

impl From<PklConfigError> for hookkit_core::HookkitError {
    fn from(err: PklConfigError) -> Self {
        std::io::Error::other(err.to_string()).into()
    }
}
