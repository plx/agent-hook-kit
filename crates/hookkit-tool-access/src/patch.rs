use crate::structured::path_expression;
use crate::{
    AccessCandidate, AccessCertainty, AccessIntent, AccessProvenance, AccessScope, AccessSource,
    AccessTarget, JsonRef, PatchOperation, ToolAccessGapReason, ToolAccessReport, ToolCallRef,
};

pub(crate) fn is_patch_tool(tool_name: &str) -> bool {
    let name = tool_name.to_ascii_lowercase();
    name == "apply_patch" || name.ends_with(".apply_patch")
}

pub(crate) fn analyze_patch(call: &ToolCallRef<'_>, report: &mut ToolAccessReport) {
    let Some((payload_pointer, payload)) = patch_payload(call.tool_input, report, call.tool_name)
    else {
        return;
    };
    parse_patch(payload, payload_pointer, call, report);
}

fn patch_payload<'a>(
    input: JsonRef<'a>,
    report: &mut ToolAccessReport,
    tool_name: &str,
) -> Option<(&'static str, &'a str)> {
    for (key, pointer) in [("patch", "/patch"), ("input", "/input")] {
        let Some(value) = input.get(key) else {
            continue;
        };
        let Some(payload) = value.as_str() else {
            report.push_gap(
                AccessSource::Patch,
                ToolAccessGapReason::PatchPayloadNotString {
                    pointer: pointer.to_owned(),
                },
            );
            return None;
        };
        return Some((pointer, payload));
    }
    report.push_gap(
        AccessSource::Patch,
        ToolAccessGapReason::MissingPatchPayload {
            tool_name: tool_name.to_owned(),
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

fn parse_patch(
    payload: &str,
    payload_pointer: &str,
    call: &ToolCallRef<'_>,
    report: &mut ToolAccessReport,
) {
    let initial_candidates = report.candidates.len();
    let mut pending_update: Option<PendingHeader<'_>> = None;
    let mut pending_old: Option<PendingHeader<'_>> = None;

    for (index, line) in payload.lines().enumerate() {
        let line_number = index + 1;
        if let Some(raw) = line.strip_prefix("*** Update File: ") {
            flush_update(pending_update.take(), payload_pointer, call, report);
            pending_update = header(raw, line_number, "*** Update File", report);
        } else if let Some(raw) = line.strip_prefix("*** Move to: ") {
            let destination = header(raw, line_number, "*** Move to", report);
            match (pending_update.take(), destination) {
                (Some(source), Some(destination)) => {
                    emit(
                        &source,
                        payload_pointer,
                        call,
                        PatchOperation::MoveSource,
                        AccessIntent::MoveSource,
                        report,
                    );
                    emit(
                        &destination,
                        payload_pointer,
                        call,
                        PatchOperation::MoveDestination,
                        AccessIntent::MoveDestination,
                        report,
                    );
                }
                (None, Some(destination)) => {
                    emit(
                        &destination,
                        payload_pointer,
                        call,
                        PatchOperation::MoveDestination,
                        AccessIntent::MoveDestination,
                        report,
                    );
                    malformed(
                        report,
                        Some(line_number),
                        "move destination has no preceding update source",
                    );
                }
                (Some(source), None) => {
                    emit(
                        &source,
                        payload_pointer,
                        call,
                        PatchOperation::MoveSource,
                        AccessIntent::MoveSource,
                        report,
                    );
                }
                (None, None) => {}
            }
        } else if let Some(raw) = line.strip_prefix("*** Add File: ") {
            flush_update(pending_update.take(), payload_pointer, call, report);
            if let Some(header) = header(raw, line_number, "*** Add File", report) {
                emit(
                    &header,
                    payload_pointer,
                    call,
                    PatchOperation::Add,
                    AccessIntent::Modify,
                    report,
                );
            }
        } else if let Some(raw) = line.strip_prefix("*** Delete File: ") {
            flush_update(pending_update.take(), payload_pointer, call, report);
            if let Some(header) = header(raw, line_number, "*** Delete File", report) {
                emit(
                    &header,
                    payload_pointer,
                    call,
                    PatchOperation::Delete,
                    AccessIntent::Delete,
                    report,
                );
            }
        } else if let Some(raw) = line.strip_prefix("--- ") {
            flush_old(pending_old.take(), payload_pointer, call, report);
            pending_old = Some(PendingHeader {
                raw: unified_path(raw),
                line: line_number,
                header: "---",
            });
        } else if let Some(raw) = line.strip_prefix("+++ ") {
            let new = PendingHeader {
                raw: unified_path(raw),
                line: line_number,
                header: "+++",
            };
            emit_unified_pair(pending_old.take(), new, payload_pointer, call, report);
        }
    }

    flush_update(pending_update, payload_pointer, call, report);
    flush_old(pending_old, payload_pointer, call, report);
    if report.candidates.len() == initial_candidates && report.gaps.is_empty() {
        malformed(report, None, "no recognized file header");
    }
}

fn header<'a>(
    raw: &'a str,
    line: usize,
    name: &'a str,
    report: &mut ToolAccessReport,
) -> Option<PendingHeader<'a>> {
    let raw = raw.trim();
    if raw.is_empty() {
        malformed(report, Some(line), "file header has an empty path");
        None
    } else {
        Some(PendingHeader {
            raw,
            line,
            header: name,
        })
    }
}

fn flush_update(
    pending: Option<PendingHeader<'_>>,
    payload_pointer: &str,
    call: &ToolCallRef<'_>,
    report: &mut ToolAccessReport,
) {
    if let Some(update) = pending {
        emit(
            &update,
            payload_pointer,
            call,
            PatchOperation::Update,
            AccessIntent::ReadModify,
            report,
        );
    }
}

fn flush_old(
    pending: Option<PendingHeader<'_>>,
    payload_pointer: &str,
    call: &ToolCallRef<'_>,
    report: &mut ToolAccessReport,
) {
    if let Some(old) = pending {
        if old.raw != "/dev/null" {
            emit(
                &old,
                payload_pointer,
                call,
                PatchOperation::UnifiedOld,
                AccessIntent::Unclassified,
                report,
            );
        }
        malformed(
            report,
            Some(old.line),
            "unified old-path header has no matching new-path header",
        );
    }
}

fn emit_unified_pair(
    old: Option<PendingHeader<'_>>,
    new: PendingHeader<'_>,
    payload_pointer: &str,
    call: &ToolCallRef<'_>,
    report: &mut ToolAccessReport,
) {
    let Some(old) = old else {
        if new.raw != "/dev/null" {
            emit(
                &new,
                payload_pointer,
                call,
                PatchOperation::UnifiedNew,
                AccessIntent::Modify,
                report,
            );
        }
        malformed(
            report,
            Some(new.line),
            "unified new-path header has no matching old-path header",
        );
        return;
    };

    match (old.raw, new.raw) {
        ("/dev/null", "/dev/null") => {
            malformed(report, Some(new.line), "both unified paths are /dev/null")
        }
        ("/dev/null", _) => emit(
            &new,
            payload_pointer,
            call,
            PatchOperation::Add,
            AccessIntent::Modify,
            report,
        ),
        (_, "/dev/null") => emit(
            &old,
            payload_pointer,
            call,
            PatchOperation::Delete,
            AccessIntent::Delete,
            report,
        ),
        (old_path, new_path) if old_path == new_path => emit(
            &new,
            payload_pointer,
            call,
            PatchOperation::Update,
            AccessIntent::ReadModify,
            report,
        ),
        _ => {
            emit(
                &old,
                payload_pointer,
                call,
                PatchOperation::MoveSource,
                AccessIntent::MoveSource,
                report,
            );
            emit(
                &new,
                payload_pointer,
                call,
                PatchOperation::MoveDestination,
                AccessIntent::MoveDestination,
                report,
            );
        }
    }
}

fn emit(
    header: &PendingHeader<'_>,
    payload_pointer: &str,
    call: &ToolCallRef<'_>,
    operation: PatchOperation,
    intent: AccessIntent,
    report: &mut ToolAccessReport,
) {
    let raw = header.raw.trim();
    if raw.is_empty() || raw == "/dev/null" {
        return;
    }
    let (expression, unresolved) = path_expression(raw, call.cwd);
    report.candidates.push(AccessCandidate {
        target: AccessTarget::Path {
            expression,
            scope: AccessScope::Exact,
        },
        intent,
        certainty: AccessCertainty::Direct,
        provenance: AccessProvenance::Patch {
            payload_pointer: payload_pointer.to_owned(),
            operation,
            header: header.header.to_owned(),
            line: header.line,
        },
    });
    if unresolved {
        report.push_gap(
            AccessSource::Patch,
            ToolAccessGapReason::MissingWorkingDirectory {
                raw: raw.to_owned(),
                pointer: Some(payload_pointer.to_owned()),
            },
        );
    }
}

fn malformed(report: &mut ToolAccessReport, line: Option<usize>, detail: impl Into<String>) {
    report.push_gap(
        AccessSource::Patch,
        ToolAccessGapReason::MalformedPatch {
            line,
            detail: detail.into(),
        },
    );
}

fn unified_path(raw: &str) -> &str {
    let raw = raw.trim();
    raw.strip_prefix("a/")
        .or_else(|| raw.strip_prefix("b/"))
        .unwrap_or(raw)
}
