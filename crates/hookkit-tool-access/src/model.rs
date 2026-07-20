use hookkit_core::Utf8PathBuf;
use hookkit_shell::{FileAccessOrigin, SourceSpan, UnresolvedFileAccess};
use std::collections::BTreeSet;
use std::fmt;

/// Whether the observable operation may access a target in a particular way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AccessIntent {
    /// A path reference was observed but its access semantics are unknown.
    Unclassified,
    Read,
    Modify,
    ReadModify,
    Enumerate,
    Delete,
    MoveSource,
    MoveDestination,
}

impl AccessIntent {
    pub const fn may_read(self) -> bool {
        matches!(
            self,
            Self::Read | Self::ReadModify | Self::Enumerate | Self::MoveSource
        )
    }

    pub const fn may_modify(self) -> bool {
        matches!(
            self,
            Self::Modify
                | Self::ReadModify
                | Self::Delete
                | Self::MoveSource
                | Self::MoveDestination
        )
    }
}

/// Static-analysis confidence, not a promise that execution reaches the access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AccessCertainty {
    Direct,
    Conditional,
    Heuristic,
}

/// Lexical basis used to resolve a raw path expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum PathBase {
    Absolute,
    InvocationCwd,
    UnknownAfterDirectoryChange,
    MissingWorkingDirectory,
}

/// A raw path and any justified lexical resolution.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PathExpression {
    pub raw: String,
    pub resolved: Option<Utf8PathBuf>,
    pub base: PathBase,
}

/// Scope denoted by a file target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum AccessScope {
    Exact,
    Descendants,
    ExactOrDescendants,
    Glob,
}

/// Unified target retained across structured, patch, shell, and custom evidence.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum AccessTarget {
    Path {
        expression: PathExpression,
        scope: AccessScope,
    },
    Workspace {
        root: Option<Utf8PathBuf>,
    },
}

/// How a structured path-bearing field was selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StructuredFieldMatch {
    ExactPointer,
    KeyHeuristic,
}

impl fmt::Display for StructuredFieldMatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExactPointer => formatter.write_str("exact pointer"),
            Self::KeyHeuristic => formatter.write_str("configured key"),
        }
    }
}

/// Patch operation recovered from a literal payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PatchOperation {
    Add,
    Update,
    Delete,
    MoveSource,
    MoveDestination,
    UnifiedOld,
    UnifiedNew,
}

impl fmt::Display for PatchOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Add => "add",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::MoveSource => "move source",
            Self::MoveDestination => "move destination",
            Self::UnifiedOld => "unified old path",
            Self::UnifiedNew => "unified new path",
        };
        formatter.write_str(name)
    }
}

/// Detailed origin of one candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AccessProvenance {
    StructuredField {
        pointer: String,
        matched_by: StructuredFieldMatch,
    },
    Patch {
        payload_pointer: String,
        operation: PatchOperation,
        header: String,
        line: usize,
    },
    Shell {
        origin: FileAccessOrigin,
        command_span: SourceSpan,
        inferred_by: String,
    },
    ShellPatch {
        command_index: usize,
        command_span: SourceSpan,
        heredoc_span: SourceSpan,
        delimiter: String,
        operation: PatchOperation,
        header: String,
        line: usize,
    },
    Custom {
        analyzer: String,
        detail: Option<String>,
    },
}

impl AccessProvenance {
    pub const fn source(&self) -> AccessSource {
        match self {
            Self::StructuredField { .. } => AccessSource::Structured,
            Self::Patch { .. } => AccessSource::Patch,
            Self::Shell { .. } => AccessSource::Shell,
            Self::ShellPatch { .. } => AccessSource::Shell,
            Self::Custom { .. } => AccessSource::Custom,
        }
    }
}

impl fmt::Display for AccessProvenance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StructuredField {
                pointer,
                matched_by,
            } => write!(formatter, "structured field {pointer} ({matched_by})"),
            Self::Patch {
                payload_pointer,
                operation,
                line,
                ..
            } => write!(
                formatter,
                "patch {operation} at {payload_pointer}, source line {line}"
            ),
            Self::Shell {
                command_span,
                inferred_by,
                ..
            } => write!(
                formatter,
                "shell inference {inferred_by} at bytes {}..{}",
                command_span.start_byte, command_span.end_byte
            ),
            Self::ShellPatch {
                command_span,
                operation,
                line,
                ..
            } => write!(
                formatter,
                "shell patch {operation} at bytes {}..{}, patch line {line}",
                command_span.start_byte, command_span.end_byte
            ),
            Self::Custom { analyzer, detail } => {
                write!(formatter, "custom analyzer {analyzer}")?;
                if let Some(detail) = detail {
                    write!(formatter, ": {detail}")?;
                }
                Ok(())
            }
        }
    }
}

/// One possible file access recovered from observable tool-call data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessCandidate {
    pub target: AccessTarget,
    pub intent: AccessIntent,
    pub certainty: AccessCertainty,
    pub provenance: AccessProvenance,
}

/// Top-level source responsible for evidence or a known gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AccessSource {
    Structured,
    Patch,
    Shell,
    Custom,
}

/// Typed reason that observable file-access analysis was incomplete.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToolAccessGapReason {
    MissingToolCall,
    UnknownInputArm,
    MissingToolName,
    MissingToolInput,
    MalformedShellCall(hookkit_shell::ShellToolCallError),
    ShellUnresolved(UnresolvedFileAccess),
    UnsupportedShellEvidence,
    MissingWorkingDirectory {
        raw: String,
        pointer: Option<String>,
    },
    PathValueNotString {
        pointer: String,
    },
    UnknownStructuredTool {
        tool_name: String,
    },
    UnclassifiedAccess {
        tool_name: String,
        pointer: String,
    },
    RecognizedToolWithoutPath {
        tool_name: String,
    },
    MissingPatchPayload {
        tool_name: String,
    },
    PatchPayloadNotString {
        pointer: String,
    },
    MalformedPatch {
        line: Option<usize>,
        detail: String,
    },
    DynamicShellPatchHereDocument {
        command_span: SourceSpan,
        delimiter: String,
        reasons: Vec<hookkit_shell::DynamicReason>,
    },
    MissingShellPatchHereDocument {
        command_span: SourceSpan,
    },
    ShellPatchWorkingDirectoryMayHaveChanged {
        command_span: SourceSpan,
    },
}

/// One known blind spot retained alongside safely recovered candidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolAccessGap {
    pub source: AccessSource,
    pub reason: ToolAccessGapReason,
}

impl fmt::Display for ToolAccessGap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.reason {
            ToolAccessGapReason::MissingToolCall => {
                formatter.write_str("native event does not include the originating tool call")
            }
            ToolAccessGapReason::UnknownInputArm => {
                formatter.write_str("unknown aligned tool input arm")
            }
            ToolAccessGapReason::MissingToolName => {
                formatter.write_str("native tool name is missing or is not a string")
            }
            ToolAccessGapReason::MissingToolInput => {
                formatter.write_str("native tool input is missing")
            }
            ToolAccessGapReason::MalformedShellCall(error) => error.fmt(formatter),
            ToolAccessGapReason::ShellUnresolved(unresolved) => {
                write_shell_gap(formatter, unresolved)
            }
            ToolAccessGapReason::UnsupportedShellEvidence => {
                formatter.write_str("shell analyzer returned an unknown evidence arm")
            }
            ToolAccessGapReason::MissingWorkingDirectory { raw, pointer } => {
                write!(
                    formatter,
                    "relative path `{raw}` has no native working directory"
                )?;
                if let Some(pointer) = pointer {
                    write!(formatter, " at {pointer}")?;
                }
                Ok(())
            }
            ToolAccessGapReason::PathValueNotString { pointer } => {
                write!(
                    formatter,
                    "configured path value at {pointer} is not a string"
                )
            }
            ToolAccessGapReason::UnknownStructuredTool { tool_name } => {
                write!(
                    formatter,
                    "structured tool `{tool_name}` has unknown access semantics"
                )
            }
            ToolAccessGapReason::UnclassifiedAccess { tool_name, pointer } => write!(
                formatter,
                "path at {pointer} for structured tool `{tool_name}` has unclassified access"
            ),
            ToolAccessGapReason::RecognizedToolWithoutPath { tool_name } => write!(
                formatter,
                "recognized structured tool `{tool_name}` exposed no configured path"
            ),
            ToolAccessGapReason::MissingPatchPayload { tool_name } => {
                write!(
                    formatter,
                    "patch tool `{tool_name}` exposed no patch payload"
                )
            }
            ToolAccessGapReason::PatchPayloadNotString { pointer } => {
                write!(formatter, "patch payload at {pointer} is not a string")
            }
            ToolAccessGapReason::MalformedPatch { line, detail } => {
                formatter.write_str("malformed or incomplete patch")?;
                if let Some(line) = line {
                    write!(formatter, " at source line {line}")?;
                }
                write!(formatter, ": {detail}")
            }
            ToolAccessGapReason::DynamicShellPatchHereDocument {
                command_span,
                delimiter,
                ..
            } => write!(
                formatter,
                "shell apply_patch here-document `{delimiter}` at bytes {}..{} is dynamic",
                command_span.start_byte, command_span.end_byte
            ),
            ToolAccessGapReason::MissingShellPatchHereDocument { command_span } => write!(
                formatter,
                "shell apply_patch at bytes {}..{} has no observable here-document body",
                command_span.start_byte, command_span.end_byte
            ),
            ToolAccessGapReason::ShellPatchWorkingDirectoryMayHaveChanged { command_span } => {
                write!(
                    formatter,
                    "shell apply_patch working directory may have changed before bytes {}..{}",
                    command_span.start_byte, command_span.end_byte
                )
            }
        }
    }
}

fn write_shell_gap(
    formatter: &mut fmt::Formatter<'_>,
    unresolved: &UnresolvedFileAccess,
) -> fmt::Result {
    use hookkit_shell::UnresolvedFileAccessReason as Reason;

    if let Some(raw) = &unresolved.raw {
        write!(formatter, "unresolved shell access `{raw}`: ")?;
    } else {
        formatter.write_str("unresolved shell access: ")?;
    }
    match &unresolved.reason {
        Reason::AnalysisIncomplete(_) => formatter.write_str("Bash analysis was incomplete"),
        Reason::AnalysisUnavailable(_) => formatter.write_str("Bash analysis was unavailable"),
        Reason::DynamicCommandName { .. } => {
            formatter.write_str("command name depends on runtime expansion")
        }
        Reason::DynamicPath { argv_index, .. } => match argv_index {
            Some(index) => write!(formatter, "argument {index} depends on runtime expansion"),
            None => formatter.write_str("path depends on runtime expansion"),
        },
        Reason::UnknownCommandSemantics { command } => {
            write!(formatter, "unknown file semantics for command `{command}`")
        }
        Reason::AmbiguousArguments { detail } => formatter.write_str(detail),
        Reason::IndirectEvaluation { command } => {
            write!(
                formatter,
                "command `{command}` evaluates file access indirectly"
            )
        }
        Reason::WorkingDirectoryMayHaveChanged => {
            formatter.write_str("working directory may have changed")
        }
        Reason::MissingWorkingDirectory => {
            formatter.write_str("native working directory is missing")
        }
        Reason::UnsupportedRedirection => formatter.write_str("redirection is unsupported"),
        _ => formatter.write_str("unknown shell-analysis gap"),
    }
}

/// All retained evidence and typed gaps for one observable tool call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolAccessReport {
    pub candidates: Vec<AccessCandidate>,
    pub gaps: Vec<ToolAccessGap>,
}

impl ToolAccessReport {
    pub fn may_read(&self) -> impl Iterator<Item = &AccessCandidate> {
        self.candidates
            .iter()
            .filter(|candidate| candidate.intent.may_read())
    }

    pub fn may_modify(&self) -> impl Iterator<Item = &AccessCandidate> {
        self.candidates
            .iter()
            .filter(|candidate| candidate.intent.may_modify())
    }

    pub fn is_complete(&self) -> bool {
        self.gaps.is_empty()
            && self
                .candidates
                .iter()
                .all(|candidate| candidate.intent != AccessIntent::Unclassified)
    }

    /// Deterministic target-only view for consumers that explicitly choose to
    /// discard role, certainty, and provenance differences.
    pub fn unique_targets(&self) -> BTreeSet<&AccessTarget> {
        self.candidates
            .iter()
            .map(|candidate| &candidate.target)
            .collect()
    }

    pub(crate) fn push_gap(&mut self, source: AccessSource, reason: ToolAccessGapReason) {
        self.gaps.push(ToolAccessGap { source, reason });
    }
}
