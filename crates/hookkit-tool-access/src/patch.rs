use crate::structured::path_expression;
use crate::{
    AccessCandidate, AccessCertainty, AccessIntent, AccessProvenance, AccessScope, AccessSource,
    AccessTarget, PatchOperation, PathBase, PathExpression, ToolAccessGapReason, ToolAccessReport,
    ToolCallRef,
};
use hookkit_core::{HarnessId, Utf8Path};
use hookkit_shell::{BashAnalysis, RedirectionKind, SourceSpan};

const BEGIN_PATCH: &str = "*** Begin Patch";
const END_PATCH: &str = "*** End Patch";
const ENVIRONMENT_ID: &str = "*** Environment ID:";
const ADD_FILE: &str = "*** Add File: ";
const DELETE_FILE: &str = "*** Delete File: ";
const UPDATE_FILE: &str = "*** Update File: ";
const MOVE_TO: &str = "*** Move to: ";

/// Command names Codex intercepts and applies as patches.
const SHELL_PATCH_COMMANDS: [&str; 2] = ["apply_patch", "applypatch"];

#[derive(Debug, Clone, Copy)]
struct PatchContext<'a> {
    cwd: Option<&'a Utf8Path>,
    /// Label for paths resolved against `cwd`, or the reason they cannot be.
    base: PathBase,
    evidence: PatchEvidence<'a>,
    /// Whether header paths may contain unexpanded shell syntax.
    dynamic_body: bool,
}

impl PatchContext<'_> {
    fn source(&self) -> AccessSource {
        match self.evidence {
            PatchEvidence::Structured => AccessSource::Patch,
            PatchEvidence::Shell { .. } => AccessSource::Shell,
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
        },
        report,
    );
}

/// Analyzes every `apply_patch`/`applypatch` here-document in `source`.
///
/// Codex applies an intercepted here-document body verbatim, and in an
/// unquoted here-document only `$`, `` ` ``, and `\` are special, so header
/// paths free of those characters are recovered even when other lines of
/// the body are dynamic.
pub(crate) fn analyze_shell_patches(
    analysis: &BashAnalysis,
    source: &str,
    cwd: Option<&Utf8Path>,
    base: PathBase,
    report: &mut ToolAccessReport,
) {
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

    for (command_index, command) in analysis.commands.iter().enumerate() {
        let Some(name) = command
            .name
            .as_ref()
            .and_then(|name| name.literal.as_deref())
        else {
            continue;
        };
        if !is_shell_patch_command(name) {
            continue;
        }
        let heredocs = command
            .redirections
            .iter()
            .filter(|redirection| redirection.kind == RedirectionKind::HereDocument)
            .collect::<Vec<_>>();
        if heredocs.is_empty() {
            report.push_gap(
                AccessSource::Shell,
                ToolAccessGapReason::MissingShellPatchHereDocument {
                    command_span: command.span,
                },
            );
            continue;
        }
        let cwd_may_have_changed = directory_changes
            .iter()
            .any(|offset| *offset < command.span.start_byte);
        for redirection in heredocs {
            let Some(heredoc) = &redirection.here_document else {
                report.push_gap(
                    AccessSource::Shell,
                    ToolAccessGapReason::MissingShellPatchHereDocument {
                        command_span: command.span,
                    },
                );
                continue;
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
                Some(payload) => (payload, false),
                None => match heredoc
                    .delimiter
                    .literal
                    .as_ref()
                    .and(heredoc.body_span)
                    .and_then(|span| source.get(span.start_byte..span.end_byte))
                    .filter(|body| !has_line_continuation(body))
                {
                    Some(body) => (body, true),
                    None => {
                        report.push_gap(AccessSource::Shell, dynamic_gap());
                        continue;
                    }
                },
            };
            if cwd_may_have_changed {
                report.push_gap(
                    AccessSource::Shell,
                    ToolAccessGapReason::ShellPatchWorkingDirectoryMayHaveChanged {
                        command_span: command.span,
                    },
                );
            }
            let parsed = parse_patch(
                payload,
                "<shell-heredoc>",
                PatchContext {
                    cwd,
                    base: if cwd_may_have_changed {
                        PathBase::UnknownAfterDirectoryChange
                    } else {
                        base
                    },
                    evidence: PatchEvidence::Shell {
                        command_index,
                        command_span: command.span,
                        heredoc_span: heredoc.body_span.unwrap_or(redirection.span),
                        delimiter,
                    },
                    dynamic_body,
                },
                report,
            );
            if parsed.dynamic_headers {
                report.push_gap(AccessSource::Shell, dynamic_gap());
            }
        }
    }
}

fn is_shell_patch_command(name: &str) -> bool {
    let basename = name.rsplit('/').next().unwrap_or(name);
    SHELL_PATCH_COMMANDS.contains(&basename)
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
            certainty: AccessCertainty::Direct,
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
