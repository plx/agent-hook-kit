//! Bounded Tree-sitter Bash analysis.
//!
//! Literal word recovery is adapted from OpenAI Codex's Apache-2.0-licensed
//! `codex-rs/shell-command/src/bash.rs` at commit
//! `9e552e9d15ba52bed7077d5357f3e18e330f8f38`. This implementation has been
//! substantially changed to retain per-command uncertainty, nesting context,
//! redirections, and partial-analysis status.

use std::time::{Duration, Instant};

use tree_sitter::{Node, ParseOptions, Parser, Point};

/// Resource limits applied before and during parsing and AST traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BashAnalyzerLimits {
    pub max_source_bytes: usize,
    pub max_parse_time: Duration,
    pub max_nodes: usize,
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

/// Reusable Bash analyzer configuration.
#[derive(Debug, Clone, Copy, Default)]
pub struct BashAnalyzer {
    limits: BashAnalyzerLimits,
}

impl BashAnalyzer {
    pub const fn new(limits: BashAnalyzerLimits) -> Self {
        Self { limits }
    }

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

        let mut parser = Parser::new();
        let language = tree_sitter_bash::LANGUAGE.into();
        if let Err(error) = parser.set_language(&language) {
            return BashAnalysisOutcome::Unavailable(UnavailableReason::ParserInitialization(
                error.to_string(),
            ));
        }

        let started = Instant::now();
        let mut timed_out = false;
        let tree = {
            let mut read = |offset: usize, _position: Point| &source.as_bytes()[offset..];
            let mut cancel = |_state: &tree_sitter::ParseState| {
                if started.elapsed() >= self.limits.max_parse_time {
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
            return BashAnalysisOutcome::Unavailable(UnavailableReason::ParseTimeLimit {
                max_millis: duration_millis(self.limits.max_parse_time),
            });
        }
        let Some(tree) = tree else {
            return BashAnalysisOutcome::Unavailable(UnavailableReason::ParserReturnedNoTree);
        };

        let root = tree.root_node();
        let syntax_errors = root.has_error();
        let mut walker = Walker::new(source, self.limits);
        let mut contexts = Vec::new();
        walker.visit(root, &mut contexts, 0);
        let analysis = walker.analysis;

        if let Some(reason) = walker.incomplete {
            BashAnalysisOutcome::Partial { analysis, reason }
        } else if syntax_errors {
            BashAnalysisOutcome::Partial {
                analysis,
                reason: IncompleteReason::SyntaxErrors,
            }
        } else {
            BashAnalysisOutcome::Complete(analysis)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum BashAnalysisOutcome {
    Complete(BashAnalysis),
    Partial {
        analysis: BashAnalysis,
        reason: IncompleteReason,
    },
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

    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Complete(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BashAnalysis {
    pub commands: Vec<CommandOccurrence>,
    pub constructs: Vec<ConstructOccurrence>,
}

impl BashAnalysis {
    /// Literal argv recovered for individual commands. Redirections and other
    /// surrounding constructs remain visible separately and must not be ignored
    /// when making a policy decision.
    pub fn literal_commands(&self) -> impl Iterator<Item = &[String]> {
        self.commands
            .iter()
            .filter_map(CommandOccurrence::literal_argv)
    }

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

    pub fn constructs(&self, kind: ConstructKind) -> impl Iterator<Item = &ConstructOccurrence> {
        self.constructs
            .iter()
            .filter(move |construct| construct.kind == kind)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CommandOccurrence {
    pub span: SourceSpan,
    pub raw: String,
    pub name: Option<ShellWord>,
    pub arguments: Vec<ShellWord>,
    pub argv: ArgvStatus,
    pub context: Vec<ExecutionContext>,
    pub redirections: Vec<Redirection>,
}

impl CommandOccurrence {
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
pub enum ArgvStatus {
    Literal(Vec<String>),
    Dynamic { reasons: Vec<DynamicReason> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ShellWord {
    pub span: SourceSpan,
    pub raw: String,
    pub literal: Option<String>,
    pub dynamic_reasons: Vec<DynamicReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum DynamicReason {
    ParameterExpansion,
    CommandSubstitution,
    ProcessSubstitution,
    ArithmeticExpansion,
    BraceExpansion,
    Glob,
    TildeExpansion,
    EscapeSequence,
    AnsiCString,
    LocaleTranslation,
    ParseError,
    UnsupportedSyntax,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ExecutionContext {
    Pipeline,
    AndOrList,
    Sequence,
    Conditional,
    Loop,
    Case,
    FunctionDefinition,
    Subshell,
    CommandSubstitution,
    ProcessSubstitution,
    Background,
    Negated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ConstructOccurrence {
    pub kind: ConstructKind,
    pub span: SourceSpan,
    pub raw: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ConstructKind {
    Pipeline,
    AndOrList,
    Sequence,
    Background,
    Conditional,
    Loop,
    Case,
    FunctionDefinition,
    Subshell,
    CommandSubstitution,
    ProcessSubstitution,
    Declaration,
    Unset,
    Test,
    Negation,
    VariableAssignment,
    FileRedirection,
    HereDocument,
    HereString,
    ParameterExpansion,
    ArithmeticExpansion,
    BraceExpansion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Redirection {
    pub kind: RedirectionKind,
    /// Typed shell operator. This is `None` only when error-recovered syntax
    /// did not contain a recognizable operator.
    pub operator: Option<RedirectionOperator>,
    pub span: SourceSpan,
    pub raw: String,
    pub descriptor: Option<String>,
    pub target: Option<ShellWord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum RedirectionKind {
    File,
    HereDocument,
    HereString,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum RedirectionOperator {
    Input,
    Output,
    Append,
    OutputAndError,
    AppendOutputAndError,
    DuplicateInput,
    DuplicateOutput,
    Clobber,
    CloseInput,
    CloseOutput,
    HereDocument,
    HereDocumentStripTabs,
    HereString,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SourceSpan {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start: SourcePosition,
    pub end: SourcePosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SourcePosition {
    /// Zero-based row.
    pub row: usize,
    /// Zero-based byte column.
    pub column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum IncompleteReason {
    SyntaxErrors,
    NodeLimit { max_nodes: usize },
    DepthLimit { max_depth: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum UnavailableReason {
    InputTooLarge {
        actual_bytes: usize,
        max_bytes: usize,
    },
    ParseTimeLimit {
        max_millis: u64,
    },
    ParserInitialization(String),
    ParserReturnedNoTree,
}

struct Walker<'source> {
    source: &'source str,
    limits: BashAnalyzerLimits,
    visited_nodes: usize,
    analysis: BashAnalysis,
    incomplete: Option<IncompleteReason>,
}

impl<'source> Walker<'source> {
    fn new(source: &'source str, limits: BashAnalyzerLimits) -> Self {
        Self {
            source,
            limits,
            visited_nodes: 0,
            analysis: BashAnalysis {
                commands: Vec::new(),
                constructs: Vec::new(),
            },
            incomplete: None,
        }
    }

    fn visit(&mut self, node: Node<'_>, contexts: &mut Vec<ExecutionContext>, depth: usize) {
        if self.incomplete.is_some() {
            return;
        }
        if depth > self.limits.max_depth {
            self.incomplete = Some(IncompleteReason::DepthLimit {
                max_depth: self.limits.max_depth,
            });
            return;
        }
        if self.visited_nodes >= self.limits.max_nodes {
            self.incomplete = Some(IncompleteReason::NodeLimit {
                max_nodes: self.limits.max_nodes,
            });
            return;
        }
        self.visited_nodes += 1;

        for kind in construct_kinds(node) {
            self.analysis.constructs.push(ConstructOccurrence {
                kind,
                span: source_span(node),
                raw: node_text(node, self.source).to_owned(),
            });
        }

        if node.kind() == "command" {
            self.analysis
                .commands
                .push(analyze_command(node, self.source, contexts));
        }

        let added_contexts = execution_contexts(node);
        contexts.extend(added_contexts.iter().copied());
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.visit(child, contexts, depth + 1);
            if self.incomplete.is_some() {
                break;
            }
        }
        contexts.truncate(contexts.len() - added_contexts.len());
    }
}

fn analyze_command(
    node: Node<'_>,
    source: &str,
    contexts: &[ExecutionContext],
) -> CommandOccurrence {
    let name = node
        .child_by_field_name("name")
        .and_then(first_named_child_or_self)
        .map(|node| analyze_word(node, source));

    let mut arguments = Vec::new();
    let mut redirections = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "command_name" | "variable_assignment" | "comment" => {}
            "file_redirect" | "heredoc_redirect" | "herestring_redirect" => {
                redirections.push(analyze_redirection(child, source));
            }
            kind if is_argument_kind(kind) => arguments.push(analyze_word(child, source)),
            _ => {}
        }
    }
    if let Some(parent) = node
        .parent()
        .filter(|parent| parent.kind() == "redirected_statement")
    {
        let mut cursor = parent.walk();
        for child in parent.named_children(&mut cursor) {
            if matches!(
                child.kind(),
                "file_redirect" | "heredoc_redirect" | "herestring_redirect"
            ) {
                redirections.push(analyze_redirection(child, source));
            }
        }
    }

    let argv = command_argv(node, name.as_ref(), &arguments);
    CommandOccurrence {
        span: source_span(node),
        raw: node_text(node, source).to_owned(),
        name,
        arguments,
        argv,
        context: contexts.to_vec(),
        redirections,
    }
}

fn command_argv(
    command: Node<'_>,
    name: Option<&ShellWord>,
    arguments: &[ShellWord],
) -> ArgvStatus {
    let mut reasons = Vec::new();
    if command.has_error() {
        reasons.push(DynamicReason::ParseError);
    }

    let mut argv = Vec::with_capacity(arguments.len() + 1);
    match name {
        Some(ShellWord {
            literal: Some(name),
            ..
        }) => argv.push(name.clone()),
        Some(name) => reasons.extend(name.dynamic_reasons.iter().copied()),
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

fn analyze_word(node: Node<'_>, source: &str) -> ShellWord {
    let raw = node_text(node, source).to_owned();
    let literal = literal_word(node, source);
    let mut dynamic_reasons = if literal.is_some() {
        Vec::new()
    } else {
        dynamic_reasons(node, source)
    };
    if literal.is_none() && dynamic_reasons.is_empty() {
        dynamic_reasons.push(DynamicReason::UnsupportedSyntax);
    }
    ShellWord {
        span: source_span(node),
        raw,
        literal,
        dynamic_reasons,
    }
}

fn literal_word(node: Node<'_>, source: &str) -> Option<String> {
    if node.has_error() {
        return None;
    }
    match node.kind() {
        "command_name" => literal_word(node.named_child(0)?, source),
        "word" | "number" => {
            if node.named_child_count() != 0 {
                return None;
            }
            let raw = node_text(node, source);
            if has_unquoted_runtime_syntax(raw) {
                None
            } else {
                Some(raw.to_owned())
            }
        }
        "raw_string" => node_text(node, source)
            .strip_prefix('\'')
            .and_then(|value| value.strip_suffix('\''))
            .map(str::to_owned),
        "string" => {
            let mut cursor = node.walk();
            if node
                .named_children(&mut cursor)
                .any(|child| child.kind() != "string_content")
            {
                return None;
            }
            let value = node_text(node, source)
                .strip_prefix('"')?
                .strip_suffix('"')?;
            if value.contains('\\') {
                None
            } else {
                Some(value.to_owned())
            }
        }
        "concatenation" => {
            let mut value = String::new();
            let mut count = 0;
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                value.push_str(&literal_word(child, source)?);
                count += 1;
            }
            (count > 0).then_some(value)
        }
        _ => None,
    }
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

fn analyze_redirection(node: Node<'_>, source: &str) -> Redirection {
    let kind = match node.kind() {
        "heredoc_redirect" => RedirectionKind::HereDocument,
        "herestring_redirect" => RedirectionKind::HereString,
        _ => RedirectionKind::File,
    };
    let descriptor = node
        .child_by_field_name("descriptor")
        .map(|descriptor| node_text(descriptor, source).to_owned());
    let operator = redirection_operator(node);
    let target_node = match kind {
        RedirectionKind::File => node.child_by_field_name("destination"),
        RedirectionKind::HereString => {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .find(|child| child.kind() != "file_descriptor")
        }
        RedirectionKind::HereDocument => None,
    };
    Redirection {
        kind,
        operator,
        span: source_span(node),
        raw: node_text(node, source).to_owned(),
        descriptor,
        target: target_node.map(|target| analyze_word(target, source)),
    }
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

fn has_unquoted_runtime_syntax(value: &str) -> bool {
    value.starts_with('~') || value.contains('\\') || contains_glob_syntax(value)
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
}
