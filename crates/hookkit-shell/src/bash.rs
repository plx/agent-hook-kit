//! Bounded Tree-sitter Bash analysis.
//!
//! Literal word recovery is adapted from OpenAI Codex's Apache-2.0-licensed
//! `codex-rs/shell-command/src/bash.rs` at commit
//! `9e552e9d15ba52bed7077d5357f3e18e330f8f38`. This implementation has been
//! substantially changed to retain per-command uncertainty, nesting context,
//! redirections, and partial-analysis status.
//!
//! Where the Tree-sitter grammar's tree shape differs from how Bash executes a
//! command, the analyzer follows Bash:
//!
//! - words that the grammar attaches to a redirection after its target
//!   (`rm 2>/dev/null .env`) or to a here-document start (`rm <<EOF .env`)
//!   are command arguments;
//! - redirections the grammar hangs on a whole `&&`/`||` list or pipeline
//!   (`cd dir && apply_patch <<EOF`) belong to its last simple command;
//! - a backslash-newline line continuation inside a word joins its pieces;
//! - comma and sequence brace expansion (`.e{n,}v`, `{1..3}`) is expanded
//!   statically into separate argv words.

use std::collections::HashMap;
use std::ops::Range;
use std::time::{Duration, Instant};

use tree_sitter::{Node, ParseOptions, Parser, Point, Tree};

/// Resource limits applied before and during parsing and AST traversal.
///
/// Construct values with [`Default`] and the `with_*` builders so new limits
/// can be added without breaking callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct BashAnalyzerLimits {
    /// Maximum UTF-8 source length accepted before parsing.
    pub max_source_bytes: usize,
    /// Wall-clock budget passed to Tree-sitter's cancellation callback. The
    /// budget covers the main parse and any embedded re-parse of command
    /// substitutions the grammar leaves unparsed.
    pub max_parse_time: Duration,
    /// Maximum AST nodes visited while extracting facts.
    pub max_nodes: usize,
    /// Maximum recursive AST traversal depth. Chains of `&&`/`||` list
    /// elements are walked iteratively and do not count toward the depth.
    pub max_depth: usize,
}

impl Default for BashAnalyzerLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 256 * 1024,
            max_parse_time: Duration::from_millis(100),
            max_nodes: 50_000,
            max_depth: 128,
        }
    }
}

impl BashAnalyzerLimits {
    /// Returns the limits with a different pre-parse source-size limit.
    pub const fn with_max_source_bytes(mut self, max_source_bytes: usize) -> Self {
        self.max_source_bytes = max_source_bytes;
        self
    }

    /// Returns the limits with a different parser wall-clock budget.
    pub const fn with_max_parse_time(mut self, max_parse_time: Duration) -> Self {
        self.max_parse_time = max_parse_time;
        self
    }

    /// Returns the limits with a different visited-node limit.
    pub const fn with_max_nodes(mut self, max_nodes: usize) -> Self {
        self.max_nodes = max_nodes;
        self
    }

    /// Returns the limits with a different traversal-depth limit.
    pub const fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = max_depth;
        self
    }
}

/// Reusable Bash analyzer configuration.
#[derive(Debug, Clone, Copy, Default)]
pub struct BashAnalyzer {
    limits: BashAnalyzerLimits,
}

impl BashAnalyzer {
    /// Creates an analyzer with explicit resource limits.
    pub const fn new(limits: BashAnalyzerLimits) -> Self {
        Self { limits }
    }

    /// Returns the configured resource limits.
    pub const fn limits(&self) -> BashAnalyzerLimits {
        self.limits
    }

    /// Parses Bash syntax without executing or expanding it.
    pub fn analyze(&self, source: &str) -> BashAnalysisOutcome {
        if source.len() > self.limits.max_source_bytes {
            return BashAnalysisOutcome::Unavailable(UnavailableReason::InputTooLarge {
                actual_bytes: source.len(),
                max_bytes: self.limits.max_source_bytes,
            });
        }

        let started = Instant::now();
        let tree = match parse_bash(source, None, self.limits.max_parse_time, started) {
            Ok(tree) => tree,
            Err(ParseFailure::Initialization(message)) => {
                return BashAnalysisOutcome::Unavailable(UnavailableReason::ParserInitialization(
                    message,
                ));
            }
            Err(ParseFailure::TimeLimit) => {
                return BashAnalysisOutcome::Unavailable(UnavailableReason::ParseTimeLimit {
                    max_millis: duration_millis(self.limits.max_parse_time),
                });
            }
            Err(ParseFailure::NoTree) => {
                return BashAnalysisOutcome::Unavailable(UnavailableReason::ParserReturnedNoTree);
            }
        };

        let root = tree.root_node();
        let mut walker = Walker::new(source, self.limits, started);
        walker.visit(root, &mut Vec::new(), 0);
        if root.has_error() {
            walker.mark_incomplete(IncompleteReason::SyntaxErrors);
        }
        let (analysis, incomplete) = walker.finish();

        match incomplete {
            Some(reason) => BashAnalysisOutcome::Partial { analysis, reason },
            None => BashAnalysisOutcome::Complete(analysis),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
/// Result of bounded Bash parsing and fact extraction.
pub enum BashAnalysisOutcome {
    /// Parsing and traversal completed without syntax errors or limit hits.
    Complete(BashAnalysis),
    /// Some facts were recovered, but the report is known to be incomplete.
    Partial {
        /// Facts recovered before or despite the incomplete condition.
        analysis: BashAnalysis,
        /// Condition that made the report incomplete.
        reason: IncompleteReason,
    },
    /// No analysis facts are available.
    Unavailable(UnavailableReason),
}

impl BashAnalysisOutcome {
    /// Returns all facts recovered from complete or partial analysis.
    pub fn analysis(&self) -> Option<&BashAnalysis> {
        match self {
            Self::Complete(analysis) | Self::Partial { analysis, .. } => Some(analysis),
            Self::Unavailable(_) => None,
        }
    }

    /// Reports whether analysis is complete and free of known syntax errors.
    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Complete(_))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
/// Commands and notable constructs recovered from one Bash source string.
pub struct BashAnalysis {
    /// Command occurrences in traversal/source order.
    pub commands: Vec<CommandOccurrence>,
    /// Notable syntax constructs in traversal/source order.
    pub constructs: Vec<ConstructOccurrence>,
    /// Redirections applied to a compound statement, subshell, control-flow
    /// statement, declaration, test, or function definition rather than to one
    /// simple command, in the order their statements finished traversal.
    ///
    /// Redirections on a simple command, including ones the grammar hangs on
    /// an enclosing `&&`/`||` list or pipeline, are reported on the owning
    /// [`CommandOccurrence`] instead.
    pub statement_redirections: Vec<StatementRedirection>,
}

impl BashAnalysis {
    /// Literal argv recovered for individual commands. Redirections and other
    /// surrounding constructs remain visible separately and must not be ignored
    /// when making a policy decision.
    ///
    /// A redirection-only command such as `> file` or `$(< file)` yields an
    /// empty argv.
    pub fn literal_commands(&self) -> impl Iterator<Item = &[String]> {
        self.commands
            .iter()
            .filter_map(CommandOccurrence::literal_argv)
    }

    /// Iterates over literal command names recovered without expansion.
    pub fn command_names(&self) -> impl Iterator<Item = &str> {
        self.commands
            .iter()
            .filter_map(|command| command.name.as_ref())
            .filter_map(|name| name.literal.as_deref())
    }

    /// Whether Bash syntax in the report contains runtime-dependent words or
    /// substitutions. This does not recognize semantic indirection performed
    /// by literal commands such as `eval` or `source`.
    pub fn contains_dynamic_syntax(&self) -> bool {
        self.commands
            .iter()
            .any(|command| matches!(command.argv, ArgvStatus::Dynamic { .. }))
            || self.constructs.iter().any(|construct| {
                matches!(
                    construct.kind,
                    ConstructKind::CommandSubstitution
                        | ConstructKind::ProcessSubstitution
                        | ConstructKind::ParameterExpansion
                        | ConstructKind::ArithmeticExpansion
                        | ConstructKind::BraceExpansion
                )
            })
    }

    /// Iterates over occurrences of one construct kind.
    pub fn constructs(&self, kind: ConstructKind) -> impl Iterator<Item = &ConstructOccurrence> {
        self.constructs
            .iter()
            .filter(move |construct| construct.kind == kind)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
/// One simple command occurrence and its surrounding syntax context.
pub struct CommandOccurrence {
    /// Byte/line span of the complete command node.
    pub span: SourceSpan,
    /// Exact source slice covered by [`Self::span`].
    pub raw: String,
    /// Command-name word (`argv[0]`), or `None` for a redirection-only command
    /// such as `> file`, `< file`, or `$(< file)`.
    ///
    /// This is the first word in source order, following Bash rather than the
    /// grammar when words precede the grammar's command name.
    pub name: Option<ShellWord>,
    /// Argument words excluding the command name, in source order.
    ///
    /// Static brace expansion can produce several words from one source word;
    /// each shares that word's span and raw text.
    pub arguments: Vec<ShellWord>,
    /// Complete literal argv or reasons exact recovery was impossible.
    pub argv: ArgvStatus,
    /// Ordered enclosing execution contexts, outermost first.
    pub context: Vec<ExecutionContext>,
    /// Redirections attached to the command, in source order.
    pub redirections: Vec<Redirection>,
}

impl CommandOccurrence {
    /// Returns the complete recovered argv only when every word is literal.
    pub fn literal_argv(&self) -> Option<&[String]> {
        match &self.argv {
            ArgvStatus::Literal(argv) => Some(argv),
            ArgvStatus::Dynamic { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
/// Whether a command's complete argv was statically recoverable.
pub enum ArgvStatus {
    /// Exact argv after static quote removal, concatenation, line-continuation
    /// removal, and brace expansion.
    Literal(Vec<String>),
    /// At least one word depends on runtime behavior or unsupported syntax.
    Dynamic {
        /// Deduplicated reasons literal recovery was not sound.
        reasons: Vec<DynamicReason>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
/// One shell word with exact source and conservative literal recovery.
pub struct ShellWord {
    /// Byte/line span of the word. A word joined by backslash-newline line
    /// continuations spans all of its pieces.
    pub span: SourceSpan,
    /// Exact source slice covered by [`Self::span`].
    pub raw: String,
    /// Statically evaluated value, or `None` when runtime behavior is needed.
    pub literal: Option<String>,
    /// Reasons the word could not be recovered literally.
    pub dynamic_reasons: Vec<DynamicReason>,
    /// Quote-removed text of a word whose only runtime syntax is pathname
    /// expansion ([`DynamicReason::Glob`]) and/or tilde expansion
    /// ([`DynamicReason::TildeExpansion`]).
    ///
    /// Tilde prefixes are retained unexpanded. For glob words, quoted glob
    /// metacharacters and every brace are bracket-escaped (`[*]`, `[{]`) so
    /// the value can be compiled as a glob pattern without reinterpreting
    /// quoted text. `None` for literal words and all other dynamic words.
    pub pattern: Option<String>,
}

/// Syntax feature preventing conservative literal word recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum DynamicReason {
    /// `$name` or `${...}` parameter expansion.
    ParameterExpansion,
    /// `$(...)` or backtick command substitution.
    CommandSubstitution,
    /// `<(...)` or `>(...)` process substitution.
    ProcessSubstitution,
    /// `$((...))` arithmetic expansion.
    ArithmeticExpansion,
    /// `{a,b}` or `{x..y}` brace expansion that could not be expanded
    /// statically, because the word also contains runtime syntax, the sequence
    /// form is unsupported, or expansion would exceed the analyzer's word
    /// budget. Statically expandable brace expansion yields literal words.
    BraceExpansion,
    /// Unquoted glob metacharacters.
    Glob,
    /// Tilde expansion: a leading `~`, or a `~` after `=` or `:` in an
    /// assignment-shaped word, which Bash expands outside POSIX mode.
    TildeExpansion,
    /// Escape whose runtime value is not statically modeled.
    EscapeSequence,
    /// ANSI-C `$'...'` quoting.
    AnsiCString,
    /// Locale-translated `$"..."` quoting.
    LocaleTranslation,
    /// Tree-sitter error-recovery syntax.
    ParseError,
    /// Valid or recovered syntax outside the analyzer's literal model.
    UnsupportedSyntax,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
/// Enclosing syntax that affects when or where a command executes.
pub enum ExecutionContext {
    /// A pipeline element.
    Pipeline,
    /// An `&&` or `||` list element.
    AndOrList,
    /// A sequential command-list element.
    Sequence,
    /// An `if`/`elif`/conditional command body or condition.
    Conditional,
    /// A loop body or condition.
    Loop,
    /// A `case` expression or arm.
    Case,
    /// A function definition body.
    FunctionDefinition,
    /// A parenthesized subshell.
    Subshell,
    /// A command substitution body.
    CommandSubstitution,
    /// A process substitution body.
    ProcessSubstitution,
    /// A backgrounded command.
    Background,
    /// A command under shell negation (`!`).
    Negated,
    /// A pipeline element after the first, whose standard input is the
    /// preceding element's output. Commands nested inside such an element,
    /// including in its substitutions, inherit the piped input.
    PipelineInput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
/// One notable Bash syntax construct.
pub struct ConstructOccurrence {
    /// Construct classification.
    pub kind: ConstructKind,
    /// Byte/line span of the construct.
    pub span: SourceSpan,
    /// Exact source slice covered by [`Self::span`].
    pub raw: String,
}

/// Notable syntax construct retained independently of command argv.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ConstructKind {
    /// Pipeline.
    Pipeline,
    /// `&&` or `||` list.
    AndOrList,
    /// Sequential command list.
    Sequence,
    /// Background execution.
    Background,
    /// Conditional statement.
    Conditional,
    /// Loop statement.
    Loop,
    /// Case statement.
    Case,
    /// Function definition.
    FunctionDefinition,
    /// Parenthesized subshell.
    Subshell,
    /// Command substitution, including backtick substitution inside an
    /// unquoted here-document body.
    CommandSubstitution,
    /// Process substitution.
    ProcessSubstitution,
    /// Variable declaration command.
    Declaration,
    /// Variable unset command.
    Unset,
    /// Shell test expression.
    Test,
    /// Shell negation.
    Negation,
    /// Variable assignment.
    VariableAssignment,
    /// File or descriptor redirection.
    FileRedirection,
    /// Here-document redirection.
    HereDocument,
    /// Here-string redirection.
    HereString,
    /// Parameter expansion.
    ParameterExpansion,
    /// Arithmetic expansion.
    ArithmeticExpansion,
    /// A numeric sequence brace expression such as `{1..3}`, which the grammar
    /// parses as a distinct node. Comma brace expansion is not a separate
    /// node; it is expanded into argv words or reported as
    /// [`DynamicReason::BraceExpansion`].
    BraceExpansion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
/// One redirection attached to a command or statement.
pub struct Redirection {
    /// Broad redirection target kind.
    pub kind: RedirectionKind,
    /// Typed shell operator. This is `None` only when error-recovered syntax
    /// did not contain a recognizable operator.
    pub operator: Option<RedirectionOperator>,
    /// Byte/line span of the complete redirection. For a file redirection the
    /// grammar's span also covers any following words, which are reported as
    /// command arguments.
    pub span: SourceSpan,
    /// Exact source slice covered by [`Self::span`].
    pub raw: String,
    /// Optional source file descriptor text preceding the operator, including
    /// `0` and `{name}` forms the grammar parses as separate words.
    pub descriptor: Option<String>,
    /// Redirection target word, when the syntax has one.
    pub target: Option<ShellWord>,
    /// Source-backed delimiter and body facts for a here-document.
    pub here_document: Option<HereDocument>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
/// Source-backed facts for a here-document.
pub struct HereDocument {
    /// Delimiter word as written and conservatively decoded.
    pub delimiter: ShellWord,
    /// Span of the here-document body, excluding its delimiter line.
    pub body_span: Option<SourceSpan>,
    /// The bytes supplied on stdin after quote handling and optional tab
    /// stripping, but only when no runtime expansion or backslash processing
    /// can change them.
    pub literal_body: Option<String>,
    /// Runtime-expansion reasons that prevented a literal body. An unquoted
    /// delimiter's body reports [`DynamicReason::CommandSubstitution`] for
    /// backticks and [`DynamicReason::EscapeSequence`] for backslash escapes
    /// that Bash removes (`\$`, `` \` ``, `\\`, and backslash-newline).
    pub dynamic_reasons: Vec<DynamicReason>,
}

/// Broad destination class of a Bash redirection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum RedirectionKind {
    /// Filesystem path or file-descriptor redirection.
    File,
    /// Multi-line here-document input.
    HereDocument,
    /// Single-word here-string input.
    HereString,
}

/// Parsed Bash redirection operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum RedirectionOperator {
    /// Input from a file (`<`).
    Input,
    /// Truncating output to a file (`>`).
    Output,
    /// Appending output to a file (`>>`).
    Append,
    /// Combined stdout/stderr output (`&>` or `>&word`).
    OutputAndError,
    /// Appending combined stdout/stderr (`&>>`).
    AppendOutputAndError,
    /// Input file-descriptor duplication (`<&`).
    DuplicateInput,
    /// Output file-descriptor duplication (`>&`).
    DuplicateOutput,
    /// Output that overrides noclobber (`>|`).
    Clobber,
    /// Closing an input descriptor (`<&-`).
    CloseInput,
    /// Closing an output descriptor (`>&-`).
    CloseOutput,
    /// Here-document input (`<<`).
    HereDocument,
    /// Tab-stripping here-document input (`<<-`).
    HereDocumentStripTabs,
    /// Here-string input (`<<<`).
    HereString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
/// Redirections applied to a statement that is not a simple command.
///
/// Bash opens these redirections once for the whole statement, so every
/// command in [`Self::commands`] shares them. For example, the commands in
/// `{ cat; } < file` and `(cd dir && apply_patch) <<EOF` read the redirected
/// input.
pub struct StatementRedirection {
    /// Kind of statement the redirections apply to.
    pub kind: StatementKind,
    /// Span from the start of the redirected statement to the end of its last
    /// redirection.
    pub span: SourceSpan,
    /// Span of the redirected statement itself, excluding its redirections.
    pub body_span: SourceSpan,
    /// Commands enclosed by the statement, as a range of indexes into
    /// [`BashAnalysis::commands`]. The range is empty when the statement body
    /// contains no simple commands or was not traversed because of a limit.
    pub commands: Range<usize>,
    /// Ordered execution contexts enclosing the statement, outermost first.
    /// Function-definition redirections include
    /// [`ExecutionContext::FunctionDefinition`], because they apply whenever
    /// the function is called.
    pub context: Vec<ExecutionContext>,
    /// Redirections in source order.
    pub redirections: Vec<Redirection>,
}

/// Kind of statement carrying a [`StatementRedirection`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum StatementKind {
    /// A `{ ...; }` command group.
    Group,
    /// A `( ... )` subshell.
    Subshell,
    /// An `if` statement.
    Conditional,
    /// A `for`, `while`, or `until` loop.
    Loop,
    /// A `case` statement.
    Case,
    /// A function definition; the redirections apply each time it is called.
    FunctionDefinition,
    /// A `[ ... ]` or `[[ ... ]]` test command.
    Test,
    /// A `declare`, `export`, `local`, `readonly`, or `typeset` command.
    Declaration,
    /// An `unset` command.
    Unset,
    /// A negated statement.
    Negation,
    /// A variable assignment without a command.
    VariableAssignment,
    /// Other or error-recovered syntax, including redirections whose owning
    /// command was not traversed because of a limit.
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// Half-open byte range with corresponding Tree-sitter source positions.
pub struct SourceSpan {
    /// Inclusive UTF-8 byte offset.
    pub start_byte: usize,
    /// Exclusive UTF-8 byte offset.
    pub end_byte: usize,
    /// Inclusive start position.
    pub start: SourcePosition,
    /// Exclusive end position.
    pub end: SourcePosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
/// Zero-based position in Bash source.
pub struct SourcePosition {
    /// Zero-based row.
    pub row: usize,
    /// Zero-based byte column.
    pub column: usize,
}

/// Reason an analysis contains useful but incomplete facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum IncompleteReason {
    /// Tree-sitter reported syntax errors in its recovered tree.
    SyntaxErrors,
    /// AST traversal stopped at the configured node limit.
    NodeLimit {
        /// Configured maximum nodes.
        max_nodes: usize,
    },
    /// A subtree deeper than the configured recursion limit was skipped.
    /// Shallower sibling syntax is still analyzed.
    DepthLimit {
        /// Configured maximum depth.
        max_depth: usize,
    },
    /// A command substitution that the grammar leaves unparsed, such as a
    /// backtick substitution in an unquoted here-document body, could not be
    /// re-parsed exactly (for example because it contains backslash escapes or
    /// is unterminated), so the commands it runs are unknown.
    UnparsedCommandSubstitution,
}

/// Reason no Bash analysis facts could be produced.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum UnavailableReason {
    /// Source exceeded the pre-parse byte limit.
    InputTooLarge {
        /// Actual UTF-8 source length.
        actual_bytes: usize,
        /// Configured maximum source length.
        max_bytes: usize,
    },
    /// Tree-sitter parsing exceeded the wall-clock budget.
    ParseTimeLimit {
        /// Configured budget truncated to whole milliseconds.
        max_millis: u64,
    },
    /// The bundled Bash grammar could not be installed in the parser.
    ParserInitialization(String),
    /// Tree-sitter returned no syntax tree without reporting a timeout.
    ParserReturnedNoTree,
}

enum ParseFailure {
    Initialization(String),
    TimeLimit,
    NoTree,
}

/// Parses `source`, or only the `included` byte range of it, within the
/// remaining share of `budget` measured from `started`. Node offsets and
/// positions always refer to the full `source`.
fn parse_bash(
    source: &str,
    included: Option<tree_sitter::Range>,
    budget: Duration,
    started: Instant,
) -> Result<Tree, ParseFailure> {
    let mut parser = Parser::new();
    let language = tree_sitter_bash::LANGUAGE.into();
    parser
        .set_language(&language)
        .map_err(|error| ParseFailure::Initialization(error.to_string()))?;
    if let Some(range) = included {
        parser
            .set_included_ranges(&[range])
            .map_err(|_| ParseFailure::Initialization("invalid embedded parse range".to_owned()))?;
    }

    let mut timed_out = false;
    let tree = {
        let mut read =
            |offset: usize, _position: Point| source.as_bytes().get(offset..).unwrap_or_default();
        let mut cancel = |_state: &tree_sitter::ParseState| {
            if started.elapsed() >= budget {
                timed_out = true;
                true
            } else {
                false
            }
        };
        let options = ParseOptions::new().progress_callback(&mut cancel);
        parser.parse_with_options(&mut read, None, Some(options))
    };
    if timed_out {
        return Err(ParseFailure::TimeLimit);
    }
    tree.ok_or(ParseFailure::NoTree)
}

/// A statement whose redirections are recorded once its owner node has been
/// traversed, so the enclosed command range is known.
struct PendingStatement {
    owner: usize,
    kind: StatementKind,
    span: SourceSpan,
    body_span: SourceSpan,
    redirections: Vec<Redirection>,
    started: Option<(usize, Vec<ExecutionContext>)>,
}

struct Walker<'source, 'tree> {
    source: &'source str,
    limits: BashAnalyzerLimits,
    started: Instant,
    visited_nodes: usize,
    analysis: BashAnalysis,
    incomplete: Option<IncompleteReason>,
    halted: bool,
    /// Redirection nodes that Bash applies to one simple command but the
    /// grammar hung on an enclosing statement, keyed by the command node id.
    attached_redirects: HashMap<usize, Vec<Node<'tree>>>,
    pending_statements: Vec<PendingStatement>,
}

impl<'source, 'tree> Walker<'source, 'tree> {
    fn new(source: &'source str, limits: BashAnalyzerLimits, started: Instant) -> Self {
        Self {
            source,
            limits,
            started,
            visited_nodes: 0,
            analysis: BashAnalysis::default(),
            incomplete: None,
            halted: false,
            attached_redirects: HashMap::new(),
            pending_statements: Vec::new(),
        }
    }

    fn mark_incomplete(&mut self, reason: IncompleteReason) {
        if self.incomplete.is_none() {
            self.incomplete = Some(reason);
        }
    }

    /// Returns the analysis plus the first incomplete reason. Redirections
    /// whose owner was never traversed are retained as statement redirections
    /// so their file effects are not silently dropped.
    fn finish(mut self) -> (BashAnalysis, Option<IncompleteReason>) {
        let end = self.analysis.commands.len();
        for pending in std::mem::take(&mut self.pending_statements) {
            let (start, context) = pending.started.unwrap_or((end, Vec::new()));
            self.analysis
                .statement_redirections
                .push(StatementRedirection {
                    kind: pending.kind,
                    span: pending.span,
                    body_span: pending.body_span,
                    commands: start..end,
                    context,
                    redirections: pending.redirections,
                });
        }
        let mut leftovers = std::mem::take(&mut self.attached_redirects)
            .into_values()
            .collect::<Vec<_>>();
        leftovers.sort_by_key(|nodes| nodes.iter().map(Node::start_byte).min());
        for nodes in leftovers {
            let (redirects, _, _) = expand_redirect_nodes(nodes);
            let (Some(first), Some(last)) = (redirects.first(), redirects.last()) else {
                continue;
            };
            let span = span_between(*first, *last);
            self.analysis
                .statement_redirections
                .push(StatementRedirection {
                    kind: StatementKind::Other,
                    span,
                    body_span: span,
                    commands: end..end,
                    context: Vec::new(),
                    redirections: analyze_statement_redirections(&redirects, self.source),
                });
        }
        (self.analysis, self.incomplete)
    }

    /// Applies traversal limits and records constructs. Returns `false` when
    /// the node must be skipped.
    fn enter(&mut self, node: Node<'tree>, depth: usize) -> bool {
        if self.halted {
            return false;
        }
        if depth > self.limits.max_depth {
            // Skip only this subtree; shallower siblings remain visible.
            self.mark_incomplete(IncompleteReason::DepthLimit {
                max_depth: self.limits.max_depth,
            });
            return false;
        }
        if self.visited_nodes >= self.limits.max_nodes {
            self.mark_incomplete(IncompleteReason::NodeLimit {
                max_nodes: self.limits.max_nodes,
            });
            self.halted = true;
            return false;
        }
        self.visited_nodes += 1;

        for kind in construct_kinds(node) {
            self.analysis.constructs.push(ConstructOccurrence {
                kind,
                span: source_span(node),
                raw: node_text(node, self.source).to_owned(),
            });
        }
        true
    }

    fn visit(&mut self, node: Node<'tree>, contexts: &mut Vec<ExecutionContext>, depth: usize) {
        if !self.enter(node, depth) {
            return;
        }
        self.begin_statements(node, contexts);

        match node.kind() {
            "command" => {
                let command = self.analyze_command(node, contexts);
                self.analysis.commands.push(command);
            }
            "redirected_statement" => self.prepare_redirected_statement(node, contexts),
            "function_definition" => self.prepare_function_redirects(node),
            "command_substitution" => self.analyze_bare_substitution(node, contexts),
            "heredoc_body" => self.analyze_heredoc_substitutions(node, contexts, depth),
            _ => {}
        }

        let base = contexts.len();
        if node.kind() == "list" {
            self.visit_list_chain(node, contexts, depth);
        } else {
            contexts.extend(execution_contexts(node));
            self.visit_children(node, contexts, depth);
        }
        contexts.truncate(base);
        self.finish_statements(node);
    }

    fn visit_children(
        &mut self,
        node: Node<'tree>,
        contexts: &mut Vec<ExecutionContext>,
        depth: usize,
    ) {
        let kind = node.kind();
        let mut cursor = node.walk();
        if !cursor.goto_first_child() {
            return;
        }
        let mut seen_pipeline_element = false;
        loop {
            let child = cursor.node();
            if child.is_named() {
                let base = contexts.len();
                match kind {
                    "pipeline" if child.kind() != "comment" => {
                        if seen_pipeline_element {
                            contexts.push(ExecutionContext::PipelineInput);
                        }
                        seen_pipeline_element = true;
                    }
                    "heredoc_redirect" if cursor.field_name() == Some("right") => {
                        contexts.extend(heredoc_right_contexts(node));
                    }
                    _ => {}
                }
                self.visit(child, contexts, depth + 1);
                contexts.truncate(base);
                if self.halted {
                    break;
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    /// Walks a left-recursive `&&`/`||` list chain iteratively so that the
    /// chain's length does not count toward the traversal depth.
    fn visit_list_chain(
        &mut self,
        node: Node<'tree>,
        contexts: &mut Vec<ExecutionContext>,
        depth: usize,
    ) {
        let base = contexts.len();
        let mut chain = vec![node];
        contexts.extend(execution_contexts(node));
        let mut lengths = vec![contexts.len()];
        while let Some(left) = chain
            .last()
            .and_then(|list| list.named_child(0))
            .filter(|left| left.kind() == "list")
        {
            if !self.enter(left, depth) {
                break;
            }
            chain.push(left);
            contexts.extend(execution_contexts(left));
            lengths.push(contexts.len());
        }

        'levels: for level in (0..chain.len()).rev() {
            contexts.truncate(lengths[level]);
            let nested = chain.get(level + 1).map(Node::id);
            let mut cursor = chain[level].walk();
            for child in chain[level].named_children(&mut cursor) {
                if Some(child.id()) == nested {
                    continue;
                }
                self.visit(child, contexts, depth + 1);
                if self.halted {
                    break 'levels;
                }
            }
        }
        contexts.truncate(base);
    }

    fn begin_statements(&mut self, node: Node<'tree>, contexts: &[ExecutionContext]) {
        if self.pending_statements.is_empty() {
            return;
        }
        let start = self.analysis.commands.len();
        for pending in &mut self.pending_statements {
            if pending.owner == node.id() && pending.started.is_none() {
                pending.started = Some((start, contexts.to_vec()));
            }
        }
    }

    fn finish_statements(&mut self, node: Node<'tree>) {
        if self.pending_statements.is_empty() {
            return;
        }
        let end = self.analysis.commands.len();
        let mut index = 0;
        while index < self.pending_statements.len() {
            let pending = &self.pending_statements[index];
            if pending.owner != node.id() || pending.started.is_none() {
                index += 1;
                continue;
            }
            let pending = self.pending_statements.remove(index);
            let (start, context) = pending.started.unwrap_or((end, Vec::new()));
            self.analysis
                .statement_redirections
                .push(StatementRedirection {
                    kind: pending.kind,
                    span: pending.span,
                    body_span: pending.body_span,
                    commands: start..end,
                    context,
                    redirections: pending.redirections,
                });
        }
    }

    fn take_attached(&mut self, owner: usize) -> Vec<Node<'tree>> {
        self.attached_redirects.remove(&owner).unwrap_or_default()
    }

    fn analyze_command(
        &mut self,
        node: Node<'tree>,
        contexts: &[ExecutionContext],
    ) -> CommandOccurrence {
        let name = node
            .child_by_field_name("name")
            .and_then(first_named_child_or_self)
            .filter(|name| name.start_byte() < name.end_byte());
        let mut words = Vec::new();
        let mut redirects = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "command_name" | "variable_assignment" | "comment" => {}
                kind if is_redirect_kind(kind) => redirects.push(child),
                kind if is_argument_kind(kind) => words.push(child),
                _ => {}
            }
        }
        redirects.extend(self.take_attached(node.id()));
        build_command(
            self.source,
            source_span(node),
            node.has_error(),
            name,
            words,
            redirects,
            contexts,
        )
    }

    fn prepare_redirected_statement(&mut self, node: Node<'tree>, contexts: &[ExecutionContext]) {
        let body = node.child_by_field_name("body");
        let mut cursor = node.walk();
        let redirects = node
            .named_children(&mut cursor)
            .filter(|child| is_redirect_kind(child.kind()))
            .filter(|child| Some(child.id()) != body.map(|body| body.id()))
            .collect::<Vec<_>>();
        if redirects.is_empty() {
            return;
        }
        let Some(body) = body else {
            // A redirection written without a command (`> file`, `< file`).
            // Bash still opens the target, and zsh runs NULLCMD/READNULLCMD.
            let command = build_command(
                self.source,
                source_span(node),
                node.has_error(),
                None,
                Vec::new(),
                redirects,
                contexts,
            );
            self.analysis.commands.push(command);
            return;
        };
        let owner = redirect_owner(body);
        if owner.kind() == "command" {
            self.attached_redirects
                .entry(owner.id())
                .or_default()
                .extend(redirects);
            return;
        }
        self.register_statement(owner, statement_kind(owner.kind()), redirects);
    }

    fn prepare_function_redirects(&mut self, node: Node<'tree>) {
        let mut cursor = node.walk();
        let redirects = node
            .children_by_field_name("redirect", &mut cursor)
            .collect::<Vec<_>>();
        if redirects.is_empty() {
            return;
        }
        let owner = node.child_by_field_name("body").unwrap_or(node);
        self.register_statement(owner, StatementKind::FunctionDefinition, redirects);
    }

    fn register_statement(
        &mut self,
        owner: Node<'tree>,
        kind: StatementKind,
        redirects: Vec<Node<'tree>>,
    ) {
        let (redirects, _, _) = expand_redirect_nodes(redirects);
        let Some(last) = redirects.last() else {
            return;
        };
        let end = if last.end_byte() >= owner.end_byte() {
            *last
        } else {
            owner
        };
        self.pending_statements.push(PendingStatement {
            owner: owner.id(),
            kind,
            span: span_between(owner, end),
            body_span: source_span(owner),
            redirections: analyze_statement_redirections(&redirects, self.source),
            started: None,
        });
    }

    /// Bash's `$(< file)` content-read idiom has no command node: the
    /// redirection is a direct child of the substitution.
    fn analyze_bare_substitution(&mut self, node: Node<'tree>, contexts: &[ExecutionContext]) {
        let mut cursor = node.walk();
        let redirects = node
            .named_children(&mut cursor)
            .filter(|child| is_redirect_kind(child.kind()))
            .collect::<Vec<_>>();
        let (Some(first), Some(last)) = (redirects.first(), redirects.last()) else {
            return;
        };
        let span = span_between(*first, *last);
        let mut context = contexts.to_vec();
        context.push(ExecutionContext::CommandSubstitution);
        let command = build_command(
            self.source,
            span,
            node.has_error(),
            None,
            Vec::new(),
            redirects,
            &context,
        );
        self.analysis.commands.push(command);
    }

    /// Re-parses backtick substitutions in an unquoted here-document body,
    /// which the grammar leaves as plain text although Bash executes them.
    fn analyze_heredoc_substitutions(
        &mut self,
        body: Node<'tree>,
        contexts: &[ExecutionContext],
        depth: usize,
    ) {
        let Some(redirect) = body
            .parent()
            .filter(|parent| parent.kind() == "heredoc_redirect")
        else {
            return;
        };
        let Some(start) = heredoc_start(redirect) else {
            return;
        };
        if heredoc_delimiter_quoted(node_text(start, self.source)) {
            return;
        }
        let mut points = PointCursor::new(body);
        for region in backtick_regions(body, self.source) {
            let Some(close) = region.close else {
                self.mark_incomplete(IncompleteReason::UnparsedCommandSubstitution);
                continue;
            };
            if self.halted {
                return;
            }
            let content = region.open + 1..close;
            let open_point = points.advance(self.source, region.open);
            let content_start = points.advance(self.source, content.start);
            let content_end = points.advance(self.source, content.end);
            let close_point = points.advance(self.source, close + 1);
            self.analysis.constructs.push(ConstructOccurrence {
                kind: ConstructKind::CommandSubstitution,
                span: SourceSpan {
                    start_byte: region.open,
                    end_byte: close + 1,
                    start: source_position(open_point),
                    end: source_position(close_point),
                },
                raw: self.source[region.open..close + 1].to_owned(),
            });
            let text = &self.source[content.clone()];
            if text.contains('\\') {
                // Bash removes backslashes inside backticks before parsing, so
                // the verbatim text is not what runs.
                self.mark_incomplete(IncompleteReason::UnparsedCommandSubstitution);
                continue;
            }
            if text.trim().is_empty() {
                continue;
            }
            let mut context = contexts.to_vec();
            context.push(ExecutionContext::CommandSubstitution);
            let included = tree_sitter::Range {
                start_byte: content.start,
                end_byte: content.end,
                start_point: content_start,
                end_point: content_end,
            };
            self.walk_embedded(included, &mut context, depth + 1);
        }
    }

    /// Parses and traverses one byte range of the source as an embedded Bash
    /// program, merging its facts into this analysis.
    fn walk_embedded(
        &mut self,
        included: tree_sitter::Range,
        contexts: &mut Vec<ExecutionContext>,
        depth: usize,
    ) {
        if self.halted {
            return;
        }
        if depth > self.limits.max_depth {
            self.mark_incomplete(IncompleteReason::DepthLimit {
                max_depth: self.limits.max_depth,
            });
            return;
        }
        // Each embedded parse costs at least one node of the budget, and the
        // shared wall-clock budget bounds the parses themselves.
        if self.visited_nodes >= self.limits.max_nodes {
            self.mark_incomplete(IncompleteReason::NodeLimit {
                max_nodes: self.limits.max_nodes,
            });
            self.halted = true;
            return;
        }
        if self.started.elapsed() >= self.limits.max_parse_time {
            self.mark_incomplete(IncompleteReason::UnparsedCommandSubstitution);
            return;
        }
        let Ok(tree) = parse_bash(
            self.source,
            Some(included),
            self.limits.max_parse_time,
            self.started,
        ) else {
            self.mark_incomplete(IncompleteReason::UnparsedCommandSubstitution);
            return;
        };
        let root = tree.root_node();
        let mut embedded = Walker::new(self.source, self.limits, self.started);
        embedded.visited_nodes = self.visited_nodes;
        embedded.visit(root, contexts, depth);
        if root.has_error() {
            embedded.mark_incomplete(IncompleteReason::SyntaxErrors);
        }
        let visited_nodes = embedded.visited_nodes;
        let halted = embedded.halted;
        let (analysis, incomplete) = embedded.finish();

        let offset = self.analysis.commands.len();
        self.analysis.commands.extend(analysis.commands);
        self.analysis.constructs.extend(analysis.constructs);
        self.analysis.statement_redirections.extend(
            analysis
                .statement_redirections
                .into_iter()
                .map(|mut statement| {
                    statement.commands =
                        statement.commands.start + offset..statement.commands.end + offset;
                    statement
                }),
        );
        self.visited_nodes = visited_nodes;
        self.halted |= halted;
        if let Some(reason) = incomplete {
            self.mark_incomplete(reason);
        }
    }
}

/// Builds one command occurrence from its name, argument words, and
/// redirection nodes, following Bash rather than the grammar's tree shape.
fn build_command<'tree>(
    source: &str,
    span: SourceSpan,
    has_error: bool,
    name: Option<Node<'tree>>,
    mut words: Vec<Node<'tree>>,
    redirect_nodes: Vec<Node<'tree>>,
    contexts: &[ExecutionContext],
) -> CommandOccurrence {
    let (redirects, heredoc_arguments, extra_contexts) = expand_redirect_nodes(redirect_nodes);
    words.extend(heredoc_arguments);

    // Only the first destination group is a file redirection's target; Bash
    // treats any later words as command arguments.
    let mut targets = Vec::with_capacity(redirects.len());
    for redirect in &redirects {
        let (target, trailing) = redirect_target_nodes(*redirect, source);
        words.extend(trailing);
        targets.push(target);
    }

    // `0<file` and `{fd}>file` descriptors are lexed as separate words.
    let mut descriptors = redirects
        .iter()
        .map(|redirect| {
            redirect
                .child_by_field_name("descriptor")
                .map(|descriptor| node_text(descriptor, source).to_owned())
        })
        .collect::<Vec<_>>();
    words.retain(|word| {
        if !is_descriptor_word(*word, source) {
            return true;
        }
        // Redirections are sorted by start byte.
        let index = redirects.partition_point(|redirect| redirect.start_byte() < word.end_byte());
        match redirects
            .get(index)
            .filter(|redirect| redirect.start_byte() == word.end_byte())
        {
            Some(_) if descriptors[index].is_none() => {
                descriptors[index] = Some(node_text(*word, source).to_owned());
                false
            }
            _ => true,
        }
    });

    let mut all_words = name.into_iter().chain(words).collect::<Vec<_>>();
    all_words.sort_by_key(Node::start_byte);
    all_words.dedup_by_key(|word| word.id());
    let mut shell_words = Vec::with_capacity(all_words.len());
    for group in group_continued_words(&all_words, source) {
        shell_words.extend(analyze_word_group(&group, source));
    }
    let mut shell_words = shell_words.into_iter();
    let name = shell_words.next();
    let arguments = shell_words.collect::<Vec<_>>();

    let redirections = redirects
        .iter()
        .zip(targets)
        .zip(descriptors)
        .map(|((redirect, target), descriptor)| {
            analyze_redirection(*redirect, source, descriptor, &target)
        })
        .collect();

    let mut context = contexts.to_vec();
    context.extend(extra_contexts);
    let argv = command_argv(has_error, name.as_ref(), &arguments);
    CommandOccurrence {
        span,
        raw: source
            .get(span.start_byte..span.end_byte)
            .unwrap_or("")
            .to_owned(),
        name,
        arguments,
        argv,
        context,
        redirections,
    }
}

/// Flattens redirections nested in here-document redirects, returning the
/// redirections in source order, the argument words the grammar attached to
/// here-document starts, and contexts implied by a here-document's trailing
/// `&&`/`||` operator.
fn expand_redirect_nodes(
    nodes: Vec<Node<'_>>,
) -> (Vec<Node<'_>>, Vec<Node<'_>>, Vec<ExecutionContext>) {
    let mut redirects = Vec::with_capacity(nodes.len());
    let mut arguments = Vec::new();
    let mut contexts = Vec::new();
    let mut queue = nodes;
    while let Some(redirect) = queue.pop() {
        if redirect.kind() == "heredoc_redirect" {
            let mut cursor = redirect.walk();
            queue.extend(redirect.children_by_field_name("redirect", &mut cursor));
            let mut cursor = redirect.walk();
            arguments.extend(redirect.children_by_field_name("argument", &mut cursor));
            if let Some(operator) = redirect.child_by_field_name("operator") {
                contexts.extend(operator_owner_context(operator.kind()));
            }
        }
        redirects.push(redirect);
    }
    redirects.sort_by_key(Node::start_byte);
    redirects.dedup_by_key(|redirect| redirect.id());
    (redirects, arguments, contexts)
}

fn analyze_statement_redirections(redirects: &[Node<'_>], source: &str) -> Vec<Redirection> {
    redirects
        .iter()
        .map(|redirect| {
            let (target, _) = redirect_target_nodes(*redirect, source);
            let descriptor = redirect
                .child_by_field_name("descriptor")
                .map(|descriptor| node_text(descriptor, source).to_owned());
            analyze_redirection(*redirect, source, descriptor, &target)
        })
        .collect()
}

/// Returns the node group forming a redirection's target plus any later
/// destination words.
fn redirect_target_nodes<'tree>(
    redirect: Node<'tree>,
    source: &str,
) -> (Vec<Node<'tree>>, Vec<Node<'tree>>) {
    match redirect.kind() {
        "file_redirect" => {
            let mut cursor = redirect.walk();
            let destinations = redirect
                .children_by_field_name("destination", &mut cursor)
                .collect::<Vec<_>>();
            let mut groups = group_continued_words(&destinations, source).into_iter();
            let target = groups.next().unwrap_or_default();
            (target, groups.flatten().collect())
        }
        "herestring_redirect" => {
            let mut cursor = redirect.walk();
            let target = redirect
                .named_children(&mut cursor)
                .find(|child| child.kind() != "file_descriptor")
                .into_iter()
                .collect();
            (target, Vec::new())
        }
        _ => (Vec::new(), Vec::new()),
    }
}

/// Follows the grammar's attachment of trailing redirections to a list,
/// pipeline, or negation down to the statement Bash applies them to: the last
/// element.
fn redirect_owner(body: Node<'_>) -> Node<'_> {
    let mut current = body;
    loop {
        let next = match current.kind() {
            "list" | "pipeline" | "negated_command" => {
                let mut cursor = current.walk();
                current
                    .named_children(&mut cursor)
                    .filter(|child| child.kind() != "comment")
                    .last()
            }
            "redirected_statement" => current.child_by_field_name("body"),
            _ => None,
        };
        match next {
            Some(next) => current = next,
            None => return current,
        }
    }
}

fn statement_kind(kind: &str) -> StatementKind {
    match kind {
        "compound_statement" => StatementKind::Group,
        "subshell" => StatementKind::Subshell,
        "if_statement" => StatementKind::Conditional,
        "for_statement" | "c_style_for_statement" | "while_statement" => StatementKind::Loop,
        "case_statement" => StatementKind::Case,
        "function_definition" => StatementKind::FunctionDefinition,
        "test_command" => StatementKind::Test,
        "declaration_command" => StatementKind::Declaration,
        "unset_command" => StatementKind::Unset,
        "negated_command" => StatementKind::Negation,
        "variable_assignment" | "variable_assignments" => StatementKind::VariableAssignment,
        _ => StatementKind::Other,
    }
}

fn is_redirect_kind(kind: &str) -> bool {
    matches!(
        kind,
        "file_redirect" | "heredoc_redirect" | "herestring_redirect"
    )
}

/// Whether a word the grammar lexed as an argument is really the descriptor of
/// an immediately following redirection (`0<file`, `{fd}>file`).
fn is_descriptor_word(word: Node<'_>, source: &str) -> bool {
    let text = node_text(word, source);
    match word.kind() {
        "number" => !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()),
        "word" | "concatenation" => text
            .strip_prefix('{')
            .and_then(|text| text.strip_suffix('}'))
            .is_some_and(is_identifier),
        _ => false,
    }
}

fn is_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

/// Groups adjacent word nodes that Bash reads as one word: pieces separated
/// only by backslash-newline line continuations (or by nothing at all).
fn group_continued_words<'tree>(nodes: &[Node<'tree>], source: &str) -> Vec<Vec<Node<'tree>>> {
    let mut groups: Vec<Vec<Node<'tree>>> = Vec::new();
    for node in nodes {
        let joined = groups
            .last()
            .and_then(|group| group.last())
            .is_some_and(|previous| {
                previous.end_byte() <= node.start_byte()
                    && source
                        .get(previous.end_byte()..node.start_byte())
                        .is_some_and(is_line_continuation_gap)
            });
        if joined {
            if let Some(group) = groups.last_mut() {
                group.push(*node);
            }
        } else {
            groups.push(vec![*node]);
        }
    }
    groups
}

fn is_line_continuation_gap(gap: &str) -> bool {
    gap.len() % 2 == 0 && gap.as_bytes().chunks_exact(2).all(|pair| pair == b"\\\n")
}

fn command_argv(has_error: bool, name: Option<&ShellWord>, arguments: &[ShellWord]) -> ArgvStatus {
    let mut reasons = Vec::new();
    if has_error {
        reasons.push(DynamicReason::ParseError);
    }

    let mut argv = Vec::with_capacity(arguments.len() + 1);
    match name {
        Some(ShellWord {
            literal: Some(name),
            ..
        }) => argv.push(name.clone()),
        Some(name) => reasons.extend(name.dynamic_reasons.iter().copied()),
        None if arguments.is_empty() => {}
        None => reasons.push(DynamicReason::UnsupportedSyntax),
    }
    for argument in arguments {
        if let Some(literal) = &argument.literal {
            argv.push(literal.clone());
        } else {
            reasons.extend(argument.dynamic_reasons.iter().copied());
        }
    }
    deduplicate(&mut reasons);
    if reasons.is_empty() {
        ArgvStatus::Literal(argv)
    } else {
        ArgvStatus::Dynamic { reasons }
    }
}

/// One character of a word after quote removal, or an opaque runtime piece.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WordItem {
    Char { value: char, quoted: bool },
    Opaque,
}

/// Maximum words one source word may produce through static brace expansion.
const MAX_BRACE_WORDS: usize = 64;
/// Longest word, in characters, that static brace expansion examines.
const MAX_BRACE_WORD_CHARS: usize = 1024;
/// Maximum nesting of brace expansions within one word.
const MAX_BRACE_DEPTH: usize = 16;

/// Analyzes a group of word nodes that Bash reads as one source word,
/// returning one shell word per brace-expansion result.
fn analyze_word_group(nodes: &[Node<'_>], source: &str) -> Vec<ShellWord> {
    let (Some(first), Some(last)) = (nodes.first(), nodes.last()) else {
        return Vec::new();
    };
    let span = span_between(*first, *last);
    let raw = source
        .get(span.start_byte..span.end_byte)
        .unwrap_or("")
        .to_owned();
    let mut items = Vec::new();
    let mut reasons = Vec::new();
    for node in nodes {
        collect_word_items(*node, source, &mut items, &mut reasons);
    }

    let opaque = items.contains(&WordItem::Opaque);
    let expansions = match brace_expand(&items) {
        Ok(expansions) => expansions,
        Err(BraceOverflow) => {
            reasons.push(DynamicReason::BraceExpansion);
            return vec![dynamic_word(span, raw, reasons)];
        }
    };
    if opaque {
        if expansions.is_some() {
            reasons.push(DynamicReason::BraceExpansion);
        }
        return vec![dynamic_word(span, raw, reasons)];
    }
    match expansions {
        None => vec![word_from_items(span, raw, &items)],
        Some(expansions) => expansions
            .iter()
            .map(|items| word_from_items(span, raw.clone(), items))
            .collect(),
    }
}

fn dynamic_word(span: SourceSpan, raw: String, mut reasons: Vec<DynamicReason>) -> ShellWord {
    if reasons.is_empty() {
        reasons.push(DynamicReason::UnsupportedSyntax);
    }
    deduplicate(&mut reasons);
    ShellWord {
        span,
        raw,
        literal: None,
        dynamic_reasons: reasons,
        pattern: None,
    }
}

/// Builds a word from fully static items, classifying tilde and glob syntax.
fn word_from_items(span: SourceSpan, raw: String, items: &[WordItem]) -> ShellWord {
    let mut reasons = Vec::new();
    if has_tilde_expansion(items) {
        reasons.push(DynamicReason::TildeExpansion);
    }
    let glob = items.iter().any(|item| {
        matches!(
            item,
            WordItem::Char {
                value: '*' | '?' | '[',
                quoted: false
            }
        )
    });
    if glob {
        reasons.push(DynamicReason::Glob);
    }
    deduplicate(&mut reasons);
    let text = items_text(items);
    if reasons.is_empty() {
        return ShellWord {
            span,
            raw,
            literal: Some(text),
            dynamic_reasons: reasons,
            pattern: None,
        };
    }
    ShellWord {
        span,
        raw,
        literal: None,
        pattern: word_pattern(items, glob),
        dynamic_reasons: reasons,
    }
}

fn items_text(items: &[WordItem]) -> String {
    items
        .iter()
        .filter_map(|item| match item {
            WordItem::Char { value, .. } => Some(*value),
            WordItem::Opaque => None,
        })
        .collect()
}

/// Bash expands an unquoted leading `~`, and, outside POSIX mode, an unquoted
/// `~` directly after the first `=` or a later `:` of an assignment-shaped
/// word, even when the word is an ordinary argument.
fn has_tilde_expansion(items: &[WordItem]) -> bool {
    let unquoted = |index: usize, expected: char| matches!(items.get(index), Some(WordItem::Char { value, quoted: false }) if *value == expected);
    if unquoted(0, '~') {
        return true;
    }
    let Some(equals) = items.iter().position(|item| {
        matches!(
            item,
            WordItem::Char {
                value: '=',
                quoted: false
            }
        )
    }) else {
        return false;
    };
    let name = items[..equals]
        .iter()
        .map(|item| match item {
            WordItem::Char {
                value,
                quoted: false,
            } => Some(*value),
            _ => None,
        })
        .collect::<Option<String>>();
    if !name.as_deref().is_some_and(is_identifier) {
        return false;
    }
    (equals + 1..items.len())
        .filter(|index| *index == equals + 1 || unquoted(index - 1, ':'))
        .any(|index| unquoted(index, '~'))
}

/// Pattern text for a tilde or glob word; `None` if a quoted backslash makes
/// a faithful glob pattern awkward to express.
fn word_pattern(items: &[WordItem], glob: bool) -> Option<String> {
    let mut pattern = String::with_capacity(items.len());
    for item in items {
        let WordItem::Char { value, quoted } = *item else {
            return None;
        };
        if !glob {
            pattern.push(value);
            continue;
        }
        match value {
            '{' | '}' => {
                pattern.push('[');
                pattern.push(value);
                pattern.push(']');
            }
            '*' | '?' | '[' | ']' if quoted => {
                pattern.push('[');
                pattern.push(value);
                pattern.push(']');
            }
            '\\' if quoted => return None,
            _ => pattern.push(value),
        }
    }
    Some(pattern)
}

fn collect_word_items(
    node: Node<'_>,
    source: &str,
    items: &mut Vec<WordItem>,
    reasons: &mut Vec<DynamicReason>,
) {
    let text = node_text(node, source);
    if node.has_error() {
        reasons.push(DynamicReason::ParseError);
        reasons.extend(dynamic_reasons(node, source));
        items.push(WordItem::Opaque);
        return;
    }
    match node.kind() {
        "command_name" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_word_items(child, source, items, reasons);
            }
        }
        "word" | "number" => {
            if node.named_child_count() != 0 {
                reasons.extend(dynamic_reasons(node, source));
                items.push(WordItem::Opaque);
            } else if text.contains('\\') {
                reasons.push(DynamicReason::EscapeSequence);
                items.push(WordItem::Opaque);
            } else {
                items.extend(text.chars().map(|value| WordItem::Char {
                    value,
                    quoted: false,
                }));
            }
        }
        "brace_expression" => items.extend(text.chars().map(|value| WordItem::Char {
            value,
            quoted: false,
        })),
        "raw_string" => match text
            .strip_prefix('\'')
            .and_then(|value| value.strip_suffix('\''))
        {
            Some(value) => items.extend(value.chars().map(|value| WordItem::Char {
                value,
                quoted: true,
            })),
            None => {
                reasons.push(DynamicReason::UnsupportedSyntax);
                items.push(WordItem::Opaque);
            }
        },
        "string" => {
            let mut cursor = node.walk();
            let plain = node
                .named_children(&mut cursor)
                .all(|child| child.kind() == "string_content");
            let value = text
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'));
            match value {
                Some(value) if plain && !value.contains('\\') => {
                    items.extend(value.chars().map(|value| WordItem::Char {
                        value,
                        quoted: true,
                    }));
                }
                _ => {
                    reasons.extend(dynamic_reasons(node, source));
                    items.push(WordItem::Opaque);
                }
            }
        }
        "concatenation" => {
            let mut cursor = node.walk();
            let mut covered = node.start_byte();
            for child in node.children(&mut cursor) {
                if !child.is_named() {
                    if child.start_byte() < child.end_byte() {
                        // Text carried by an anonymous token is outside the
                        // literal model.
                        reasons.push(DynamicReason::UnsupportedSyntax);
                        items.push(WordItem::Opaque);
                    }
                    covered = child.end_byte();
                    continue;
                }
                if child.start_byte() > covered {
                    reasons.push(DynamicReason::UnsupportedSyntax);
                    items.push(WordItem::Opaque);
                }
                collect_word_items(child, source, items, reasons);
                covered = child.end_byte();
            }
        }
        _ => {
            reasons.extend(dynamic_reasons(node, source));
            items.push(WordItem::Opaque);
        }
    }
}

struct BraceOverflow;

/// Statically expands Bash brace expansion. Returns `Ok(None)` when the word
/// contains no expandable brace, and an error when expansion would exceed
/// [`MAX_BRACE_WORDS`] or uses an unsupported sequence form.
fn brace_expand(items: &[WordItem]) -> Result<Option<Vec<Vec<WordItem>>>, BraceOverflow> {
    let has_brace = items.iter().any(|item| {
        matches!(
            item,
            WordItem::Char {
                value: '{',
                quoted: false
            }
        )
    });
    if !has_brace {
        return Ok(None);
    }
    if items.len() > MAX_BRACE_WORD_CHARS {
        return Err(BraceOverflow);
    }
    if find_brace(items)?.is_none() {
        return Ok(None);
    }
    let mut expanded = Vec::new();
    expand_braces(items.to_vec(), &mut expanded, 0)?;
    Ok(Some(expanded))
}

fn expand_braces(
    items: Vec<WordItem>,
    expanded: &mut Vec<Vec<WordItem>>,
    depth: usize,
) -> Result<(), BraceOverflow> {
    match find_brace(&items)? {
        None => {
            if expanded.len() >= MAX_BRACE_WORDS {
                return Err(BraceOverflow);
            }
            expanded.push(items);
            Ok(())
        }
        Some(_) if depth >= MAX_BRACE_DEPTH => Err(BraceOverflow),
        Some((open, close, alternatives)) => {
            for alternative in alternatives {
                let mut next = Vec::with_capacity(items.len() + alternative.len());
                next.extend_from_slice(&items[..open]);
                next.extend(alternative);
                next.extend_from_slice(&items[close + 1..]);
                if next.len() > MAX_BRACE_WORD_CHARS {
                    return Err(BraceOverflow);
                }
                expand_braces(next, expanded, depth + 1)?;
            }
            Ok(())
        }
    }
}

type BraceMatch = (usize, usize, Vec<Vec<WordItem>>);

/// Finds the first unquoted `{...}` that Bash expands: one containing a
/// top-level unquoted comma or a sequence expression.
fn find_brace(items: &[WordItem]) -> Result<Option<BraceMatch>, BraceOverflow> {
    let unquoted = |item: &WordItem, expected: char| matches!(item, WordItem::Char { value, quoted: false } if *value == expected);
    for open in 0..items.len() {
        if !unquoted(&items[open], '{') {
            continue;
        }
        let mut depth = 0usize;
        let mut close = None;
        for (index, item) in items.iter().enumerate().skip(open) {
            if unquoted(item, '{') {
                depth += 1;
            } else if unquoted(item, '}') {
                depth -= 1;
                if depth == 0 {
                    close = Some(index);
                    break;
                }
            }
        }
        let Some(close) = close else {
            continue;
        };
        let content = &items[open + 1..close];
        let mut alternatives = Vec::new();
        let mut current = Vec::new();
        let mut nested = 0usize;
        let mut comma = false;
        for item in content {
            if unquoted(item, '{') {
                nested += 1;
            } else if unquoted(item, '}') {
                nested = nested.saturating_sub(1);
            } else if nested == 0 && unquoted(item, ',') {
                comma = true;
                alternatives.push(std::mem::take(&mut current));
                continue;
            }
            current.push(*item);
        }
        if comma {
            alternatives.push(current);
            return Ok(Some((open, close, alternatives)));
        }
        if let Some(sequence) = brace_sequence(content)? {
            return Ok(Some((open, close, sequence)));
        }
    }
    Ok(None)
}

/// Expands a `{x..y[..step]}` sequence of integers or same-case letters.
fn brace_sequence(content: &[WordItem]) -> Result<Option<Vec<Vec<WordItem>>>, BraceOverflow> {
    let mut text = String::with_capacity(content.len());
    for item in content {
        match item {
            WordItem::Char {
                value,
                quoted: false,
            } => text.push(*value),
            _ => return Ok(None),
        }
    }
    let parts = text.split("..").collect::<Vec<_>>();
    if !(2..=3).contains(&parts.len()) {
        return Ok(None);
    }
    let step = match parts.get(2) {
        Some(step) => match step.parse::<i64>() {
            Ok(step) => step.unsigned_abs().max(1),
            Err(_) => return Ok(None),
        },
        None => 1,
    };
    let to_items = |value: String| {
        value
            .chars()
            .map(|value| WordItem::Char {
                value,
                quoted: false,
            })
            .collect::<Vec<_>>()
    };

    if let (Ok(start), Ok(end)) = (parts[0].parse::<i64>(), parts[1].parse::<i64>()) {
        let padded = |part: &str| {
            let digits = part.strip_prefix('-').unwrap_or(part);
            digits.len() > 1 && digits.starts_with('0')
        };
        if padded(parts[0]) || padded(parts[1]) {
            // Zero-padded sequences are not modeled.
            return Err(BraceOverflow);
        }
        let count = (start.abs_diff(end) / step).saturating_add(1);
        if count > MAX_BRACE_WORDS as u64 {
            return Err(BraceOverflow);
        }
        let values = (0..count)
            .map(|index| {
                let offset = i128::from(index) * i128::from(step);
                let value = if start <= end {
                    i128::from(start) + offset
                } else {
                    i128::from(start) - offset
                };
                to_items(value.to_string())
            })
            .collect();
        return Ok(Some(values));
    }

    let letter = |part: &str| {
        let mut characters = part.chars();
        match (characters.next(), characters.next()) {
            (Some(character), None) if character.is_ascii_alphabetic() => Some(character),
            _ => None,
        }
    };
    let (Some(start), Some(end)) = (letter(parts[0]), letter(parts[1])) else {
        return Ok(None);
    };
    if start.is_ascii_lowercase() != end.is_ascii_lowercase() {
        // Mixed-case ranges step through punctuation; not modeled.
        return Err(BraceOverflow);
    }
    let (start, end) = (start as u8, end as u8);
    let count = u64::from(start.abs_diff(end)) / step + 1;
    if count > MAX_BRACE_WORDS as u64 {
        return Err(BraceOverflow);
    }
    let values = (0..count)
        .map(|index| {
            let offset = (index * step) as u8;
            let value = if start <= end {
                start + offset
            } else {
                start - offset
            };
            to_items(char::from(value).to_string())
        })
        .collect();
    Ok(Some(values))
}

fn dynamic_reasons(node: Node<'_>, source: &str) -> Vec<DynamicReason> {
    let mut reasons = Vec::new();
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        match current.kind() {
            "expansion" | "simple_expansion" => reasons.push(DynamicReason::ParameterExpansion),
            "command_substitution" => reasons.push(DynamicReason::CommandSubstitution),
            "process_substitution" => reasons.push(DynamicReason::ProcessSubstitution),
            "arithmetic_expansion" => reasons.push(DynamicReason::ArithmeticExpansion),
            "brace_expression" => reasons.push(DynamicReason::BraceExpansion),
            "ansi_c_string" => reasons.push(DynamicReason::AnsiCString),
            "translated_string" => reasons.push(DynamicReason::LocaleTranslation),
            "ERROR" => reasons.push(DynamicReason::ParseError),
            "word" => {
                let raw = node_text(current, source);
                if contains_glob_syntax(raw) {
                    reasons.push(DynamicReason::Glob);
                }
                if raw.starts_with('~') {
                    reasons.push(DynamicReason::TildeExpansion);
                }
                if raw.contains('\\') {
                    reasons.push(DynamicReason::EscapeSequence);
                }
            }
            "string" if node_text(current, source).contains('\\') => {
                reasons.push(DynamicReason::EscapeSequence);
            }
            _ => {}
        }
        let mut cursor = current.walk();
        for child in current.named_children(&mut cursor) {
            stack.push(child);
        }
    }
    deduplicate(&mut reasons);
    reasons
}

fn analyze_redirection(
    node: Node<'_>,
    source: &str,
    descriptor: Option<String>,
    target: &[Node<'_>],
) -> Redirection {
    let kind = match node.kind() {
        "heredoc_redirect" => RedirectionKind::HereDocument,
        "herestring_redirect" => RedirectionKind::HereString,
        _ => RedirectionKind::File,
    };
    let operator = redirection_operator(node);
    let target = (!target.is_empty()).then(|| {
        let mut words = analyze_word_group(target, source);
        if words.len() == 1 {
            words.remove(0)
        } else {
            // A target that expands to several words is an ambiguous
            // redirect in Bash; keep it visible as dynamic.
            let span = span_between(target[0], target[target.len() - 1]);
            let raw = source
                .get(span.start_byte..span.end_byte)
                .unwrap_or("")
                .to_owned();
            dynamic_word(span, raw, vec![DynamicReason::BraceExpansion])
        }
    });
    let here_document = (kind == RedirectionKind::HereDocument)
        .then(|| analyze_here_document(node, source, operator))
        .flatten();
    Redirection {
        kind,
        operator,
        span: source_span(node),
        raw: node_text(node, source).to_owned(),
        descriptor,
        target,
        here_document,
    }
}

fn heredoc_start(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == "heredoc_start")
}

/// Any quoting of a here-document delimiter disables expansion of its body.
fn heredoc_delimiter_quoted(raw: &str) -> bool {
    raw.contains(['\'', '"', '\\'])
}

fn analyze_here_document(
    node: Node<'_>,
    source: &str,
    operator: Option<RedirectionOperator>,
) -> Option<HereDocument> {
    let delimiter_node = heredoc_start(node)?;
    let delimiter_raw = node_text(delimiter_node, source).to_owned();
    let delimiter_literal = literal_heredoc_delimiter(&delimiter_raw);
    let delimiter_quoted = heredoc_delimiter_quoted(&delimiter_raw);
    let delimiter = ShellWord {
        span: source_span(delimiter_node),
        raw: delimiter_raw,
        literal: delimiter_literal.clone(),
        dynamic_reasons: if delimiter_literal.is_some() {
            Vec::new()
        } else {
            vec![DynamicReason::UnsupportedSyntax]
        },
        pattern: None,
    };
    let mut cursor = node.walk();
    let body_node = node
        .named_children(&mut cursor)
        .find(|child| child.kind() == "heredoc_body");
    let mut body_dynamic_reasons = body_node.map_or_else(Vec::new, |body| {
        if delimiter_quoted {
            return Vec::new();
        }
        let mut reasons = dynamic_reasons(body, source);
        if !backtick_regions(body, source).is_empty() {
            reasons.push(DynamicReason::CommandSubstitution);
        }
        if has_active_heredoc_escape(node_text(body, source)) {
            reasons.push(DynamicReason::EscapeSequence);
        }
        reasons
    });
    if delimiter_literal.is_none() {
        body_dynamic_reasons.push(DynamicReason::UnsupportedSyntax);
    }
    deduplicate(&mut body_dynamic_reasons);
    let literal_body = body_node
        .filter(|_| body_dynamic_reasons.is_empty())
        .map(|body| node_text(body, source))
        .map(|body| {
            if operator == Some(RedirectionOperator::HereDocumentStripTabs) {
                strip_heredoc_tabs(body)
            } else {
                body.to_owned()
            }
        });
    Some(HereDocument {
        delimiter,
        body_span: body_node.map(source_span),
        literal_body,
        dynamic_reasons: body_dynamic_reasons,
    })
}

/// In an unquoted here-document body, Bash removes a backslash before `$`,
/// `` ` ``, `\`, or a newline.
fn has_active_heredoc_escape(body: &str) -> bool {
    let bytes = body.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            if matches!(bytes.get(index + 1), Some(b'$' | b'`' | b'\\' | b'\n')) {
                return true;
            }
            index += 2;
        } else {
            index += 1;
        }
    }
    false
}

/// One backtick command substitution in here-document text.
struct BacktickRegion {
    open: usize,
    close: Option<usize>,
}

/// Finds backtick substitutions in the plain-text portions of an unquoted
/// here-document body. Escaped backticks are literal, and ranges the grammar
/// already parsed (such as `$(...)`) are skipped.
fn backtick_regions(body: Node<'_>, source: &str) -> Vec<BacktickRegion> {
    let mut cursor = body.walk();
    let parsed = body
        .named_children(&mut cursor)
        .filter(|child| child.kind() != "heredoc_content")
        .map(|child| child.start_byte()..child.end_byte())
        .collect::<Vec<_>>();
    let bytes = source.as_bytes();
    let end = body.end_byte().min(bytes.len());
    let mut regions = Vec::new();
    let mut open = None;
    let mut index = body.start_byte();
    let mut next_parsed = 0;
    while index < end {
        // Children are in source order, so one forward pass skips them.
        while parsed
            .get(next_parsed)
            .is_some_and(|range| range.end <= index)
        {
            next_parsed += 1;
        }
        if let Some(range) = parsed.get(next_parsed).filter(|range| range.start <= index) {
            index = range.end;
            continue;
        }
        match bytes[index] {
            b'\\' => {
                index += 2;
                continue;
            }
            b'`' => match open.take() {
                Some(start) => regions.push(BacktickRegion {
                    open: start,
                    close: Some(index),
                }),
                None => open = Some(index),
            },
            _ => {}
        }
        index += 1;
    }
    if let Some(start) = open {
        regions.push(BacktickRegion {
            open: start,
            close: None,
        });
    }
    regions
}

fn literal_heredoc_delimiter(raw: &str) -> Option<String> {
    if raw.len() >= 2 {
        if let Some(value) = raw
            .strip_prefix('\'')
            .and_then(|value| value.strip_suffix('\''))
        {
            return (!value.is_empty()).then(|| value.to_owned());
        }
        if let Some(value) = raw
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
        {
            return (!value.is_empty() && !value.contains('\\')).then(|| value.to_owned());
        }
    }
    (!raw.is_empty()
        && raw
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_-+.".contains(character)))
    .then(|| raw.to_owned())
}

fn strip_heredoc_tabs(body: &str) -> String {
    let mut stripped = String::with_capacity(body.len());
    for line in body.split_inclusive('\n') {
        stripped.push_str(line.trim_start_matches('\t'));
    }
    stripped
}

fn redirection_operator(node: Node<'_>) -> Option<RedirectionOperator> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| !child.is_named())
        .find_map(|child| match child.kind() {
            "<" => Some(RedirectionOperator::Input),
            ">" => Some(RedirectionOperator::Output),
            ">>" => Some(RedirectionOperator::Append),
            "&>" => Some(RedirectionOperator::OutputAndError),
            "&>>" => Some(RedirectionOperator::AppendOutputAndError),
            "<&" => Some(RedirectionOperator::DuplicateInput),
            ">&" => Some(RedirectionOperator::DuplicateOutput),
            ">|" => Some(RedirectionOperator::Clobber),
            "<&-" => Some(RedirectionOperator::CloseInput),
            ">&-" => Some(RedirectionOperator::CloseOutput),
            "<<" => Some(RedirectionOperator::HereDocument),
            "<<-" => Some(RedirectionOperator::HereDocumentStripTabs),
            "<<<" => Some(RedirectionOperator::HereString),
            _ => None,
        })
}

/// Contexts for a command written after a here-document start on the same
/// line (`cat <<EOF > out && echo ok`).
fn heredoc_right_contexts(heredoc: Node<'_>) -> Vec<ExecutionContext> {
    match heredoc
        .child_by_field_name("operator")
        .map(|operator| operator.kind())
    {
        Some("&&" | "||") => vec![ExecutionContext::AndOrList],
        Some("|" | "|&") => vec![ExecutionContext::Pipeline, ExecutionContext::PipelineInput],
        Some(";") => vec![ExecutionContext::Sequence],
        Some("&") => vec![ExecutionContext::Background],
        _ => Vec::new(),
    }
}

/// Context the here-document's trailing operator implies for the command that
/// owns the here-document.
fn operator_owner_context(operator: &str) -> Option<ExecutionContext> {
    match operator {
        "&&" | "||" => Some(ExecutionContext::AndOrList),
        "|" | "|&" => Some(ExecutionContext::Pipeline),
        ";" => Some(ExecutionContext::Sequence),
        "&" => Some(ExecutionContext::Background),
        _ => None,
    }
}

fn is_argument_kind(kind: &str) -> bool {
    matches!(
        kind,
        "word"
            | "number"
            | "string"
            | "raw_string"
            | "concatenation"
            | "expansion"
            | "simple_expansion"
            | "command_substitution"
            | "process_substitution"
            | "arithmetic_expansion"
            | "brace_expression"
            | "ansi_c_string"
            | "translated_string"
            | "regex"
    )
}

fn construct_kinds(node: Node<'_>) -> Vec<ConstructKind> {
    match node.kind() {
        "pipeline" => vec![ConstructKind::Pipeline],
        "list" => {
            let has_and_or = has_immediate_token(node, &["&&", "||"]);
            let has_sequence = has_immediate_token(node, &[";"]);
            let has_background = has_immediate_token(node, &["&"]);
            let mut kinds = Vec::with_capacity(3);
            if has_and_or {
                kinds.push(ConstructKind::AndOrList);
            }
            if has_sequence {
                kinds.push(ConstructKind::Sequence);
            }
            if has_background {
                kinds.push(ConstructKind::Background);
            }
            kinds
        }
        "if_statement" => vec![ConstructKind::Conditional],
        "for_statement" | "c_style_for_statement" | "while_statement" => {
            vec![ConstructKind::Loop]
        }
        "case_statement" => vec![ConstructKind::Case],
        "function_definition" => vec![ConstructKind::FunctionDefinition],
        "subshell" => vec![ConstructKind::Subshell],
        "command_substitution" => vec![ConstructKind::CommandSubstitution],
        "process_substitution" => vec![ConstructKind::ProcessSubstitution],
        "declaration_command" => vec![ConstructKind::Declaration],
        "unset_command" => vec![ConstructKind::Unset],
        "test_command" => vec![ConstructKind::Test],
        "negated_command" => vec![ConstructKind::Negation],
        "variable_assignment" => vec![ConstructKind::VariableAssignment],
        "file_redirect" => vec![ConstructKind::FileRedirection],
        "heredoc_redirect" => vec![ConstructKind::HereDocument],
        "herestring_redirect" => vec![ConstructKind::HereString],
        "expansion" | "simple_expansion" => vec![ConstructKind::ParameterExpansion],
        "arithmetic_expansion" => vec![ConstructKind::ArithmeticExpansion],
        "brace_expression" => vec![ConstructKind::BraceExpansion],
        _ => Vec::new(),
    }
}

fn execution_contexts(node: Node<'_>) -> Vec<ExecutionContext> {
    match node.kind() {
        "pipeline" => vec![ExecutionContext::Pipeline],
        "list" => {
            let mut contexts = Vec::with_capacity(3);
            if has_immediate_token(node, &["&&", "||"]) {
                contexts.push(ExecutionContext::AndOrList);
            }
            if has_immediate_token(node, &[";"]) {
                contexts.push(ExecutionContext::Sequence);
            }
            if has_immediate_token(node, &["&"]) {
                contexts.push(ExecutionContext::Background);
            }
            contexts
        }
        "if_statement" => vec![ExecutionContext::Conditional],
        "for_statement" | "c_style_for_statement" | "while_statement" => {
            vec![ExecutionContext::Loop]
        }
        "case_statement" => vec![ExecutionContext::Case],
        "function_definition" => vec![ExecutionContext::FunctionDefinition],
        "subshell" => vec![ExecutionContext::Subshell],
        "command_substitution" => vec![ExecutionContext::CommandSubstitution],
        "process_substitution" => vec![ExecutionContext::ProcessSubstitution],
        "negated_command" => vec![ExecutionContext::Negated],
        _ => Vec::new(),
    }
}

fn has_immediate_token(node: Node<'_>, tokens: &[&str]) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| !child.is_named() && tokens.contains(&child.kind()))
}

fn first_named_child_or_self(node: Node<'_>) -> Option<Node<'_>> {
    node.named_child(0).or(Some(node))
}

fn contains_glob_syntax(value: &str) -> bool {
    value.contains(['*', '?', '['])
}

fn node_text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    source.get(node.byte_range()).unwrap_or("")
}

fn source_span(node: Node<'_>) -> SourceSpan {
    SourceSpan {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        start: source_position(node.start_position()),
        end: source_position(node.end_position()),
    }
}

fn span_between(first: Node<'_>, last: Node<'_>) -> SourceSpan {
    SourceSpan {
        start_byte: first.start_byte(),
        end_byte: last.end_byte().max(first.end_byte()),
        start: source_position(first.start_position()),
        end: source_position(if last.end_byte() >= first.end_byte() {
            last.end_position()
        } else {
            first.end_position()
        }),
    }
}

/// Converts increasing byte offsets into Tree-sitter points in one forward
/// pass over the source.
struct PointCursor {
    byte: usize,
    point: Point,
}

impl PointCursor {
    fn new(anchor: Node<'_>) -> Self {
        Self {
            byte: anchor.start_byte(),
            point: anchor.start_position(),
        }
    }

    /// Returns the point of `target`, which must not precede earlier targets.
    fn advance(&mut self, source: &str, target: usize) -> Point {
        let segment = source.as_bytes().get(self.byte..target).unwrap_or_default();
        match segment.iter().rposition(|byte| *byte == b'\n') {
            None => self.point.column += segment.len(),
            Some(last) => {
                self.point.row += segment.iter().filter(|byte| **byte == b'\n').count();
                self.point.column = segment.len() - last - 1;
            }
        }
        self.byte = self.byte.max(target);
        self.point
    }
}

fn source_position(point: Point) -> SourcePosition {
    SourcePosition {
        row: point.row,
        column: point.column,
    }
}

fn deduplicate<T: Ord>(values: &mut Vec<T>) {
    values.sort_unstable();
    values.dedup();
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn complete(source: &str) -> BashAnalysis {
        match BashAnalyzer::default().analyze(source) {
            BashAnalysisOutcome::Complete(analysis) => analysis,
            outcome => panic!("expected complete analysis, got {outcome:?}"),
        }
    }

    #[test]
    fn recovers_literal_commands_across_operators() {
        let analysis = complete("ls && pwd; echo 'hi there' | wc -l");
        assert_eq!(
            analysis.literal_commands().collect::<Vec<_>>(),
            vec![
                ["ls"].as_slice(),
                ["pwd"].as_slice(),
                ["echo", "hi there"].as_slice(),
                ["wc", "-l"].as_slice(),
            ]
        );
        assert_eq!(analysis.constructs(ConstructKind::Pipeline).count(), 1);
        assert_eq!(analysis.constructs(ConstructKind::AndOrList).count(), 1);
    }

    #[test]
    fn keeps_dynamic_outer_command_and_nested_substitution() {
        let analysis = complete("echo $(git status --short)");
        assert_eq!(analysis.commands.len(), 2);
        assert!(matches!(
            analysis.commands[0].argv,
            ArgvStatus::Dynamic {
                ref reasons
            } if reasons.contains(&DynamicReason::CommandSubstitution)
        ));
        assert_eq!(
            analysis.commands[1]
                .literal_argv()
                .map(|argv| argv.iter().map(String::as_str).collect::<Vec<_>>()),
            Some(vec!["git", "status", "--short"])
        );
        assert!(
            analysis.commands[1]
                .context
                .contains(&ExecutionContext::CommandSubstitution)
        );
    }

    #[test]
    fn distinguishes_quoted_literals_from_runtime_expansions() {
        let analysis = complete(r#"echo '*.rs' *.rs "$HOME" ~"#);
        assert_eq!(analysis.commands.len(), 1);
        assert_eq!(
            analysis.commands[0].arguments[0].literal.as_deref(),
            Some("*.rs")
        );
        assert!(
            analysis.commands[0].arguments[1]
                .dynamic_reasons
                .contains(&DynamicReason::Glob)
        );
        assert!(
            analysis.commands[0].arguments[2]
                .dynamic_reasons
                .contains(&DynamicReason::ParameterExpansion)
        );
        assert!(
            analysis.commands[0].arguments[3]
                .dynamic_reasons
                .contains(&DynamicReason::TildeExpansion)
        );
    }

    #[test]
    fn reports_redirections_without_hiding_literal_argv() {
        let analysis = complete("printf hello < in.txt > out.txt 2>> error.log");
        let command = &analysis.commands[0];
        assert_eq!(
            command
                .literal_argv()
                .map(|argv| argv.iter().map(String::as_str).collect::<Vec<_>>()),
            Some(vec!["printf", "hello"])
        );
        assert_eq!(command.redirections.len(), 3);
        assert_eq!(command.redirections[0].kind, RedirectionKind::File);
        assert_eq!(
            command.redirections[0].operator,
            Some(RedirectionOperator::Input)
        );
        assert_eq!(
            command.redirections[0]
                .target
                .as_ref()
                .and_then(|target| target.literal.as_deref()),
            Some("in.txt")
        );
        assert_eq!(
            command.redirections[1].operator,
            Some(RedirectionOperator::Output)
        );
        assert_eq!(
            command.redirections[1]
                .target
                .as_ref()
                .and_then(|target| target.literal.as_deref()),
            Some("out.txt")
        );
        assert_eq!(
            command.redirections[2].operator,
            Some(RedirectionOperator::Append)
        );
        assert_eq!(command.redirections[2].descriptor.as_deref(), Some("2"));
    }

    proptest! {
        /// Property: whitespace-separated words from Bash's unambiguous safe
        /// subset are recovered in order as literal argv. The command's source
        /// span and raw text continue to point at the exact original source.
        #[test]
        fn safe_words_round_trip_as_literal_argv(
            args in prop::collection::vec("[a-zA-Z0-9_./-]{1,20}", 0..20),
        ) {
            let expected = std::iter::once("hookkit_cmd".to_owned())
                .chain(args)
                .collect::<Vec<_>>();
            let source = expected.join(" ");
            let outcome = BashAnalyzer::default().analyze(&source);
            let Some(analysis) = outcome.analysis() else {
                return Err(TestCaseError::fail("safe command was unavailable"));
            };

            prop_assert!(outcome.is_complete());
            prop_assert_eq!(analysis.commands.len(), 1);
            prop_assert_eq!(analysis.commands[0].literal_argv(), Some(expected.as_slice()));
            prop_assert_eq!(&analysis.commands[0].raw, &source);
            let span = analysis.commands[0].span;
            prop_assert_eq!(&source[span.start_byte..span.end_byte], source.as_str());
        }

        /// Property: the source-size limit is a hard pre-parse boundary. Inputs
        /// above it are unavailable with exact actual/maximum byte counts.
        #[test]
        fn source_byte_limit_is_exact(extra in 1usize..512) {
            let max = 64usize;
            let source = "x".repeat(max + extra);
            let analyzer = BashAnalyzer::new(BashAnalyzerLimits {
                max_source_bytes: max,
                ..BashAnalyzerLimits::default()
            });

            prop_assert_eq!(
                analyzer.analyze(&source),
                BashAnalysisOutcome::Unavailable(UnavailableReason::InputTooLarge {
                    actual_bytes: max + extra,
                    max_bytes: max,
                })
            );
        }

        /// Property: an unquoted parameter expansion is never reported as a
        /// literal argv, even when surrounded by otherwise literal words.
        #[test]
        fn unquoted_parameters_always_mark_argv_dynamic(name in "[A-Z_][A-Z0-9_]{0,15}") {
            let source = format!("hookkit_cmd ${name}");
            let outcome = BashAnalyzer::default().analyze(&source);
            let analysis = outcome.analysis().unwrap();

            prop_assert!(analysis.commands[0].literal_argv().is_none());
            prop_assert!(analysis.contains_dynamic_syntax());
        }
    }

    #[test]
    fn reports_conditional_loop_and_function_context() {
        let analysis = complete("f() { if test -f x; then for x in a b; do echo $x; done; fi; }");
        assert!(
            analysis
                .constructs
                .iter()
                .any(|item| item.kind == ConstructKind::FunctionDefinition)
        );
        assert!(
            analysis
                .constructs
                .iter()
                .any(|item| item.kind == ConstructKind::Conditional)
        );
        assert!(
            analysis
                .constructs
                .iter()
                .any(|item| item.kind == ConstructKind::Loop)
        );
        let echo = analysis
            .commands
            .iter()
            .find(|command| {
                command
                    .name
                    .as_ref()
                    .and_then(|name| name.literal.as_deref())
                    == Some("echo")
            })
            .unwrap();
        assert!(echo.context.contains(&ExecutionContext::FunctionDefinition));
        assert!(echo.context.contains(&ExecutionContext::Conditional));
        assert!(echo.context.contains(&ExecutionContext::Loop));
    }

    #[test]
    fn reports_process_substitution_and_heredoc_structure() {
        let process = complete("cat <(printf x)");
        assert_eq!(
            process
                .constructs(ConstructKind::ProcessSubstitution)
                .count(),
            1
        );
        let printf = process
            .commands
            .iter()
            .find(|command| {
                command
                    .name
                    .as_ref()
                    .and_then(|name| name.literal.as_deref())
                    == Some("printf")
            })
            .unwrap();
        assert!(
            printf
                .context
                .contains(&ExecutionContext::ProcessSubstitution)
        );

        let heredoc = complete("cat <<'EOF'\nhello\nEOF\n");
        assert_eq!(heredoc.constructs(ConstructKind::HereDocument).count(), 1);
        assert_eq!(heredoc.commands[0].redirections.len(), 1);
        assert_eq!(
            heredoc.commands[0].redirections[0].kind,
            RedirectionKind::HereDocument
        );
        let here_document = heredoc.commands[0].redirections[0]
            .here_document
            .as_ref()
            .unwrap();
        assert_eq!(here_document.delimiter.literal.as_deref(), Some("EOF"));
        assert_eq!(here_document.literal_body.as_deref(), Some("hello\n"));
        assert!(here_document.body_span.is_some());

        let dynamic = complete("cat <<EOF\nhello $USER\nEOF\n");
        let here_document = dynamic.commands[0].redirections[0]
            .here_document
            .as_ref()
            .unwrap();
        assert_eq!(here_document.literal_body, None);
        assert!(
            here_document
                .dynamic_reasons
                .contains(&DynamicReason::ParameterExpansion)
        );
    }

    #[test]
    fn retains_specialized_shell_statements_and_negation_context() {
        let analysis = complete("export FOO=$BAR; unset BAZ; ! grep needle file");
        assert_eq!(analysis.constructs(ConstructKind::Declaration).count(), 1);
        assert_eq!(analysis.constructs(ConstructKind::Unset).count(), 1);
        assert_eq!(analysis.constructs(ConstructKind::Negation).count(), 1);
        let grep = analysis
            .commands
            .iter()
            .find(|command| {
                command
                    .name
                    .as_ref()
                    .and_then(|name| name.literal.as_deref())
                    == Some("grep")
            })
            .unwrap();
        assert!(grep.context.contains(&ExecutionContext::Negated));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_feature_serializes_owned_outcomes() {
        let outcome = BashAnalyzer::default().analyze("echo $HOME");
        let value = serde_json::to_value(outcome).unwrap();
        assert!(value.get("Complete").is_some());
    }

    #[test]
    fn syntax_errors_are_partial_not_successful() {
        assert!(matches!(
            BashAnalyzer::default().analyze("echo hi &&"),
            BashAnalysisOutcome::Partial {
                reason: IncompleteReason::SyntaxErrors,
                ..
            }
        ));
        assert!(matches!(
            BashAnalyzer::default().analyze("cat <<EOF\nunterminated\n"),
            BashAnalysisOutcome::Partial {
                reason: IncompleteReason::SyntaxErrors,
                ..
            }
        ));
    }

    #[test]
    fn applies_source_and_traversal_limits() {
        let input_limit = BashAnalyzer::new(BashAnalyzerLimits {
            max_source_bytes: 3,
            ..BashAnalyzerLimits::default()
        });
        assert!(matches!(
            input_limit.analyze("echo"),
            BashAnalysisOutcome::Unavailable(UnavailableReason::InputTooLarge { .. })
        ));

        let node_limit = BashAnalyzer::new(BashAnalyzerLimits {
            max_nodes: 1,
            ..BashAnalyzerLimits::default()
        });
        assert!(matches!(
            node_limit.analyze("echo hi"),
            BashAnalysisOutcome::Partial {
                reason: IncompleteReason::NodeLimit { max_nodes: 1 },
                ..
            }
        ));

        let depth_limit = BashAnalyzer::new(BashAnalyzerLimits {
            max_depth: 2,
            ..BashAnalyzerLimits::default()
        });
        assert!(matches!(
            depth_limit.analyze("echo $(printf '%s' $(pwd))"),
            BashAnalysisOutcome::Partial {
                reason: IncompleteReason::DepthLimit { max_depth: 2 },
                ..
            }
        ));
    }

    fn argv(analysis: &BashAnalysis, index: usize) -> Vec<&str> {
        analysis.commands[index]
            .literal_argv()
            .expect("literal argv")
            .iter()
            .map(String::as_str)
            .collect()
    }

    fn named<'a>(analysis: &'a BashAnalysis, name: &str) -> &'a CommandOccurrence {
        analysis
            .commands
            .iter()
            .find(|command| {
                command
                    .name
                    .as_ref()
                    .and_then(|word| word.literal.as_deref())
                    == Some(name)
            })
            .unwrap_or_else(|| panic!("no command named {name}"))
    }

    fn target(redirection: &Redirection) -> Option<&str> {
        redirection
            .target
            .as_ref()
            .and_then(|target| target.literal.as_deref())
    }

    #[test]
    fn words_after_a_redirection_target_are_command_arguments() {
        let analysis = complete("rm 2>/dev/null .env");
        assert_eq!(argv(&analysis, 0), ["rm", ".env"]);
        let redirections = &analysis.commands[0].redirections;
        assert_eq!(redirections.len(), 1);
        assert_eq!(target(&redirections[0]), Some("/dev/null"));
        assert_eq!(redirections[0].descriptor.as_deref(), Some("2"));

        assert_eq!(argv(&complete("cat 2>&1 .env"), 0), ["cat", ".env"]);
        assert_eq!(argv(&complete("cat </dev/null .env"), 0), ["cat", ".env"]);
        let tee = complete("tee < .env out");
        assert_eq!(argv(&tee, 0), ["tee", "out"]);
        assert_eq!(target(&tee.commands[0].redirections[0]), Some(".env"));
    }

    #[test]
    fn heredoc_owned_redirections_and_arguments_belong_to_the_command() {
        let write = complete("cat <<EOF > .env\nhello\nEOF\n");
        let redirections = &write.commands[0].redirections;
        assert_eq!(redirections.len(), 2);
        assert_eq!(redirections[0].kind, RedirectionKind::HereDocument);
        assert_eq!(redirections[1].operator, Some(RedirectionOperator::Output));
        assert_eq!(target(&redirections[1]), Some(".env"));

        assert_eq!(argv(&complete("rm <<EOF .env\nEOF\n"), 0), ["rm", ".env"]);

        let chained = complete("cat <<EOF >> .env && echo ok\nhello\nEOF\n");
        let cat = named(&chained, "cat");
        assert!(cat.context.contains(&ExecutionContext::AndOrList));
        assert_eq!(cat.redirections.len(), 2);
        assert!(
            named(&chained, "echo")
                .context
                .contains(&ExecutionContext::AndOrList)
        );
    }

    #[test]
    fn list_level_redirections_attach_to_the_last_command() {
        let analysis = complete("cd dir && apply_patch <<'EOF'\n*** Begin Patch\nEOF\n");
        assert!(named(&analysis, "cd").redirections.is_empty());
        let patch = named(&analysis, "apply_patch");
        assert_eq!(patch.redirections.len(), 1);
        assert_eq!(
            patch.redirections[0]
                .here_document
                .as_ref()
                .and_then(|document| document.literal_body.as_deref()),
            Some("*** Begin Patch\n")
        );
        assert!(analysis.statement_redirections.is_empty());

        let piped = complete("a | b 2>/dev/null .env");
        assert_eq!(argv(&piped, 1), ["b", ".env"]);
        assert!(piped.commands[0].redirections.is_empty());

        let negated = complete("! cat > .env");
        assert_eq!(
            target(&named(&negated, "cat").redirections[0]),
            Some(".env")
        );
    }

    #[test]
    fn compound_statement_redirections_are_reported_with_their_commands() {
        let group = complete("{ cat; } < .env");
        let statement = &group.statement_redirections[0];
        assert_eq!(statement.kind, StatementKind::Group);
        assert_eq!(statement.commands, 0..1);
        assert_eq!(
            statement.redirections[0].operator,
            Some(RedirectionOperator::Input)
        );
        assert_eq!(target(&statement.redirections[0]), Some(".env"));

        for (source, kind) in [
            ("(echo x) > .env", StatementKind::Subshell),
            (
                "if true; then echo x; fi > .env",
                StatementKind::Conditional,
            ),
            ("for f in a; do echo; done > .env", StatementKind::Loop),
            ("case x in x) echo;; esac > .env", StatementKind::Case),
            ("[ -f x ] > .env", StatementKind::Test),
            ("declare -p > .env", StatementKind::Declaration),
            ("a && { b; } > .env", StatementKind::Group),
        ] {
            let analysis = complete(source);
            assert_eq!(analysis.statement_redirections.len(), 1, "{source}");
            assert_eq!(analysis.statement_redirections[0].kind, kind, "{source}");
            assert_eq!(
                target(&analysis.statement_redirections[0].redirections[0]),
                Some(".env"),
                "{source}"
            );
        }

        let function = complete("f() { echo x; } > .env; f");
        let statement = &function.statement_redirections[0];
        assert_eq!(statement.kind, StatementKind::FunctionDefinition);
        assert!(
            statement
                .context
                .contains(&ExecutionContext::FunctionDefinition)
        );

        let subshell_patch = complete("(cd x && apply_patch) <<'EOF'\nbody\nEOF\n");
        let statement = &subshell_patch.statement_redirections[0];
        assert_eq!(statement.commands, 0..2);
        assert_eq!(
            statement.redirections[0]
                .here_document
                .as_ref()
                .and_then(|document| document.literal_body.as_deref()),
            Some("body\n")
        );
    }

    #[test]
    fn redirection_only_statements_are_nameless_commands() {
        let truncate = complete("> .env");
        assert_eq!(truncate.commands.len(), 1);
        assert_eq!(truncate.commands[0].name, None);
        assert_eq!(truncate.commands[0].literal_argv(), Some(&[][..]));
        assert_eq!(target(&truncate.commands[0].redirections[0]), Some(".env"));

        for source in ["echo \"$(< .env)\"", "echo $(<.env)", "x=$(< .env)"] {
            let analysis = complete(source);
            let read = analysis
                .commands
                .iter()
                .find(|command| command.name.is_none())
                .unwrap_or_else(|| panic!("{source}"));
            assert!(
                read.context
                    .contains(&ExecutionContext::CommandSubstitution)
            );
            assert_eq!(
                read.redirections[0].operator,
                Some(RedirectionOperator::Input)
            );
            assert_eq!(target(&read.redirections[0]), Some(".env"));
        }
    }

    #[test]
    fn line_continuations_join_word_pieces() {
        assert_eq!(argv(&complete("cat .e\\\nnv"), 0), ["cat", ".env"]);
        assert_eq!(argv(&complete("ca\\\nt .e\\\nnv"), 0), ["cat", ".env"]);
        assert_eq!(argv(&complete("cat 'a'\\\n.env"), 0), ["cat", "a.env"]);
        let redirect = complete("echo x > .e\\\nnv");
        assert_eq!(argv(&redirect, 0), ["echo", "x"]);
        assert_eq!(target(&redirect.commands[0].redirections[0]), Some(".env"));
        // Whitespace around a continuation still separates words.
        assert_eq!(
            argv(&complete("cargo test \\\n  --all"), 0),
            ["cargo", "test", "--all"]
        );
    }

    #[test]
    fn brace_expansion_is_expanded_or_marked_dynamic() {
        assert_eq!(argv(&complete("cat .e{n,}v"), 0), ["cat", ".env", ".ev"]);
        assert_eq!(
            argv(&complete("rm -f {.env,x}"), 0),
            ["rm", "-f", ".env", "x"]
        );
        assert_eq!(argv(&complete("cp f{,.bak}"), 0), ["cp", "f", "f.bak"]);
        assert_eq!(argv(&complete("cat {1..3}"), 0), ["cat", "1", "2", "3"]);
        assert_eq!(argv(&complete("echo {a..c}"), 0), ["echo", "a", "b", "c"]);
        assert_eq!(
            argv(&complete("echo a{b,{c,d}}e"), 0),
            ["echo", "abe", "ace", "ade"]
        );
        assert_eq!(
            argv(&complete("find . -exec rm {} +"), 0),
            ["find", ".", "-exec", "rm", "{}", "+"]
        );
        assert_eq!(
            argv(&complete("git show HEAD@{1}"), 0),
            ["git", "show", "HEAD@{1}"]
        );
        assert_eq!(argv(&complete("echo '{a,b}'"), 0), ["echo", "{a,b}"]);

        let expanded = complete("cat .e{n,}v");
        let arguments = &expanded.commands[0].arguments;
        assert_eq!(arguments[0].span, arguments[1].span);
        assert_eq!(arguments[0].raw, ".e{n,}v");

        for source in [
            "cat {1..1000}",
            "cat x{a,b}$Y",
            "cat {01..03}",
            "cat {-9223372036854775808..9223372036854775807}",
            "cat {a..Z}",
        ] {
            let analysis = complete(source);
            assert!(
                matches!(
                    &analysis.commands[0].argv,
                    ArgvStatus::Dynamic { reasons } if reasons.contains(&DynamicReason::BraceExpansion)
                ),
                "{source}"
            );
        }
    }

    #[test]
    fn separately_lexed_descriptors_are_not_arguments() {
        let zero = complete("cat 0<.env x");
        assert_eq!(argv(&zero, 0), ["cat", "x"]);
        assert_eq!(
            zero.commands[0].redirections[0].descriptor.as_deref(),
            Some("0")
        );
        assert_eq!(target(&zero.commands[0].redirections[0]), Some(".env"));

        let named_fd = complete("cat {fd}>out");
        assert_eq!(argv(&named_fd, 0), ["cat"]);
        assert_eq!(
            named_fd.commands[0].redirections[0].descriptor.as_deref(),
            Some("{fd}")
        );

        // A separated number is still an argument.
        assert_eq!(argv(&complete("head -n 0 <file"), 0), ["head", "-n", "0"]);
    }

    #[test]
    fn heredoc_backticks_are_parsed_as_command_substitutions() {
        let analysis = complete("cat <<EOF\n`cat .env` and $(pwd)\nEOF\n");
        let inner = analysis
            .commands
            .iter()
            .find(|command| {
                command.literal_argv() == Some(&["cat".to_owned(), ".env".to_owned()][..])
            })
            .expect("backtick command");
        assert!(
            inner
                .context
                .contains(&ExecutionContext::CommandSubstitution)
        );
        assert_eq!(&analysis.commands.len(), &3);
        let here_document = analysis.commands[0].redirections[0]
            .here_document
            .as_ref()
            .unwrap();
        assert_eq!(here_document.literal_body, None);
        assert!(
            here_document
                .dynamic_reasons
                .contains(&DynamicReason::CommandSubstitution)
        );
        let span = inner.span;
        assert_eq!(
            &"cat <<EOF\n`cat .env` and $(pwd)\nEOF\n"[span.start_byte..span.end_byte],
            "cat .env"
        );
        assert_eq!(span.start.row, 1);
        assert_eq!(span.start.column, 1);

        // Quoted delimiters keep backticks literal.
        let quoted = complete("cat <<'EOF'\n`cat .env`\nEOF\n");
        assert_eq!(quoted.commands.len(), 1);
        assert_eq!(
            quoted.commands[0].redirections[0]
                .here_document
                .as_ref()
                .and_then(|document| document.literal_body.as_deref()),
            Some("`cat .env`\n")
        );

        for source in [
            "cat <<EOF\n`cat \\.env`\nEOF\n",
            "cat <<EOF\n`cat .env\nEOF\n",
        ] {
            assert!(
                matches!(
                    BashAnalyzer::default().analyze(source),
                    BashAnalysisOutcome::Partial {
                        reason: IncompleteReason::UnparsedCommandSubstitution,
                        ..
                    }
                ),
                "{source}"
            );
        }
    }

    #[test]
    fn unquoted_heredoc_escapes_prevent_a_literal_body() {
        let escaped = complete("cat <<EOF\na\\$b\nEOF\n");
        let document = escaped.commands[0].redirections[0]
            .here_document
            .as_ref()
            .unwrap();
        assert_eq!(document.literal_body, None);
        assert!(
            document
                .dynamic_reasons
                .contains(&DynamicReason::EscapeSequence)
        );

        let continued = complete("apply_patch <<EOF\n*** Delete File: .e\\\nnv\nEOF\n");
        assert_eq!(
            continued.commands[0].redirections[0]
                .here_document
                .as_ref()
                .unwrap()
                .literal_body,
            None
        );

        // Backslashes Bash leaves alone keep the body literal.
        let inactive = complete("cat <<EOF\nlet re = \\d+;\nEOF\n");
        assert_eq!(
            inactive.commands[0].redirections[0]
                .here_document
                .as_ref()
                .and_then(|document| document.literal_body.as_deref()),
            Some("let re = \\d+;\n")
        );
    }

    #[test]
    fn depth_limit_skips_only_the_deep_subtree() {
        let chain = format!("{}cat .env", "true && ".repeat(200));
        let analysis = complete(&chain);
        assert_eq!(analysis.commands.len(), 201);
        assert_eq!(argv(&analysis, 200), ["cat", ".env"]);

        let nested = format!("echo {}x{}; cat .env", "$(".repeat(80), ")".repeat(80));
        let outcome = BashAnalyzer::default().analyze(&nested);
        let BashAnalysisOutcome::Partial {
            analysis,
            reason: IncompleteReason::DepthLimit { .. },
        } = outcome
        else {
            panic!("expected a depth-limited partial analysis");
        };
        assert!(
            analysis
                .literal_commands()
                .any(|argv| argv == ["cat".to_owned(), ".env".to_owned()])
        );
    }

    #[test]
    fn glob_and_tilde_words_carry_quote_removed_patterns() {
        let quoted = complete("cat \".en\"v*");
        let word = &quoted.commands[0].arguments[0];
        assert_eq!(word.dynamic_reasons, vec![DynamicReason::Glob]);
        assert_eq!(word.pattern.as_deref(), Some(".env*"));

        let class = complete("cat '[x]'* a{b}*");
        assert_eq!(
            class.commands[0].arguments[0].pattern.as_deref(),
            Some("[[]x[]]*")
        );
        assert_eq!(
            class.commands[0].arguments[1].pattern.as_deref(),
            Some("a[{]b[}]*")
        );

        let tilde = complete("cat ~/\".ssh\"/id_rsa");
        let word = &tilde.commands[0].arguments[0];
        assert_eq!(word.dynamic_reasons, vec![DynamicReason::TildeExpansion]);
        assert_eq!(word.pattern.as_deref(), Some("~/.ssh/id_rsa"));

        let assignment = complete("dd if=~/.ssh/id_rsa of=x:~/y name='~'");
        let arguments = &assignment.commands[0].arguments;
        assert!(
            arguments[0]
                .dynamic_reasons
                .contains(&DynamicReason::TildeExpansion)
        );
        assert!(
            arguments[1]
                .dynamic_reasons
                .contains(&DynamicReason::TildeExpansion)
        );
        assert_eq!(arguments[2].literal.as_deref(), Some("name=~"));
    }

    #[test]
    fn later_pipeline_elements_receive_piped_input() {
        let analysis = complete("ls | grep x | { rg y; }");
        assert!(
            !named(&analysis, "ls")
                .context
                .contains(&ExecutionContext::PipelineInput)
        );
        for name in ["grep", "rg"] {
            assert!(
                named(&analysis, name)
                    .context
                    .contains(&ExecutionContext::PipelineInput),
                "{name}"
            );
        }
    }

    #[test]
    fn limits_have_builders() {
        let limits = BashAnalyzerLimits::default()
            .with_max_source_bytes(1)
            .with_max_parse_time(Duration::from_millis(5))
            .with_max_nodes(2)
            .with_max_depth(3);
        assert_eq!(limits.max_source_bytes, 1);
        assert_eq!(limits.max_parse_time, Duration::from_millis(5));
        assert_eq!(limits.max_nodes, 2);
        assert_eq!(limits.max_depth, 3);
    }

    #[test]
    fn pathological_inputs_stay_bounded() {
        let inputs = [
            format!("echo {}", "{".repeat(100_000)),
            format!("echo {}", "{a,".repeat(20_000)),
            format!("cat <<EOF\n{}\nEOF\n", "`a` ".repeat(20_000)),
            format!("cat <<EOF\n{}\nEOF\n", "$x ` ".repeat(20_000)),
            format!("echo {} {}", "1 ".repeat(10_000), ">a ".repeat(10_000)),
            format!(
                "{{ {} }} && b {}",
                "a; ".repeat(10_000),
                ">x ".repeat(10_000)
            ),
        ];
        for input in inputs {
            let started = Instant::now();
            let _ = BashAnalyzer::default().analyze(&input);
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "{} took {:?}",
                &input[..40],
                started.elapsed()
            );
        }
    }
}
