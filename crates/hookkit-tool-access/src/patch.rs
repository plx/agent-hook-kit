use crate::structured::path_expression;
use crate::{
    AccessCandidate, AccessCertainty, AccessIntent, AccessProvenance, AccessScope, AccessSource,
    AccessTarget, PatchOperation, PathBase, PathExpression, ToolAccessGapReason, ToolAccessReport,
    ToolCallRef,
};
use hookkit_core::{HarnessId, Utf8Path, Utf8PathBuf, normalize_utf8_path};
use hookkit_shell::{
    BashAnalysis, CommandOccurrence, ConstructKind, ExecutionContext, Redirection, RedirectionKind,
    RedirectionOperator, ShellWord, SourceSpan,
};
use std::borrow::Cow;
use std::ops::Range;

const BEGIN_PATCH: &str = "*** Begin Patch";
const END_PATCH: &str = "*** End Patch";
const ENVIRONMENT_ID: &str = "*** Environment ID:";
const ADD_FILE: &str = "*** Add File: ";
const DELETE_FILE: &str = "*** Delete File: ";
const UPDATE_FILE: &str = "*** Update File: ";
const MOVE_TO: &str = "*** Move to: ";

/// Command names Codex intercepts and applies as patches.
const SHELL_PATCH_COMMANDS: [&str; 2] = ["apply_patch", "applypatch"];

/// Commands that change the shell's working directory.
const DIRECTORY_COMMANDS: [&str; 3] = ["cd", "pushd", "popd"];

/// Commands that run another command with the caller's standard input,
/// matching the wrappers `hookkit-shell` file-access inference unwraps.
const COMMAND_WRAPPERS: [&str; 11] = [
    "builtin", "command", "doas", "env", "exec", "nice", "nohup", "stdbuf", "sudo", "time",
    "timeout",
];

/// Location reported for a relative path recovered from a patch argument.
const SHELL_ARGUMENT_POINTER: &str = "<shell-argument>";

#[derive(Debug, Clone, Copy)]
struct PatchContext<'a> {
    cwd: Option<&'a Utf8Path>,
    /// Label for paths resolved against `cwd`, or the reason they cannot be.
    base: PathBase,
    evidence: PatchEvidence<'a>,
    /// Whether header paths may contain unexpanded shell syntax.
    dynamic_body: bool,
    /// Certainty recorded on every recovered path.
    certainty: AccessCertainty,
}

impl PatchContext<'_> {
    fn source(&self) -> AccessSource {
        match self.evidence {
            PatchEvidence::Structured => AccessSource::Patch,
            PatchEvidence::Shell { .. } | PatchEvidence::ShellArgument { .. } => {
                AccessSource::Shell
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum PatchEvidence<'a> {
    Structured,
    Shell {
        command_index: usize,
        command_span: SourceSpan,
        heredoc_span: SourceSpan,
        delimiter: &'a str,
    },
    ShellArgument {
        command_index: usize,
        command_span: SourceSpan,
        argument_span: SourceSpan,
    },
}

pub(crate) fn is_patch_tool(tool_name: &str) -> bool {
    let name = tool_name.to_ascii_lowercase();
    name == "apply_patch" || name.ends_with(".apply_patch")
}

pub(crate) fn analyze_patch(call: &ToolCallRef<'_>, report: &mut ToolAccessReport) {
    let Some((payload_pointer, payload)) = patch_payload(call, report) else {
        return;
    };
    parse_patch(
        payload,
        payload_pointer,
        PatchContext {
            cwd: call.cwd,
            base: call.cwd_base,
            evidence: PatchEvidence::Structured,
            dynamic_body: false,
            certainty: AccessCertainty::Direct,
        },
        report,
    );
}

/// Analyzes every shell `apply_patch`/`applypatch` command in `source`,
/// including one run through a wrapper such as `command`, `env`, `nohup`,
/// `sudo`, or `timeout`.
///
/// The patch text is the command's single argument, which the standalone
/// `apply_patch` executable reads instead of standard input. Without an
/// argument it is the command's last standard-input redirection or, when the
/// command has none and does not read a pipe, the last one of the innermost
/// enclosing statement that redirects standard input, as in
/// `(cd dir && apply_patch) <<'EOF'` or `{ apply_patch; } <<'EOF'`. Only a
/// here-document there is analyzed; anything else (a file, a pipe, a
/// here-string, a closed input, or a dynamic or extra argument) records
/// [`ToolAccessGapReason::MissingShellPatchHereDocument`]. A statement's
/// here-document is only [`AccessCertainty::Heuristic`] evidence when another
/// command of the statement (other than `cd`) runs first and may consume the
/// input.
///
/// With `codex_intercepts`, a script that is exactly one of the forms Codex
/// intercepts (see [`codex_intercepted`]) is applied by Codex from the raw
/// here-document body, so header paths free of `$`, `` ` ``, and `\` are
/// exact even when other lines of an unquoted body are dynamic. Otherwise the
/// shell expands a dynamic body before `apply_patch` reads it, and an
/// expansion can add file headers, so literal headers are still recovered but
/// [`ToolAccessGapReason::DynamicShellPatchHereDocument`] is always recorded.
///
/// Paths resolve against `cwd` unless a directory change may take effect
/// first (see [`patch_directory`]).
pub(crate) fn analyze_shell_patches(
    analysis: &BashAnalysis,
    source: &str,
    cwd: Option<&Utf8Path>,
    base: PathBase,
    codex_intercepts: bool,
    report: &mut ToolAccessReport,
) {
    let directory_changes = directory_changes(analysis);

    for (command_index, command) in analysis.commands.iter().enumerate() {
        let Some(offset) = wrapped_command_index(command, &SHELL_PATCH_COMMANDS) else {
            continue;
        };
        let intercepted = codex_intercepts && codex_intercepted(analysis, source, command_index);
        let directory = if offset > 0 && wrapper_changes_directory(&command.arguments[..offset - 1])
        {
            PatchDirectory::Unknown
        } else {
            patch_directory(
                analysis,
                source,
                command_index,
                &directory_changes,
                intercepted,
                cwd,
                base,
            )
        };
        let (patch_cwd, patch_base, certainty) = match &directory {
            PatchDirectory::Unchanged => (cwd, base, AccessCertainty::Direct),
            PatchDirectory::Changed { cwd, base } => {
                (cwd.as_deref(), *base, AccessCertainty::Direct)
            }
            PatchDirectory::Dynamic => (
                None,
                PathBase::UnknownAfterDirectoryChange,
                AccessCertainty::Heuristic,
            ),
            PatchDirectory::Unknown => (
                None,
                PathBase::UnknownAfterDirectoryChange,
                AccessCertainty::Direct,
            ),
        };
        let cwd_may_have_changed =
            matches!(directory, PatchDirectory::Dynamic | PatchDirectory::Unknown);
        let missing_gap = || ToolAccessGapReason::MissingShellPatchHereDocument {
            command_span: command.span,
        };
        let directory_gap = || ToolAccessGapReason::ShellPatchWorkingDirectoryMayHaveChanged {
            command_span: command.span,
        };

        let (redirection, shared) =
            match patch_input(analysis, source, command_index, offset, intercepted) {
                PatchInput::HereDocument {
                    redirection,
                    shared,
                } => (redirection, shared),
                PatchInput::Argument { argument, patch } => {
                    if cwd_may_have_changed {
                        report.push_gap(AccessSource::Shell, directory_gap());
                    }
                    parse_patch(
                        patch,
                        SHELL_ARGUMENT_POINTER,
                        PatchContext {
                            cwd: patch_cwd,
                            base: patch_base,
                            evidence: PatchEvidence::ShellArgument {
                                command_index,
                                command_span: command.span,
                                argument_span: argument.span,
                            },
                            dynamic_body: false,
                            certainty,
                        },
                        report,
                    );
                    continue;
                }
                PatchInput::Missing => {
                    report.push_gap(AccessSource::Shell, missing_gap());
                    continue;
                }
            };
        let Some(heredoc) = &redirection.here_document else {
            report.push_gap(AccessSource::Shell, missing_gap());
            continue;
        };
        let certainty = if shared {
            AccessCertainty::Heuristic
        } else {
            certainty
        };
        let delimiter = heredoc
            .delimiter
            .literal
            .as_deref()
            .unwrap_or(heredoc.delimiter.raw.as_str());
        let dynamic_gap = || ToolAccessGapReason::DynamicShellPatchHereDocument {
            command_span: command.span,
            delimiter: delimiter.to_owned(),
            reasons: heredoc.dynamic_reasons.clone(),
        };
        let (payload, dynamic_body) = match heredoc.literal_body.as_deref() {
            Some(payload) => (Cow::Borrowed(payload), false),
            None => match heredoc
                .delimiter
                .literal
                .as_ref()
                .and(heredoc.body_span)
                .and_then(|span| source.get(span.start_byte..span.end_byte))
                .filter(|body| !has_line_continuation(body))
            {
                // Codex applies the raw body; a shell strips `<<-` tabs.
                Some(body)
                    if !intercepted
                        && redirection.operator
                            == Some(RedirectionOperator::HereDocumentStripTabs) =>
                {
                    (Cow::Owned(strip_leading_tabs(body)), true)
                }
                Some(body) => (Cow::Borrowed(body), true),
                None => {
                    report.push_gap(AccessSource::Shell, dynamic_gap());
                    continue;
                }
            },
        };
        if cwd_may_have_changed {
            report.push_gap(AccessSource::Shell, directory_gap());
        }
        let parsed = parse_patch(
            &payload,
            "<shell-heredoc>",
            PatchContext {
                cwd: patch_cwd,
                base: patch_base,
                evidence: PatchEvidence::Shell {
                    command_index,
                    command_span: command.span,
                    heredoc_span: heredoc.body_span.unwrap_or(redirection.span),
                    delimiter,
                },
                dynamic_body,
                certainty,
            },
            report,
        );
        // Outside Codex's interception the shell expands the body, and an
        // expansion may produce headers that no literal line shows.
        if parsed.dynamic_headers || (dynamic_body && !intercepted) {
            report.push_gap(AccessSource::Shell, dynamic_gap());
        }
    }
}

fn literal_name(command: &CommandOccurrence) -> Option<&str> {
    command
        .name
        .as_ref()
        .and_then(|name| name.literal.as_deref())
}

fn basename(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

/// Returns the argv index of the command that `command` runs when its
/// basename is one of `names`: `0` for the command itself, or the index of
/// the command run by a wrapper chain such as `sudo env FOO=1 apply_patch`.
///
/// Wrapper options are not parsed. After a wrapper, the first literal word
/// whose basename is in `names` is taken as the wrapped command, so an option
/// value that happens to equal a name over-reports rather than hiding the
/// command. `command -v` and `command -V` run nothing.
fn wrapped_command_index(command: &CommandOccurrence, names: &[&str]) -> Option<usize> {
    let name = command.name.as_ref()?;
    let mut wrapper: Option<&str> = None;
    for (index, word) in std::iter::once(name).chain(&command.arguments).enumerate() {
        let Some(value) = word.literal.as_deref() else {
            // A dynamic command name is `hookkit-shell`'s gap; a dynamic
            // wrapper argument may be an option.
            wrapper?;
            continue;
        };
        if wrapper.is_some() && value.contains('=') {
            // An environment assignment such as `PATH=/opt/apply_patch`.
            continue;
        }
        let name = basename(value);
        if names.contains(&name) {
            return Some(index);
        }
        match wrapper {
            None if COMMAND_WRAPPERS.contains(&name) => wrapper = Some(name),
            None => return None,
            Some("command")
                if value.starts_with('-')
                    && !value.starts_with("--")
                    && value.contains(['v', 'V']) =>
            {
                return None;
            }
            Some(_) if !value.starts_with('-') && COMMAND_WRAPPERS.contains(&name) => {
                wrapper = Some(name);
            }
            Some(_) => {}
        }
    }
    None
}

/// Whether a wrapper option may run the wrapped command in another directory,
/// such as `env -C dir` or `sudo --chdir=dir`.
fn wrapper_changes_directory(words: &[ShellWord]) -> bool {
    words.iter().any(|word| {
        word.literal.as_deref().is_some_and(|value| {
            value.starts_with("--chdir")
                || (value.starts_with('-')
                    && !value.starts_with("--")
                    && value.contains(['C', 'D']))
        })
    })
}

/// Where a shell patch command reads its patch text.
enum PatchInput<'a> {
    /// A here-document supplies standard input.
    HereDocument {
        redirection: &'a Redirection,
        /// The here-document belongs to an enclosing statement in which
        /// another command runs first and may consume the input.
        shared: bool,
    },
    /// The command's single literal argument is the patch.
    Argument {
        argument: &'a ShellWord,
        patch: &'a str,
    },
    /// The patch text is not observable.
    Missing,
}

/// Finds the patch text of the patch command at argv index `offset` of
/// `command_index`.
///
/// The standalone `apply_patch` executable reads its single argument when it
/// has one, so arguments take precedence over standard input, except in a
/// Codex-intercepted script, where Codex reads the here-document. Standard
/// input is the command's last standard-input redirection or, when it has
/// none and does not read a pipe, the last one of the innermost enclosing
/// statement that redirects standard input.
fn patch_input<'a>(
    analysis: &'a BashAnalysis,
    source: &str,
    command_index: usize,
    offset: usize,
    intercepted: bool,
) -> PatchInput<'a> {
    let command = &analysis.commands[command_index];
    let arguments = &command.arguments[offset..];
    if !intercepted && !arguments.is_empty() {
        return match arguments {
            [argument] => match argument.literal.as_deref() {
                Some(patch) => PatchInput::Argument { argument, patch },
                None => PatchInput::Missing,
            },
            // The executable refuses extra arguments.
            _ => PatchInput::Missing,
        };
    }
    if let Some(input) = last_standard_input(source, &command.redirections) {
        return here_document_input(input, false);
    }
    if command.context.contains(&ExecutionContext::PipelineInput) {
        return PatchInput::Missing;
    }
    // Nested statements may enclose the same commands, as in
    // `{ (apply_patch) < evil.patch; } <<'EOF'`, so the innermost one is the
    // one with the narrowest body.
    let Some((statement, input)) = analysis
        .statement_redirections
        .iter()
        .filter(|statement| statement.commands.contains(&command_index))
        .filter_map(|statement| {
            last_standard_input(source, &statement.redirections).map(|input| (statement, input))
        })
        .min_by_key(|(statement, _)| {
            statement
                .body_span
                .end_byte
                .saturating_sub(statement.body_span.start_byte)
        })
    else {
        return PatchInput::Missing;
    };
    let shared = (statement.commands.start..command_index)
        .any(|index| literal_name(&analysis.commands[index]) != Some("cd"));
    here_document_input(input, shared)
}

fn here_document_input(redirection: &Redirection, shared: bool) -> PatchInput<'_> {
    if redirection.kind == RedirectionKind::HereDocument {
        PatchInput::HereDocument {
            redirection,
            shared,
        }
    } else {
        PatchInput::Missing
    }
}

/// The redirection that finally supplies standard input: the last one in
/// source order that replaces descriptor 0.
fn last_standard_input<'a>(
    source: &str,
    redirections: &'a [Redirection],
) -> Option<&'a Redirection> {
    redirections
        .iter()
        .filter(|redirection| redirects_standard_input(source, redirection))
        .max_by_key(|redirection| redirection.span.start_byte)
}

/// Whether a redirection may replace standard input (descriptor 0): any
/// redirection of descriptor `0`, or an input, read-write, input-duplicating,
/// input-closing, or unrecognized one without a descriptor. Bash opens a
/// read-write `<>` without a descriptor on descriptor 0.
///
/// When `0<>` follows a here-document, the Bash grammar loses the `0<` and
/// reports an output `>` without a descriptor, although Bash opens the file
/// as standard input, so an output operator written directly after `<`
/// counts too.
fn redirects_standard_input(source: &str, redirection: &Redirection) -> bool {
    match redirection.descriptor.as_deref() {
        Some(descriptor) => descriptor == "0",
        None => {
            matches!(
                redirection.kind,
                RedirectionKind::HereDocument | RedirectionKind::HereString
            ) || matches!(
                redirection.operator,
                None | Some(
                    RedirectionOperator::Input
                        | RedirectionOperator::ReadWrite
                        | RedirectionOperator::DuplicateInput
                        | RedirectionOperator::CloseInput
                )
            ) || (redirection.operator == Some(RedirectionOperator::Output)
                && source
                    .get(..redirection.span.start_byte)
                    .is_some_and(|before| before.ends_with('<')))
        }
    }
}

/// Whether `source` is exactly one of the scripts Codex intercepts and
/// applies itself instead of running (`codex-rs/apply-patch/src/invocation.rs`):
/// `apply_patch <<DELIM` without arguments, or `cd <dir> && apply_patch
/// <<DELIM` (whose `apply_patch` may have arguments, which Codex ignores), as
/// the only statement, with the here-document as the only redirection.
/// Anything before or after it, including a comment, makes the shell run the
/// script.
fn codex_intercepted(analysis: &BashAnalysis, source: &str, command_index: usize) -> bool {
    let command = &analysis.commands[command_index];
    let plain_name = |command: &CommandOccurrence, names: &[&str]| {
        command
            .name
            .as_ref()
            .is_some_and(|name| names.contains(&name.raw.as_str()))
    };
    if !plain_name(command, &SHELL_PATCH_COMMANDS)
        || analysis.commands.len() != command_index + 1
        || !analysis.statement_redirections.is_empty()
    {
        return false;
    }
    let [redirection] = command.redirections.as_slice() else {
        return false;
    };
    let Some((heredoc, delimiter, body)) = redirection
        .here_document
        .as_ref()
        .filter(|_| redirection.descriptor.is_none())
        .and_then(|heredoc| {
            Some((
                heredoc,
                heredoc.delimiter.literal.as_deref()?,
                heredoc.body_span?,
            ))
        })
    else {
        return false;
    };
    let first = match command_index {
        0 if command.arguments.is_empty() && command.context.is_empty() => command,
        1 => {
            let cd = &analysis.commands[0];
            let simple_operand = match cd.arguments.as_slice() {
                [operand] => operand.literal.as_deref().is_some_and(|literal| {
                    operand.raw == literal
                        || operand.raw == format!("'{literal}'")
                        || operand.raw == format!("\"{literal}\"")
                }),
                _ => false,
            };
            let and_list = [ExecutionContext::AndOrList];
            if !plain_name(cd, &["cd"])
                || !simple_operand
                || !cd.redirections.is_empty()
                || cd.context != and_list
                || command.context != and_list
                || !joined_by_and(source, cd, command)
            {
                return false;
            }
            cd
        }
        _ => return false,
    };
    let heredoc_start = source
        .get(command.span.end_byte..body.start_byte)
        .map(str::trim)
        .and_then(|start| start.strip_prefix("<<"))
        .map(|start| start.strip_prefix('-').unwrap_or(start).trim_start());
    source
        .get(..first.span.start_byte)
        .is_some_and(|before| before.trim().is_empty())
        && heredoc_start == Some(heredoc.delimiter.raw.as_str())
        && source
            .get(body.end_byte..)
            .is_some_and(|after| after.trim() == delimiter)
}

/// Working directory of a shell patch command relative to the invocation.
#[derive(Debug)]
enum PatchDirectory {
    /// No directory change precedes the command.
    Unchanged,
    /// Only a `cd <literal> && ...` chain precedes the command; `cwd` is the
    /// directory it enters, when that can be resolved.
    Changed {
        cwd: Option<Utf8PathBuf>,
        base: PathBase,
    },
    /// A `cd` chain with a non-literal operand, or one that `CDPATH` or
    /// `cdable_vars` may redirect, precedes the command.
    Dynamic,
    /// Some other directory change may precede the command.
    Unknown,
}

/// A command that may change the shell's working directory.
struct DirectoryChange {
    index: usize,
    /// Byte offset from which the change may affect later commands: the
    /// command's own start, or the start of the outermost loop or function
    /// body enclosing it, whose later iterations and calls run after it.
    from: usize,
    /// The change is inside a loop or function body.
    deferred: bool,
}

/// Finds every `cd`, `pushd`, and `popd`, including ones run through a
/// wrapper such as `builtin cd` or `command cd`.
fn directory_changes(analysis: &BashAnalysis) -> Vec<DirectoryChange> {
    let mut deferred_ranges = analysis
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
    deferred_ranges.sort_by_key(|range| (range.start, std::cmp::Reverse(range.end)));
    let mut outermost: Vec<Range<usize>> = Vec::with_capacity(deferred_ranges.len());
    for range in deferred_ranges {
        if outermost
            .last()
            .is_none_or(|outer| range.start >= outer.end)
        {
            outermost.push(range);
        }
    }

    analysis
        .commands
        .iter()
        .enumerate()
        .filter(|(_, command)| wrapped_command_index(command, &DIRECTORY_COMMANDS).is_some())
        .map(|(index, command)| {
            let start = command.span.start_byte;
            let deferred = command.context.iter().any(|context| {
                matches!(
                    context,
                    ExecutionContext::Loop | ExecutionContext::FunctionDefinition
                )
            });
            let from = if deferred {
                outermost
                    .iter()
                    .find(|range| range.contains(&start))
                    .map_or(start, |range| range.start)
            } else {
                start
            };
            DirectoryChange {
                index,
                from,
                deferred,
            }
        })
        .collect()
}

/// Resolves the working directory of the patch command at `command_index`.
///
/// The Codex-intercepted `cd <dir> && apply_patch` form, including a chain of
/// plain `cd` commands joined by `&&` inside a subshell or not, resolves
/// against `cwd` joined with each literal `cd` operand. The chain must begin
/// its list, so a negated `! cd dir` or `true || cd dir` (after which the
/// patch may run in the original directory) is not one. A non-literal
/// operand makes the directory [`PatchDirectory::Dynamic`], and so does any
/// mention of `CDPATH` or `cdable_vars` in a script that Bash runs rather
/// than Codex intercepting it (Codex joins the operand itself); a `CDPATH`
/// inherited from the environment is not modeled. Any other directory change that may
/// take effect first, including a wrapped `builtin cd`, one inside a loop or
/// function body, and any directory change at all when the patch command
/// itself is in a function body, makes it [`PatchDirectory::Unknown`].
fn patch_directory(
    analysis: &BashAnalysis,
    source: &str,
    command_index: usize,
    directory_changes: &[DirectoryChange],
    intercepted: bool,
    cwd: Option<&Utf8Path>,
    base: PathBase,
) -> PatchDirectory {
    let command = &analysis.commands[command_index];
    if command
        .context
        .contains(&ExecutionContext::FunctionDefinition)
        && !directory_changes.is_empty()
    {
        // The function may be called after any directory change.
        return PatchDirectory::Unknown;
    }
    let start = command.span.start_byte;
    let earlier = directory_changes
        .iter()
        .filter(|change| change.from < start)
        .collect::<Vec<_>>();
    if earlier.is_empty() {
        return PatchDirectory::Unchanged;
    }
    if earlier.iter().any(|change| change.deferred) {
        return PatchDirectory::Unknown;
    }
    // Walk back through the `cd <dir> &&` links immediately preceding the
    // command; every earlier directory change must be one of them.
    let mut chain = Vec::new();
    let mut next = command_index;
    while let Some(previous) = next.checked_sub(1) {
        let command = &analysis.commands[previous];
        if literal_name(command) != Some("cd")
            || !joined_by_and(source, command, &analysis.commands[next])
        {
            break;
        }
        chain.push(previous);
        next = previous;
    }
    if chain.is_empty()
        || earlier.iter().any(|change| !chain.contains(&change.index))
        || !begins_list(source, analysis.commands[next].span.start_byte)
    {
        return PatchDirectory::Unknown;
    }
    if !intercepted && may_redirect_cd(source) {
        return PatchDirectory::Dynamic;
    }
    let mut directory = cwd.map(Utf8Path::to_path_buf);
    let mut base = base;
    for index in chain.into_iter().rev() {
        let Some(operand) = literal_cd_operand(&analysis.commands[index]) else {
            return PatchDirectory::Dynamic;
        };
        let operand = Utf8Path::new(operand);
        if operand.is_absolute() {
            // An absolute operand is the directory the patch applies in,
            // however uncertain the invocation directory was.
            directory = Some(normalize_utf8_path(operand));
            base = PathBase::InvocationCwd;
        } else {
            directory = directory.map(|directory| normalize_utf8_path(directory.join(operand)));
        }
    }
    PatchDirectory::Changed {
        cwd: directory,
        base,
    }
}

/// Whether only `&&` (with whitespace or line continuations) separates two
/// commands, so `next` runs only after `previous` entered its directory.
fn joined_by_and(source: &str, previous: &CommandOccurrence, next: &CommandOccurrence) -> bool {
    source
        .get(previous.span.end_byte..next.span.start_byte)
        .is_some_and(|between| between.replace("\\\n", " ").trim() == "&&")
}

/// Whether the command starting at byte `start` runs whenever the rest of its
/// `&&` chain does: it starts a statement, follows an opening delimiter or
/// keyword, or follows `&&`. After `!`, `|`, or `||` it may be negated, run
/// in a pipeline subshell, or skipped.
fn begins_list(source: &str, start: usize) -> bool {
    let mut before = source.get(..start).unwrap_or_default();
    loop {
        let trimmed = before.trim_end_matches([' ', '\t']);
        match trimmed.strip_suffix("\\\n") {
            Some(joined) => before = joined,
            None => {
                before = trimmed;
                break;
            }
        }
    }
    if before.is_empty() || before.ends_with("&&") {
        return true;
    }
    if before.ends_with(['|', '!']) || before.ends_with("|&") {
        return false;
    }
    if before.ends_with(['\n', ';', '&', '(', '{', '`', ')']) {
        return true;
    }
    let keyword = before
        .rsplit(|character: char| character.is_ascii_whitespace() || character == ';')
        .next()
        .unwrap_or_default();
    matches!(
        keyword,
        "if" | "then" | "else" | "elif" | "do" | "while" | "until"
    )
}

/// Whether the script may set `CDPATH`, which makes `cd` search other
/// directories first, or enable `cdable_vars`, which makes an operand that
/// names no directory a variable holding one. Quotes and backslashes are
/// ignored, so `export CD"PATH"=...` also counts.
fn may_redirect_cd(source: &str) -> bool {
    let unquoted = source
        .chars()
        .filter(|character| !matches!(character, '"' | '\'' | '\\'))
        .collect::<String>();
    unquoted.contains("CDPATH") || unquoted.contains("cdable_vars")
}

/// The single literal directory operand of `cd`, excluding options, `cd -`,
/// and a bare `cd` (which enters `$HOME`).
fn literal_cd_operand(command: &CommandOccurrence) -> Option<&str> {
    let mut arguments = command.arguments.iter();
    let mut operand = arguments.next()?;
    if operand.literal.as_deref() == Some("--") {
        operand = arguments.next()?;
    }
    if arguments.next().is_some() {
        return None;
    }
    operand
        .literal
        .as_deref()
        .filter(|operand| !operand.is_empty() && !operand.starts_with('-'))
}

/// Removes the leading tabs a `<<-` here-document strips from every line.
fn strip_leading_tabs(body: &str) -> String {
    body.split_inclusive('\n')
        .map(|line| line.trim_start_matches('\t'))
        .collect()
}

/// Whether a line ends in an unescaped backslash, which an unquoted
/// here-document joins with the following line.
fn has_line_continuation(body: &str) -> bool {
    body.lines().any(|line| {
        let trailing = line.bytes().rev().take_while(|byte| *byte == b'\\').count();
        trailing % 2 == 1
    })
}

/// Selects the patch text. Codex sends `apply_patch` hook input as
/// `{"command": "<patch>"}`; `patch` and `input` remain accepted for custom
/// patch tools, and a bare string input is treated as the patch itself.
fn patch_payload<'a>(
    call: &ToolCallRef<'a>,
    report: &mut ToolAccessReport,
) -> Option<(&'static str, &'a str)> {
    const CODEX_KEYS: &[(&str, &str)] = &[
        ("command", "/command"),
        ("patch", "/patch"),
        ("input", "/input"),
    ];
    const CUSTOM_KEYS: &[(&str, &str)] = &[("patch", "/patch"), ("input", "/input")];
    let keys = if call.harness() == &HarnessId::CODEX {
        CODEX_KEYS
    } else {
        CUSTOM_KEYS
    };
    for (key, pointer) in keys {
        let Some(value) = call.tool_input.get(key) else {
            continue;
        };
        let Some(payload) = value.as_str() else {
            report.push_gap(
                AccessSource::Patch,
                ToolAccessGapReason::PatchPayloadNotString {
                    pointer: (*pointer).to_owned(),
                },
            );
            return None;
        };
        return Some((pointer, payload));
    }
    if let Some(payload) = call.tool_input.as_str() {
        return Some(("", payload));
    }
    report.push_gap(
        AccessSource::Patch,
        ToolAccessGapReason::MissingPatchPayload {
            tool_name: call.tool_name.to_owned(),
        },
    );
    None
}

#[derive(Debug)]
struct PendingHeader<'a> {
    raw: &'a str,
    line: usize,
    header: &'a str,
}

#[derive(Debug, Default)]
struct ParsedPatch {
    /// A header path contained shell syntax and was not recorded.
    dynamic_headers: bool,
}

struct Parser<'a, 'r> {
    payload_pointer: &'a str,
    context: PatchContext<'a>,
    environment_id: Option<&'a str>,
    report: &'r mut ToolAccessReport,
    parsed: ParsedPatch,
}

fn parse_patch(
    payload: &str,
    payload_pointer: &str,
    context: PatchContext<'_>,
    report: &mut ToolAccessReport,
) -> ParsedPatch {
    let initial_candidates = report.candidates.len();
    let initial_gaps = report.gaps.len();
    let lines = payload
        .lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line))
        .collect::<Vec<_>>();
    let mut parser = Parser {
        payload_pointer,
        context,
        environment_id: None,
        report,
        parsed: ParsedPatch::default(),
    };
    let first = lines
        .iter()
        .position(|(_, line)| !line.trim().is_empty())
        .unwrap_or(lines.len());
    let first_line = lines.get(first).map_or("", |(_, line)| line.trim());
    if is_lenient_heredoc_start(first_line) {
        // Codex's lenient mode unwraps a literal `<<'EOF'` ... `EOF` wrapper.
        let mut body = &lines[first + 1..];
        if let Some(last) = body
            .iter()
            .rposition(|(_, line)| !line.trim().is_empty())
            .filter(|last| body[*last].1.trim_end().ends_with("EOF"))
        {
            body = &body[..last];
        }
        parser.codex(body);
    } else if first_line.starts_with("*** ") {
        parser.codex(&lines[first..]);
    } else {
        parser.unified(&lines[first..]);
    }

    let parsed = std::mem::take(&mut parser.parsed);
    if report.candidates.len() == initial_candidates
        && report.gaps.len() == initial_gaps
        && !parsed.dynamic_headers
    {
        malformed(report, context.source(), None, "no recognized file header");
    }
    parsed
}

fn is_lenient_heredoc_start(line: &str) -> bool {
    matches!(line, "<<EOF" | "<<'EOF'" | "<<\"EOF\"")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexMode {
    Started,
    Add,
    Delete,
    Update { has_changes: bool },
    Ended,
}

impl<'a> Parser<'a, '_> {
    /// Follows Codex's `apply_patch` grammar: file headers are recognized on
    /// trimmed lines, except inside an update hunk, where a header must start
    /// the line and every ` `/`+`/`-` line is content. A missing
    /// `*** Begin Patch` is tolerated so partial evidence is retained.
    fn codex(&mut self, lines: &[(usize, &'a str)]) {
        let mut mode = CodexMode::Started;
        let mut pending_update: Option<PendingHeader<'a>> = None;
        for &(line_number, line) in lines {
            let header_text = match mode {
                CodexMode::Update { .. } => line.trim_end(),
                _ => line.trim(),
            };
            if mode == CodexMode::Ended {
                continue;
            }
            if header_text == BEGIN_PATCH && mode == CodexMode::Started {
                continue;
            }
            if header_text == END_PATCH {
                self.flush_update(pending_update.take());
                mode = CodexMode::Ended;
                continue;
            }
            if let Some(environment_id) = header_text
                .strip_prefix(ENVIRONMENT_ID)
                .filter(|_| mode == CodexMode::Started)
            {
                let environment_id = environment_id.trim();
                if environment_id.is_empty() {
                    self.malformed(Some(line_number), "environment ID is empty");
                } else {
                    self.environment_id = Some(environment_id);
                    self.report.push_gap(
                        self.context.source(),
                        ToolAccessGapReason::UnknownExecutionEnvironment {
                            environment_id: environment_id.to_owned(),
                        },
                    );
                }
                continue;
            }
            if let Some(raw) = header_text.strip_prefix(ADD_FILE) {
                self.flush_update(pending_update.take());
                if let Some(header) = self.header(raw, line_number, "*** Add File") {
                    self.emit(&header, PatchOperation::Add, AccessIntent::Modify);
                }
                mode = CodexMode::Add;
            } else if let Some(raw) = header_text.strip_prefix(DELETE_FILE) {
                self.flush_update(pending_update.take());
                if let Some(header) = self.header(raw, line_number, "*** Delete File") {
                    self.emit(&header, PatchOperation::Delete, AccessIntent::Delete);
                }
                mode = CodexMode::Delete;
            } else if let Some(raw) = header_text.strip_prefix(UPDATE_FILE) {
                self.flush_update(pending_update.take());
                pending_update = self.header(raw, line_number, "*** Update File");
                mode = CodexMode::Update { has_changes: false };
            } else if let Some(raw) = header_text.strip_prefix(MOVE_TO) {
                let destination = self.header(raw, line_number, "*** Move to");
                let source = match mode {
                    CodexMode::Update { has_changes: false } => pending_update.take(),
                    _ => None,
                };
                match (source, destination) {
                    (Some(source), Some(destination)) => {
                        self.emit(
                            &source,
                            PatchOperation::MoveSource,
                            AccessIntent::MoveSource,
                        );
                        self.emit(
                            &destination,
                            PatchOperation::MoveDestination,
                            AccessIntent::MoveDestination,
                        );
                    }
                    (None, Some(destination)) => {
                        self.emit(
                            &destination,
                            PatchOperation::MoveDestination,
                            AccessIntent::MoveDestination,
                        );
                        self.malformed(
                            Some(line_number),
                            "move destination has no preceding update source",
                        );
                    }
                    (Some(source), None) => {
                        self.emit(
                            &source,
                            PatchOperation::MoveSource,
                            AccessIntent::MoveSource,
                        );
                    }
                    (None, None) => {}
                }
            } else if let CodexMode::Update { has_changes } = &mut mode {
                // Context, change, and `@@` lines are hunk content.
                *has_changes |= !header_text.is_empty();
            }
        }
        self.flush_update(pending_update);
    }

    /// Parses unified-diff `---`/`+++` headers, honoring `@@ -a,b +c,d @@`
    /// line counts so removed or added lines that begin with `-- ` or `++ `
    /// are never mistaken for headers. A hunk header without counts is
    /// followed loosely: a `---` line is a header only when a `+++` line
    /// immediately follows it.
    fn unified(&mut self, lines: &[(usize, &'a str)]) {
        let mut pending_old: Option<PendingHeader<'a>> = None;
        let mut remaining: Option<(usize, usize)> = None;
        let mut loose_hunk = false;
        for (index, &(line_number, line)) in lines.iter().enumerate() {
            if let Some((old, new)) = remaining.as_mut() {
                match line.as_bytes().first() {
                    Some(b'-') => *old = old.saturating_sub(1),
                    Some(b'+') => *new = new.saturating_sub(1),
                    Some(b'\\') => {}
                    _ => {
                        *old = old.saturating_sub(1);
                        *new = new.saturating_sub(1);
                    }
                }
                if *old == 0 && *new == 0 {
                    remaining = None;
                }
                continue;
            }
            if line.starts_with("@@") {
                match hunk_counts(line) {
                    Some((0, 0)) => {}
                    Some(counts) => {
                        remaining = Some(counts);
                        loose_hunk = false;
                    }
                    None => loose_hunk = true,
                }
                continue;
            }
            if let Some(raw) = line.strip_prefix("--- ") {
                let pairs_with_next = lines
                    .get(index + 1)
                    .is_some_and(|(_, next)| next.starts_with("+++ "));
                if loose_hunk && !pairs_with_next {
                    continue;
                }
                loose_hunk = false;
                self.flush_old(pending_old.take());
                pending_old = Some(PendingHeader {
                    raw: unified_path(raw),
                    line: line_number,
                    header: "---",
                });
            } else if let Some(raw) = line.strip_prefix("+++ ") {
                if loose_hunk && pending_old.is_none() {
                    continue;
                }
                let new = PendingHeader {
                    raw: unified_path(raw),
                    line: line_number,
                    header: "+++",
                };
                self.emit_unified_pair(pending_old.take(), new);
            } else if loose_hunk && !matches!(line.as_bytes().first(), Some(b' ' | b'+' | b'-')) {
                loose_hunk = false;
            }
        }
        self.flush_old(pending_old);
    }

    fn header(
        &mut self,
        raw: &'a str,
        line: usize,
        name: &'static str,
    ) -> Option<PendingHeader<'a>> {
        let raw = raw.trim();
        if raw.is_empty() {
            self.malformed(Some(line), "file header has an empty path");
            None
        } else {
            Some(PendingHeader {
                raw,
                line,
                header: name,
            })
        }
    }

    fn flush_update(&mut self, pending: Option<PendingHeader<'_>>) {
        if let Some(update) = pending {
            self.emit(&update, PatchOperation::Update, AccessIntent::ReadModify);
        }
    }

    fn flush_old(&mut self, pending: Option<PendingHeader<'_>>) {
        if let Some(old) = pending {
            if old.raw != "/dev/null" {
                self.emit(&old, PatchOperation::UnifiedOld, AccessIntent::Unclassified);
            }
            self.malformed(
                Some(old.line),
                "unified old-path header has no matching new-path header",
            );
        }
    }

    fn emit_unified_pair(&mut self, old: Option<PendingHeader<'_>>, new: PendingHeader<'_>) {
        let Some(old) = old else {
            if new.raw != "/dev/null" {
                self.emit(&new, PatchOperation::UnifiedNew, AccessIntent::Modify);
            }
            self.malformed(
                Some(new.line),
                "unified new-path header has no matching old-path header",
            );
            return;
        };

        match (old.raw, new.raw) {
            ("/dev/null", "/dev/null") => {
                self.malformed(Some(new.line), "both unified paths are /dev/null")
            }
            ("/dev/null", _) => self.emit(&new, PatchOperation::Add, AccessIntent::Modify),
            (_, "/dev/null") => self.emit(&old, PatchOperation::Delete, AccessIntent::Delete),
            (old_path, new_path) if old_path == new_path => {
                self.emit(&new, PatchOperation::Update, AccessIntent::ReadModify)
            }
            _ => {
                self.emit(&old, PatchOperation::MoveSource, AccessIntent::MoveSource);
                self.emit(
                    &new,
                    PatchOperation::MoveDestination,
                    AccessIntent::MoveDestination,
                );
            }
        }
    }

    fn emit(
        &mut self,
        header: &PendingHeader<'_>,
        operation: PatchOperation,
        intent: AccessIntent,
    ) {
        let raw = header.raw.trim();
        if raw.is_empty() || raw == "/dev/null" {
            return;
        }
        if self.context.dynamic_body && raw.contains(['$', '`', '\\']) {
            self.parsed.dynamic_headers = true;
            return;
        }
        let expression = match self.environment_id {
            Some(_) => PathExpression {
                raw: raw.to_owned(),
                resolved: None,
                base: PathBase::UnknownEnvironment,
            },
            None => path_expression(raw, self.context.cwd, self.context.base),
        };
        let missing_cwd = expression.base == PathBase::MissingWorkingDirectory;
        self.report.candidates.push(AccessCandidate {
            target: AccessTarget::Path {
                expression,
                scope: AccessScope::Exact,
            },
            intent,
            certainty: self.context.certainty,
            provenance: match self.context.evidence {
                PatchEvidence::Structured => AccessProvenance::Patch {
                    payload_pointer: self.payload_pointer.to_owned(),
                    operation,
                    header: header.header.to_owned(),
                    line: header.line,
                },
                PatchEvidence::Shell {
                    command_index,
                    command_span,
                    heredoc_span,
                    delimiter,
                } => AccessProvenance::ShellPatch {
                    command_index,
                    command_span,
                    heredoc_span,
                    delimiter: delimiter.to_owned(),
                    operation,
                    header: header.header.to_owned(),
                    line: header.line,
                },
                PatchEvidence::ShellArgument {
                    command_index,
                    command_span,
                    argument_span,
                } => AccessProvenance::ShellPatchArgument {
                    command_index,
                    command_span,
                    argument_span,
                    operation,
                    header: header.header.to_owned(),
                    line: header.line,
                },
            },
        });
        if missing_cwd {
            self.report.push_gap(
                self.context.source(),
                ToolAccessGapReason::MissingWorkingDirectory {
                    raw: raw.to_owned(),
                    pointer: Some(self.payload_pointer.to_owned()),
                },
            );
        }
    }

    fn malformed(&mut self, line: Option<usize>, detail: &str) {
        malformed(self.report, self.context.source(), line, detail);
    }
}

fn malformed(
    report: &mut ToolAccessReport,
    source: AccessSource,
    line: Option<usize>,
    detail: impl Into<String>,
) {
    report.push_gap(
        source,
        ToolAccessGapReason::MalformedPatch {
            line,
            detail: detail.into(),
        },
    );
}

/// Parses the old and new line counts from `@@ -a[,b] +c[,d] @@`.
fn hunk_counts(line: &str) -> Option<(usize, usize)> {
    let mut ranges = line.strip_prefix("@@ ")?.split_whitespace();
    let old = ranges.next()?.strip_prefix('-')?;
    let new = ranges.next()?.strip_prefix('+')?;
    (ranges.next()? == "@@").then_some(())?;
    let count = |range: &str| match range.split_once(',') {
        Some((start, count)) => start.parse::<usize>().ok().and(count.parse().ok()),
        None => range.parse::<usize>().ok().map(|_| 1),
    };
    Some((count(old)?, count(new)?))
}

fn unified_path(raw: &str) -> &str {
    let raw = raw.trim();
    let raw = raw.split('\t').next().unwrap_or(raw);
    raw.strip_prefix("a/")
        .or_else(|| raw.strip_prefix("b/"))
        .unwrap_or(raw)
}
