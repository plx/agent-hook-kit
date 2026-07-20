use crate::{
    AccessCandidate, AccessCertainty, AccessIntent, AccessProvenance, AccessScope, AccessSource,
    AccessTarget, JsonRef, PathBase, PathExpression, StructuredFieldMatch, ToolAccessGapReason,
    ToolAccessReport, ToolCallRef,
};
use hookkit_core::{Utf8Path, normalize_utf8_path, resolve_utf8_path};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Configurable recursive structured-field analyzer.
#[derive(Debug, Clone)]
pub struct StructuredFieldAnalyzer {
    path_keys: BTreeSet<String>,
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
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an exact key-name heuristic used at any object depth.
    pub fn with_path_key(mut self, key: impl Into<String>) -> Self {
        self.path_keys.insert(key.into());
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

    pub fn path_keys(&self) -> &BTreeSet<String> {
        &self.path_keys
    }

    pub fn exact_pointers(&self) -> &BTreeMap<String, AccessIntent> {
        &self.exact_pointers
    }

    pub(crate) fn analyze(&self, call: &ToolCallRef<'_>, report: &mut ToolAccessReport) {
        let semantics = structured_semantics(call.tool_name);
        let mut fields = Vec::new();
        match call.tool_input {
            JsonRef::Value(value) => self.visit(value, "", None, &mut fields, report),
            JsonRef::Object(object) => {
                for (key, value) in object {
                    self.visit_child(value, "", key, &mut fields, report);
                }
            }
        }

        for field in fields {
            let intent = field
                .intent
                .unwrap_or_else(|| semantics.intent(field.key.as_deref()));
            let (expression, unresolved) = path_expression(&field.raw, call.cwd);
            report.candidates.push(AccessCandidate {
                target: AccessTarget::Path {
                    expression,
                    scope: AccessScope::Exact,
                },
                intent,
                certainty: match field.matched_by {
                    StructuredFieldMatch::ExactPointer => AccessCertainty::Direct,
                    StructuredFieldMatch::KeyHeuristic => AccessCertainty::Heuristic,
                },
                provenance: AccessProvenance::StructuredField {
                    pointer: field.pointer.clone(),
                    matched_by: field.matched_by,
                },
            });
            if unresolved {
                report.push_gap(
                    AccessSource::Structured,
                    ToolAccessGapReason::MissingWorkingDirectory {
                        raw: field.raw.clone(),
                        pointer: Some(field.pointer.clone()),
                    },
                );
            }
            if intent == AccessIntent::Unclassified {
                report.push_gap(
                    AccessSource::Structured,
                    ToolAccessGapReason::UnclassifiedAccess {
                        tool_name: call.tool_name.to_owned(),
                        pointer: field.pointer,
                    },
                );
            }
        }

        if report.candidates.is_empty() {
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

    fn visit(
        &self,
        value: &Value,
        pointer: &str,
        key: Option<&str>,
        fields: &mut Vec<StructuredFieldValue>,
        report: &mut ToolAccessReport,
    ) {
        if let Some(intent) = self.exact_pointers.get(pointer) {
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
        if let Some(intent) = self.exact_pointers.get(&pointer) {
            collect_recognized_value(
                value,
                &pointer,
                Some(key),
                StructuredFieldMatch::ExactPointer,
                Some(*intent),
                fields,
                report,
            );
        } else if self.path_keys.contains(key) {
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
pub enum StructuredFieldConfigError {
    EmptyPointer,
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

#[derive(Debug, Clone, Copy)]
enum StructuredSemantics {
    Intent(AccessIntent),
    Move,
    Unknown,
}

impl StructuredSemantics {
    fn intent(self, key: Option<&str>) -> AccessIntent {
        match self {
            Self::Intent(intent) => intent,
            Self::Move => match key.map(normalize_key).as_deref() {
                Some("source" | "oldpath" | "from") => AccessIntent::MoveSource,
                Some("destination" | "newpath" | "to") => AccessIntent::MoveDestination,
                _ => AccessIntent::Unclassified,
            },
            Self::Unknown => AccessIntent::Unclassified,
        }
    }
}

fn structured_semantics(tool_name: &str) -> StructuredSemantics {
    let name = tool_name.to_ascii_lowercase();
    if contains_any(&name, &["move", "rename"]) {
        StructuredSemantics::Move
    } else if contains_any(&name, &["delete", "remove"]) {
        StructuredSemantics::Intent(AccessIntent::Delete)
    } else if contains_any(&name, &["edit", "update", "replace", "patch"]) {
        StructuredSemantics::Intent(AccessIntent::ReadModify)
    } else if contains_any(&name, &["write", "save", "create"]) {
        StructuredSemantics::Intent(AccessIntent::Modify)
    } else if contains_any(&name, &["list", "glob", "search", "find"]) {
        StructuredSemantics::Intent(AccessIntent::Enumerate)
    } else if contains_any(&name, &["read", "open", "load"]) {
        StructuredSemantics::Intent(AccessIntent::Read)
    } else {
        StructuredSemantics::Unknown
    }
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|character| *character != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

pub(crate) fn path_expression(raw: &str, cwd: Option<&Utf8Path>) -> (PathExpression, bool) {
    let path = Utf8Path::new(raw);
    if path.is_absolute() {
        (
            PathExpression {
                raw: raw.to_owned(),
                resolved: Some(normalize_utf8_path(path)),
                base: PathBase::Absolute,
            },
            false,
        )
    } else if let Some(cwd) = cwd {
        (
            PathExpression {
                raw: raw.to_owned(),
                resolved: Some(resolve_utf8_path(cwd, path)),
                base: PathBase::InvocationCwd,
            },
            false,
        )
    } else {
        (
            PathExpression {
                raw: raw.to_owned(),
                resolved: None,
                base: PathBase::MissingWorkingDirectory,
            },
            true,
        )
    }
}

fn join_pointer(parent: &str, segment: &str) -> String {
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
