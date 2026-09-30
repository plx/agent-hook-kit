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
/// `download_file` a write; a run-together word such as `writefile`,
/// `readtextfile`, or `fileremove` counts when it joins a known verb and a
/// file-system noun. Source/destination-style keys are consulted for tools
/// whose name implies a move or copy. Tools that write also consult the
/// destination-style keys (such as `destination` or `output_path`), and tools
/// that read the source-style keys (such as `source`), except the generic
/// `from` and `to`, so fields such as a calendar event's `from`/`to` are
/// never mistaken for paths.
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
                "src",
                "destination",
                "dest",
                "old_path",
                "oldPath",
                "new_path",
                "newPath",
                "output",
                "output_path",
                "outputPath",
                "target_path",
                "targetPath",
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
    /// but only for tools whose name implies a move or copy, or that read or
    /// write when the key names a source or destination role respectively.
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
    /// copy-like tools, and by role for tools that read or write.
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
            move_keys: semantics.move_keys(),
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
    move_keys: MoveKeys,
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
            || (self.analyzer.move_path_keys.contains(key) && self.move_keys.consults(key))
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
    /// Which source/destination keys the tool consults: all of them for a
    /// move or copy, the destination-style ones for a write, and the
    /// source-style ones for a read.
    fn move_keys(self) -> MoveKeys {
        match self {
            Self::Move | Self::Copy => MoveKeys::All,
            Self::Intent(AccessIntent::Modify | AccessIntent::ReadModify) => {
                MoveKeys::Role(KeyRole::Destination)
            }
            Self::Intent(AccessIntent::Read) => MoveKeys::Role(KeyRole::Source),
            _ => MoveKeys::None,
        }
    }

    fn intent(self, key: Option<&str>) -> AccessIntent {
        let role = key.and_then(key_role).map(|(role, _)| role);
        match self {
            Self::Intent(intent) => intent,
            Self::Move => match role {
                Some(KeyRole::Source) => AccessIntent::MoveSource,
                Some(KeyRole::Destination) => AccessIntent::MoveDestination,
                None => AccessIntent::Unclassified,
            },
            Self::Copy => match role {
                Some(KeyRole::Source) => AccessIntent::Read,
                Some(KeyRole::Destination) => AccessIntent::Modify,
                None => AccessIntent::Unclassified,
            },
            Self::Unknown => AccessIntent::Unclassified,
        }
    }
}

/// Source/destination keys a tool consults.
#[derive(Debug, Clone, Copy)]
enum MoveKeys {
    None,
    /// Every configured key.
    All,
    /// Configured keys that name this role, except the generic `from`/`to`.
    Role(KeyRole),
}

impl MoveKeys {
    fn consults(self, key: &str) -> bool {
        match self {
            Self::None => false,
            Self::All => true,
            Self::Role(role) => key_role(key) == Some((role, false)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyRole {
    Source,
    Destination,
}

/// The role a source/destination key names, and whether the key is the
/// generic `from` or `to`, which non-file tools also use (for example a
/// calendar event's time range).
fn key_role(key: &str) -> Option<(KeyRole, bool)> {
    match normalize_key(key).as_str() {
        "from" => Some((KeyRole::Source, true)),
        "to" => Some((KeyRole::Destination, true)),
        "source" | "src" | "oldpath" => Some((KeyRole::Source, false)),
        "destination" | "dest" | "newpath" | "output" | "outputpath" | "targetpath" => {
            Some((KeyRole::Destination, false))
        }
        _ => None,
    }
}

/// Name verbs by effect, most specific first.
const MOVE_VERBS: &[&str] = &["move", "rename", "mv"];
const COPY_VERBS: &[&str] = &["copy", "cp", "duplicate"];
const DELETE_VERBS: &[&str] = &["delete", "remove", "rm", "rmdir", "unlink", "trash"];
const READ_MODIFY_VERBS: &[&str] = &[
    "edit", "update", "replace", "patch", "modify", "append", "insert",
];
const MODIFY_VERBS: &[&str] = &[
    "write",
    "save",
    "create",
    "download",
    "export",
    "put",
    "mkdir",
    "touch",
    "overwrite",
];
const ENUMERATE_VERBS: &[&str] = &["list", "ls", "glob", "search", "find", "tree"];
const READ_VERBS: &[&str] = &["read", "open", "load", "view", "cat", "grep", "upload"];

/// File-system nouns that a run-together name word such as `writefile` or
/// `fileread` joins with a verb.
const FILE_NOUNS: &[&str] = &[
    "file",
    "files",
    "dir",
    "dirs",
    "directory",
    "directories",
    "folder",
    "folders",
    "path",
    "paths",
];

/// Classifies a tool by whole words in its name. More specific effects are
/// checked first, so `remove_file` is a delete and `download_file` a write.
/// A run-together lowercase word counts as a verb when it joins the verb and
/// a file-system noun in either order (`writefile`, `fileremove`), or a verb
/// of four or more letters and a phrase ending in a noun (`readtextfile`), so
/// `readme` and `catalogfile` stay unclassified and `fileremove` is a delete
/// rather than a move.
fn structured_semantics(tool_name: &str) -> StructuredSemantics {
    let words = name_words(tool_name);
    let is_noun = |text: &str| FILE_NOUNS.contains(&text);
    let has = |verbs: &[&str]| {
        words.iter().any(|word| {
            verbs.iter().any(|verb| {
                word == verb
                    || word.strip_prefix(verb).is_some_and(|rest| {
                        is_noun(rest)
                            || (verb.len() >= 4
                                && FILE_NOUNS.iter().any(|noun| rest.ends_with(noun)))
                    })
                    || word.strip_suffix(verb).is_some_and(is_noun)
            })
        })
    };
    if has(MOVE_VERBS) {
        StructuredSemantics::Move
    } else if has(COPY_VERBS) {
        StructuredSemantics::Copy
    } else if has(DELETE_VERBS) {
        StructuredSemantics::Intent(AccessIntent::Delete)
    } else if has(READ_MODIFY_VERBS) {
        StructuredSemantics::Intent(AccessIntent::ReadModify)
    } else if has(MODIFY_VERBS) {
        StructuredSemantics::Intent(AccessIntent::Modify)
    } else if has(ENUMERATE_VERBS) {
        StructuredSemantics::Intent(AccessIntent::Enumerate)
    } else if has(READ_VERBS) {
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
        // Run-together words join a verb and a file-system noun.
        assert_eq!(intent("mcp__fs__writefile"), Some(AccessIntent::Modify));
        assert_eq!(intent("mcp__fs__readtextfile"), Some(AccessIntent::Read));
        assert_eq!(intent("mcp__fs__fileremove"), Some(AccessIntent::Delete));
        assert_eq!(intent("mcp__fs__listdir"), Some(AccessIntent::Enumerate));
        assert_eq!(intent("mcp__docs__readme"), None);
        assert_eq!(intent("mcp__shop__catalogfile"), None);
        assert!(matches!(
            structured_semantics("mcp__fs__move_file"),
            StructuredSemantics::Move
        ));
        assert!(matches!(
            structured_semantics("copy_file"),
            StructuredSemantics::Copy
        ));
        assert!(matches!(
            structured_semantics("mcp__fs__renamefile"),
            StructuredSemantics::Move
        ));
    }

    #[test]
    fn role_keys_are_consulted_by_effect() {
        let consults = |name, key| structured_semantics(name).move_keys().consults(key);
        // Moves and copies consult every key, including `from`/`to`.
        assert!(consults("move_file", "to"));
        assert!(consults("copy_file", "from"));
        // Writes consult destinations, reads sources, neither the generic keys.
        assert!(consults("download_file", "destination"));
        assert!(consults("export_chart", "outputPath"));
        assert!(!consults("download_file", "source"));
        assert!(!consults("create_event", "to"));
        assert!(consults("read_file", "src"));
        assert!(!consults("read_file", "dest"));
        assert!(!consults("read_file", "from"));
        assert!(!consults("remove_file", "destination"));
        assert!(!consults("credit_card_lookup", "source"));
    }
}
