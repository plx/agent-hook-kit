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
    /// Reads contents or metadata.
    Read,
    /// Creates or changes a target without necessarily reading it.
    Modify,
    /// Both reads and changes a target.
    ReadModify,
    /// Lists a target or its children.
    Enumerate,
    /// Removes a target.
    Delete,
    /// Reads and removes the source of a move.
    MoveSource,
    /// Creates or replaces the destination of a move.
    MoveDestination,
}

impl AccessIntent {
    /// Returns whether the intent includes a read-like operation.
    pub const fn may_read(self) -> bool {
        matches!(
            self,
            Self::Read | Self::ReadModify | Self::Enumerate | Self::MoveSource
        )
    }

    /// Returns whether the intent includes a mutating operation.
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
    /// Directly identified by syntax or explicit tool semantics.
    Direct,
    /// Directly identified, but located in conditional or deferred shell code.
    Conditional,
    /// Conservatively inferred and potentially over-inclusive.
    Heuristic,
}

/// Lexical basis used to resolve a raw path expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum PathBase {
    /// The raw expression is already absolute.
    Absolute,
    /// The expression is relative to the invocation's initial working directory.
    InvocationCwd,
    /// A preceding directory change makes the runtime base unknowable.
    UnknownAfterDirectoryChange,
    /// The invocation omitted a working directory for a relative expression.
    MissingWorkingDirectory,
}

/// A raw path and any justified lexical resolution.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PathExpression {
    /// Path text retained from the observable input.
    pub raw: String,
    /// Lexically normalized path when a stable base is available.
    ///
    /// This value is not filesystem-canonicalized and does not imply that the
    /// target exists.
    pub resolved: Option<Utf8PathBuf>,
    /// Basis used, or needed, to interpret `raw`.
    pub base: PathBase,
}

/// Scope denoted by a file target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum AccessScope {
    /// Only the named path.
    Exact,
    /// Children of the named directory, excluding the directory itself.
    Descendants,
    /// The named path and, when applicable, its descendants.
    ExactOrDescendants,
    /// Paths selected by a glob expression.
    Glob,
}

/// Unified target retained across structured, patch, shell, and custom evidence.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum AccessTarget {
    /// A path expression with an explicit match scope.
    Path {
        /// Original expression and any lexical resolution.
        expression: PathExpression,
        /// Portion of the file tree selected by the expression.
        scope: AccessScope,
    },
    /// One observable workspace as a whole.
    Workspace {
        /// Workspace root, or `None` when no root was observable.
        root: Option<Utf8PathBuf>,
    },
}

/// How a structured path-bearing field was selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StructuredFieldMatch {
    /// Selected by an explicitly configured JSON Pointer.
    ExactPointer,
    /// Selected because the object's key appears in the configured key set.
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
    /// Adds a new file.
    Add,
    /// Updates an existing file.
    Update,
    /// Deletes an existing file.
    Delete,
    /// Reads and removes the old path of a move.
    MoveSource,
    /// Creates the new path of a move.
    MoveDestination,
    /// Reads the old path named by a unified-diff header.
    UnifiedOld,
    /// Writes the new path named by a unified-diff header.
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
    /// A path-bearing structured input field.
    StructuredField {
        /// RFC 6901 JSON Pointer to the value.
        pointer: String,
        /// Configuration mechanism that selected the value.
        matched_by: StructuredFieldMatch,
    },
    /// A path recovered from a standalone patch-tool payload.
    Patch {
        /// JSON Pointer to the patch string.
        payload_pointer: String,
        /// Patch role assigned to the path.
        operation: PatchOperation,
        /// Original patch header containing the path.
        header: String,
        /// One-based line number in the patch payload.
        line: usize,
    },
    /// A path inferred from shell syntax or argv semantics.
    Shell {
        /// Shell syntax or argv location that supplied the path.
        origin: FileAccessOrigin,
        /// Byte and source position span of the containing command.
        command_span: SourceSpan,
        /// Stable identifier of the semantics implementation.
        inferred_by: String,
    },
    /// A path recovered from an `apply_patch` shell here-document.
    ShellPatch {
        /// Zero-based command index in the Bash analysis.
        command_index: usize,
        /// Span of the containing `apply_patch` command.
        command_span: SourceSpan,
        /// Span of the here-document body.
        heredoc_span: SourceSpan,
        /// Literal here-document delimiter.
        delimiter: String,
        /// Patch role assigned to the path.
        operation: PatchOperation,
        /// Original patch header containing the path.
        header: String,
        /// One-based line number in the patch body.
        line: usize,
    },
    /// Evidence emitted by an application-defined analyzer.
    Custom {
        /// Stable analyzer identifier.
        analyzer: String,
        /// Optional human-readable origin detail.
        detail: Option<String>,
    },
}

impl AccessProvenance {
    /// Returns the top-level analyzer source represented by this provenance.
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
    /// Path or workspace region that may be accessed.
    pub target: AccessTarget,
    /// Operation that may be performed on the target.
    pub intent: AccessIntent,
    /// Strength of the static association.
    pub certainty: AccessCertainty,
    /// Detailed origin of the evidence.
    pub provenance: AccessProvenance,
}

/// Top-level source responsible for evidence or a known gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AccessSource {
    /// Structured JSON fields.
    Structured,
    /// Literal patch payloads.
    Patch,
    /// Shell syntax, command semantics, or shell patch payloads.
    Shell,
    /// An application-defined analyzer.
    Custom,
}

/// Typed reason that observable file-access analysis was incomplete.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToolAccessGapReason {
    /// The native event omits its originating tool call.
    MissingToolCall,
    /// A newer or application-defined aligned input arm was encountered.
    UnknownInputArm,
    /// The native tool name is absent or not a string.
    MissingToolName,
    /// The native tool input is absent.
    MissingToolInput,
    /// A recognized shell call does not match its configured JSON shape.
    MalformedShellCall(hookkit_shell::ShellToolCallError),
    /// Shell analysis retained a specific unresolved effect.
    ShellUnresolved(UnresolvedFileAccess),
    /// Shell extraction returned an unknown future evidence arm.
    UnsupportedShellEvidence,
    /// A relative path cannot be resolved without an invocation directory.
    MissingWorkingDirectory {
        /// Raw relative path.
        raw: String,
        /// JSON Pointer when the path came from structured input.
        pointer: Option<String>,
    },
    /// A configured structured path value is neither a string nor string array.
    PathValueNotString {
        /// JSON Pointer to the invalid value.
        pointer: String,
    },
    /// No built-in structured semantics recognize the tool.
    UnknownStructuredTool {
        /// Native tool name.
        tool_name: String,
    },
    /// A path was found but its read/write role remains unknown.
    UnclassifiedAccess {
        /// Native tool name.
        tool_name: String,
        /// JSON Pointer to the path.
        pointer: String,
    },
    /// A recognized structured tool exposed no configured path value.
    RecognizedToolWithoutPath {
        /// Native tool name.
        tool_name: String,
    },
    /// A recognized patch tool exposed no supported payload field.
    MissingPatchPayload {
        /// Native tool name.
        tool_name: String,
    },
    /// The selected patch payload is not a string.
    PatchPayloadNotString {
        /// JSON Pointer to the invalid payload.
        pointer: String,
    },
    /// A literal patch payload is malformed or incomplete.
    MalformedPatch {
        /// One-based source line, when the parser can localize the error.
        line: Option<usize>,
        /// Human-readable parse failure.
        detail: String,
    },
    /// An `apply_patch` here-document uses expansions or other dynamic syntax.
    DynamicShellPatchHereDocument {
        /// Span of the containing shell command.
        command_span: SourceSpan,
        /// Recovered here-document delimiter.
        delimiter: String,
        /// Constructs that make the body dynamic.
        reasons: Vec<hookkit_shell::DynamicReason>,
    },
    /// An `apply_patch` command has no observable here-document body.
    MissingShellPatchHereDocument {
        /// Span of the containing shell command.
        command_span: SourceSpan,
    },
    /// A directory-changing command precedes the shell patch.
    ShellPatchWorkingDirectoryMayHaveChanged {
        /// Span of the containing `apply_patch` command.
        command_span: SourceSpan,
    },
}

/// One known blind spot retained alongside safely recovered candidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolAccessGap {
    /// Top-level analyzer that encountered the gap.
    pub source: AccessSource,
    /// Specific reason analysis is incomplete.
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
    /// Safely recovered possible accesses.
    pub candidates: Vec<AccessCandidate>,
    /// Known blind spots retained alongside the candidates.
    pub gaps: Vec<ToolAccessGap>,
}

impl ToolAccessReport {
    /// Iterates over candidates whose intent may read data.
    pub fn may_read(&self) -> impl Iterator<Item = &AccessCandidate> {
        self.candidates
            .iter()
            .filter(|candidate| candidate.intent.may_read())
    }

    /// Iterates over candidates whose intent may modify data.
    pub fn may_modify(&self) -> impl Iterator<Item = &AccessCandidate> {
        self.candidates
            .iter()
            .filter(|candidate| candidate.intent.may_modify())
    }

    /// Returns whether there are no known gaps or unclassified candidates.
    ///
    /// Completeness is relative to observable input and the configured static
    /// analyzers; it is not a guarantee that a process performs no other I/O.
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
