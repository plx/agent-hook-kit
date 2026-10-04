//! Errors raised by Pkl evaluation and config discovery.

use crate::catalog::CatalogValidationError;
use std::path::PathBuf;
use thiserror::Error;

/// Error raised while staging, evaluating, or decoding Pkl configuration.
///
/// Variants that wrap an I/O, JSON, or validation failure keep it as their
/// [`std::error::Error::source`], so callers can inspect the underlying error;
/// each message also includes the source's text.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PklConfigError {
    #[error(
        "pkl is not installed or not on PATH (see https://pkl-lang.org/main/current/pkl-cli/index.html)"
    )]
    /// The `pkl` executable could not be found.
    PklNotFound,

    #[error("failed to execute pkl: {0}")]
    /// The `pkl` process could not be spawned or waited on.
    PklExec(#[source] std::io::Error),

    #[error(
        "pkl eval did not finish within {} seconds and was stopped (a configuration import may be waiting on the network)",
        timeout.as_secs()
    )]
    /// `pkl eval` exceeded [`crate::eval::PKL_EVAL_TIMEOUT`] and was killed.
    PklTimedOut {
        /// The deadline that was exceeded.
        timeout: std::time::Duration,
    },

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

    #[error("failed to decode pkl JSON output for {path}: {source}", path = path.display())]
    /// Pkl output was not valid for the requested Rust schema.
    JsonDecode {
        /// Configuration file whose evaluated output failed to decode.
        path: PathBuf,
        /// JSON or schema decoding error.
        #[source]
        source: serde_json::Error,
    },

    #[error(
        "pkl evaluated {actual} configuration layers for {path}, expected {expected}",
        path = path.display()
    )]
    /// The generated layer aggregator returned the wrong number of layers.
    LayerCountMismatch {
        /// First configuration file of the evaluated chain.
        path: PathBuf,
        /// Number of layers staged for evaluation.
        expected: usize,
        /// Number of layers Pkl returned.
        actual: usize,
    },

    #[error("temporary pkl file IO failed for {path}: {source}", path = path.display())]
    /// Creating or populating the temporary staging tree failed.
    TempIo {
        /// Affected staging path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    #[error("failed to read pkl file {path}: {source}", path = path.display())]
    /// Reading a source file for staging failed.
    ReadIo {
        /// Source path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    #[error("builtin catalog validation failed:\n{0}")]
    /// One or more embedded builtin tool definitions are inconsistent.
    CatalogValidation(#[source] CatalogValidationError),
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

    #[test]
    fn wrapped_failures_keep_their_typed_source_and_message() {
        use std::error::Error as _;

        let io = PklConfigError::ReadIo {
            path: PathBuf::from("/config/post-tool-use.pkl"),
            source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
        };
        assert_eq!(
            io.to_string(),
            "failed to read pkl file /config/post-tool-use.pkl: denied"
        );
        assert_eq!(
            io.source()
                .and_then(|source| source.downcast_ref::<std::io::Error>())
                .map(std::io::Error::kind),
            Some(std::io::ErrorKind::PermissionDenied)
        );

        let json = PklConfigError::JsonDecode {
            path: PathBuf::from("/config/post-tool-use.pkl"),
            source: serde_json::from_str::<serde_json::Value>("{").unwrap_err(),
        };
        assert!(
            json.to_string()
                .starts_with("failed to decode pkl JSON output for /config/post-tool-use.pkl: ")
        );
        assert!(
            json.source()
                .is_some_and(|source| source.is::<serde_json::Error>())
        );

        let catalog = PklConfigError::CatalogValidation(CatalogValidationError {
            errors: vec!["tool `x` has no check".into()],
        });
        assert_eq!(
            catalog.to_string(),
            "builtin catalog validation failed:\n- tool `x` has no check"
        );
        assert!(
            catalog
                .source()
                .is_some_and(|source| source.is::<CatalogValidationError>())
        );
    }
}
