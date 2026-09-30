//! Exact argument contracts for documented harness built-in tools.
//!
//! Built-in tools are matched by harness and exact tool name, and their path
//! arguments by exact JSON Pointer, so documented shapes such as Antigravity's
//! PascalCase `TargetFile` never depend on the generic key heuristics (which
//! would also match unrelated MCP tools). Tools that are documented not to
//! touch files yield an empty, complete report.

use crate::structured::{PathField, join_pointer, push_path_candidate};
use crate::{
    AccessCertainty, AccessIntent, AccessScope, AccessSource, PathBase, StructuredFieldMatch,
    ToolAccessGapReason, ToolAccessReport, ToolCallRef,
};
use hookkit_core::HarnessId;
use serde_json::Value;

/// Claude Code built-ins that do not read or write workspace files. Subagent
/// tools are included because the subagent's own tool calls fire hooks.
const CLAUDE_FILE_FREE: &[&str] = &[
    "Agent",
    "AskUserQuestion",
    "EnterPlanMode",
    "ExitPlanMode",
    "KillShell",
    "SubagentHandback",
    "Task",
    "TaskCreate",
    "TaskGet",
    "TaskList",
    "TaskStop",
    "TaskUpdate",
    "TodoWrite",
    "ToolSearch",
    "WebFetch",
    "WebSearch",
];

/// Codex function tools (as named in hook payloads) that do not read or write
/// workspace files. Namespaced multi-agent tools are flattened by Codex
/// without a separator.
const CODEX_FILE_FREE: &[&str] = &[
    "close_agent",
    "curr_time",
    "followup_task",
    "get_context_remaining",
    "interrupt_agent",
    "list_agents",
    "list_mcp_resource_templates",
    "list_mcp_resources",
    "multi_agent_v1close_agent",
    "multi_agent_v1resume_agent",
    "multi_agent_v1send_input",
    "multi_agent_v1wait_agent",
    "new_context",
    "read_mcp_resource",
    "request_permissions",
    "request_user_input",
    "request_user_input_async",
    "resume_agent",
    "send_input",
    "send_message",
    "spawn_agent",
    "tool_search",
    "update_plan",
    "wait",
    "wait_agent",
    "web_search",
];

/// Antigravity documented tools that do not read or write workspace files.
const ANTIGRAVITY_FILE_FREE: &[&str] = &[
    "ask_permission",
    "ask_question",
    "define_subagent",
    "invoke_subagent",
    "list_permissions",
    "manage_subagents",
    "manage_task",
    "read_url_content",
    "schedule",
    "search_web",
    "send_message",
];

/// Analyzes a documented built-in tool and returns `true`, or returns `false`
/// when the tool has no built-in contract for its harness.
pub(crate) fn analyze(call: &ToolCallRef<'_>, report: &mut ToolAccessReport) -> bool {
    let harness = call.harness();
    let name = call.tool_name;
    if *harness == HarnessId::CLAUDE_CODE {
        analyze_claude(call, name, report)
    } else if *harness == HarnessId::CODEX {
        analyze_codex(call, name, report)
    } else if *harness == HarnessId::ANTIGRAVITY {
        analyze_antigravity(call, name, report)
    } else {
        false
    }
}

fn analyze_claude(call: &ToolCallRef<'_>, name: &str, report: &mut ToolAccessReport) -> bool {
    match name {
        "Read" => required_path(call, report, "/file_path", AccessIntent::Read),
        "Write" => required_path(call, report, "/file_path", AccessIntent::Modify),
        "Edit" | "MultiEdit" => required_path(call, report, "/file_path", AccessIntent::ReadModify),
        "NotebookEdit" => required_path(call, report, "/notebook_path", AccessIntent::ReadModify),
        "LS" => required_path(call, report, "/path", AccessIntent::Enumerate),
        // Grep searches `path` (a file or directory, default cwd), optionally
        // narrowed by a ripgrep `glob` filter.
        "Grep" => search(
            call,
            report,
            Search {
                root: Root::DefaultCwd("/path"),
                filters: Filters::One("/glob"),
                intent: AccessIntent::Read,
            },
        ),
        // Glob lists paths under `path` (default cwd) matching `pattern`.
        "Glob" => glob(call, report, "/path", "/pattern"),
        _ => CLAUDE_FILE_FREE.contains(&name),
    }
}

fn analyze_codex(call: &ToolCallRef<'_>, name: &str, report: &mut ToolAccessReport) -> bool {
    match name {
        "view_image" => {
            if let Some(environment_id) = environment_id(call) {
                report.push_gap(
                    AccessSource::Structured,
                    ToolAccessGapReason::UnknownExecutionEnvironment {
                        environment_id: environment_id.to_owned(),
                    },
                );
                let foreign = call.clone().with_cwd_base(PathBase::UnknownEnvironment);
                return required_path(&foreign, report, "/path", AccessIntent::Read);
            }
            required_path(call, report, "/path", AccessIntent::Read)
        }
        _ => CODEX_FILE_FREE.contains(&name),
    }
}

fn analyze_antigravity(call: &ToolCallRef<'_>, name: &str, report: &mut ToolAccessReport) -> bool {
    match name {
        "view_file" => required_path(call, report, "/AbsolutePath", AccessIntent::Read),
        "write_to_file" => required_path(call, report, "/TargetFile", AccessIntent::Modify),
        "replace_file_content" | "multi_replace_file_content" => {
            required_path(call, report, "/TargetFile", AccessIntent::ReadModify)
        }
        "list_dir" => required_path(call, report, "/DirectoryPath", AccessIntent::Enumerate),
        "find_by_name" => {
            // `FullPath` matches `Pattern` against whole paths, which a
            // descendant-relative glob cannot express.
            let full_path = call
                .tool_input
                .get("FullPath")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            search(
                call,
                report,
                Search {
                    root: Root::Required("/SearchDirectory"),
                    filters: if full_path {
                        Filters::None
                    } else {
                        Filters::One("/Pattern")
                    },
                    intent: AccessIntent::Enumerate,
                },
            )
        }
        "grep_search" => search(
            call,
            report,
            Search {
                root: Root::Required("/SearchPath"),
                filters: Filters::Many("/Includes"),
                intent: AccessIntent::Read,
            },
        ),
        "generate_image" => {
            // `ImagePaths` optionally names input images; the generated
            // image itself is written to the conversation's artifacts.
            match call.tool_input.pointer("/ImagePaths") {
                None | Some(Value::Null) => {}
                Some(Value::Array(paths)) => {
                    for (index, path) in paths.iter().enumerate() {
                        let pointer = join_pointer("/ImagePaths", &index.to_string());
                        string_path(call, report, path, &pointer, AccessIntent::Read);
                    }
                }
                Some(path) => string_path(call, report, path, "/ImagePaths", AccessIntent::Read),
            }
            true
        }
        _ => ANTIGRAVITY_FILE_FREE.contains(&name),
    }
}

fn environment_id<'a>(call: &ToolCallRef<'a>) -> Option<&'a str> {
    call.tool_input
        .get("environment_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
}

/// Records a required single-path argument with exact scope.
fn required_path(
    call: &ToolCallRef<'_>,
    report: &mut ToolAccessReport,
    pointer: &str,
    intent: AccessIntent,
) -> bool {
    match call.tool_input.pointer(pointer) {
        Some(value) => string_path(call, report, value, pointer, intent),
        None => report.push_gap(
            AccessSource::Structured,
            ToolAccessGapReason::RecognizedToolWithoutPath {
                tool_name: call.tool_name.to_owned(),
            },
        ),
    }
    true
}

fn string_path(
    call: &ToolCallRef<'_>,
    report: &mut ToolAccessReport,
    value: &Value,
    pointer: &str,
    intent: AccessIntent,
) {
    let Some(raw) = value.as_str() else {
        report.push_gap(
            AccessSource::Structured,
            ToolAccessGapReason::PathValueNotString {
                pointer: pointer.to_owned(),
            },
        );
        return;
    };
    push_path_candidate(
        call,
        report,
        PathField {
            raw,
            pointer,
            scope: AccessScope::Exact,
            intent,
            certainty: AccessCertainty::Direct,
            matched_by: StructuredFieldMatch::BuiltinTool,
        },
    );
}

#[derive(Clone, Copy)]
enum Root {
    /// The argument is required.
    Required(&'static str),
    /// The argument defaults to the invocation working directory.
    DefaultCwd(&'static str),
}

#[derive(Clone, Copy)]
enum Filters {
    None,
    /// One optional glob string.
    One(&'static str),
    /// An optional array of glob strings.
    Many(&'static str),
}

struct Search {
    root: Root,
    filters: Filters,
    intent: AccessIntent,
}

struct ResolvedRoot<'a> {
    raw: &'a str,
    pointer: &'static str,
    matched_by: StructuredFieldMatch,
}

/// Resolves a search root argument, applying the documented cwd default.
fn search_root<'a>(
    call: &ToolCallRef<'a>,
    report: &mut ToolAccessReport,
    root: Root,
) -> Option<ResolvedRoot<'a>> {
    let (pointer, defaults_to_cwd) = match root {
        Root::Required(pointer) => (pointer, false),
        Root::DefaultCwd(pointer) => (pointer, true),
    };
    let supplied = match call.tool_input.pointer(pointer) {
        None | Some(Value::Null) => None,
        Some(Value::String(raw)) => Some(raw.as_str()).filter(|raw| !raw.is_empty()),
        Some(_) => {
            report.push_gap(
                AccessSource::Structured,
                ToolAccessGapReason::PathValueNotString {
                    pointer: pointer.to_owned(),
                },
            );
            return None;
        }
    };
    match supplied {
        Some(raw) => Some(ResolvedRoot {
            raw,
            pointer,
            matched_by: StructuredFieldMatch::BuiltinTool,
        }),
        None if defaults_to_cwd => Some(ResolvedRoot {
            raw: ".",
            pointer,
            matched_by: StructuredFieldMatch::BuiltinDefault,
        }),
        None => {
            report.push_gap(
                AccessSource::Structured,
                ToolAccessGapReason::RecognizedToolWithoutPath {
                    tool_name: call.tool_name.to_owned(),
                },
            );
            None
        }
    }
}

/// Records a content or name search below a root.
///
/// Without usable filters the whole root (a file, or a directory and its
/// descendants) is the target. With filters, the root itself is recorded
/// exactly and every filter becomes a descendant glob: a filter without `/`
/// matches basenames at any depth, as ripgrep and fd-style globs do. Negated
/// or non-string filters fall back to the conservative whole-root scope.
fn search(call: &ToolCallRef<'_>, report: &mut ToolAccessReport, search: Search) -> bool {
    let Some(root) = search_root(call, report, search.root) else {
        return true;
    };
    let filters = match search.filters {
        Filters::None => Some(Vec::new()),
        Filters::One(pointer) => match call.tool_input.pointer(pointer) {
            None | Some(Value::Null) => Some(Vec::new()),
            Some(Value::String(glob)) => Some(vec![(pointer.to_owned(), glob.as_str())]),
            Some(_) => None,
        },
        Filters::Many(pointer) => match call.tool_input.pointer(pointer) {
            None | Some(Value::Null) => Some(Vec::new()),
            Some(Value::Array(globs)) => globs
                .iter()
                .enumerate()
                .map(|(index, glob)| {
                    glob.as_str()
                        .map(|glob| (join_pointer(pointer, &index.to_string()), glob))
                })
                .collect(),
            Some(_) => None,
        },
    };
    let filters = filters.filter(|filters| {
        filters
            .iter()
            .all(|(_, glob)| !glob.trim().is_empty() && !glob.trim_start().starts_with('!'))
    });

    match filters {
        Some(filters) if !filters.is_empty() => {
            root_candidate(call, report, &root, AccessScope::Exact, search.intent);
            for (pointer, glob) in filters {
                let pattern = descendant_glob(root.raw, glob.trim());
                push_path_candidate(
                    call,
                    report,
                    PathField {
                        raw: &pattern,
                        pointer: &pointer,
                        scope: AccessScope::Glob,
                        intent: search.intent,
                        certainty: AccessCertainty::Heuristic,
                        matched_by: StructuredFieldMatch::BuiltinTool,
                    },
                );
            }
        }
        _ => root_candidate(
            call,
            report,
            &root,
            AccessScope::ExactOrDescendants,
            search.intent,
        ),
    }
    true
}

/// Records Claude `Glob`: paths under `path` (default cwd) matching `pattern`.
fn glob(
    call: &ToolCallRef<'_>,
    report: &mut ToolAccessReport,
    root_pointer: &'static str,
    pattern_pointer: &'static str,
) -> bool {
    let Some(root) = search_root(call, report, Root::DefaultCwd(root_pointer)) else {
        return true;
    };
    match call.tool_input.pointer(pattern_pointer) {
        Some(Value::String(pattern)) if !pattern.trim().is_empty() => {
            let pattern = pattern.trim();
            let raw = if pattern.starts_with('/') || root.raw == "." {
                pattern.to_owned()
            } else {
                format!("{}/{pattern}", root.raw.trim_end_matches('/'))
            };
            push_path_candidate(
                call,
                report,
                PathField {
                    raw: &raw,
                    pointer: pattern_pointer,
                    scope: AccessScope::Glob,
                    intent: AccessIntent::Enumerate,
                    certainty: AccessCertainty::Direct,
                    matched_by: StructuredFieldMatch::BuiltinTool,
                },
            );
        }
        Some(Value::String(_)) | None | Some(Value::Null) => report.push_gap(
            AccessSource::Structured,
            ToolAccessGapReason::RecognizedToolWithoutPath {
                tool_name: call.tool_name.to_owned(),
            },
        ),
        Some(_) => report.push_gap(
            AccessSource::Structured,
            ToolAccessGapReason::PathValueNotString {
                pointer: pattern_pointer.to_owned(),
            },
        ),
    }
    true
}

fn root_candidate(
    call: &ToolCallRef<'_>,
    report: &mut ToolAccessReport,
    root: &ResolvedRoot<'_>,
    scope: AccessScope,
    intent: AccessIntent,
) {
    push_path_candidate(
        call,
        report,
        PathField {
            raw: root.raw,
            pointer: root.pointer,
            scope,
            intent,
            certainty: AccessCertainty::Direct,
            matched_by: root.matched_by,
        },
    );
}

/// Joins a filter glob below a search root.
fn descendant_glob(root: &str, glob: &str) -> String {
    let relative = match glob.strip_prefix('/') {
        Some(anchored) => anchored.to_owned(),
        None if glob.contains('/') || glob.starts_with("**") => glob.to_owned(),
        None => format!("**/{glob}"),
    };
    if root == "." {
        relative
    } else {
        format!("{}/{relative}", root.trim_end_matches('/'))
    }
}

#[cfg(test)]
mod tests {
    use super::descendant_glob;

    #[test]
    fn filters_become_descendant_globs() {
        assert_eq!(descendant_glob(".", "*.ts"), "**/*.ts");
        assert_eq!(
            descendant_glob("/repo/", "*.{ts,tsx}"),
            "/repo/**/*.{ts,tsx}"
        );
        assert_eq!(descendant_glob("/repo", "src/*.rs"), "/repo/src/*.rs");
        assert_eq!(descendant_glob("/repo", "/top.rs"), "/repo/top.rs");
        assert_eq!(descendant_glob("src", "**/.env"), "src/**/.env");
    }

    #[test]
    fn file_free_lists_are_sorted_and_unique() {
        for list in [
            super::CLAUDE_FILE_FREE,
            super::CODEX_FILE_FREE,
            super::ANTIGRAVITY_FILE_FREE,
        ] {
            assert!(list.windows(2).all(|pair| pair[0] < pair[1]), "{list:?}");
        }
    }
}
