//! Opt-in shell tool-call extraction and bounded Bash syntax analysis.
#![deny(missing_docs)]
//!
//! This crate deliberately separates four concerns:
//!
//! 1. [`call`] recognizes an exact harness tool-call shape and extracts its
//!    command without guessing across field names.
//! 2. [`bash`] reports syntax facts, source spans, and literal argv only where
//!    those values can be recovered conservatively.
//! 3. [`summary`] provides small, best-effort descriptions of common
//!    inspection commands. It is not a general intent or safety classifier.
//! 4. [`file_access`] infers possible explicit reads and modifications while
//!    retaining command, path, and working-directory uncertainty.
//!
//! A successful parse is not a sandbox or a proof that a command is safe.
//! Shell expansion, sourced code, executable behavior, and runtime filesystem
//! state remain outside static syntax analysis.

pub mod bash;
pub mod call;
/// Loss-aware inference of explicit file reads and modifications from Bash.
pub mod file_access;
pub mod summary;

#[cfg(any(feature = "claude", feature = "codex", feature = "antigravity"))]
mod adapters;

#[cfg(feature = "antigravity")]
pub use adapters::ANTIGRAVITY_RUN_COMMAND_PROFILE;
#[cfg(feature = "claude")]
pub use adapters::CLAUDE_BASH_PROFILE;
#[cfg(feature = "codex")]
pub use adapters::CODEX_BASH_PROFILE;

pub use bash::{
    ArgvStatus, BashAnalysis, BashAnalysisOutcome, BashAnalyzer, BashAnalyzerLimits,
    CommandOccurrence, ConstructKind, ConstructOccurrence, DynamicReason, ExecutionContext,
    HereDocument, IncompleteReason, Redirection, RedirectionKind, RedirectionOperator, ShellWord,
    SourcePosition, SourceSpan, StatementKind, StatementRedirection, UnavailableReason,
};
pub use call::{
    JsonRef, ShellCwdOrigin, ShellDialect, ShellToolCallError, ShellToolCallErrorKind,
    ShellToolCallExt, ShellToolCallMatch, ShellToolCallRef, ShellToolProfile,
    ShellToolProfileError, ToolPhase,
};
pub use file_access::{
    BuiltinCommandFileSemantics, CommandFileContext, CommandFileSemantics, FileAccessAnalyzer,
    FileAccessCandidate, FileAccessCertainty, FileAccessKind, FileAccessOrigin, FileAccessReport,
    FileAccessSink, FileInferenceContext, FileTarget, FileTargetScope, PathBase, PathExpression,
    UnknownCommandFallback, UnresolvedFileAccess, UnresolvedFileAccessReason,
};
pub use summary::{CommandSummarizer, InspectionSummarizer, InspectionSummary, PathCandidate};
