//! Errors raised by Pkl evaluation and config discovery.

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PklConfigError {
    #[error(
        "pkl is not installed or not on PATH (see https://pkl-lang.org/main/current/pkl-cli/index.html)"
    )]
    PklNotFound,

    #[error("failed to execute pkl: {0}")]
    PklExec(String),

    #[error("pkl eval failed for {path}:\n{stderr}", path = path.display())]
    PklEvalFailed { path: PathBuf, stderr: String },

    #[error("failed to decode pkl JSON output for {path}: {error}", path = path.display())]
    JsonDecode { path: PathBuf, error: String },

    #[error("temporary pkl file IO failed for {path}: {error}", path = path.display())]
    TempIo { path: PathBuf, error: String },

    #[error("failed to read pkl file {path}: {error}", path = path.display())]
    ReadIo { path: PathBuf, error: String },
}

impl From<PklConfigError> for hookkit_core::HookkitError {
    fn from(err: PklConfigError) -> Self {
        std::io::Error::other(err.to_string()).into()
    }
}
