//! Best-effort file-access inference over parsed Bash commands.
//!
//! This module reports explicit or command-semantics-derived candidates. It
//! never claims to enumerate every file a process will touch: executables may
//! load configuration, follow symlinks, evaluate generated code, or perform
//! arbitrary I/O that is not represented in their argv.

use std::fmt;

use hookkit_core::{Utf8Path, Utf8PathBuf, normalize_utf8_path};

use crate::bash::{
    ArgvStatus, BashAnalysis, BashAnalysisOutcome, CommandOccurrence, ConstructKind, DynamicReason,
    ExecutionContext, IncompleteReason, Redirection, RedirectionOperator, ShellWord, SourceSpan,
    UnavailableReason,
};
use crate::call::{ShellDialect, ShellToolCallRef};

/// Identifier recorded on candidates derived from shell redirections.
const REDIRECTION_RULE: &str = "shell-redirection";
/// Identifier recorded on candidates from the literal-operand fallback.
const FALLBACK_RULE: &str = "hookkit-literal-operand-fallback";
/// Maximum nested command wrappers (`sudo env nice ...`) unwrapped.
const MAX_WRAPPER_DEPTH: usize = 8;

/// Initial path context supplied by the native shell tool call and hook runtime.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct FileInferenceContext<'a> {
    /// Working directory in effect when the shell invocation begins.
    pub cwd: Option<&'a Utf8Path>,
    /// Workspace root, when the surrounding harness identifies one.
    pub workspace_root: Option<&'a Utf8Path>,
    /// Home directory used to resolve `~` paths, when the caller knows it.
    pub home: Option<&'a Utf8Path>,
    /// Shell language the command runs under.
    ///
    /// Unless this is [`ShellDialect::Bash`], syntax whose file effects differ
    /// in zsh (such as `>! file`) is reported as unresolved in addition to its
    /// Bash reading, and [`ShellDialect::PowerShell`] marks the whole analysis
    /// unresolved.
    pub dialect: ShellDialect,
}

impl<'a> FileInferenceContext<'a> {
    /// Creates a context with an optional invocation working directory.
    pub const fn new(cwd: Option<&'a Utf8Path>) -> Self {
        Self {
            cwd,
            workspace_root: None,
            home: None,
            dialect: ShellDialect::Unknown,
        }
    }

    /// Creates a context from a matched shell call: its
    /// [`effective_cwd`](ShellToolCallRef::effective_cwd) and dialect.
    pub fn for_call(call: &'a ShellToolCallRef<'_>) -> Self {
        Self::new(call.effective_cwd()).with_dialect(call.dialect)
    }

    /// Attaches the optional workspace root used by workspace-wide rules.
    pub const fn with_workspace_root(mut self, workspace_root: Option<&'a Utf8Path>) -> Self {
        self.workspace_root = workspace_root;
        self
    }

    /// Attaches the optional home directory used to resolve `~` paths.
    pub const fn with_home(mut self, home: Option<&'a Utf8Path>) -> Self {
        self.home = home;
        self
    }

    /// Sets the shell dialect the command runs under.
    pub const fn with_dialect(mut self, dialect: ShellDialect) -> Self {
        self.dialect = dialect;
        self
    }
}

/// An inferred set of possible explicit file accesses plus known blind spots.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
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
#[non_exhaustive]
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
    /// Source span of the command or redirected statement containing the
    /// access.
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
#[non_exhaustive]
pub struct PathExpression {
    /// Operand value as a string.
    ///
    /// For a literal operand this is the statically evaluated value (quotes and
    /// escapes removed). For a glob or `~` operand it is the word's
    /// quote-removed [`ShellWord::pattern`], with a tilde prefix unexpanded and,
    /// for globs, quoted metacharacters bracket-escaped. For a rule- or
    /// option-derived operand (such as the working-directory default `.`) it is
    /// whatever the rule supplied. It is not guaranteed to be the exact source
    /// text of a shell word.
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
    /// The expression starts with `~` and is relative to the home directory
    /// (`$HOME` at run time). It is resolved only when
    /// [`FileInferenceContext::home`] is supplied.
    Home,
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
    /// A redirection applied to a compound statement, subshell, control-flow
    /// statement, or function definition.
    StatementRedirection {
        /// Zero-based index in [`BashAnalysis::statement_redirections`].
        statement_index: usize,
        /// Zero-based redirection index within the statement.
        redirection_index: usize,
    },
}

/// A possible file effect that static inference could not resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct UnresolvedFileAccess {
    /// Command index, or `None` for statement-level and analysis-wide records.
    pub command_index: Option<usize>,
    /// Command or redirected-statement span, or `None` for analysis-wide
    /// failures.
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
    /// The command interprets an operand as additional executable code, or is
    /// an interactive program that can open other files or run commands.
    IndirectEvaluation {
        /// Literal command name that performs the indirect evaluation.
        command: String,
    },
    /// A preceding `cd`, `pushd`, or `popd` may have changed a relative base.
    WorkingDirectoryMayHaveChanged,
    /// A relative operand was found but the invocation working directory is unknown.
    MissingWorkingDirectory,
    /// The redirection form has file effects the analyzer cannot model,
    /// including forms whose effect depends on the shell dialect.
    UnsupportedRedirection,
    /// The command runs under a shell dialect the Bash analyzer cannot model.
    UnsupportedShellDialect {
        /// Dialect supplied through [`FileInferenceContext::dialect`].
        dialect: ShellDialect,
    },
}

/// Read-only command view supplied to custom semantics implementations.
///
/// When the command is run through a wrapper such as `sudo`, `env`, `nice`,
/// `nohup`, `time`, `timeout`, `command`, `exec`, `builtin`, or `stdbuf`, the
/// view starts at the wrapped command: [`Self::name`] and [`Self::word`] use
/// indexes relative to it, and [`Self::argv_offset`] records the skipped
/// prefix.
#[derive(Debug, Clone, Copy)]
pub struct CommandFileContext<'a> {
    command_index: usize,
    command: &'a CommandOccurrence,
    name: &'a str,
    offset: usize,
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

    /// Returns the position of this view's `argv[0]` in the complete argv:
    /// zero unless wrapper commands were skipped.
    pub const fn argv_offset(self) -> usize {
        self.offset
    }

    /// Number of words including `argv[0]`.
    pub fn argv_len(self) -> usize {
        (self.command.arguments.len() + 1).saturating_sub(self.offset)
    }

    /// Returns an argv word, treating the command name as `argv[0]`.
    pub fn word(self, argv_index: usize) -> Option<&'a ShellWord> {
        match argv_index + self.offset {
            0 => self.command.name.as_ref(),
            full => self.command.arguments.get(full - 1),
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

    const fn full_index(self, argv_index: usize) -> usize {
        argv_index + self.offset
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
    statement_index: Option<usize>,
    inference: FileInferenceContext<'command>,
    cwd_may_have_changed: bool,
    cwd_issue_emitted: bool,
    inferred_by: String,
    report: &'report mut FileAccessReport,
}

impl fmt::Debug for FileAccessSink<'_, '_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FileAccessSink")
            .field("command_index", &self.command.command_index)
            .field("command", &self.command.name)
            .field("statement_index", &self.statement_index)
            .field("cwd_may_have_changed", &self.cwd_may_have_changed)
            .field("inferred_by", &self.inferred_by)
            .finish_non_exhaustive()
    }
}

impl FileAccessSink<'_, '_> {
    /// Emits an access derived from an argv operand.
    ///
    /// A missing operand, or a non-literal operand other than a glob or `~`
    /// pattern, is recorded in [`FileAccessReport::unresolved`] instead. A glob
    /// operand is emitted with [`FileTargetScope::Glob`], and a `~` operand is
    /// emitted with [`PathBase::Home`] while its runtime dependence on `$HOME`
    /// is also recorded as unresolved. `argv_index` uses conventional indexing,
    /// with the (possibly unwrapped) executable at zero.
    pub fn emit_argument(
        &mut self,
        argv_index: usize,
        access: FileAccessKind,
        scope: FileTargetScope,
    ) {
        let full_index = self.command.full_index(argv_index);
        let Some(word) = self.command.word(argv_index) else {
            self.unresolved(
                None,
                UnresolvedFileAccessReason::AmbiguousArguments {
                    detail: format!("missing argv[{full_index}]"),
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
                argv_index: full_index,
            },
            FileAccessCertainty::Direct,
            Some(full_index),
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
        let origin = FileAccessOrigin::Argument {
            command_index: self.command.command_index,
            argv_index: self.command.full_index(argv_index),
        };
        self.emit_pattern(raw, access, scope, origin, certainty);
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
            command_index: self
                .statement_index
                .is_none()
                .then_some(self.command.command_index),
            command_span: Some(self.command.command.span),
            raw,
            reason,
        });
    }

    fn redirection_origin(&self, redirection_index: usize) -> FileAccessOrigin {
        match self.statement_index {
            Some(statement_index) => FileAccessOrigin::StatementRedirection {
                statement_index,
                redirection_index,
            },
            None => FileAccessOrigin::Redirection {
                command_index: self.command.command_index,
                redirection_index,
            },
        }
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
        if let Some(pattern) = &word.pattern {
            let scope = if word.dynamic_reasons.contains(&DynamicReason::Glob) {
                FileTargetScope::Glob
            } else {
                scope
            };
            if !word
                .dynamic_reasons
                .contains(&DynamicReason::TildeExpansion)
            {
                self.emit_path(pattern, access, scope, origin, certainty);
                return;
            }
            // `~` depends on `$HOME` at run time: keep the candidate so
            // home-anchored policies can match it, and keep the gap.
            if is_home_pattern(pattern) {
                self.emit_home_path(pattern, access, scope, origin, certainty);
            }
        }
        self.unresolved(
            Some(word.raw.clone()),
            UnresolvedFileAccessReason::DynamicPath {
                argv_index,
                reasons: word.dynamic_reasons.clone(),
            },
        );
    }

    /// Emits a literal path or a `~`-prefixed home pattern.
    fn emit_pattern(
        &mut self,
        raw: &str,
        access: FileAccessKind,
        scope: FileTargetScope,
        origin: FileAccessOrigin,
        certainty: FileAccessCertainty,
    ) {
        if is_home_pattern(raw) {
            self.emit_home_path(raw, access, scope, origin, certainty);
        } else {
            self.emit_path(raw, access, scope, origin, certainty);
        }
    }

    fn emit_home_path(
        &mut self,
        pattern: &str,
        access: FileAccessKind,
        scope: FileTargetScope,
        origin: FileAccessOrigin,
        certainty: FileAccessCertainty,
    ) {
        let rest = pattern
            .strip_prefix("~/")
            .or_else(|| (pattern == "~").then_some(""))
            .unwrap_or(pattern);
        let resolved = self
            .inference
            .home
            .filter(|home| home.is_absolute())
            .map(|home| normalize_utf8_path(home.join(rest)));
        self.push_path(
            pattern,
            resolved,
            PathBase::Home,
            access,
            scope,
            origin,
            certainty,
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
            match self.inference.cwd.filter(|cwd| cwd.is_absolute()) {
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
        self.push_path(raw, resolved, base, access, scope, origin, certainty);
    }

    #[allow(clippy::too_many_arguments)]
    fn push_path(
        &mut self,
        raw: &str,
        resolved: Option<Utf8PathBuf>,
        base: PathBase,
        access: FileAccessKind,
        scope: FileTargetScope,
        origin: FileAccessOrigin,
        certainty: FileAccessCertainty,
    ) {
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

fn is_home_pattern(pattern: &str) -> bool {
    pattern == "~" || pattern.starts_with("~/")
}

/// File-access inference engine. Custom rules registered later take precedence
/// over the bundled command table.
pub struct FileAccessAnalyzer {
    semantics: Vec<Box<dyn CommandFileSemantics>>,
    unknown_command_fallback: UnknownCommandFallback,
}

impl fmt::Debug for FileAccessAnalyzer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FileAccessAnalyzer")
            .field(
                "semantics",
                &self
                    .semantics
                    .iter()
                    .map(|semantics| semantics.id())
                    .collect::<Vec<_>>(),
            )
            .field("unknown_command_fallback", &self.unknown_command_fallback)
            .finish()
    }
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
        let mut report = match outcome {
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
        };
        if context.dialect == ShellDialect::PowerShell {
            report.unresolved.push(UnresolvedFileAccess {
                command_index: None,
                command_span: None,
                raw: None,
                reason: UnresolvedFileAccessReason::UnsupportedShellDialect {
                    dialect: context.dialect,
                },
            });
        }
        report
    }

    /// Infers file access from a parsed Bash analysis.
    ///
    /// Relative paths following any directory-changing command are intentionally
    /// left unresolved because static analysis cannot know whether that command
    /// executed or succeeded. A directory change inside a loop or function body
    /// affects every command after the start of the outermost enclosing loop or
    /// function, because later iterations and calls run after it.
    pub fn infer_analysis(
        &self,
        analysis: &BashAnalysis,
        context: FileInferenceContext<'_>,
    ) -> FileAccessReport {
        let directory_change = earliest_directory_change(analysis);
        let cwd_changed_before =
            |offset: usize| directory_change.is_some_and(|change| change < offset);

        let mut report = FileAccessReport::default();
        for (command_index, command) in analysis.commands.iter().enumerate() {
            let name = command.name.as_ref().map_or("", |name| {
                name.literal.as_deref().unwrap_or(name.raw.as_str())
            });
            let mut sink = FileAccessSink {
                command: CommandFileContext {
                    command_index,
                    command,
                    name,
                    offset: 0,
                },
                statement_index: None,
                inference: context,
                cwd_may_have_changed: cwd_changed_before(command.span.start_byte),
                cwd_issue_emitted: false,
                inferred_by: REDIRECTION_RULE.to_owned(),
                report: &mut report,
            };

            infer_redirections(&command.redirections, &mut sink);

            // A redirection-only command (`> file`, `$(< file)`) runs nothing.
            let Some(name_word) = &command.name else {
                continue;
            };
            if name_word.literal.is_none() {
                sink.unresolved(
                    Some(name_word.raw.clone()),
                    UnresolvedFileAccessReason::DynamicCommandName {
                        reasons: name_word.dynamic_reasons.clone(),
                    },
                );
                continue;
            }

            let unresolved_before = sink.report.unresolved.len();
            match unwrap_wrappers(sink.command) {
                Unwrapped::Command(target) => {
                    sink.command = target;
                    let mut handled = false;
                    for semantics in &self.semantics {
                        sink.inferred_by.clear();
                        sink.inferred_by.push_str(semantics.id());
                        if semantics.infer(target, &mut sink) {
                            handled = true;
                            break;
                        }
                    }
                    if !handled {
                        sink.unresolved(
                            Some(target.name.to_owned()),
                            UnresolvedFileAccessReason::UnknownCommandSemantics {
                                command: target.name.to_owned(),
                            },
                        );
                    }
                }
                Unwrapped::Finished => {}
                Unwrapped::Unresolved(raw, reason) => sink.unresolved(raw, reason),
            }

            let ambiguous = sink.report.unresolved[unresolved_before..]
                .iter()
                .any(|unresolved| {
                    matches!(
                        unresolved.reason,
                        UnresolvedFileAccessReason::UnknownCommandSemantics { .. }
                            | UnresolvedFileAccessReason::AmbiguousArguments { .. }
                            | UnresolvedFileAccessReason::IndirectEvaluation { .. }
                    )
                });
            if ambiguous
                && self.unknown_command_fallback == UnknownCommandFallback::LiteralPathOperands
            {
                sink.inferred_by.clear();
                sink.inferred_by.push_str(FALLBACK_RULE);
                let view = sink.command;
                infer_literal_path_operands(view, &mut sink);
            }
        }

        for (statement_index, statement) in analysis.statement_redirections.iter().enumerate() {
            let view = CommandOccurrence {
                span: statement.span,
                raw: String::new(),
                name: None,
                arguments: Vec::new(),
                argv: ArgvStatus::Literal(Vec::new()),
                context: statement.context.clone(),
                redirections: Vec::new(),
            };
            let mut sink = FileAccessSink {
                command: CommandFileContext {
                    command_index: statement.commands.start,
                    command: &view,
                    name: "",
                    offset: 0,
                },
                statement_index: Some(statement_index),
                inference: context,
                cwd_may_have_changed: cwd_changed_before(statement.span.start_byte),
                cwd_issue_emitted: false,
                inferred_by: REDIRECTION_RULE.to_owned(),
                report: &mut report,
            };
            infer_redirections(&statement.redirections, &mut sink);
        }
        report
    }
}

/// Returns the earliest byte offset after which a directory change may have
/// taken effect, or `None` when no command changes directory.
fn earliest_directory_change(analysis: &BashAnalysis) -> Option<usize> {
    // Outermost loop and function bodies, sorted and disjoint.
    let mut deferred = analysis
        .constructs
        .iter()
        .filter(|construct| {
            matches!(
                construct.kind,
                ConstructKind::Loop | ConstructKind::FunctionDefinition
            )
        })
        .map(|construct| construct.span.start_byte..construct.span.end_byte)
        .collect::<Vec<_>>();
    deferred.sort_by_key(|range| (range.start, std::cmp::Reverse(range.end)));
    let mut outermost: Vec<std::ops::Range<usize>> = Vec::with_capacity(deferred.len());
    for range in deferred {
        if outermost
            .last()
            .is_none_or(|outer| range.start >= outer.end)
        {
            outermost.push(range);
        }
    }

    let mut earliest: Option<usize> = None;
    for (command_index, command) in analysis.commands.iter().enumerate() {
        let Some(name) = command
            .name
            .as_ref()
            .and_then(|name| name.literal.as_deref())
        else {
            continue;
        };
        let view = CommandFileContext {
            command_index,
            command,
            name,
            offset: 0,
        };
        let Unwrapped::Command(target) = unwrap_wrappers(view) else {
            continue;
        };
        if !matches!(builtin_command_name(target.name), "cd" | "pushd" | "popd") {
            continue;
        }
        let mut offset = command.span.start_byte;
        if command.context.iter().any(|context| {
            matches!(
                context,
                ExecutionContext::Loop | ExecutionContext::FunctionDefinition
            )
        }) {
            let index = outermost.partition_point(|range| range.start <= offset);
            if let Some(range) = index
                .checked_sub(1)
                .map(|index| &outermost[index])
                .filter(|range| range.contains(&offset))
            {
                offset = range.start;
            }
        }
        earliest = Some(earliest.map_or(offset, |earliest| earliest.min(offset)));
    }
    earliest
}

/// Returns the name that selects bundled semantics: the name itself, or the
/// basename of an absolute path in a standard system binary directory.
pub(crate) fn builtin_command_name(name: &str) -> &str {
    const SYSTEM_BIN_DIRECTORIES: &[&str] = &[
        "/bin/",
        "/sbin/",
        "/usr/bin/",
        "/usr/sbin/",
        "/usr/local/bin/",
        "/opt/homebrew/bin/",
    ];
    for directory in SYSTEM_BIN_DIRECTORIES {
        if let Some(base) = name
            .strip_prefix(directory)
            .filter(|base| !base.is_empty() && !base.contains('/'))
        {
            return base;
        }
    }
    name
}

/// Bundled semantics for common inspection and direct file-manipulation tools.
///
/// Names match exactly, or as the basename of an absolute path in a standard
/// system binary directory such as `/usr/bin`.
///
/// `apply_patch` is recognized without candidates: its targets are in the
/// patch body, which `hookkit-tool-access` parses. A standalone consumer must
/// not read a fully resolved report for an `apply_patch` command as proof that
/// no file is modified.
#[derive(Debug, Clone, Copy, Default)]
pub struct BuiltinCommandFileSemantics;

impl CommandFileSemantics for BuiltinCommandFileSemantics {
    fn id(&self) -> &str {
        "hookkit-builtin-command-semantics"
    }

    fn infer(&self, command: CommandFileContext<'_>, output: &mut FileAccessSink<'_, '_>) -> bool {
        match builtin_command_name(command.name()) {
            "echo" | "printf" | "pwd" | "true" | "false" | ":" | "test" | "[" | "cd" | "pushd"
            | "popd" => true,
            "cat" | "bat" | "nl" => {
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
            "less" | "more" | "most" => {
                emit_operands(
                    command,
                    output,
                    1,
                    Options::none(),
                    FileAccessKind::Read,
                    FileTargetScope::Exact,
                );
                // Interactive pagers can open other files (`:e`) and run shell
                // commands (`!`) from input the hook never sees, for example
                // through Codex `write_stdin` on a TTY session.
                output.unresolved(
                    Some(command.name().to_owned()),
                    UnresolvedFileAccessReason::IndirectEvaluation {
                        command: command.name().to_owned(),
                    },
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
            "rg" | "grep" | "egrep" | "fgrep" => {
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
            "find" if finds_and_executes(command) => {
                indirect_evaluation(command, output);
                true
            }
            name if is_code_interpreter(name) => {
                indirect_evaluation(command, output);
                true
            }
            "apply_patch" => true,
            _ => false,
        }
    }
}

fn indirect_evaluation(command: CommandFileContext<'_>, output: &mut FileAccessSink<'_, '_>) {
    output.unresolved(
        Some(command.command().raw.clone()),
        UnresolvedFileAccessReason::IndirectEvaluation {
            command: command.name().to_owned(),
        },
    );
}

/// Shells, language runtimes, and command runners whose operands are code or
/// commands with unknown file effects.
fn is_code_interpreter(name: &str) -> bool {
    matches!(
        name,
        "eval"
            | "bash"
            | "sh"
            | "zsh"
            | "dash"
            | "ksh"
            | "mksh"
            | "fish"
            | "csh"
            | "tcsh"
            | "pwsh"
            | "powershell"
            | "xargs"
            | "parallel"
            | "watch"
            | "perl"
            | "ruby"
            | "node"
            | "nodejs"
            | "deno"
            | "bun"
            | "php"
            | "lua"
            | "tclsh"
            | "expect"
            | "Rscript"
            | "osascript"
            | "awk"
            | "gawk"
            | "mawk"
            | "nawk"
    ) || name
        .strip_prefix("python")
        .is_some_and(|version| version.chars().all(|c| c.is_ascii_digit() || c == '.'))
}

fn finds_and_executes(command: CommandFileContext<'_>) -> bool {
    (1..command.argv_len()).any(|index| {
        let word = command.word(index);
        match word.and_then(|word| word.literal.as_deref()) {
            Some(value) => matches!(value, "-exec" | "-execdir" | "-ok" | "-okdir"),
            // A dynamic word could expand to an executing action.
            None => word.is_some(),
        }
    })
}

/// Result of skipping wrapper commands that run another command.
enum Unwrapped<'a> {
    /// The command that finally runs.
    Command(CommandFileContext<'a>),
    /// The wrapper runs no command (for example `env` alone or `command -v`).
    Finished,
    /// The wrapped command could not be identified.
    Unresolved(Option<String>, UnresolvedFileAccessReason),
}

/// Option grammar of a command wrapper.
struct WrapperSpec {
    /// Options with a separate (`-u root`) or attached (`-uroot`,
    /// `--user=root`) value.
    values: &'static [&'static str],
    /// Options without a value.
    flags: &'static [&'static str],
    /// Options after which no command runs.
    informational: &'static [&'static str],
    /// Options that run the command through a shell or split a string.
    indirect: &'static [&'static str],
    /// Leading `NAME=VALUE` words set environment variables.
    assignments: bool,
    /// Operands that precede the command, such as the `timeout` duration.
    leading_operands: usize,
    /// Numeric short options such as `nice -10`.
    numeric_short: bool,
}

const EMPTY_WRAPPER: WrapperSpec = WrapperSpec {
    values: &[],
    flags: &[],
    informational: &[],
    indirect: &[],
    assignments: false,
    leading_operands: 0,
    numeric_short: false,
};

fn wrapper_spec(name: &str) -> Option<WrapperSpec> {
    Some(match name {
        "env" => WrapperSpec {
            values: &["-u", "--unset"],
            flags: &[
                "-",
                "-i",
                "--ignore-environment",
                "-0",
                "--null",
                "-v",
                "--debug",
            ],
            indirect: &["-S", "--split-string"],
            assignments: true,
            ..EMPTY_WRAPPER
        },
        "sudo" => WrapperSpec {
            values: &[
                "-u",
                "--user",
                "-g",
                "--group",
                "-p",
                "--prompt",
                "-C",
                "--close-from",
                "-r",
                "--role",
                "-t",
                "--type",
                "-T",
                "--command-timeout",
                "-U",
                "--other-user",
            ],
            flags: &[
                "-A",
                "--askpass",
                "-b",
                "--background",
                "-B",
                "--bell",
                "-E",
                "--preserve-env",
                "-H",
                "--set-home",
                "-k",
                "--reset-timestamp",
                "-n",
                "--non-interactive",
                "-N",
                "--no-update",
                "-P",
                "--preserve-groups",
                "-S",
                "--stdin",
            ],
            informational: &[
                "-l",
                "--list",
                "-v",
                "--validate",
                "-V",
                "--version",
                "-K",
                "--remove-timestamp",
            ],
            indirect: &["-i", "--login", "-s", "--shell"],
            assignments: true,
            ..EMPTY_WRAPPER
        },
        "doas" => WrapperSpec {
            values: &["-u", "-C"],
            flags: &["-n"],
            indirect: &["-s"],
            ..EMPTY_WRAPPER
        },
        "nice" => WrapperSpec {
            values: &["-n", "--adjustment"],
            numeric_short: true,
            ..EMPTY_WRAPPER
        },
        "nohup" | "builtin" => EMPTY_WRAPPER,
        "time" => WrapperSpec {
            values: &["-f", "--format"],
            flags: &["-p", "--portability", "-v", "--verbose", "-q", "--quiet"],
            ..EMPTY_WRAPPER
        },
        "timeout" => WrapperSpec {
            values: &["-s", "--signal", "-k", "--kill-after"],
            flags: &["--preserve-status", "--foreground", "-v", "--verbose"],
            leading_operands: 1,
            ..EMPTY_WRAPPER
        },
        "command" => WrapperSpec {
            flags: &["-p"],
            informational: &["-v", "-V"],
            ..EMPTY_WRAPPER
        },
        "exec" => WrapperSpec {
            values: &["-a"],
            flags: &["-c", "-l"],
            ..EMPTY_WRAPPER
        },
        "stdbuf" => WrapperSpec {
            values: &["-i", "-o", "-e", "--input", "--output", "--error"],
            ..EMPTY_WRAPPER
        },
        _ => return None,
    })
}

fn unwrap_wrappers(mut command: CommandFileContext<'_>) -> Unwrapped<'_> {
    for _ in 0..MAX_WRAPPER_DEPTH {
        let wrapper = builtin_command_name(command.name);
        let Some(spec) = wrapper_spec(wrapper) else {
            return Unwrapped::Command(command);
        };
        let mut index = 1;
        // Options.
        loop {
            let Some(word) = command.word(index) else {
                return Unwrapped::Finished;
            };
            let Some(value) = word.literal.as_deref() else {
                return Unwrapped::Unresolved(
                    Some(word.raw.clone()),
                    UnresolvedFileAccessReason::AmbiguousArguments {
                        detail: format!("dynamic argument to {wrapper} may be an option"),
                    },
                );
            };
            if value == "--" {
                index += 1;
                break;
            }
            let is_option = spec.flags.contains(&value) || (value.starts_with('-') && value != "-");
            if !is_option {
                break;
            }
            let name = value.split_once('=').map_or(value, |(name, _)| name);
            if spec.informational.contains(&value) {
                return Unwrapped::Finished;
            }
            if spec.indirect.contains(&name)
                || spec
                    .indirect
                    .iter()
                    .any(|option| option.len() == 2 && value.starts_with(option))
            {
                return Unwrapped::Unresolved(
                    Some(command.command.raw.clone()),
                    UnresolvedFileAccessReason::IndirectEvaluation {
                        command: wrapper.to_owned(),
                    },
                );
            }
            if spec.flags.contains(&value) || (name != value && spec.flags.contains(&name)) {
                index += 1;
                continue;
            }
            if spec.values.contains(&value) {
                if command.word(index + 1).is_none() {
                    return Unwrapped::Finished;
                }
                index += 2;
                continue;
            }
            let attached_value = (name != value && value.starts_with("--"))
                || (!value.starts_with("--") && value.len() > 2);
            if attached_value
                && spec.values.iter().any(|option| {
                    value.starts_with(option) && (option.len() == 2 || name == *option)
                })
            {
                index += 1;
                continue;
            }
            if spec.numeric_short && value[1..].chars().all(|c| c.is_ascii_digit()) {
                index += 1;
                continue;
            }
            let short_bundle = !value.starts_with("--")
                && value[1..].chars().all(|letter| {
                    spec.flags
                        .iter()
                        .any(|flag| flag.len() == 2 && flag.ends_with(letter))
                });
            if short_bundle {
                index += 1;
                continue;
            }
            return Unwrapped::Unresolved(
                Some(value.to_owned()),
                UnresolvedFileAccessReason::AmbiguousArguments {
                    detail: format!("unrecognized option {value} for {wrapper}"),
                },
            );
        }
        if spec.assignments {
            while command
                .literal(index)
                .is_some_and(|value| assignment_value(value).is_some())
            {
                index += 1;
            }
        }
        index += spec.leading_operands;
        let Some(word) = command.word(index) else {
            return Unwrapped::Finished;
        };
        let Some(name) = word.literal.as_deref() else {
            return Unwrapped::Unresolved(
                Some(word.raw.clone()),
                UnresolvedFileAccessReason::DynamicCommandName {
                    reasons: word.dynamic_reasons.clone(),
                },
            );
        };
        command = CommandFileContext {
            name,
            offset: command.offset + index,
            ..command
        };
    }
    Unwrapped::Unresolved(
        Some(command.command.raw.clone()),
        UnresolvedFileAccessReason::AmbiguousArguments {
            detail: "too many nested command wrappers".to_owned(),
        },
    )
}

fn infer_literal_path_operands(
    command: CommandFileContext<'_>,
    output: &mut FileAccessSink<'_, '_>,
) {
    let mut operands_only = false;
    for argv_index in 1..command.argv_len() {
        let Some(word) = command.word(argv_index) else {
            return;
        };
        let (value, pattern) = match (&word.literal, &word.pattern) {
            (Some(literal), _) => (literal.as_str(), false),
            (None, Some(pattern)) => (pattern.as_str(), true),
            // A dynamic word may be an option or operand; skip it.
            (None, None) => continue,
        };
        if !operands_only && !pattern && value == "--" {
            operands_only = true;
            continue;
        }
        if !operands_only && value.starts_with('-') {
            continue;
        }
        let value = assignment_value(value).unwrap_or(value);
        if !is_path_like(value) {
            continue;
        }
        let glob = pattern && word.dynamic_reasons.contains(&DynamicReason::Glob);
        output.emit_argument_literal(
            argv_index,
            value,
            FileAccessKind::ReadModify,
            if glob {
                FileTargetScope::Glob
            } else {
                FileTargetScope::ExactOrDescendants
            },
            FileAccessCertainty::Heuristic,
        );
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
        || value == "~"
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

/// Whether the command's standard input is a pipe or an input redirection.
fn stdin_is_redirected(command: &CommandOccurrence) -> bool {
    command.context.contains(&ExecutionContext::PipelineInput)
        || command.redirections.iter().any(|redirection| {
            matches!(
                redirection.operator,
                Some(
                    RedirectionOperator::Input
                        | RedirectionOperator::DuplicateInput
                        | RedirectionOperator::HereDocument
                        | RedirectionOperator::HereDocumentStripTabs
                        | RedirectionOperator::HereString
                )
            ) && redirection
                .descriptor
                .as_deref()
                .is_none_or(|descriptor| descriptor == "0")
        })
}

fn infer_search(command: CommandFileContext<'_>, output: &mut FileAccessSink<'_, '_>) {
    let ripgrep = builtin_command_name(command.name()) == "rg";
    let mut index = 1;
    let mut query_seen = false;
    let mut operands_only = false;
    let mut recursive = false;
    let mut ambiguous = false;
    let mut paths = Vec::new();
    while index < command.argv_len() {
        let Some(word) = command.word(index) else {
            break;
        };
        let Some(literal) = word.literal.as_deref() else {
            // Before `--`, an expansion may be an option such as `-f FILE`;
            // an unquoted query expansion may split into a query and paths.
            if (!operands_only || !query_seen) && !ambiguous {
                ambiguous = true;
                output.unresolved(
                    Some(word.raw.clone()),
                    UnresolvedFileAccessReason::AmbiguousArguments {
                        detail: format!(
                            "dynamic {} argument may be an option or several words",
                            command.name()
                        ),
                    },
                );
            }
            if query_seen {
                paths.push(index);
            } else {
                query_seen = true;
            }
            index += 1;
            continue;
        };
        if !operands_only && literal == "--" {
            operands_only = true;
            index += 1;
            continue;
        }
        if !operands_only {
            if matches!(literal, "-e" | "--regexp") {
                if index + 1 >= command.argv_len() {
                    output.unresolved(
                        Some(literal.to_owned()),
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
            if literal.starts_with("--regexp=") {
                query_seen = true;
                index += 1;
                continue;
            }
            if matches!(literal, "-f" | "--file") {
                if index + 1 >= command.argv_len() {
                    output.unresolved(
                        Some(literal.to_owned()),
                        UnresolvedFileAccessReason::AmbiguousArguments {
                            detail: "search pattern-file option is missing its value".to_owned(),
                        },
                    );
                    return;
                }
                output.emit_argument(index + 1, FileAccessKind::Read, FileTargetScope::Exact);
                query_seen = true;
                index += 2;
                continue;
            }
            if is_search_boolean(literal) {
                recursive |= matches!(literal, "-r" | "-R" | "--recursive");
                index += 1;
                continue;
            }
            if is_search_value_option(literal) {
                if index + 1 >= command.argv_len() {
                    output.unresolved(
                        Some(literal.to_owned()),
                        UnresolvedFileAccessReason::AmbiguousArguments {
                            detail: "search option is missing its value".to_owned(),
                        },
                    );
                    return;
                }
                index += 2;
                continue;
            }
            if literal.starts_with('-') && literal != "-" {
                output.unresolved(
                    Some(literal.to_owned()),
                    UnresolvedFileAccessReason::AmbiguousArguments {
                        detail: format!("unrecognized search option for {}", command.name()),
                    },
                );
                return;
            }
        }
        if !query_seen {
            query_seen = true;
        } else if literal != "-" {
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
        // Without a path, grep reads stdin unless it is recursive; ripgrep
        // searches the working directory unless stdin is piped or redirected.
        let searches_cwd = if ripgrep {
            !stdin_is_redirected(command.command())
        } else {
            recursive
        };
        if searches_cwd {
            output.emit_working_directory(FileAccessKind::Read, FileTargetScope::Descendants);
        }
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
    let mut sandbox = false;
    let mut script_file = false;
    let mut scripts: Vec<(usize, &str)> = Vec::new();
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
                    | "--regexp-extended"
                    | "-s"
                    | "--separate"
                    | "-u"
                    | "--unbuffered"
                    | "-z"
                    | "--null-data"
                    | "--posix"
                    | "--debug"
            )
        {
            index += 1;
            continue;
        }
        if !operands_only && literal == "--sandbox" {
            sandbox = true;
            index += 1;
            continue;
        }
        if !operands_only && matches!(literal, "-e" | "--expression") {
            let Some(script) = command.literal(index + 1) else {
                sed_ambiguous(
                    output,
                    literal,
                    "sed expression option is missing its script",
                );
                return;
            };
            scripts.push((index + 1, script));
            script_configured = true;
            index += 2;
            continue;
        }
        if !operands_only {
            if let Some(script) = literal.strip_prefix("--expression=").or_else(|| {
                literal
                    .strip_prefix("-e")
                    .filter(|script| !script.is_empty())
            }) {
                scripts.push((index, script));
                script_configured = true;
                index += 1;
                continue;
            }
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
            script_file = true;
            index += 2;
            continue;
        }
        if !operands_only {
            if let Some(file) = literal
                .strip_prefix("--file=")
                .filter(|value| !value.is_empty())
                .or_else(|| literal.strip_prefix("-f").filter(|value| !value.is_empty()))
            {
                output.emit_argument_literal(
                    index,
                    file,
                    FileAccessKind::Read,
                    FileTargetScope::Exact,
                    FileAccessCertainty::Direct,
                );
                script_configured = true;
                script_file = true;
                index += 1;
                continue;
            }
        }
        if !operands_only && matches!(literal, "-i" | "--in-place") {
            in_place = true;
            index += 1;
            // BSD sed takes the backup suffix as a separate, often empty,
            // argument (`sed -i '' script file`).
            if literal == "-i" && command.literal(index) == Some("") {
                index += 1;
            }
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
            scripts.push((index, literal));
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
    // `--sandbox` makes GNU sed reject the commands that open files or run
    // programs, so script contents need no inspection.
    if !sandbox {
        if script_file {
            sed_ambiguous(
                output,
                command.command().raw.as_str(),
                "sed script file may read, write, or execute files",
            );
        }
        for (script_index, script) in scripts {
            match sed_script_effects(script) {
                Some(effects) => {
                    for effect in effects {
                        match effect {
                            SedEffect::Read(path) => output.emit_argument_literal(
                                script_index,
                                path,
                                FileAccessKind::Read,
                                FileTargetScope::Exact,
                                FileAccessCertainty::Direct,
                            ),
                            SedEffect::Write(path) => output.emit_argument_literal(
                                script_index,
                                path,
                                FileAccessKind::Modify,
                                FileTargetScope::Exact,
                                FileAccessCertainty::Direct,
                            ),
                            SedEffect::Execute => output.unresolved(
                                Some(script.to_owned()),
                                UnresolvedFileAccessReason::IndirectEvaluation {
                                    command: command.name().to_owned(),
                                },
                            ),
                        }
                    }
                }
                None => sed_ambiguous(
                    output,
                    script,
                    "sed script could not be checked for commands that read, write, or execute",
                ),
            }
        }
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

/// A sed command that opens a file or runs a program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SedEffect<'a> {
    /// `r`/`R` reads a file.
    Read(&'a str),
    /// `w`/`W` or the `s///w` flag writes a file.
    Write(&'a str),
    /// `e` or the `s///e` flag runs a command.
    Execute,
}

/// Scans a sed script for commands that read, write, or execute. Returns
/// `None` when the script cannot be parsed confidently.
fn sed_script_effects(script: &str) -> Option<Vec<SedEffect<'_>>> {
    let bytes = script.as_bytes();
    let mut effects = Vec::new();
    let mut index = 0;
    let skip_blank = |index: &mut usize| {
        while *index < bytes.len() && matches!(bytes[*index], b' ' | b'\t') {
            *index += 1;
        }
    };
    let end_of_line = |from: usize| {
        bytes[from..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |offset| from + offset)
    };
    let filename = |index: &mut usize| -> Option<&str> {
        while *index < bytes.len() && matches!(bytes[*index], b' ' | b'\t') {
            *index += 1;
        }
        let end = end_of_line(*index);
        let name = script.get(*index..end)?;
        *index = end;
        (!name.is_empty()).then_some(name)
    };

    while index < bytes.len() {
        while index < bytes.len() && matches!(bytes[index], b' ' | b'\t' | b'\n' | b';') {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        index = skip_sed_address(bytes, index)?;
        skip_blank(&mut index);
        if bytes.get(index) == Some(&b',') {
            index += 1;
            skip_blank(&mut index);
            index = skip_sed_address(bytes, index)?;
            skip_blank(&mut index);
        }
        while index < bytes.len() && matches!(bytes[index], b'!' | b' ' | b'\t') {
            index += 1;
        }
        let command = *bytes.get(index)?;
        index += 1;
        match command {
            b'{' | b'}' | b'=' | b'd' | b'D' | b'g' | b'G' | b'h' | b'H' | b'n' | b'N' | b'p'
            | b'P' | b'x' | b'z' | b'F' => {}
            b'q' | b'Q' | b'l' | b'L' => {
                skip_blank(&mut index);
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    index += 1;
                }
            }
            b'#' => index = end_of_line(index),
            b':' | b'b' | b't' | b'T' | b'v' => {
                while index < bytes.len() && !matches!(bytes[index], b';' | b'\n') {
                    index += 1;
                }
            }
            b'a' | b'i' | b'c' => {
                // Text runs to the end of the line; a trailing backslash
                // continues it onto the next line.
                loop {
                    let end = end_of_line(index);
                    let continued = end > index && bytes[end - 1] == b'\\';
                    index = (end + 1).min(bytes.len());
                    if !continued || end >= bytes.len() {
                        break;
                    }
                }
            }
            b'r' | b'R' => effects.push(SedEffect::Read(filename(&mut index)?)),
            b'w' | b'W' => effects.push(SedEffect::Write(filename(&mut index)?)),
            b'e' => {
                effects.push(SedEffect::Execute);
                index = end_of_line(index);
            }
            b'y' => {
                let delimiter = *bytes.get(index)?;
                index = skip_sed_delimited(bytes, index + 1, delimiter)?;
                index = skip_sed_delimited(bytes, index, delimiter)?;
            }
            b's' => {
                let delimiter = *bytes.get(index)?;
                if matches!(delimiter, b'\n' | b'\\') {
                    return None;
                }
                index = skip_sed_delimited(bytes, index + 1, delimiter)?;
                index = skip_sed_delimited(bytes, index, delimiter)?;
                while index < bytes.len() {
                    match bytes[index] {
                        b'g' | b'p' | b'i' | b'I' | b'm' | b'M' | b'0'..=b'9' => index += 1,
                        b'e' => {
                            effects.push(SedEffect::Execute);
                            index += 1;
                        }
                        b'w' => {
                            index += 1;
                            effects.push(SedEffect::Write(filename(&mut index)?));
                        }
                        b';' | b'\n' | b'}' | b'#' | b' ' | b'\t' => break,
                        _ => return None,
                    }
                }
            }
            _ => return None,
        }
    }
    Some(effects)
}

/// Skips an optional sed address: a line number, `$`, `/regex/`,
/// `\cregexc`, or a GNU `first~step` or `+N`/`~N` form.
fn skip_sed_address(bytes: &[u8], mut index: usize) -> Option<usize> {
    match bytes.get(index) {
        Some(b'0'..=b'9') => {
            while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b'~') {
                index += 1;
            }
        }
        Some(b'$') => index += 1,
        Some(b'+' | b'~') => {
            index += 1;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
        }
        Some(b'/') => index = skip_sed_delimited(bytes, index + 1, b'/')?,
        Some(b'\\') => {
            let delimiter = *bytes.get(index + 1)?;
            index = skip_sed_delimited(bytes, index + 2, delimiter)?;
        }
        _ => return Some(index),
    }
    // Regex address flags.
    while index < bytes.len() && matches!(bytes[index], b'I' | b'M') {
        index += 1;
    }
    Some(index)
}

/// Skips past the next unescaped `delimiter`, returning the index after it.
fn skip_sed_delimited(bytes: &[u8], mut index: usize, delimiter: u8) -> Option<usize> {
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'\n' => return None,
            byte if byte == delimiter => return Some(index + 1),
            _ => index += 1,
        }
    }
    None
}

fn sed_ambiguous(output: &mut FileAccessSink<'_, '_>, raw: &str, detail: &str) {
    output.unresolved(
        Some(raw.to_owned()),
        UnresolvedFileAccessReason::AmbiguousArguments {
            detail: detail.to_owned(),
        },
    );
}

/// Search options without a value that change neither which files are read
/// nor whether other programs run.
fn is_search_boolean(literal: &str) -> bool {
    matches!(
        literal,
        "-n" | "--line-number"
            | "-i"
            | "--ignore-case"
            | "-F"
            | "--fixed-strings"
            | "-E"
            | "--extended-regexp"
            | "-w"
            | "--word-regexp"
            | "-x"
            | "--line-regexp"
            | "-v"
            | "--invert-match"
            | "-c"
            | "--count"
            | "-o"
            | "--only-matching"
            | "-q"
            | "--quiet"
            | "-s"
            | "--no-messages"
            | "-h"
            | "--no-filename"
            | "-H"
            | "--with-filename"
            | "-R"
            | "-r"
            | "--recursive"
            | "--hidden"
            | "--no-ignore"
            | "-l"
            | "--files-with-matches"
    )
}

fn is_search_value_option(literal: &str) -> bool {
    matches!(
        literal,
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
            | "-A"
            | "--after-context"
            | "-B"
            | "--before-context"
            | "-C"
            | "--context"
    )
}

/// The final path component of an operand, used when a destination directory
/// receives a source by name.
fn operand_basename(path: &str) -> Option<&str> {
    let trimmed = path.trim_end_matches('/');
    let base = trimmed.rsplit('/').next()?;
    (!base.is_empty() && base != "." && base != "..").then_some(base)
}

fn join_operand(directory: &str, name: &str) -> String {
    if directory.ends_with('/') {
        format!("{directory}{name}")
    } else {
        format!("{directory}/{name}")
    }
}

/// Emits heuristic `destination/basename(source)` candidates: when the
/// destination is a directory, each source lands inside it under its own name.
fn emit_destination_children(
    command: CommandFileContext<'_>,
    output: &mut FileAccessSink<'_, '_>,
    sources: &[usize],
    destination: usize,
    access: FileAccessKind,
    source_scope: FileTargetScope,
) {
    let Some(directory) = command.literal(destination) else {
        return;
    };
    for source in sources {
        let Some(word) = command.word(*source) else {
            continue;
        };
        let (value, scope) = match (&word.literal, &word.pattern) {
            (Some(literal), _) => (literal.as_str(), source_scope),
            (None, Some(pattern)) if word.dynamic_reasons == [DynamicReason::Glob] => {
                (pattern.as_str(), FileTargetScope::Glob)
            }
            _ => continue,
        };
        let Some(base) = operand_basename(value) else {
            continue;
        };
        output.emit_argument_literal(
            destination,
            &join_operand(directory, base),
            access,
            scope,
            FileAccessCertainty::Heuristic,
        );
    }
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
    // `mv` moves whole directories without a flag.
    let source_scope = if moving || recursive {
        FileTargetScope::ExactOrDescendants
    } else {
        FileTargetScope::Exact
    };
    let (destination, sources) = operands.split_last().expect("checked nonempty above");
    for source in sources {
        output.emit_argument(
            *source,
            if moving {
                FileAccessKind::MoveSource
            } else {
                FileAccessKind::Read
            },
            source_scope,
        );
    }
    let destination_access = if moving {
        FileAccessKind::MoveDestination
    } else {
        FileAccessKind::Modify
    };
    // The destination may be an existing directory that receives the sources.
    output.emit_argument(
        *destination,
        destination_access,
        FileTargetScope::ExactOrDescendants,
    );
    emit_destination_children(
        command,
        output,
        sources,
        *destination,
        destination_access,
        source_scope,
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
    let (destination, sources) = operands.split_last().expect("checked nonempty above");
    for source in sources {
        output.emit_argument(*source, FileAccessKind::Read, FileTargetScope::Exact);
    }
    output.emit_argument(
        *destination,
        FileAccessKind::Modify,
        FileTargetScope::ExactOrDescendants,
    );
    emit_destination_children(
        command,
        output,
        sources,
        *destination,
        FileAccessKind::Modify,
        FileTargetScope::Exact,
    );
}

/// Whether a `>&` target duplicates, moves, or closes a descriptor rather than
/// naming a file.
fn is_descriptor_target(value: &str) -> bool {
    let digits = value.strip_suffix('-').unwrap_or(value);
    value == "-" || (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
}

fn infer_redirections(redirections: &[Redirection], output: &mut FileAccessSink<'_, '_>) {
    let dialect_sensitive = output.inference.dialect != ShellDialect::Bash;
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
            // Without a descriptor, Bash accepts `>&word` as the legacy
            // spelling of `&>word`; a numeric word, `-`, or `N-` duplicates,
            // closes, or moves a descriptor. With an explicit descriptor Bash
            // rejects a filename as an ambiguous redirect, but zsh writes it.
            Some(RedirectionOperator::DuplicateOutput) => match redirection
                .target
                .as_ref()
                .and_then(|target| target.literal.as_deref())
            {
                Some(value) if is_descriptor_target(value) => None,
                _ => Some(FileAccessKind::Modify),
            },
            None => {
                output.unresolved(
                    Some(redirection.raw.clone()),
                    UnresolvedFileAccessReason::UnsupportedRedirection,
                );
                None
            }
        };
        let Some(access) = access else {
            continue;
        };
        let Some(target) = &redirection.target else {
            output.unresolved(
                Some(redirection.raw.clone()),
                UnresolvedFileAccessReason::UnsupportedRedirection,
            );
            continue;
        };
        let origin = output.redirection_origin(redirection_index);
        output.emit_word(
            target,
            access,
            FileTargetScope::Exact,
            origin.clone(),
            FileAccessCertainty::Direct,
            None,
        );
        if dialect_sensitive && access == FileAccessKind::Modify && target.raw.starts_with('!') {
            // zsh reads `>!`/`>>!` as noclobber-overriding output to the next
            // word, which Bash treats as a command argument.
            output.unresolved(
                Some(redirection.raw.clone()),
                UnresolvedFileAccessReason::UnsupportedRedirection,
            );
            emit_zsh_clobber_target(redirection, target, origin, output);
        }
    }
}

fn emit_zsh_clobber_target(
    redirection: &Redirection,
    target: &ShellWord,
    origin: FileAccessOrigin,
    output: &mut FileAccessSink<'_, '_>,
) {
    if target.raw == "!" {
        // Arguments are in source order; the word the grammar attached after
        // `!` lies inside the redirection's span.
        let arguments = &output.command.command.arguments;
        let index = arguments.partition_point(|word| word.span.start_byte < target.span.end_byte);
        if let Some(word) = arguments
            .get(index)
            .filter(|word| word.span.end_byte <= redirection.span.end_byte)
        {
            output.emit_word(
                word,
                FileAccessKind::Modify,
                FileTargetScope::Exact,
                origin,
                FileAccessCertainty::Heuristic,
                None,
            );
        }
    } else if let Some(path) = target
        .literal
        .as_deref()
        .and_then(|literal| literal.strip_prefix('!'))
        .filter(|path| !path.is_empty())
    {
        output.emit_path(
            path,
            FileAccessKind::Modify,
            FileTargetScope::Exact,
            origin,
            FileAccessCertainty::Heuristic,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bash::{BashAnalyzer, BashAnalyzerLimits};
    use proptest::prelude::*;

    fn infer(source: &str) -> FileAccessReport {
        infer_with(
            source,
            FileInferenceContext::new(Some(Utf8Path::new("/repo/work"))),
        )
    }

    fn infer_with(source: &str, context: FileInferenceContext<'_>) -> FileAccessReport {
        let outcome = BashAnalyzer::default().analyze(source);
        FileAccessAnalyzer::default().infer(
            &outcome,
            context.with_workspace_root(Some(Utf8Path::new("/repo"))),
        )
    }

    fn path(candidate: &FileAccessCandidate) -> (&PathExpression, FileTargetScope) {
        match &candidate.target {
            FileTarget::Path { expression, scope } => (expression, *scope),
            FileTarget::Workspace { .. } => panic!("expected path candidate"),
        }
    }

    fn summary(report: &FileAccessReport) -> Vec<(&str, FileAccessKind, FileTargetScope)> {
        report
            .candidates
            .iter()
            .map(|candidate| {
                let (expression, scope) = path(candidate);
                (expression.raw.as_str(), candidate.access, scope)
            })
            .collect()
    }

    fn has_gap(
        report: &FileAccessReport,
        matches: impl Fn(&UnresolvedFileAccessReason) -> bool,
    ) -> bool {
        report.unresolved.iter().any(|gap| matches(&gap.reason))
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
        let report = infer("printf hi >&legacy.log 2>&1 3>&2-");

        assert_eq!(report.candidates.len(), 1);
        assert_eq!(report.candidates[0].access, FileAccessKind::Modify);
        assert_eq!(path(&report.candidates[0]).0.raw, "legacy.log");
        assert!(report.unresolved.is_empty());
    }

    #[test]
    fn explicit_descriptor_duplication_to_a_word_is_a_write() {
        // Bash rejects `2>& file`, but zsh writes stderr to the file.
        let report = infer("echo x 2>& .env");
        assert_eq!(
            summary(&report),
            [(".env", FileAccessKind::Modify, FileTargetScope::Exact)]
        );

        let dynamic = infer("echo x 2>&$FD");
        assert!(has_gap(&dynamic, |reason| matches!(
            reason,
            UnresolvedFileAccessReason::DynamicPath { .. }
        )));
    }

    #[test]
    fn assigns_source_destination_and_delete_roles() {
        let copied = infer("cp -R src assets backup");
        let direct = copied
            .candidates
            .iter()
            .filter(|candidate| candidate.certainty != FileAccessCertainty::Heuristic)
            .map(|candidate| {
                (
                    path(candidate).0.raw.as_str(),
                    candidate.access,
                    path(candidate).1,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            direct,
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
        let heuristic = copied
            .candidates
            .iter()
            .filter(|candidate| candidate.certainty == FileAccessCertainty::Heuristic)
            .map(|candidate| path(candidate).0.raw.as_str())
            .collect::<Vec<_>>();
        assert_eq!(heuristic, ["backup/src", "backup/assets"]);

        let moved = infer("mv old new");
        assert_eq!(moved.candidates[0].access, FileAccessKind::MoveSource);
        assert_eq!(moved.candidates[1].access, FileAccessKind::MoveDestination);
        assert_eq!(moved.may_read().count(), 1);
        assert!(moved.is_fully_resolved());

        let removed = infer("rm -rf build");
        assert_eq!(removed.candidates[0].access, FileAccessKind::Delete);
        assert_eq!(
            path(&removed.candidates[0]).1,
            FileTargetScope::ExactOrDescendants
        );
    }

    #[test]
    fn moves_and_copies_cover_directory_contents() {
        let moved = infer("mv secrets /tmp/x");
        assert_eq!(
            path(&moved.candidates[0]),
            (
                &PathExpression {
                    raw: "secrets".into(),
                    resolved: Some("/repo/work/secrets".into()),
                    base: PathBase::InvocationCwd,
                },
                FileTargetScope::ExactOrDescendants
            )
        );

        let copied = infer("cp /tmp/.env .");
        assert!(copied.candidates.iter().any(|candidate| {
            candidate.access == FileAccessKind::Modify
                && path(candidate).0.resolved.as_deref() == Some(Utf8Path::new("/repo/work/.env"))
        }));

        let linked = infer("ln -s /etc/passwd links/");
        assert!(linked.candidates.iter().any(|candidate| {
            path(candidate).0.raw == "links/passwd" && candidate.access == FileAccessKind::Modify
        }));

        let globbed = infer("cp /tmp/*.env dest");
        assert!(globbed.candidates.iter().any(|candidate| {
            path(candidate).0.raw == "dest/*.env" && path(candidate).1 == FileTargetScope::Glob
        }));
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

        let recursive = infer("grep -r needle");
        assert_eq!(
            summary(&recursive),
            [(".", FileAccessKind::Read, FileTargetScope::Descendants)]
        );

        let pattern_file = infer("grep -f patterns.txt src");
        assert_eq!(
            summary(&pattern_file),
            [
                ("patterns.txt", FileAccessKind::Read, FileTargetScope::Exact),
                (
                    "src",
                    FileAccessKind::Read,
                    FileTargetScope::ExactOrDescendants
                ),
            ]
        );
    }

    #[test]
    fn searches_reading_stdin_do_not_read_the_working_tree() {
        for source in [
            "ls | grep foo",
            "git log | grep -i fix",
            "echo hi | rg hi",
            "grep foo",
            "rg foo < input.txt",
            "rg foo <<< text",
        ] {
            let report = infer(source);
            assert!(
                !report.candidates.iter().any(|candidate| {
                    path(candidate).0.raw == "." && candidate.access == FileAccessKind::Read
                }),
                "{source}: {report:?}"
            );
        }
        let piped_explicit = infer("ls | grep foo src");
        assert_eq!(
            piped_explicit
                .candidates
                .iter()
                .filter(|candidate| candidate.access == FileAccessKind::Read)
                .map(|candidate| path(candidate).0.raw.as_str())
                .collect::<Vec<_>>(),
            ["src"]
        );
    }

    #[test]
    fn dynamic_search_words_are_ambiguous() {
        for source in [
            "O='-f .env'; grep $O foo",
            "grep \"$O\" foo",
            "grep -- $Q foo",
        ] {
            let report = infer(source);
            assert!(
                has_gap(&report, |reason| matches!(
                    reason,
                    UnresolvedFileAccessReason::AmbiguousArguments { .. }
                )),
                "{source}"
            );
            assert!(
                report
                    .candidates
                    .iter()
                    .any(|candidate| path(candidate).0.raw == "foo"),
                "{source}"
            );
        }
    }

    #[test]
    fn retains_globs_but_marks_other_dynamic_paths_unresolved() {
        let glob = infer("cat src/*.rs");
        assert_eq!(glob.candidates.len(), 1);
        assert!(glob.unresolved.is_empty());
        let (expression, scope) = path(&glob.candidates[0]);
        assert_eq!(expression.raw, "src/*.rs");
        assert_eq!(scope, FileTargetScope::Glob);

        let quoted = infer("cat \".en\"v*");
        let (expression, scope) = path(&quoted.candidates[0]);
        assert_eq!(expression.raw, ".env*");
        assert_eq!(
            expression.resolved.as_deref(),
            Some(Utf8Path::new("/repo/work/.env*"))
        );
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
    fn tilde_operands_become_home_candidates_and_keep_their_gap() {
        let unresolved = infer("cat ~/.ssh/id_rsa");
        let (expression, scope) = path(&unresolved.candidates[0]);
        assert_eq!(expression.raw, "~/.ssh/id_rsa");
        assert_eq!(expression.base, PathBase::Home);
        assert_eq!(expression.resolved, None);
        assert_eq!(scope, FileTargetScope::Exact);
        assert!(has_gap(&unresolved, |reason| matches!(
            reason,
            UnresolvedFileAccessReason::DynamicPath { reasons, .. }
                if reasons.contains(&DynamicReason::TildeExpansion)
        )));

        let resolved = infer_with(
            "cp ~/.ssh/id_rsa /tmp/k; ls ~/.aws/*",
            FileInferenceContext::new(Some(Utf8Path::new("/repo")))
                .with_home(Some(Utf8Path::new("/Users/me"))),
        );
        assert!(resolved.candidates.iter().any(|candidate| {
            path(candidate).0.resolved.as_deref() == Some(Utf8Path::new("/Users/me/.ssh/id_rsa"))
        }));
        assert!(resolved.candidates.iter().any(|candidate| {
            path(candidate).0.resolved.as_deref() == Some(Utf8Path::new("/Users/me/.aws/*"))
                && path(candidate).1 == FileTargetScope::Glob
        }));

        // A named user's home is not modeled.
        let other = infer("cat ~root/.profile");
        assert!(other.candidates.is_empty());
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
    fn directory_changes_in_loops_functions_and_wrappers_taint_relative_paths() {
        for source in [
            "for d in a b; do cat key; cd secrets; done",
            "f() { cd secrets; }; cat key",
            "builtin cd secrets; cat key",
            "command cd secrets; cat key",
        ] {
            let report = infer(source);
            let key = report
                .candidates
                .iter()
                .find(|candidate| path(candidate).0.raw == "key")
                .unwrap_or_else(|| panic!("{source}"));
            assert_eq!(
                path(key).0.base,
                PathBase::UnknownAfterDirectoryChange,
                "{source}"
            );
        }
        let before = infer("cat key; for d in a; do cd $d; done");
        assert_eq!(path(&before.candidates[0]).0.base, PathBase::InvocationCwd);
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

        for (source, command) in [
            ("python3 -c 'print(open(\".env\").read())'", "python3"),
            ("node -e 'x'", "node"),
            ("awk '{print}' file.txt", "awk"),
            ("find . -name x -exec cat {} +", "find"),
            ("less README.md", "less"),
        ] {
            let report = infer(source);
            assert!(
                has_gap(&report, |reason| matches!(
                    reason,
                    UnresolvedFileAccessReason::IndirectEvaluation { command: name } if name == command
                )),
                "{source}: {report:?}"
            );
        }
        // Pagers still report the file they open.
        assert_eq!(summary(&infer("less README.md")).len(), 1);
    }

    #[test]
    fn wrapper_commands_are_unwrapped() {
        let sudo = infer("sudo -u root rm -rf secrets");
        assert_eq!(
            summary(&sudo),
            [(
                "secrets",
                FileAccessKind::Delete,
                FileTargetScope::ExactOrDescendants
            )]
        );
        assert!(matches!(
            sudo.candidates[0].origin,
            FileAccessOrigin::Argument { argv_index: 5, .. }
        ));
        assert!(sudo.is_fully_resolved());

        for source in [
            "env -i FOO=1 cat .env",
            "nice -n 5 cat .env",
            "nohup cat .env",
            "timeout -s KILL 5 cat .env",
            "command cat .env",
            "exec cat .env",
            "time cat .env",
            "stdbuf -oL cat .env",
            "sudo env cat .env",
            "/usr/bin/cat .env",
        ] {
            assert_eq!(
                summary(&infer(source)),
                [(".env", FileAccessKind::Read, FileTargetScope::Exact)],
                "{source}"
            );
        }
        assert!(infer("command -v cat").is_fully_resolved());
        assert!(infer("env").is_fully_resolved());
        assert!(has_gap(&infer("sudo -s cat .env"), |reason| matches!(
            reason,
            UnresolvedFileAccessReason::IndirectEvaluation { .. }
        )));
        assert!(has_gap(&infer("env $CMD .env"), |reason| matches!(
            reason,
            UnresolvedFileAccessReason::AmbiguousArguments { .. }
        )));
        assert!(has_gap(
            &infer("sudo --mystery cat .env"),
            |reason| matches!(
                reason,
                UnresolvedFileAccessReason::AmbiguousArguments { .. }
            )
        ));
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

        let bsd = infer("sed -i '' 's/a/b/' file.txt");
        assert_eq!(
            summary(&bsd),
            [(
                "file.txt",
                FileAccessKind::ReadModify,
                FileTargetScope::Exact
            )]
        );
        assert!(bsd.is_fully_resolved());

        let ambiguous = infer("sed --mystery 's/a/b/' input.txt");
        assert!(ambiguous.candidates.is_empty());
        assert!(ambiguous.unresolved.iter().any(|unresolved| matches!(
            unresolved.reason,
            UnresolvedFileAccessReason::AmbiguousArguments { .. }
        )));
    }

    #[test]
    fn sed_scripts_that_open_files_or_run_commands_are_reported() {
        let read = infer("sed 'r .env' README.md");
        assert!(summary(&read).contains(&(".env", FileAccessKind::Read, FileTargetScope::Exact)));

        for source in [
            "sed -n '1w .env' x",
            "sed 's/a/b/w .env' x",
            "sed -e '/x/,$W .env' x",
            "sed -i '' '$a\\\nfoo\nw .env' x",
        ] {
            let report = infer(source);
            assert!(
                summary(&report).contains(&(
                    ".env",
                    FileAccessKind::Modify,
                    FileTargetScope::Exact
                )),
                "{source}: {report:?}"
            );
        }
        for source in ["sed '1e cat .env' x", "sed 's/x/date/e' x"] {
            assert!(
                has_gap(&infer(source), |reason| matches!(
                    reason,
                    UnresolvedFileAccessReason::IndirectEvaluation { .. }
                )),
                "{source}"
            );
        }
        assert!(has_gap(&infer("sed 'Z' x"), |reason| matches!(
            reason,
            UnresolvedFileAccessReason::AmbiguousArguments { .. }
        )));
        assert!(has_gap(&infer("sed -f script.sed x"), |reason| matches!(
            reason,
            UnresolvedFileAccessReason::AmbiguousArguments { .. }
        )));
        let sandboxed = infer("sed --sandbox 'w .env' x");
        assert_eq!(
            summary(&sandboxed),
            [("x", FileAccessKind::Read, FileTargetScope::Exact)]
        );
    }

    #[test]
    fn statement_and_bare_redirections_are_inferred() {
        let group = infer("{ cat; } < .env");
        assert_eq!(
            summary(&group),
            [(".env", FileAccessKind::Read, FileTargetScope::Exact)]
        );
        assert!(matches!(
            group.candidates[0].origin,
            FileAccessOrigin::StatementRedirection {
                statement_index: 0,
                redirection_index: 0
            }
        ));

        let function = infer("f() { echo x; } > .env; f");
        assert_eq!(
            function.candidates[0].certainty,
            FileAccessCertainty::Conditional
        );

        let bare = infer("> .env");
        assert_eq!(
            summary(&bare),
            [(".env", FileAccessKind::Modify, FileTargetScope::Exact)]
        );
        assert!(bare.is_fully_resolved());

        let substitution = infer("echo \"$(< .env)\"");
        assert_eq!(
            summary(&substitution),
            [(".env", FileAccessKind::Read, FileTargetScope::Exact)]
        );
    }

    #[test]
    fn zsh_clobber_redirections_are_dialect_gaps() {
        let report = infer("echo PWNED >! .env");
        assert!(has_gap(&report, |reason| matches!(
            reason,
            UnresolvedFileAccessReason::UnsupportedRedirection
        )));
        assert!(report.candidates.iter().any(|candidate| {
            path(candidate).0.raw == ".env"
                && candidate.access == FileAccessKind::Modify
                && candidate.certainty == FileAccessCertainty::Heuristic
        }));

        let attached = infer("echo PWNED >!.env");
        assert!(
            attached
                .candidates
                .iter()
                .any(|candidate| path(candidate).0.raw == ".env")
        );

        let bash = infer_with(
            "echo PWNED >! .env",
            FileInferenceContext::new(Some(Utf8Path::new("/repo")))
                .with_dialect(ShellDialect::Bash),
        );
        assert!(bash.is_fully_resolved());
        assert_eq!(
            summary(&bash),
            [("!", FileAccessKind::Modify, FileTargetScope::Exact)]
        );

        let powershell = infer_with(
            "Get-Content .env",
            FileInferenceContext::new(Some(Utf8Path::new("/repo")))
                .with_dialect(ShellDialect::PowerShell),
        );
        assert!(has_gap(&powershell, |reason| matches!(
            reason,
            UnresolvedFileAccessReason::UnsupportedShellDialect {
                dialect: ShellDialect::PowerShell
            }
        )));
    }

    #[test]
    fn relative_cwd_is_never_used_as_a_resolution_base() {
        let report = infer_with(
            "cat .env",
            FileInferenceContext::new(Some(Utf8Path::new("."))),
        );
        assert_eq!(path(&report.candidates[0]).0.resolved, None);
        assert!(has_gap(&report, |reason| matches!(
            reason,
            UnresolvedFileAccessReason::MissingWorkingDirectory
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
    fn literal_operand_fallback_covers_home_and_dynamic_neighbors() {
        let analyzer = FileAccessAnalyzer::default()
            .with_unknown_command_fallback(UnknownCommandFallback::LiteralPathOperands);
        let context = FileInferenceContext::new(Some(Utf8Path::new("/repo")))
            .with_home(Some(Utf8Path::new("/Users/me")));

        let dd = analyzer.infer(
            &BashAnalyzer::default().analyze("dd if=~/.ssh/id_rsa of=/tmp/k"),
            context,
        );
        assert!(dd.candidates.iter().any(|candidate| {
            path(candidate).0.base == PathBase::Home
                && path(candidate).0.resolved.as_deref()
                    == Some(Utf8Path::new("/Users/me/.ssh/id_rsa"))
        }));

        let mixed = analyzer.infer(
            &BashAnalyzer::default().analyze("mystery $OPT secrets/key"),
            context,
        );
        assert!(
            mixed
                .candidates
                .iter()
                .any(|candidate| path(candidate).0.raw == "secrets/key")
        );

        let interpreter = analyzer.infer(
            &BashAnalyzer::default().analyze("python3 tool.py .env"),
            context,
        );
        assert!(
            interpreter
                .candidates
                .iter()
                .any(|candidate| path(candidate).0.raw == ".env")
        );
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

        // Custom semantics see the command behind a wrapper.
        let wrapped = analyzer.infer(
            &BashAnalyzer::default().analyze("nice project-lint src"),
            FileInferenceContext::new(Some(Utf8Path::new("/repo"))),
        );
        assert_eq!(wrapped.candidates[0].inferred_by, "test-linter");
        assert!(matches!(
            wrapped.candidates[0].origin,
            FileAccessOrigin::Argument { argv_index: 2, .. }
        ));
        assert!(format!("{analyzer:?}").contains("test-linter"));
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

        /// Property: a word written after a redirection target is still an
        /// operand of the command, whatever redirection precedes it.
        #[test]
        fn words_after_redirections_remain_operands(
            redirect in prop::sample::select(vec!["2>/dev/null", "2>&1", "</dev/null", ">out", "<<<x"]),
            name in "[a-z]{1,8}\\.env",
        ) {
            let source = format!("rm {redirect} {name}");
            let report = FileAccessAnalyzer::default().infer(
                &BashAnalyzer::default().analyze(&source),
                FileInferenceContext::new(Some(Utf8Path::new("/workspace"))),
            );
            let deleted = report.candidates.iter().any(|candidate| {
                candidate.access == FileAccessKind::Delete
                    && matches!(&candidate.target, FileTarget::Path { expression, .. } if expression.raw == name)
            });
            prop_assert!(deleted);
        }
    }
}
