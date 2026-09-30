use crate::{
    AccessCandidate, AccessCertainty, AccessIntent, AccessProvenance, AccessScope, AccessSource,
    AccessTarget, JsonRef, PathBase, PathExpression, StructuredFieldMatch, ToolAccessGapReason,
    ToolAccessReport, ToolCallRef,
};
use hookkit_core::{Utf8Path, normalize_utf8_path, resolve_utf8_path};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Configurable recursive structured-field analyzer for tools without a
/// built-in harness profile, such as MCP and custom tools.
///
/// Access semantics come from whole words in the tool name (split on
/// punctuation and camelCase boundaries), so `remove_file` is a delete and
/// `download_file` a write. Source/destination-style keys are consulted only
/// for tools whose name implies a move or copy, so fields such as a calendar
/// event's `from`/`to` are never mistaken for paths.
#[derive(Debug, Clone)]
pub struct StructuredFieldAnalyzer {
    path_keys: BTreeSet<String>,
    move_path_keys: BTreeSet<String>,
    exact_pointers: BTreeMap<String, AccessIntent>,
}

impl Default for StructuredFieldAnalyzer {
    fn default() -> Self {
        Self {
            path_keys: [
                "path",
                "paths",
                "file_path",
                "filePath",
                "target_file",
                "targetFile",
                "absolute_path",
                "absolutePath",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            move_path_keys: [
                "source",
                "destination",
                "old_path",
                "oldPath",
                "new_path",
                "newPath",
                "from",
                "to",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            exact_pointers: BTreeMap::new(),
        }
    }
}

impl StructuredFieldAnalyzer {
    /// Creates an analyzer with the default path-like key heuristics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an exact key-name heuristic used at any object depth for every tool.
    pub fn with_path_key(mut self, key: impl Into<String>) -> Self {
        self.path_keys.insert(key.into());
        self
    }

    /// Add a source/destination key-name heuristic used at any object depth,
    /// but only for tools whose name implies a move or copy.
    pub fn with_move_path_key(mut self, key: impl Into<String>) -> Self {
        self.move_path_keys.insert(key.into());
        self
    }

    /// Add a direct JSON Pointer with an explicit access intent.
    ///
    /// Exact pointers take precedence over key-name heuristics at the same
    /// location, preventing one field from being visited twice.
    pub fn with_pointer(
        mut self,
        pointer: impl Into<String>,
        intent: AccessIntent,
    ) -> Result<Self, StructuredFieldConfigError> {
        let pointer = pointer.into();
        validate_pointer(&pointer)?;
        self.exact_pointers.insert(pointer, intent);
        Ok(self)
    }

    /// Returns the exact key names treated as path-bearing at any object depth.
    pub fn path_keys(&self) -> &BTreeSet<String> {
        &self.path_keys
    }

    /// Returns the source/destination key names consulted for move- and
    /// copy-like tools only.
    pub fn move_path_keys(&self) -> &BTreeSet<String> {
        &self.move_path_keys
    }

    /// Returns explicitly configured JSON Pointers and their access intents.
    pub fn exact_pointers(&self) -> &BTreeMap<String, AccessIntent> {
        &self.exact_pointers
    }

    pub(crate) fn analyze(&self, call: &ToolCallRef<'_>, report: &mut ToolAccessReport) {
        let semantics = structured_semantics(call.tool_name);
        let visitor = Visitor {
            analyzer: self,
            move_keys: semantics.uses_move_keys(),
        };
        let mut fields = Vec::new();
        match call.tool_input {
            JsonRef::Value(value) => visitor.visit(value, "", None, &mut fields, report),
            JsonRef::Object(object) => {
                for (key, value) in object {
                    visitor.visit_child(value, "", key, &mut fields, report);
                }
            }
        }

        let initial_candidates = report.candidates.len();
        for field in fields {
            let intent = field
                .intent
                .unwrap_or_else(|| semantics.intent(field.key.as_deref()));
            push_path_candidate(
                call,
                report,
                PathField {
                    raw: &field.raw,
                    pointer: &field.pointer,
                    scope: AccessScope::Exact,
                    intent,
                    certainty: match field.matched_by {
                        StructuredFieldMatch::KeyHeuristic => AccessCertainty::Heuristic,
                        _ => AccessCertainty::Direct,
                    },
                    matched_by: field.matched_by,
                },
            );
        }

        if report.candidates.len() == initial_candidates {
            match semantics {
                StructuredSemantics::Unknown => report.push_gap(
                    AccessSource::Structured,
                    ToolAccessGapReason::UnknownStructuredTool {
                        tool_name: call.tool_name.to_owned(),
                    },
                ),
                _ => report.push_gap(
                    AccessSource::Structured,
                    ToolAccessGapReason::RecognizedToolWithoutPath {
                        tool_name: call.tool_name.to_owned(),
                    },
                ),
            }
        }
    }
}

struct Visitor<'a> {
    analyzer: &'a StructuredFieldAnalyzer,
    move_keys: bool,
}

impl Visitor<'_> {
    fn visit(
        &self,
        value: &Value,
        pointer: &str,
        key: Option<&str>,
        fields: &mut Vec<StructuredFieldValue>,
        report: &mut ToolAccessReport,
    ) {
        if let Some(intent) = self.analyzer.exact_pointers.get(pointer) {
            collect_recognized_value(
                value,
                pointer,
                key,
                StructuredFieldMatch::ExactPointer,
                Some(*intent),
                fields,
                report,
            );
            return;
        }

        match value {
            Value::Object(object) => {
                for (child_key, child) in object {
                    self.visit_child(child, pointer, child_key, fields, report);
                }
            }
            Value::Array(values) => {
                for (index, child) in values.iter().enumerate() {
                    let child_pointer = join_pointer(pointer, &index.to_string());
                    self.visit(child, &child_pointer, key, fields, report);
                }
            }
            _ => {}
        }
    }

    fn visit_child(
        &self,
        value: &Value,
        parent_pointer: &str,
        key: &str,
        fields: &mut Vec<StructuredFieldValue>,
        report: &mut ToolAccessReport,
    ) {
        let pointer = join_pointer(parent_pointer, key);
        if let Some(intent) = self.analyzer.exact_pointers.get(&pointer) {
            collect_recognized_value(
                value,
                &pointer,
                Some(key),
                StructuredFieldMatch::ExactPointer,
                Some(*intent),
                fields,
                report,
            );
        } else if self.analyzer.path_keys.contains(key)
            || (self.move_keys && self.analyzer.move_path_keys.contains(key))
        {
            collect_recognized_value(
                value,
                &pointer,
                Some(key),
                StructuredFieldMatch::KeyHeuristic,
                None,
                fields,
                report,
            );
        } else {
            self.visit(value, &pointer, Some(key), fields, report);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
/// Invalid exact-pointer configuration for [`StructuredFieldAnalyzer`].
pub enum StructuredFieldConfigError {
    /// The pointer is empty and would select the entire tool input.
    EmptyPointer,
    /// The pointer is not an RFC 6901 absolute JSON Pointer.
    InvalidPointer,
}

impl std::fmt::Display for StructuredFieldConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyPointer => formatter.write_str("structured path JSON Pointer is empty"),
            Self::InvalidPointer => formatter.write_str("invalid structured path JSON Pointer"),
        }
    }
}

impl std::error::Error for StructuredFieldConfigError {}

#[derive(Debug)]
struct StructuredFieldValue {
    raw: String,
    pointer: String,
    key: Option<String>,
    matched_by: StructuredFieldMatch,
    intent: Option<AccessIntent>,
}

fn collect_recognized_value(
    value: &Value,
    pointer: &str,
    key: Option<&str>,
    matched_by: StructuredFieldMatch,
    intent: Option<AccessIntent>,
    fields: &mut Vec<StructuredFieldValue>,
    report: &mut ToolAccessReport,
) {
    match value {
        Value::String(raw) => fields.push(StructuredFieldValue {
            raw: raw.clone(),
            pointer: pointer.to_owned(),
            key: key.map(str::to_owned),
            matched_by,
            intent,
        }),
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                collect_recognized_value(
                    child,
                    &join_pointer(pointer, &index.to_string()),
                    key,
                    matched_by,
                    intent,
                    fields,
                    report,
                );
            }
        }
        _ => report.push_gap(
            AccessSource::Structured,
            ToolAccessGapReason::PathValueNotString {
                pointer: pointer.to_owned(),
            },
        ),
    }
}

/// One structured path value and how it should be recorded.
pub(crate) struct PathField<'a> {
    pub(crate) raw: &'a str,
    pub(crate) pointer: &'a str,
    pub(crate) scope: AccessScope,
    pub(crate) intent: AccessIntent,
    pub(crate) certainty: AccessCertainty,
    pub(crate) matched_by: StructuredFieldMatch,
}

/// Records one structured path candidate plus the gaps its resolution needs.
///
/// A leading `~` is left unresolved because home expansion is up to the tool;
/// relative paths resolve against the call's working directory and carry its
/// [`ToolCallRef::cwd_base`] label.
pub(crate) fn push_path_candidate(
    call: &ToolCallRef<'_>,
    report: &mut ToolAccessReport,
    field: PathField<'_>,
) {
    let expression = if field.raw.starts_with('~') {
        report.push_gap(
            AccessSource::Structured,
            ToolAccessGapReason::UnexpandedHomePath {
                raw: field.raw.to_owned(),
                pointer: field.pointer.to_owned(),
            },
        );
        PathExpression {
            raw: field.raw.to_owned(),
            resolved: None,
            base: PathBase::UnexpandedHome,
        }
    } else {
        let expression = path_expression(field.raw, call.cwd, call.cwd_base);
        if expression.base == PathBase::MissingWorkingDirectory {
            report.push_gap(
                AccessSource::Structured,
                ToolAccessGapReason::MissingWorkingDirectory {
                    raw: field.raw.to_owned(),
                    pointer: Some(field.pointer.to_owned()),
                },
            );
        }
        expression
    };
    report.candidates.push(AccessCandidate {
        target: AccessTarget::Path {
            expression,
            scope: field.scope,
        },
        intent: field.intent,
        certainty: field.certainty,
        provenance: AccessProvenance::StructuredField {
            pointer: field.pointer.to_owned(),
            matched_by: field.matched_by,
        },
    });
    if field.intent == AccessIntent::Unclassified {
        report.push_gap(
            AccessSource::Structured,
            ToolAccessGapReason::UnclassifiedAccess {
                tool_name: call.tool_name.to_owned(),
                pointer: field.pointer.to_owned(),
            },
        );
    }
}

#[derive(Debug, Clone, Copy)]
enum StructuredSemantics {
    Intent(AccessIntent),
    Move,
    Copy,
    Unknown,
}

impl StructuredSemantics {
    fn uses_move_keys(self) -> bool {
        matches!(self, Self::Move | Self::Copy)
    }

    fn intent(self, key: Option<&str>) -> AccessIntent {
        let role = key.map(normalize_key);
        match self {
            Self::Intent(intent) => intent,
            Self::Move => match role.as_deref() {
                Some("source" | "oldpath" | "from") => AccessIntent::MoveSource,
                Some("destination" | "newpath" | "to") => AccessIntent::MoveDestination,
                _ => AccessIntent::Unclassified,
            },
            Self::Copy => match role.as_deref() {
                Some("source" | "oldpath" | "from") => AccessIntent::Read,
                Some("destination" | "newpath" | "to") => AccessIntent::Modify,
                _ => AccessIntent::Unclassified,
            },
            Self::Unknown => AccessIntent::Unclassified,
        }
    }
}

/// Classifies a tool by whole words in its name. More specific effects are
/// checked first, so `remove_file` is a delete and `download_file` a write.
fn structured_semantics(tool_name: &str) -> StructuredSemantics {
    let words = name_words(tool_name);
    let has = |needles: &[&str]| words.iter().any(|word| needles.contains(&word.as_str()));
    if has(&["move", "rename", "mv"]) {
        StructuredSemantics::Move
    } else if has(&["copy", "cp", "duplicate"]) {
        StructuredSemantics::Copy
    } else if has(&["delete", "remove", "rm", "rmdir", "unlink", "trash"]) {
        StructuredSemantics::Intent(AccessIntent::Delete)
    } else if has(&[
        "edit", "update", "replace", "patch", "modify", "append", "insert",
    ]) {
        StructuredSemantics::Intent(AccessIntent::ReadModify)
    } else if has(&[
        "write",
        "save",
        "create",
        "download",
        "export",
        "put",
        "mkdir",
        "touch",
        "overwrite",
    ]) {
        StructuredSemantics::Intent(AccessIntent::Modify)
    } else if has(&["list", "ls", "glob", "search", "find", "tree"]) {
        StructuredSemantics::Intent(AccessIntent::Enumerate)
    } else if has(&["read", "open", "load", "view", "cat", "grep", "upload"]) {
        StructuredSemantics::Intent(AccessIntent::Read)
    } else {
        StructuredSemantics::Unknown
    }
}

/// Splits a tool name into lowercase words on punctuation and camelCase
/// boundaries: `mcp__fs__readFile` becomes `mcp`, `fs`, `read`, `file`.
fn name_words(tool_name: &str) -> Vec<String> {
    let mut words = Vec::new();
    for segment in tool_name.split(|character: char| !character.is_ascii_alphanumeric()) {
        let characters = segment.chars().collect::<Vec<_>>();
        let mut word = String::new();
        for (index, &character) in characters.iter().enumerate() {
            if index > 0 && character.is_ascii_uppercase() {
                let previous = characters[index - 1];
                let next_is_lower = characters
                    .get(index + 1)
                    .is_some_and(char::is_ascii_lowercase);
                if (previous.is_ascii_lowercase()
                    || previous.is_ascii_digit()
                    || (previous.is_ascii_uppercase() && next_is_lower))
                    && !word.is_empty()
                {
                    words.push(std::mem::take(&mut word));
                }
            }
            word.push(character.to_ascii_lowercase());
        }
        if !word.is_empty() {
            words.push(word);
        }
    }
    words
}

fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|character| *character != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

/// Lexically resolves `raw`. Relative paths resolve against `cwd` and are
/// labeled `relative_base` when that base is observable; otherwise they stay
/// unresolved with `relative_base` (or [`PathBase::MissingWorkingDirectory`]
/// when no directory was supplied).
pub(crate) fn path_expression(
    raw: &str,
    cwd: Option<&Utf8Path>,
    relative_base: PathBase,
) -> PathExpression {
    let path = Utf8Path::new(raw);
    if path.is_absolute() {
        PathExpression {
            raw: raw.to_owned(),
            resolved: Some(normalize_utf8_path(path)),
            base: PathBase::Absolute,
        }
    } else if !relative_base.is_observable() {
        PathExpression {
            raw: raw.to_owned(),
            resolved: None,
            base: relative_base,
        }
    } else if let Some(cwd) = cwd {
        PathExpression {
            raw: raw.to_owned(),
            resolved: Some(resolve_utf8_path(cwd, path)),
            base: relative_base,
        }
    } else {
        PathExpression {
            raw: raw.to_owned(),
            resolved: None,
            base: PathBase::MissingWorkingDirectory,
        }
    }
}

pub(crate) fn join_pointer(parent: &str, segment: &str) -> String {
    let escaped = segment.replace('~', "~0").replace('/', "~1");
    format!("{parent}/{escaped}")
}

fn validate_pointer(pointer: &str) -> Result<(), StructuredFieldConfigError> {
    if pointer.is_empty() {
        return Err(StructuredFieldConfigError::EmptyPointer);
    }
    if !pointer.starts_with('/') || pointer.split('/').skip(1).any(invalid_pointer_segment) {
        return Err(StructuredFieldConfigError::InvalidPointer);
    }
    Ok(())
}

fn invalid_pointer_segment(segment: &str) -> bool {
    let mut chars = segment.chars();
    while let Some(character) = chars.next() {
        if character == '~' && !matches!(chars.next(), Some('0' | '1')) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_split_into_whole_words() {
        assert_eq!(
            name_words("mcp__fs__readFile"),
            ["mcp", "fs", "read", "file"]
        );
        assert_eq!(
            name_words("HTTPServer.remove-file"),
            ["http", "server", "remove", "file"]
        );
        assert_eq!(name_words("TodoWrite"), ["todo", "write"]);
    }

    #[test]
    fn semantics_match_whole_words_in_specific_order() {
        let intent = |name| match structured_semantics(name) {
            StructuredSemantics::Intent(intent) => Some(intent),
            _ => None,
        };
        assert_eq!(intent("mcp__fs__remove_file"), Some(AccessIntent::Delete));
        assert_eq!(intent("mcp__dl__download_file"), Some(AccessIntent::Modify));
        assert_eq!(intent("mcp__fs__read_file"), Some(AccessIntent::Read));
        assert_eq!(intent("credit_card_lookup"), None);
        assert!(matches!(
            structured_semantics("mcp__fs__move_file"),
            StructuredSemantics::Move
        ));
        assert!(matches!(
            structured_semantics("copy_file"),
            StructuredSemantics::Copy
        ));
    }
}
