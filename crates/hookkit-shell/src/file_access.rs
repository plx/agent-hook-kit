//! Best-effort file-access inference over parsed Bash commands.
//!
//! This module reports explicit or command-semantics-derived candidates. It
//! never claims to enumerate every file a process will touch: executables may
//! load configuration, follow symlinks, evaluate generated code, or perform
//! arbitrary I/O that is not represented in their argv.

use hookkit_core::{Utf8Path, Utf8PathBuf, normalize_utf8_path};

use crate::bash::{
    BashAnalysis, BashAnalysisOutcome, CommandOccurrence, DynamicReason, ExecutionContext,
    IncompleteReason, RedirectionOperator, ShellWord, SourceSpan, UnavailableReason,
};

/// Initial path context supplied by the native shell tool call and hook runtime.
#[derive(Debug, Clone, Copy, Default)]
pub struct FileInferenceContext<'a> {
    /// Working directory in effect when the shell invocation begins.
    pub cwd: Option<&'a Utf8Path>,
    /// Workspace root, when the surrounding harness identifies one.
    pub workspace_root: Option<&'a Utf8Path>,
}

impl<'a> FileInferenceContext<'a> {
    /// Creates a context with an optional invocation working directory.
    pub const fn new(cwd: Option<&'a Utf8Path>) -> Self {
        Self {
            cwd,
            workspace_root: None,
        }
    }

    /// Attaches the optional workspace root used by workspace-wide rules.
    pub const fn with_workspace_root(mut self, workspace_root: Option<&'a Utf8Path>) -> Self {
        self.workspace_root = workspace_root;
        self
    }
}

/// An inferred set of possible explicit file accesses plus known blind spots.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FileAccessReport {
    /// File targets that the analysis could associate with an access kind.
    pub candidates: Vec<FileAccessCandidate>,
    /// Commands or operands whose file effects could not be resolved.
    pub unresolved: Vec<UnresolvedFileAccess>,
}

impl FileAccessReport {
    /// Iterates over candidates whose access kind may read data.
    pub fn may_read(&self) -> impl Iterator<Item = &FileAccessCandidate> {
        self.candidates
            .iter()
            .filter(|candidate| candidate.access.may_read())
    }

    /// Iterates over candidates whose access kind may modify data.
    pub fn may_modify(&self) -> impl Iterator<Item = &FileAccessCandidate> {
        self.candidates
            .iter()
            .filter(|candidate| candidate.access.may_modify())
    }

    /// Returns whether analysis recorded no known blind spots.
    ///
    /// A `true` result is not a guarantee that the process performs no other
    /// I/O; it only means that this best-effort analyzer resolved everything
    /// it recognized in the shell syntax and configured command semantics.
    pub fn is_fully_resolved(&self) -> bool {
        self.unresolved.is_empty()
    }
}

/// A possible access to a statically identified target.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FileAccessCandidate {
    /// Path or workspace region that may be accessed.
    pub target: FileTarget,
    /// Operation the command may perform on the target.
    pub access: FileAccessKind,
    /// Certainty of the static association, not a promise that execution will
    /// reach or successfully perform the access.
    pub certainty: FileAccessCertainty,
    /// Syntax or command rule from which this candidate was derived.
    pub origin: FileAccessOrigin,
    /// Source span of the command containing the access.
    pub command_span: SourceSpan,
    /// Identifier of the semantics implementation that emitted the candidate.
    pub inferred_by: String,
}

/// Kind of file-system operation inferred for a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum FileAccessKind {
    /// Reads file contents or metadata.
    Read,
    /// Creates or changes a target without necessarily reading it.
    Modify,
    /// Both reads and changes the target.
    ReadModify,
    /// Lists the target or its children.
    Enumerate,
    /// Removes the target.
    Delete,
    /// Reads and removes the source of a move.
    MoveSource,
    /// Creates or replaces the destination of a move.
    MoveDestination,
}

impl FileAccessKind {
    /// Returns whether this kind includes a read-like operation.
    pub const fn may_read(self) -> bool {
        matches!(
            self,
            Self::Read | Self::ReadModify | Self::Enumerate | Self::MoveSource
        )
    }

    /// Returns whether this kind includes a mutating operation.
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

/// Target region associated with an inferred access.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum FileTarget {
    /// A path expression with an explicit match scope.
    Path {
        /// Original expression and any statically resolved path.
        expression: PathExpression,
        /// Portion of the file tree selected by the expression.
        scope: FileTargetScope,
    },
    /// The current workspace as a whole.
    Workspace {
        /// Known workspace root, or `None` when the harness omitted it.
        root: Option<Utf8PathBuf>,
    },
}

/// A shell path operand and its best-effort lexical resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PathExpression {
    /// Operand exactly as represented by the analyzed shell word.
    pub raw: String,
    /// Lexically normalized absolute path, when a stable base is known.
    ///
    /// Resolution does not access the file system, canonicalize symlinks, or
    /// establish that the target exists.
    pub resolved: Option<Utf8PathBuf>,
    /// Base against which `raw` was, or would need to be, interpreted.
    pub base: PathBase,
}

/// Base used to interpret a [`PathExpression`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum PathBase {
    /// The expression is already absolute.
    Absolute,
    /// The expression is relative to the invocation's initial working directory.
    InvocationCwd,
    /// A preceding directory-changing command makes the runtime base unknown.
    UnknownAfterDirectoryChange,
}

/// Region selected relative to a [`FileTarget::Path`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum FileTargetScope {
    /// Only the named path.
    Exact,
    /// Children of the named directory, but not the directory itself.
    Descendants,
    /// The named path and, if it is a directory, its descendants.
    ExactOrDescendants,
    /// Paths matched by the shell glob expression.
    Glob,
}

/// Strength of the static association between a command and a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum FileAccessCertainty {
    /// Directly identified by syntax or unambiguous command semantics.
    Direct,
    /// Directly identified, but located in conditional or deferred shell code.
    Conditional,
    /// Conservatively inferred by a rule that may over-approximate access.
    Heuristic,
}

/// Opt-in treatment of literal operands when command semantics remain unknown
/// or ambiguous.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnknownCommandFallback {
    /// Preserve only the unresolved record.
    #[default]
    Disabled,
    /// Emit path-like literal operands as heuristic read/modify candidates
    /// while preserving the unresolved record.
    LiteralPathOperands,
}

/// Location in the analyzed command from which a candidate originated.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum FileAccessOrigin {
    /// A positional command-line argument.
    Argument {
        /// Zero-based index of the command in the analysis.
        command_index: usize,
        /// Zero-based argv index, where zero is the command name.
        argv_index: usize,
    },
    /// A shell redirection target.
    Redirection {
        /// Zero-based index of the command in the analysis.
        command_index: usize,
        /// Zero-based redirection index within the command.
        redirection_index: usize,
    },
    /// An omitted operand that defaults to the working directory.
    WorkingDirectoryDefault {
        /// Zero-based index of the command in the analysis.
        command_index: usize,
    },
    /// A command-specific inference rule rather than a literal operand.
    Rule {
        /// Zero-based index of the command in the analysis.
        command_index: usize,
        /// Human-readable description of the rule.
        detail: String,
    },
}

/// A possible file effect that static inference could not resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct UnresolvedFileAccess {
    /// Command index, or `None` for analysis-wide failures.
    pub command_index: Option<usize>,
    /// Command span, or `None` for analysis-wide failures.
    pub command_span: Option<SourceSpan>,
    /// Relevant source text, when one expression can be identified.
    pub raw: Option<String>,
    /// Reason no concrete candidate could be emitted.
    pub reason: UnresolvedFileAccessReason,
}

/// Reason static file-access inference could not resolve an effect.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum UnresolvedFileAccessReason {
    /// Parsing produced a usable prefix plus an incomplete-analysis reason.
    AnalysisIncomplete(IncompleteReason),
    /// No usable Bash analysis was available.
    AnalysisUnavailable(UnavailableReason),
    /// The executable name depends on runtime expansion.
    DynamicCommandName {
        /// Shell constructs that prevent a literal command name.
        reasons: Vec<DynamicReason>,
    },
    /// A path operand depends on runtime expansion.
    DynamicPath {
        /// Affected argv position, when the operand came from argv.
        argv_index: Option<usize>,
        /// Shell constructs that prevent a literal path.
        reasons: Vec<DynamicReason>,
    },
    /// No registered semantics implementation recognized the command.
    UnknownCommandSemantics {
        /// Literal command name.
        command: String,
    },
    /// The command is recognized but its operands cannot be classified safely.
    AmbiguousArguments {
        /// Human-readable description of the ambiguity.
        detail: String,
    },
    /// The command interprets an operand as additional executable code.
    IndirectEvaluation {
        /// Literal command name that performs the indirect evaluation.
        command: String,
    },
    /// A preceding `cd`, `pushd`, or `popd` may have changed a relative base.
    WorkingDirectoryMayHaveChanged,
    /// A relative operand was found but the invocation working directory is unknown.
    MissingWorkingDirectory,
    /// The redirection form has file effects the analyzer cannot model.
    UnsupportedRedirection,
}

/// Read-only command view supplied to custom semantics implementations.
#[derive(Debug, Clone, Copy)]
pub struct CommandFileContext<'a> {
    command_index: usize,
    command: &'a CommandOccurrence,
    name: &'a str,
}

impl<'a> CommandFileContext<'a> {
    /// Returns the zero-based command index in the enclosing analysis.
    pub const fn command_index(self) -> usize {
        self.command_index
    }

    /// Returns the complete parsed command occurrence.
    pub const fn command(self) -> &'a CommandOccurrence {
        self.command
    }

    /// Returns the literal command name used to select this semantics rule.
    pub const fn name(self) -> &'a str {
        self.name
    }

    /// Number of words including `argv[0]`.
    pub fn argv_len(self) -> usize {
        self.command.arguments.len() + 1
    }

    /// Returns an argv word, treating the command name as `argv[0]`.
    pub fn word(self, argv_index: usize) -> Option<&'a ShellWord> {
        if argv_index == 0 {
            self.command.name.as_ref()
        } else {
            self.command.arguments.get(argv_index - 1)
        }
    }

    /// Returns a literal argv value, or `None` for a missing or dynamic word.
    pub fn literal(self, argv_index: usize) -> Option<&'a str> {
        self.word(argv_index)?.literal.as_deref()
    }

    /// Returns whether any operand after `argv[0]` equals `value` literally.
    pub fn has_literal(self, value: &str) -> bool {
        (1..self.argv_len()).any(|index| self.literal(index) == Some(value))
    }
}

/// Extension point for project- or tool-specific argv semantics.
pub trait CommandFileSemantics: Send + Sync {
    /// Returns the stable identifier recorded on emitted candidates.
    fn id(&self) -> &str;

    /// Returns `true` when this implementation recognizes the command, even
    /// if it emits no candidates.
    fn infer(&self, command: CommandFileContext<'_>, output: &mut FileAccessSink<'_, '_>) -> bool;
}

/// Candidate emitter made available to [`CommandFileSemantics`].
pub struct FileAccessSink<'command, 'report> {
    command: CommandFileContext<'command>,
    inference: FileInferenceContext<'command>,
    cwd_may_have_changed: bool,
    cwd_issue_emitted: bool,
    inferred_by: String,
    report: &'report mut FileAccessReport,
}

impl FileAccessSink<'_, '_> {
    /// Emits an access derived from an argv operand.
    ///
    /// A missing or dynamic operand is recorded in [`FileAccessReport::unresolved`]
    /// instead. `argv_index` uses conventional indexing, with the executable at
    /// zero.
    pub fn emit_argument(
        &mut self,
        argv_index: usize,
        access: FileAccessKind,
        scope: FileTargetScope,
    ) {
        let Some(word) = self.command.word(argv_index) else {
            self.unresolved(
                None,
                UnresolvedFileAccessReason::AmbiguousArguments {
                    detail: format!("missing argv[{argv_index}]"),
                },
            );
            return;
        };
        self.emit_word(
            word,
            access,
            scope,
            FileAccessOrigin::Argument {
                command_index: self.command.command_index,
                argv_index,
            },
            FileAccessCertainty::Direct,
            Some(argv_index),
        );
    }

    fn emit_argument_literal(
        &mut self,
        argv_index: usize,
        raw: &str,
        access: FileAccessKind,
        scope: FileTargetScope,
        certainty: FileAccessCertainty,
    ) {
        self.emit_path(
            raw,
            access,
            scope,
            FileAccessOrigin::Argument {
                command_index: self.command.command_index,
                argv_index,
            },
            certainty,
        );
    }

    /// Emits an access to the command's effective working directory.
    pub fn emit_working_directory(&mut self, access: FileAccessKind, scope: FileTargetScope) {
        self.emit_path(
            ".",
            access,
            scope,
            FileAccessOrigin::WorkingDirectoryDefault {
                command_index: self.command.command_index,
            },
            FileAccessCertainty::Direct,
        );
    }

    /// Emits a heuristic access to the whole workspace.
    ///
    /// The candidate is retained even when the workspace root is unavailable;
    /// in that case its [`FileTarget::Workspace::root`] is `None`.
    pub fn emit_workspace(&mut self, access: FileAccessKind) {
        self.report.candidates.push(FileAccessCandidate {
            target: FileTarget::Workspace {
                root: self.inference.workspace_root.map(Utf8Path::to_path_buf),
            },
            access,
            certainty: self.adjust_certainty(FileAccessCertainty::Heuristic),
            origin: FileAccessOrigin::Rule {
                command_index: self.command.command_index,
                detail: "implicit workspace scope".to_owned(),
            },
            command_span: self.command.command.span,
            inferred_by: self.inferred_by.clone(),
        });
    }

    /// Emits a heuristic path supplied by a command-specific rule.
    pub fn emit_rule_path(
        &mut self,
        raw: &str,
        access: FileAccessKind,
        scope: FileTargetScope,
        detail: impl Into<String>,
    ) {
        self.emit_path(
            raw,
            access,
            scope,
            FileAccessOrigin::Rule {
                command_index: self.command.command_index,
                detail: detail.into(),
            },
            FileAccessCertainty::Heuristic,
        );
    }

    /// Records a recognized effect that cannot be resolved to a candidate.
    pub fn unresolved(&mut self, raw: Option<String>, reason: UnresolvedFileAccessReason) {
        self.report.unresolved.push(UnresolvedFileAccess {
            command_index: Some(self.command.command_index),
            command_span: Some(self.command.command.span),
            raw,
            reason,
        });
    }

    fn emit_redirection(
        &mut self,
        redirection_index: usize,
        word: &ShellWord,
        access: FileAccessKind,
    ) {
        self.emit_word(
            word,
            access,
            FileTargetScope::Exact,
            FileAccessOrigin::Redirection {
                command_index: self.command.command_index,
                redirection_index,
            },
            FileAccessCertainty::Direct,
            None,
        );
    }

    fn emit_word(
        &mut self,
        word: &ShellWord,
        access: FileAccessKind,
        scope: FileTargetScope,
        origin: FileAccessOrigin,
        certainty: FileAccessCertainty,
        argv_index: Option<usize>,
    ) {
        if let Some(literal) = &word.literal {
            self.emit_path(literal, access, scope, origin, certainty);
            return;
        }
        if word.dynamic_reasons.as_slice() == [DynamicReason::Glob] {
            self.emit_path(&word.raw, access, FileTargetScope::Glob, origin, certainty);
            return;
        }
        self.unresolved(
            Some(word.raw.clone()),
            UnresolvedFileAccessReason::DynamicPath {
                argv_index,
                reasons: word.dynamic_reasons.clone(),
            },
        );
    }

    fn emit_path(
        &mut self,
        raw: &str,
        access: FileAccessKind,
        scope: FileTargetScope,
        origin: FileAccessOrigin,
        certainty: FileAccessCertainty,
    ) {
        let path = Utf8Path::new(raw);
        let (base, resolved) = if path.is_absolute() {
            (PathBase::Absolute, Some(normalize_utf8_path(path)))
        } else if self.cwd_may_have_changed {
            if !self.cwd_issue_emitted {
                self.cwd_issue_emitted = true;
                self.unresolved(
                    Some(raw.to_owned()),
                    UnresolvedFileAccessReason::WorkingDirectoryMayHaveChanged,
                );
            }
            (PathBase::UnknownAfterDirectoryChange, None)
        } else {
            match self.inference.cwd {
                Some(cwd) => (
                    PathBase::InvocationCwd,
                    Some(normalize_utf8_path(cwd.join(path))),
                ),
                None => {
                    if !self.cwd_issue_emitted {
                        self.cwd_issue_emitted = true;
                        self.unresolved(
                            Some(raw.to_owned()),
                            UnresolvedFileAccessReason::MissingWorkingDirectory,
                        );
                    }
                    (PathBase::InvocationCwd, None)
                }
            }
        };
        self.report.candidates.push(FileAccessCandidate {
            target: FileTarget::Path {
                expression: PathExpression {
                    raw: raw.to_owned(),
                    resolved,
                    base,
                },
                scope,
            },
            access,
            certainty: self.adjust_certainty(certainty),
            origin,
            command_span: self.command.command.span,
            inferred_by: self.inferred_by.clone(),
        });
    }

    fn adjust_certainty(&self, certainty: FileAccessCertainty) -> FileAccessCertainty {
        if certainty == FileAccessCertainty::Heuristic {
            return certainty;
        }
        if self.command.command.context.iter().any(|context| {
            matches!(
                context,
                ExecutionContext::AndOrList
                    | ExecutionContext::Conditional
                    | ExecutionContext::Loop
                    | ExecutionContext::Case
                    | ExecutionContext::FunctionDefinition
            )
        }) {
            FileAccessCertainty::Conditional
        } else {
            certainty
        }
    }
}

/// File-access inference engine. Custom rules registered later take precedence
/// over the bundled command table.
pub struct FileAccessAnalyzer {
    semantics: Vec<Box<dyn CommandFileSemantics>>,
    unknown_command_fallback: UnknownCommandFallback,
}

impl Default for FileAccessAnalyzer {
    fn default() -> Self {
        Self::with_builtins()
    }
}

impl FileAccessAnalyzer {
    /// Creates an analyzer with no command semantics registered.
    pub fn empty() -> Self {
        Self {
            semantics: Vec::new(),
            unknown_command_fallback: UnknownCommandFallback::Disabled,
        }
    }

    /// Creates an analyzer containing the bundled command semantics.
    pub fn with_builtins() -> Self {
        Self {
            semantics: vec![Box::new(BuiltinCommandFileSemantics)],
            unknown_command_fallback: UnknownCommandFallback::Disabled,
        }
    }

    /// Sets the unknown-command fallback and returns the analyzer.
    pub fn with_unknown_command_fallback(mut self, fallback: UnknownCommandFallback) -> Self {
        self.unknown_command_fallback = fallback;
        self
    }

    /// Sets the unknown-command fallback for subsequent inference.
    pub fn set_unknown_command_fallback(&mut self, fallback: UnknownCommandFallback) {
        self.unknown_command_fallback = fallback;
    }

    /// Registers command semantics with precedence over all existing rules.
    ///
    /// Rules are queried in reverse registration order and the first rule that
    /// returns `true` from [`CommandFileSemantics::infer`] wins.
    pub fn register<S>(&mut self, semantics: S)
    where
        S: CommandFileSemantics + 'static,
    {
        self.semantics.insert(0, Box::new(semantics));
    }

    /// Registers high-precedence command semantics and returns the analyzer.
    pub fn with_semantics<S>(mut self, semantics: S) -> Self
    where
        S: CommandFileSemantics + 'static,
    {
        self.register(semantics);
        self
    }

    /// Infers file access from a complete, partial, or unavailable analysis.
    ///
    /// Candidates from the usable portion of a partial analysis are preserved,
    /// and an analysis-wide unresolved record is appended. An unavailable
    /// analysis produces only an unresolved record.
    pub fn infer(
        &self,
        outcome: &BashAnalysisOutcome,
        context: FileInferenceContext<'_>,
    ) -> FileAccessReport {
        match outcome {
            BashAnalysisOutcome::Complete(analysis) => self.infer_analysis(analysis, context),
            BashAnalysisOutcome::Partial { analysis, reason } => {
                let mut report = self.infer_analysis(analysis, context);
                report.unresolved.push(UnresolvedFileAccess {
                    command_index: None,
                    command_span: None,
                    raw: None,
                    reason: UnresolvedFileAccessReason::AnalysisIncomplete(*reason),
                });
                report
            }
            BashAnalysisOutcome::Unavailable(reason) => FileAccessReport {
                candidates: Vec::new(),
                unresolved: vec![UnresolvedFileAccess {
                    command_index: None,
                    command_span: None,
                    raw: None,
                    reason: UnresolvedFileAccessReason::AnalysisUnavailable(reason.clone()),
                }],
            },
        }
    }

    /// Infers file access from a parsed Bash analysis.
    ///
    /// Relative paths following any directory-changing command are intentionally
    /// left unresolved because static analysis cannot know whether that command
    /// executed or succeeded.
    pub fn infer_analysis(
        &self,
        analysis: &BashAnalysis,
        context: FileInferenceContext<'_>,
    ) -> FileAccessReport {
        let directory_changes = analysis
            .commands
            .iter()
            .filter(|command| {
                matches!(
                    command
                        .name
                        .as_ref()
                        .and_then(|name| name.literal.as_deref()),
                    Some("cd" | "pushd" | "popd")
                )
            })
            .map(|command| command.span.start_byte)
            .collect::<Vec<_>>();

        let mut report = FileAccessReport::default();
        for (command_index, command) in analysis.commands.iter().enumerate() {
            let cwd_may_have_changed = directory_changes
                .iter()
                .any(|offset| *offset < command.span.start_byte);
            let name = command
                .name
                .as_ref()
                .and_then(|name| name.literal.as_deref());
            let command_context = name.map(|name| CommandFileContext {
                command_index,
                command,
                name,
            });
            let fallback_name = command.name.as_ref().map_or("", |name| name.raw.as_str());
            let mut sink = FileAccessSink {
                command: command_context.unwrap_or(CommandFileContext {
                    command_index,
                    command,
                    name: fallback_name,
                }),
                inference: context,
                cwd_may_have_changed,
                cwd_issue_emitted: false,
                inferred_by: "shell-redirection".to_owned(),
                report: &mut report,
            };

            infer_redirections(&mut sink);

            let Some(command_context) = command_context else {
                let reasons = command
                    .name
                    .as_ref()
                    .map(|name| name.dynamic_reasons.clone())
                    .unwrap_or_else(|| vec![DynamicReason::UnsupportedSyntax]);
                sink.unresolved(
                    command.name.as_ref().map(|name| name.raw.clone()),
                    UnresolvedFileAccessReason::DynamicCommandName { reasons },
                );
                continue;
            };

            let mut handled = false;
            let unresolved_before = sink.report.unresolved.len();
            for semantics in &self.semantics {
                sink.inferred_by = semantics.id().to_owned();
                if semantics.infer(command_context, &mut sink) {
                    handled = true;
                    break;
                }
            }
            if !handled {
                sink.unresolved(
                    Some(command_context.name.to_owned()),
                    UnresolvedFileAccessReason::UnknownCommandSemantics {
                        command: command_context.name.to_owned(),
                    },
                );
            }
            let ambiguous = sink.report.unresolved[unresolved_before..]
                .iter()
                .any(|unresolved| {
                    matches!(
                        unresolved.reason,
                        UnresolvedFileAccessReason::UnknownCommandSemantics { .. }
                            | UnresolvedFileAccessReason::AmbiguousArguments { .. }
                    )
                });
            if ambiguous
                && self.unknown_command_fallback == UnknownCommandFallback::LiteralPathOperands
            {
                sink.inferred_by = "hookkit-literal-operand-fallback".to_owned();
                infer_literal_path_operands(command_context, &mut sink);
            }
        }
        report
    }
}

/// Bundled semantics for common inspection and direct file-manipulation tools.
#[derive(Debug, Clone, Copy, Default)]
pub struct BuiltinCommandFileSemantics;

impl CommandFileSemantics for BuiltinCommandFileSemantics {
    fn id(&self) -> &str {
        "hookkit-builtin-command-semantics"
    }

    fn infer(&self, command: CommandFileContext<'_>, output: &mut FileAccessSink<'_, '_>) -> bool {
        match command.name() {
            "echo" | "printf" | "pwd" | "true" | "false" | ":" | "test" | "[" | "cd" | "pushd"
            | "popd" => true,
            "cat" | "bat" | "less" | "more" | "nl" => {
                emit_operands(
                    command,
                    output,
                    1,
                    Options::none(),
                    FileAccessKind::Read,
                    FileTargetScope::Exact,
                );
                true
            }
            "head" | "tail" => {
                emit_operands(
                    command,
                    output,
                    1,
                    Options {
                        values: &["-n", "--lines", "-c", "--bytes"],
                        booleans: &["-q", "--quiet", "-v", "--verbose"],
                        short_booleans: "qv",
                        numeric_short: true,
                    },
                    FileAccessKind::Read,
                    FileTargetScope::Exact,
                );
                true
            }
            "ls" | "eza" => {
                let emitted = emit_operands(
                    command,
                    output,
                    1,
                    Options {
                        values: &[],
                        booleans: &[
                            "--all",
                            "--almost-all",
                            "--long",
                            "--human-readable",
                            "--recursive",
                            "--classify",
                        ],
                        short_booleans: "aAlhRFrt1",
                        numeric_short: false,
                    },
                    FileAccessKind::Enumerate,
                    FileTargetScope::ExactOrDescendants,
                );
                if emitted == Some(0) {
                    output.emit_working_directory(
                        FileAccessKind::Enumerate,
                        FileTargetScope::Descendants,
                    );
                }
                true
            }
            "tree" | "du" => {
                let emitted = emit_operands(
                    command,
                    output,
                    1,
                    Options::none(),
                    FileAccessKind::Enumerate,
                    FileTargetScope::ExactOrDescendants,
                );
                if emitted == Some(0) {
                    output.emit_working_directory(
                        FileAccessKind::Enumerate,
                        FileTargetScope::Descendants,
                    );
                }
                true
            }
            "rg" | "grep" => {
                infer_search(command, output);
                true
            }
            "sed" => {
                infer_sed(command, output);
                true
            }
            "touch" | "truncate" | "mkdir" => {
                emit_operands(
                    command,
                    output,
                    1,
                    Options::none(),
                    FileAccessKind::Modify,
                    FileTargetScope::Exact,
                );
                true
            }
            "rm" | "rmdir" | "unlink" => {
                let recursive = command_has_short_flag(command, 'r')
                    || command_has_short_flag(command, 'R')
                    || command.has_literal("--recursive");
                emit_operands(
                    command,
                    output,
                    1,
                    Options {
                        values: &[],
                        booleans: &["-f", "--force", "-r", "-R", "--recursive", "-d", "--dir"],
                        short_booleans: "frRd",
                        numeric_short: false,
                    },
                    FileAccessKind::Delete,
                    if recursive {
                        FileTargetScope::ExactOrDescendants
                    } else {
                        FileTargetScope::Exact
                    },
                );
                true
            }
            "tee" => {
                emit_operands(
                    command,
                    output,
                    1,
                    Options {
                        values: &[],
                        booleans: &["-a", "--append", "-i", "--ignore-interrupts"],
                        short_booleans: "ai",
                        numeric_short: false,
                    },
                    FileAccessKind::Modify,
                    FileTargetScope::Exact,
                );
                true
            }
            "cp" | "install" => {
                infer_source_destination(command, output, false);
                true
            }
            "mv" => {
                infer_source_destination(command, output, true);
                true
            }
            "ln" => {
                infer_link(command, output);
                true
            }
            "source" | "." => {
                output.emit_argument(1, FileAccessKind::Read, FileTargetScope::Exact);
                output.unresolved(
                    Some(command.name().to_owned()),
                    UnresolvedFileAccessReason::IndirectEvaluation {
                        command: command.name().to_owned(),
                    },
                );
                true
            }
            "eval" | "bash" | "sh" | "zsh" | "xargs" => {
                output.unresolved(
                    Some(command.command().raw.clone()),
                    UnresolvedFileAccessReason::IndirectEvaluation {
                        command: command.name().to_owned(),
                    },
                );
                true
            }
            "apply_patch" => true,
            _ => false,
        }
    }
}

fn infer_literal_path_operands(
    command: CommandFileContext<'_>,
    output: &mut FileAccessSink<'_, '_>,
) {
    if command.command().literal_argv().is_none() {
        return;
    }
    let mut operands_only = false;
    for argv_index in 1..command.argv_len() {
        let Some(literal) = command.literal(argv_index) else {
            return;
        };
        if !operands_only && literal == "--" {
            operands_only = true;
            continue;
        }
        if !operands_only && literal.starts_with('-') {
            continue;
        }
        let value = assignment_value(literal).unwrap_or(literal);
        if is_path_like(value) {
            output.emit_argument_literal(
                argv_index,
                value,
                FileAccessKind::ReadModify,
                FileTargetScope::ExactOrDescendants,
                FileAccessCertainty::Heuristic,
            );
        }
    }
}

fn assignment_value(value: &str) -> Option<&str> {
    let (name, value) = value.split_once('=')?;
    let mut characters = name.chars();
    let first = characters.next()?;
    (matches!(first, '_' | 'a'..='z' | 'A'..='Z')
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric()))
    .then_some(value)
}

/// A fallback operand is path-like when it is dot/dot-dot, begins with a
/// conventional absolute or relative path prefix, contains a separator, or
/// has a filename-style dot in its final component. Bare words stay unknown.
fn is_path_like(value: &str) -> bool {
    if value.is_empty() || value == "-" || value.chars().any(char::is_whitespace) {
        return false;
    }
    value == "."
        || value == ".."
        || value.starts_with('/')
        || value.starts_with("./")
        || value.starts_with("../")
        || value.starts_with("~/")
        || value.contains('/')
        || value
            .rsplit('/')
            .next()
            .is_some_and(|component| component.contains('.'))
}

#[derive(Debug, Clone, Copy)]
struct Options {
    values: &'static [&'static str],
    booleans: &'static [&'static str],
    short_booleans: &'static str,
    numeric_short: bool,
}

impl Options {
    const fn none() -> Self {
        Self {
            values: &[],
            booleans: &[],
            short_booleans: "",
            numeric_short: false,
        }
    }
}

fn emit_operands(
    command: CommandFileContext<'_>,
    output: &mut FileAccessSink<'_, '_>,
    start: usize,
    options: Options,
    access: FileAccessKind,
    scope: FileTargetScope,
) -> Option<usize> {
    let indexes = operand_indexes(command, output, start, options)?;
    for index in &indexes {
        output.emit_argument(*index, access, scope);
    }
    Some(indexes.len())
}

fn operand_indexes(
    command: CommandFileContext<'_>,
    output: &mut FileAccessSink<'_, '_>,
    start: usize,
    options: Options,
) -> Option<Vec<usize>> {
    let mut indexes = Vec::new();
    let mut index = start;
    let mut operands_only = false;
    while index < command.argv_len() {
        let Some(word) = command.word(index) else {
            break;
        };
        let literal = word.literal.as_deref();
        if !operands_only && literal == Some("--") {
            operands_only = true;
            index += 1;
            continue;
        }
        if !operands_only && literal.is_some_and(|value| value.starts_with('-') && value != "-") {
            let value = literal.expect("checked as Some above");
            if options.values.contains(&value) {
                if index + 1 >= command.argv_len() {
                    output.unresolved(
                        Some(value.to_owned()),
                        UnresolvedFileAccessReason::AmbiguousArguments {
                            detail: format!("option {value} is missing its value"),
                        },
                    );
                    return None;
                }
                index += 2;
                continue;
            }
            let short_bundle = value.starts_with('-')
                && !value.starts_with("--")
                && value.len() > 1
                && value[1..]
                    .chars()
                    .all(|character| options.short_booleans.contains(character));
            let numeric_short = options.numeric_short
                && value[1..]
                    .chars()
                    .all(|character| character.is_ascii_digit());
            if options.booleans.contains(&value) || short_bundle || numeric_short {
                index += 1;
                continue;
            }
            output.unresolved(
                Some(value.to_owned()),
                UnresolvedFileAccessReason::AmbiguousArguments {
                    detail: format!("unrecognized option {value} for {}", command.name()),
                },
            );
            return None;
        }
        if !operands_only && literal.is_none() && word.raw.starts_with('-') {
            output.unresolved(
                Some(word.raw.clone()),
                UnresolvedFileAccessReason::AmbiguousArguments {
                    detail: format!("dynamic option for {}", command.name()),
                },
            );
            return None;
        }
        if literal != Some("-") {
            indexes.push(index);
        }
        index += 1;
    }
    Some(indexes)
}

fn infer_search(command: CommandFileContext<'_>, output: &mut FileAccessSink<'_, '_>) {
    let mut index = 1;
    let mut query_seen = false;
    let mut operands_only = false;
    let mut paths = Vec::new();
    while index < command.argv_len() {
        let Some(word) = command.word(index) else {
            break;
        };
        let literal = word.literal.as_deref();
        if !operands_only && literal == Some("--") {
            operands_only = true;
            index += 1;
            continue;
        }
        if !operands_only && matches!(literal, Some("-e" | "--regexp")) {
            if index + 1 >= command.argv_len() {
                output.unresolved(
                    literal.map(str::to_owned),
                    UnresolvedFileAccessReason::AmbiguousArguments {
                        detail: "search pattern option is missing its value".to_owned(),
                    },
                );
                return;
            }
            query_seen = true;
            index += 2;
            continue;
        }
        if !operands_only && is_search_boolean(literal) {
            index += 1;
            continue;
        }
        if !operands_only && is_search_value_option(literal) {
            if index + 1 >= command.argv_len() {
                output.unresolved(
                    literal.map(str::to_owned),
                    UnresolvedFileAccessReason::AmbiguousArguments {
                        detail: "search option is missing its value".to_owned(),
                    },
                );
                return;
            }
            index += 2;
            continue;
        }
        if !operands_only && literal.is_some_and(|value| value.starts_with('-')) {
            output.unresolved(
                literal.map(str::to_owned),
                UnresolvedFileAccessReason::AmbiguousArguments {
                    detail: format!("unrecognized search option for {}", command.name()),
                },
            );
            return;
        }
        if !query_seen {
            query_seen = true;
        } else {
            paths.push(index);
        }
        index += 1;
    }
    if !query_seen {
        output.unresolved(
            Some(command.command().raw.clone()),
            UnresolvedFileAccessReason::AmbiguousArguments {
                detail: "search command has no query".to_owned(),
            },
        );
    } else if paths.is_empty() {
        output.emit_working_directory(FileAccessKind::Read, FileTargetScope::Descendants);
    } else {
        for path in paths {
            output.emit_argument(
                path,
                FileAccessKind::Read,
                FileTargetScope::ExactOrDescendants,
            );
        }
    }
}

fn infer_sed(command: CommandFileContext<'_>, output: &mut FileAccessSink<'_, '_>) {
    let mut index = 1;
    let mut operands_only = false;
    let mut script_configured = false;
    let mut in_place = false;
    let mut input_files = Vec::new();

    while index < command.argv_len() {
        let Some(word) = command.word(index) else {
            break;
        };
        let Some(literal) = word.literal.as_deref() else {
            output.unresolved(
                Some(word.raw.clone()),
                UnresolvedFileAccessReason::AmbiguousArguments {
                    detail: "dynamic sed option, script, or operand layout".to_owned(),
                },
            );
            return;
        };

        if !operands_only && literal == "--" {
            operands_only = true;
            index += 1;
            continue;
        }
        if !operands_only
            && matches!(
                literal,
                "-n" | "--quiet"
                    | "--silent"
                    | "-E"
                    | "-r"
                    | "-s"
                    | "--separate"
                    | "-u"
                    | "--unbuffered"
                    | "-z"
                    | "--null-data"
                    | "--sandbox"
            )
        {
            index += 1;
            continue;
        }
        if !operands_only && matches!(literal, "-e" | "--expression") {
            if index + 1 >= command.argv_len() {
                sed_ambiguous(
                    output,
                    literal,
                    "sed expression option is missing its script",
                );
                return;
            }
            script_configured = true;
            index += 2;
            continue;
        }
        if !operands_only
            && (literal
                .strip_prefix("-e")
                .is_some_and(|script| !script.is_empty())
                || literal.starts_with("--expression="))
        {
            script_configured = true;
            index += 1;
            continue;
        }
        if !operands_only && matches!(literal, "-f" | "--file") {
            if index + 1 >= command.argv_len() {
                sed_ambiguous(
                    output,
                    literal,
                    "sed file option is missing its script file",
                );
                return;
            }
            output.emit_argument(index + 1, FileAccessKind::Read, FileTargetScope::Exact);
            script_configured = true;
            index += 2;
            continue;
        }
        if !operands_only {
            if let Some(script_file) = literal
                .strip_prefix("--file=")
                .filter(|value| !value.is_empty())
                .or_else(|| literal.strip_prefix("-f").filter(|value| !value.is_empty()))
            {
                output.emit_argument_literal(
                    index,
                    script_file,
                    FileAccessKind::Read,
                    FileTargetScope::Exact,
                    FileAccessCertainty::Direct,
                );
                script_configured = true;
                index += 1;
                continue;
            }
        }
        if !operands_only && matches!(literal, "-i" | "--in-place") {
            in_place = true;
            index += 1;
            continue;
        }
        if !operands_only
            && (literal.starts_with("--in-place=")
                || literal
                    .strip_prefix("-i")
                    .is_some_and(|suffix| !suffix.is_empty()))
        {
            in_place = true;
            index += 1;
            continue;
        }
        if !operands_only && literal.starts_with('-') && literal != "-" {
            sed_ambiguous(output, literal, "unrecognized or ambiguous sed option");
            return;
        }

        if script_configured {
            if literal != "-" {
                input_files.push(index);
            }
        } else {
            // Without -e/-f, the first non-option is the sed program.
            script_configured = true;
        }
        index += 1;
    }

    if !script_configured {
        sed_ambiguous(output, command.name(), "sed command has no script");
        return;
    }
    if in_place && input_files.is_empty() {
        sed_ambiguous(
            output,
            command.command().raw.as_str(),
            "in-place sed has no statically identified input file",
        );
        return;
    }
    for input in input_files {
        output.emit_argument(
            input,
            if in_place {
                FileAccessKind::ReadModify
            } else {
                FileAccessKind::Read
            },
            FileTargetScope::Exact,
        );
    }
}

fn sed_ambiguous(output: &mut FileAccessSink<'_, '_>, raw: &str, detail: &str) {
    output.unresolved(
        Some(raw.to_owned()),
        UnresolvedFileAccessReason::AmbiguousArguments {
            detail: detail.to_owned(),
        },
    );
}

fn is_search_boolean(literal: Option<&str>) -> bool {
    matches!(
        literal,
        Some(
            "-n" | "--line-number"
                | "-i"
                | "--ignore-case"
                | "-F"
                | "--fixed-strings"
                | "-R"
                | "-r"
                | "--recursive"
                | "--hidden"
                | "--no-ignore"
                | "-l"
                | "--files-with-matches"
        )
    )
}

fn is_search_value_option(literal: Option<&str>) -> bool {
    matches!(
        literal,
        Some(
            "-g" | "--glob"
                | "-t"
                | "--type"
                | "--type-add"
                | "-j"
                | "--threads"
                | "-m"
                | "--max-count"
                | "--max-depth"
                | "--encoding"
        )
    )
}

fn infer_source_destination(
    command: CommandFileContext<'_>,
    output: &mut FileAccessSink<'_, '_>,
    moving: bool,
) {
    let recursive = command_has_short_flag(command, 'r')
        || command_has_short_flag(command, 'R')
        || command.has_literal("--recursive")
        || (!moving && (command_has_short_flag(command, 'a') || command.has_literal("--archive")));
    let Some(operands) = operand_indexes(
        command,
        output,
        1,
        Options {
            values: &[],
            booleans: &[
                "-f",
                "--force",
                "-r",
                "-R",
                "--recursive",
                "-a",
                "--archive",
                "-p",
                "--preserve",
            ],
            short_booleans: "frRap",
            numeric_short: false,
        },
    ) else {
        return;
    };
    if operands.len() < 2 {
        output.unresolved(
            Some(command.command().raw.clone()),
            UnresolvedFileAccessReason::AmbiguousArguments {
                detail: format!("{} requires a source and destination", command.name()),
            },
        );
        return;
    }
    let scope = if recursive {
        FileTargetScope::ExactOrDescendants
    } else {
        FileTargetScope::Exact
    };
    for source in &operands[..operands.len() - 1] {
        output.emit_argument(
            *source,
            if moving {
                FileAccessKind::MoveSource
            } else {
                FileAccessKind::Read
            },
            scope,
        );
    }
    output.emit_argument(
        *operands.last().expect("checked nonempty above"),
        if moving {
            FileAccessKind::MoveDestination
        } else {
            FileAccessKind::Modify
        },
        scope,
    );
}

fn command_has_short_flag(command: CommandFileContext<'_>, flag: char) -> bool {
    (1..command.argv_len()).any(|index| {
        command.literal(index).is_some_and(|value| {
            value.starts_with('-')
                && !value.starts_with("--")
                && value[1..].chars().any(|character| character == flag)
        })
    })
}

fn infer_link(command: CommandFileContext<'_>, output: &mut FileAccessSink<'_, '_>) {
    let Some(operands) = operand_indexes(
        command,
        output,
        1,
        Options {
            values: &[],
            booleans: &["-s", "--symbolic", "-f", "--force"],
            short_booleans: "sf",
            numeric_short: false,
        },
    ) else {
        return;
    };
    if operands.len() < 2 {
        output.unresolved(
            Some(command.command().raw.clone()),
            UnresolvedFileAccessReason::AmbiguousArguments {
                detail: "ln requires a target and link name".to_owned(),
            },
        );
        return;
    }
    for source in &operands[..operands.len() - 1] {
        output.emit_argument(*source, FileAccessKind::Read, FileTargetScope::Exact);
    }
    output.emit_argument(
        *operands.last().expect("checked nonempty above"),
        FileAccessKind::Modify,
        FileTargetScope::Exact,
    );
}

fn infer_redirections(output: &mut FileAccessSink<'_, '_>) {
    let redirections = output.command.command.redirections.clone();
    for (redirection_index, redirection) in redirections.iter().enumerate() {
        let access = match redirection.operator {
            Some(RedirectionOperator::Input) => Some(FileAccessKind::Read),
            Some(
                RedirectionOperator::Output
                | RedirectionOperator::Append
                | RedirectionOperator::OutputAndError
                | RedirectionOperator::AppendOutputAndError
                | RedirectionOperator::Clobber,
            ) => Some(FileAccessKind::Modify),
            Some(
                RedirectionOperator::DuplicateInput
                | RedirectionOperator::CloseInput
                | RedirectionOperator::CloseOutput
                | RedirectionOperator::HereDocument
                | RedirectionOperator::HereDocumentStripTabs
                | RedirectionOperator::HereString,
            ) => None,
            // With no explicit descriptor, Bash accepts `>&word` as the
            // legacy spelling of `&>word`. A numeric word still duplicates a
            // descriptor, and an explicit descriptor always selects the
            // descriptor-duplication form.
            Some(RedirectionOperator::DuplicateOutput) => redirection
                .target
                .as_ref()
                .filter(|target| {
                    redirection.descriptor.is_none()
                        && target
                            .literal
                            .as_deref()
                            .is_none_or(|value| value != "-" && value.parse::<u32>().is_err())
                })
                .map(|_| FileAccessKind::Modify),
            None => {
                output.unresolved(
                    Some(redirection.raw.clone()),
                    UnresolvedFileAccessReason::UnsupportedRedirection,
                );
                None
            }
        };
        if let Some(access) = access {
            if let Some(target) = &redirection.target {
                output.emit_redirection(redirection_index, target, access);
            } else {
                output.unresolved(
                    Some(redirection.raw.clone()),
                    UnresolvedFileAccessReason::UnsupportedRedirection,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bash::{BashAnalyzer, BashAnalyzerLimits};
    use proptest::prelude::*;

    fn infer(source: &str) -> FileAccessReport {
        let outcome = BashAnalyzer::default().analyze(source);
        FileAccessAnalyzer::default().infer(
            &outcome,
            FileInferenceContext::new(Some(Utf8Path::new("/repo/work")))
                .with_workspace_root(Some(Utf8Path::new("/repo"))),
        )
    }

    fn path(candidate: &FileAccessCandidate) -> (&PathExpression, FileTargetScope) {
        match &candidate.target {
            FileTarget::Path { expression, scope } => (expression, *scope),
            FileTarget::Workspace { .. } => panic!("expected path candidate"),
        }
    }

    #[test]
    fn infers_argument_and_redirection_accesses() {
        let report = infer("cat input.txt < fallback.txt > output.txt");

        assert_eq!(report.candidates.len(), 3);
        assert!(report.unresolved.is_empty());
        assert_eq!(report.may_read().count(), 2);
        assert_eq!(report.may_modify().count(), 1);

        let (argument, scope) = path(&report.candidates[2]);
        assert_eq!(argument.raw, "input.txt");
        assert_eq!(
            argument.resolved.as_deref(),
            Some(Utf8Path::new("/repo/work/input.txt"))
        );
        assert_eq!(argument.base, PathBase::InvocationCwd);
        assert_eq!(scope, FileTargetScope::Exact);
        assert!(matches!(
            report.candidates[2].origin,
            FileAccessOrigin::Argument { argv_index: 1, .. }
        ));

        assert!(report.candidates.iter().any(|candidate| {
            candidate.access == FileAccessKind::Read
                && path(candidate).0.raw == "fallback.txt"
                && matches!(candidate.origin, FileAccessOrigin::Redirection { .. })
        }));
        assert!(report.candidates.iter().any(|candidate| {
            candidate.access == FileAccessKind::Modify
                && path(candidate).0.raw == "output.txt"
                && matches!(candidate.origin, FileAccessOrigin::Redirection { .. })
        }));
    }

    #[test]
    fn distinguishes_legacy_output_redirection_from_descriptor_duplication() {
        let report = infer("printf hi >&legacy.log 2>&1");

        assert_eq!(report.candidates.len(), 1);
        assert_eq!(report.candidates[0].access, FileAccessKind::Modify);
        assert_eq!(path(&report.candidates[0]).0.raw, "legacy.log");
        assert!(report.unresolved.is_empty());
    }

    #[test]
    fn assigns_source_destination_and_delete_roles() {
        let copied = infer("cp -R src assets backup");
        assert_eq!(
            copied
                .candidates
                .iter()
                .map(|candidate| (
                    path(candidate).0.raw.as_str(),
                    candidate.access,
                    path(candidate).1
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    "src",
                    FileAccessKind::Read,
                    FileTargetScope::ExactOrDescendants
                ),
                (
                    "assets",
                    FileAccessKind::Read,
                    FileTargetScope::ExactOrDescendants
                ),
                (
                    "backup",
                    FileAccessKind::Modify,
                    FileTargetScope::ExactOrDescendants
                ),
            ]
        );

        let moved = infer("mv old new");
        assert_eq!(moved.candidates[0].access, FileAccessKind::MoveSource);
        assert_eq!(moved.candidates[1].access, FileAccessKind::MoveDestination);
        assert_eq!(moved.may_read().count(), 1);
        assert_eq!(moved.may_modify().count(), 2);

        let removed = infer("rm -rf build");
        assert_eq!(removed.candidates[0].access, FileAccessKind::Delete);
        assert_eq!(
            path(&removed.candidates[0]).1,
            FileTargetScope::ExactOrDescendants
        );
    }

    #[test]
    fn models_search_defaults_and_explicit_roots() {
        let implicit = infer("rg needle");
        let (expression, scope) = path(&implicit.candidates[0]);
        assert_eq!(expression.raw, ".");
        assert_eq!(
            expression.resolved.as_deref(),
            Some(Utf8Path::new("/repo/work"))
        );
        assert_eq!(scope, FileTargetScope::Descendants);
        assert!(matches!(
            implicit.candidates[0].origin,
            FileAccessOrigin::WorkingDirectoryDefault { .. }
        ));

        let explicit = infer("grep -R needle src tests");
        assert_eq!(explicit.candidates.len(), 2);
        assert!(explicit.candidates.iter().all(|candidate| {
            candidate.access == FileAccessKind::Read
                && path(candidate).1 == FileTargetScope::ExactOrDescendants
        }));
    }

    #[test]
    fn retains_globs_but_marks_other_dynamic_paths_unresolved() {
        let glob = infer("cat src/*.rs");
        assert_eq!(glob.candidates.len(), 1);
        assert!(glob.unresolved.is_empty());
        let (expression, scope) = path(&glob.candidates[0]);
        assert_eq!(expression.raw, "src/*.rs");
        assert_eq!(scope, FileTargetScope::Glob);

        let dynamic = infer("cat \"$INPUT\"");
        assert!(dynamic.candidates.is_empty());
        assert!(matches!(
            dynamic.unresolved.as_slice(),
            [UnresolvedFileAccess {
                reason: UnresolvedFileAccessReason::DynamicPath {
                    argv_index: Some(1),
                    reasons,
                },
                ..
            }] if reasons.contains(&DynamicReason::ParameterExpansion)
        ));
    }

    #[test]
    fn stops_resolving_relative_paths_after_directory_changes() {
        let report = infer("cat before; cd subdir; cat after");
        assert_eq!(report.candidates.len(), 2);
        assert_eq!(
            path(&report.candidates[0]).0.resolved.as_deref(),
            Some(Utf8Path::new("/repo/work/before"))
        );
        let after = path(&report.candidates[1]).0;
        assert_eq!(after.raw, "after");
        assert_eq!(after.resolved, None);
        assert_eq!(after.base, PathBase::UnknownAfterDirectoryChange);
        assert!(report.unresolved.iter().any(|unresolved| matches!(
            unresolved.reason,
            UnresolvedFileAccessReason::WorkingDirectoryMayHaveChanged
        )));
    }

    #[test]
    fn keeps_recovered_candidates_from_partial_analysis() {
        let outcome = BashAnalyzer::new(BashAnalyzerLimits {
            max_nodes: 5,
            ..BashAnalyzerLimits::default()
        })
        .analyze("cat first; cat second; cat third");
        let report = FileAccessAnalyzer::default().infer(
            &outcome,
            FileInferenceContext::new(Some(Utf8Path::new("/repo"))),
        );

        assert!(matches!(outcome, BashAnalysisOutcome::Partial { .. }));
        assert!(report.unresolved.iter().any(|unresolved| matches!(
            unresolved.reason,
            UnresolvedFileAccessReason::AnalysisIncomplete(_)
        )));
    }

    #[test]
    fn reports_unknown_and_indirect_command_semantics() {
        let unknown = infer("my-tool path/to/file");
        assert!(unknown.candidates.is_empty());
        assert!(matches!(
            unknown.unresolved[0].reason,
            UnresolvedFileAccessReason::UnknownCommandSemantics { ref command }
                if command == "my-tool"
        ));

        let indirect = infer("source ./generated.sh");
        assert_eq!(indirect.candidates.len(), 1);
        assert!(indirect.unresolved.iter().any(|unresolved| matches!(
            unresolved.reason,
            UnresolvedFileAccessReason::IndirectEvaluation { .. }
        )));
    }

    #[test]
    fn models_sed_reads_in_place_modification_and_ambiguous_layouts() {
        let read = infer("sed -n 's/a/b/p' input.txt");
        assert_eq!(read.candidates.len(), 1);
        assert_eq!(read.candidates[0].access, FileAccessKind::Read);
        assert_eq!(path(&read.candidates[0]).0.raw, "input.txt");
        assert!(read.unresolved.is_empty());

        let in_place = infer("sed -i.bak -e 's/a/b/' first.txt second.txt");
        assert_eq!(in_place.candidates.len(), 2);
        assert!(
            in_place
                .candidates
                .iter()
                .all(|candidate| candidate.access == FileAccessKind::ReadModify)
        );
        assert!(in_place.unresolved.is_empty());

        let ambiguous = infer("sed --mystery 's/a/b/' input.txt");
        assert!(ambiguous.candidates.is_empty());
        assert!(ambiguous.unresolved.iter().any(|unresolved| matches!(
            unresolved.reason,
            UnresolvedFileAccessReason::AmbiguousArguments { .. }
        )));
    }

    #[test]
    fn literal_operand_fallback_is_opt_in_and_preserves_unknown_semantics() {
        let outcome = BashAnalyzer::default()
            .analyze("mystery --mode fast path/to/input CONFIG=generated/output.txt bareword");
        let context = FileInferenceContext::new(Some(Utf8Path::new("/repo")));
        let conservative = FileAccessAnalyzer::default().infer(&outcome, context);
        assert!(conservative.candidates.is_empty());

        let heuristic = FileAccessAnalyzer::default()
            .with_unknown_command_fallback(UnknownCommandFallback::LiteralPathOperands)
            .infer(&outcome, context);
        assert_eq!(heuristic.candidates.len(), 2);
        assert!(heuristic.candidates.iter().all(|candidate| {
            candidate.certainty == FileAccessCertainty::Heuristic
                && candidate.inferred_by == "hookkit-literal-operand-fallback"
                && matches!(candidate.origin, FileAccessOrigin::Argument { .. })
        }));
        assert!(
            heuristic
                .candidates
                .iter()
                .any(|candidate| { path(candidate).0.raw == "generated/output.txt" })
        );
        assert!(heuristic.unresolved.iter().any(|unresolved| matches!(
            unresolved.reason,
            UnresolvedFileAccessReason::UnknownCommandSemantics { .. }
        )));
    }

    #[test]
    fn literal_operand_fallback_does_not_resolve_after_directory_change() {
        let outcome = BashAnalyzer::default().analyze("cd subdir; mystery path/to/input");
        let report = FileAccessAnalyzer::default()
            .with_unknown_command_fallback(UnknownCommandFallback::LiteralPathOperands)
            .infer(
                &outcome,
                FileInferenceContext::new(Some(Utf8Path::new("/repo"))),
            );
        assert_eq!(report.candidates.len(), 1);
        let expression = path(&report.candidates[0]).0;
        assert_eq!(expression.resolved, None);
        assert_eq!(expression.base, PathBase::UnknownAfterDirectoryChange);
        assert!(report.unresolved.iter().any(|unresolved| matches!(
            unresolved.reason,
            UnresolvedFileAccessReason::WorkingDirectoryMayHaveChanged
        )));
    }

    #[test]
    fn marks_branch_dependent_accesses_as_conditional() {
        let report = infer("test -f marker && cat input");
        assert_eq!(report.candidates.len(), 1);
        assert_eq!(
            report.candidates[0].certainty,
            FileAccessCertainty::Conditional
        );
    }

    struct LinterSemantics;

    impl CommandFileSemantics for LinterSemantics {
        fn id(&self) -> &str {
            "test-linter"
        }

        fn infer(
            &self,
            command: CommandFileContext<'_>,
            output: &mut FileAccessSink<'_, '_>,
        ) -> bool {
            if command.name() != "project-lint" {
                return false;
            }
            if command.argv_len() == 1 {
                output.emit_workspace(FileAccessKind::ReadModify);
            } else {
                for index in 1..command.argv_len() {
                    output.emit_argument(
                        index,
                        FileAccessKind::ReadModify,
                        FileTargetScope::ExactOrDescendants,
                    );
                }
            }
            true
        }
    }

    #[test]
    fn custom_semantics_extend_and_override_builtins() {
        let analyzer = FileAccessAnalyzer::default().with_semantics(LinterSemantics);
        let outcome = BashAnalyzer::default().analyze("project-lint");
        let report = analyzer.infer(
            &outcome,
            FileInferenceContext::new(Some(Utf8Path::new("/repo")))
                .with_workspace_root(Some(Utf8Path::new("/repo"))),
        );

        assert!(report.unresolved.is_empty());
        assert_eq!(report.candidates[0].inferred_by, "test-linter");
        assert!(matches!(
            &report.candidates[0].target,
            FileTarget::Workspace { root: Some(root) } if root == Utf8Path::new("/repo")
        ));
        assert_eq!(report.candidates[0].access, FileAccessKind::ReadModify);
        assert_eq!(
            report.candidates[0].certainty,
            FileAccessCertainty::Heuristic
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_feature_serializes_file_access_reports() {
        let value = serde_json::to_value(infer("cat input > output")).unwrap();
        assert!(value.get("candidates").is_some());
        assert!(value.get("unresolved").is_some());
    }

    proptest! {
        /// Property: output redirection to a literal relative path always
        /// denotes a direct modification resolved beneath the supplied cwd.
        #[test]
        fn literal_output_redirection_is_a_direct_write(path in "[a-z][a-z0-9_-]{0,10}\\.txt") {
            let source = format!("printf data > {path}");
            let report = FileAccessAnalyzer::default().infer(
                &BashAnalyzer::default().analyze(&source),
                FileInferenceContext::new(Some(Utf8Path::new("/workspace"))),
            );
            let expected = Utf8Path::new("/workspace").join(&path);

            let has_direct_write = report.candidates.iter().any(|candidate| {
                candidate.access == FileAccessKind::Modify
                    && candidate.certainty == FileAccessCertainty::Direct
                    && matches!(&candidate.target, FileTarget::Path { expression, scope: FileTargetScope::Exact }
                        if expression.resolved.as_ref() == Some(&expected))
            });
            prop_assert!(has_direct_write);
        }
    }
}
