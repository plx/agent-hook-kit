//! Best-effort file-access inference over parsed Bash commands.
//!
//! This module reports explicit or command-semantics-derived candidates. It
//! never claims to enumerate every file a process will touch: executables may
//! load configuration, follow symlinks, evaluate generated code, or perform
//! arbitrary I/O that is not represented in their argv.

use std::path::{Component, Path, PathBuf};

use hookkit_core::{Utf8Path, Utf8PathBuf};

use crate::bash::{
    BashAnalysis, BashAnalysisOutcome, CommandOccurrence, DynamicReason, ExecutionContext,
    IncompleteReason, RedirectionOperator, ShellWord, SourceSpan, UnavailableReason,
};

/// Initial path context supplied by the native shell tool call and hook runtime.
#[derive(Debug, Clone, Copy, Default)]
pub struct FileInferenceContext<'a> {
    pub cwd: Option<&'a Utf8Path>,
    pub workspace_root: Option<&'a Utf8Path>,
}

impl<'a> FileInferenceContext<'a> {
    pub const fn new(cwd: Option<&'a Utf8Path>) -> Self {
        Self {
            cwd,
            workspace_root: None,
        }
    }

    pub const fn with_workspace_root(mut self, workspace_root: Option<&'a Utf8Path>) -> Self {
        self.workspace_root = workspace_root;
        self
    }
}

/// An inferred set of possible explicit file accesses plus known blind spots.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FileAccessReport {
    pub candidates: Vec<FileAccessCandidate>,
    pub unresolved: Vec<UnresolvedFileAccess>,
}

impl FileAccessReport {
    pub fn may_read(&self) -> impl Iterator<Item = &FileAccessCandidate> {
        self.candidates
            .iter()
            .filter(|candidate| candidate.access.may_read())
    }

    pub fn may_modify(&self) -> impl Iterator<Item = &FileAccessCandidate> {
        self.candidates
            .iter()
            .filter(|candidate| candidate.access.may_modify())
    }

    pub fn is_fully_resolved(&self) -> bool {
        self.unresolved.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FileAccessCandidate {
    pub target: FileTarget,
    pub access: FileAccessKind,
    /// Certainty of the static association, not a promise that execution will
    /// reach or successfully perform the access.
    pub certainty: FileAccessCertainty,
    pub origin: FileAccessOrigin,
    pub command_span: SourceSpan,
    pub inferred_by: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum FileAccessKind {
    Read,
    Modify,
    ReadModify,
    Enumerate,
    Delete,
    MoveSource,
    MoveDestination,
}

impl FileAccessKind {
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

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum FileTarget {
    Path {
        expression: PathExpression,
        scope: FileTargetScope,
    },
    Workspace {
        root: Option<Utf8PathBuf>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PathExpression {
    pub raw: String,
    pub resolved: Option<Utf8PathBuf>,
    pub base: PathBase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum PathBase {
    Absolute,
    InvocationCwd,
    UnknownAfterDirectoryChange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum FileTargetScope {
    Exact,
    Descendants,
    ExactOrDescendants,
    Glob,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum FileAccessCertainty {
    Direct,
    Conditional,
    Heuristic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum FileAccessOrigin {
    Argument {
        command_index: usize,
        argv_index: usize,
    },
    Redirection {
        command_index: usize,
        redirection_index: usize,
    },
    WorkingDirectoryDefault {
        command_index: usize,
    },
    Rule {
        command_index: usize,
        detail: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct UnresolvedFileAccess {
    pub command_index: Option<usize>,
    pub command_span: Option<SourceSpan>,
    pub raw: Option<String>,
    pub reason: UnresolvedFileAccessReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum UnresolvedFileAccessReason {
    AnalysisIncomplete(IncompleteReason),
    AnalysisUnavailable(UnavailableReason),
    DynamicCommandName {
        reasons: Vec<DynamicReason>,
    },
    DynamicPath {
        argv_index: Option<usize>,
        reasons: Vec<DynamicReason>,
    },
    UnknownCommandSemantics {
        command: String,
    },
    AmbiguousArguments {
        detail: String,
    },
    IndirectEvaluation {
        command: String,
    },
    WorkingDirectoryMayHaveChanged,
    MissingWorkingDirectory,
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
    pub const fn command_index(self) -> usize {
        self.command_index
    }

    pub const fn command(self) -> &'a CommandOccurrence {
        self.command
    }

    pub const fn name(self) -> &'a str {
        self.name
    }

    /// Number of words including `argv[0]`.
    pub fn argv_len(self) -> usize {
        self.command.arguments.len() + 1
    }

    pub fn word(self, argv_index: usize) -> Option<&'a ShellWord> {
        if argv_index == 0 {
            self.command.name.as_ref()
        } else {
            self.command.arguments.get(argv_index - 1)
        }
    }

    pub fn literal(self, argv_index: usize) -> Option<&'a str> {
        self.word(argv_index)?.literal.as_deref()
    }

    pub fn has_literal(self, value: &str) -> bool {
        (1..self.argv_len()).any(|index| self.literal(index) == Some(value))
    }
}

/// Extension point for project- or tool-specific argv semantics.
pub trait CommandFileSemantics: Send + Sync {
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
            (PathBase::Absolute, Some(normalize_utf8(path)))
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
                    Some(normalize_utf8(&cwd.join(path))),
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
}

impl Default for FileAccessAnalyzer {
    fn default() -> Self {
        Self::with_builtins()
    }
}

impl FileAccessAnalyzer {
    pub fn empty() -> Self {
        Self {
            semantics: Vec::new(),
        }
    }

    pub fn with_builtins() -> Self {
        Self {
            semantics: vec![Box::new(BuiltinCommandFileSemantics)],
        }
    }

    pub fn register<S>(&mut self, semantics: S)
    where
        S: CommandFileSemantics + 'static,
    {
        self.semantics.insert(0, Box::new(semantics));
    }

    pub fn with_semantics<S>(mut self, semantics: S) -> Self
    where
        S: CommandFileSemantics + 'static,
    {
        self.register(semantics);
        self
    }

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
            _ => false,
        }
    }
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

fn normalize_utf8(path: &Utf8Path) -> Utf8PathBuf {
    let mut normalized = PathBuf::new();
    for component in Path::new(path.as_str()).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(
                    normalized.components().next_back(),
                    Some(Component::Normal(_))
                ) {
                    normalized.pop();
                } else if !path.is_absolute() {
                    normalized.push(component.as_os_str());
                }
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    Utf8PathBuf::from_path_buf(normalized)
        .expect("normalizing an existing UTF-8 path preserves UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bash::{BashAnalyzer, BashAnalyzerLimits};

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
}
