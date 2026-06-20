//! Normalized semantic views over lossless common input wrappers.

use crate::input::CommonPostToolUseInput;
use hookkit_claude::input as claude;
use hookkit_codex::input as codex;
use hookkit_core::{Harness, HookEventKey};
use hookkit_gemini::input as gemini;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

static RAW_INPUT_UNAVAILABLE: serde_json::Value = serde_json::Value::Null;

/// Event metadata normalized across harness payloads.
#[derive(Debug, Clone)]
pub struct CommonEventMeta<'a> {
    pub harness: Harness,
    pub event: HookEventKey,
    pub session_id: Option<&'a str>,
    pub cwd: Option<&'a str>,
    pub turn_id: Option<&'a str>,
    pub tool_use_id: Option<&'a str>,
    pub raw_input: &'a serde_json::Value,
}

/// Confidence assigned to a value derived from input or tool result shapes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DerivedConfidence {
    Exact,
    Likely,
    Unknown,
}

/// Where a derived value came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DerivedSource {
    NativeField(&'static str),
    ToolInputField(&'static str),
    ToolResultField(&'static str),
    RawField(String),
    Heuristic,
}

/// Normalized tool-use fields shared by post-tool hooks.
#[derive(Debug, Clone, Copy)]
pub struct CommonToolUseView<'a> {
    pub name: Option<&'a str>,
    pub input: Option<&'a serde_json::Value>,
}

/// Normalized tool-result fields shared by post-tool hooks.
#[derive(Debug, Clone, Copy)]
pub struct CommonToolResultView<'a> {
    pub value: Option<&'a serde_json::Value>,
    pub status: ToolExecutionStatus,
    pub stdout: Option<&'a str>,
    pub stderr: Option<&'a str>,
    pub exit_code: Option<i64>,
}

/// Best-effort execution status inferred from common result fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolExecutionStatus {
    Success,
    Failure,
    Unknown,
}

/// Candidate path found in tool input or result data.
#[derive(Debug, Clone)]
pub struct PathCandidate<'a> {
    pub path: &'a str,
    pub absolute_path: PathBuf,
    pub project_relative_path: Option<PathBuf>,
    pub role: PathRole,
    pub source: DerivedSource,
    pub confidence: DerivedConfidence,
}

/// Why a candidate path appears relevant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathRole {
    ModifiedFile,
    ReadFile,
    Directory,
    Unknown,
}

/// Lossless native escape hatch for the post-tool semantic view.
#[derive(Debug, Clone, Copy)]
pub enum CommonPostToolUseNative<'a> {
    Claude(&'a claude::PostToolUse),
    Codex(&'a codex::PostToolUse),
    Gemini(&'a gemini::AfterTool),
}

/// Normalized post-tool view.
#[derive(Debug, Clone)]
pub struct CommonPostToolUseView<'a> {
    pub meta: CommonEventMeta<'a>,
    pub tool: CommonToolUseView<'a>,
    pub result: CommonToolResultView<'a>,
    pub path_candidates: Vec<PathCandidate<'a>>,
    pub native: CommonPostToolUseNative<'a>,
}

impl<'a> CommonPostToolUseView<'a> {
    pub fn path_candidates(
        &self,
        cwd: &Path,
        project_root: Option<&Path>,
    ) -> Vec<PathCandidate<'a>> {
        path_candidates_from_parts(
            self.tool.name,
            self.tool.input,
            self.result.value,
            cwd,
            project_root,
        )
    }

    pub fn modified_files(
        &self,
        cwd: &Path,
        project_root: Option<&Path>,
    ) -> Vec<PathCandidate<'a>> {
        self.path_candidates(cwd, project_root)
            .into_iter()
            .filter(|candidate| candidate.role == PathRole::ModifiedFile)
            .collect()
    }
}

impl CommonPostToolUseInput {
    /// Build a normalized view over this post-tool event.
    ///
    /// `raw_input` in the returned metadata is a placeholder because the
    /// lossless raw payload lives in `hookkit_runtime::RuntimeContext`.
    /// Use `view_with_raw` when the caller has the original JSON value.
    pub fn view(&self) -> CommonPostToolUseView<'_> {
        self.view_with_raw(&RAW_INPUT_UNAVAILABLE)
    }

    /// Build a normalized view with the original raw JSON payload attached.
    pub fn view_with_raw<'a>(
        &'a self,
        raw_input: &'a serde_json::Value,
    ) -> CommonPostToolUseView<'a> {
        let cwd = self.cwd();
        let cwd_path = Path::new(cwd);
        CommonPostToolUseView {
            meta: self.meta(raw_input),
            tool: CommonToolUseView {
                name: self.tool_name(),
                input: self.raw_tool_input(),
            },
            result: self.tool_result_view(),
            path_candidates: self.path_candidates(cwd_path, None),
            native: self.native_view(),
        }
    }

    pub fn tool_result(&self) -> Option<&serde_json::Value> {
        match self {
            Self::Claude(ev) => ev.tool_response.as_ref(),
            Self::Codex(ev) => ev.tool_result.as_ref(),
            Self::Gemini(ev) => ev.tool_response.as_ref(),
        }
    }

    pub fn execution_status(&self) -> ToolExecutionStatus {
        infer_status(self.tool_result())
    }

    pub fn path_candidates<'a>(
        &'a self,
        cwd: &Path,
        project_root: Option<&Path>,
    ) -> Vec<PathCandidate<'a>> {
        path_candidates_from_parts(
            self.tool_name(),
            self.raw_tool_input(),
            self.tool_result(),
            cwd,
            project_root,
        )
    }

    pub fn modified_files<'a>(
        &'a self,
        cwd: &Path,
        project_root: Option<&Path>,
    ) -> Vec<PathCandidate<'a>> {
        self.path_candidates(cwd, project_root)
            .into_iter()
            .filter(|candidate| candidate.role == PathRole::ModifiedFile)
            .collect()
    }

    fn meta<'a>(&'a self, raw_input: &'a serde_json::Value) -> CommonEventMeta<'a> {
        CommonEventMeta {
            harness: self.harness(),
            event: HookEventKey::PostToolUse,
            session_id: Some(self.session_id()),
            cwd: Some(self.cwd()),
            turn_id: self
                .common_extra_str("turnId")
                .or_else(|| self.common_extra_str("turn_id")),
            tool_use_id: self
                .native_tool_use_id()
                .or_else(|| self.common_extra_str("toolUseId"))
                .or_else(|| self.common_extra_str("tool_use_id")),
            raw_input,
        }
    }

    fn native_view(&self) -> CommonPostToolUseNative<'_> {
        match self {
            Self::Claude(ev) => CommonPostToolUseNative::Claude(ev),
            Self::Codex(ev) => CommonPostToolUseNative::Codex(ev),
            Self::Gemini(ev) => CommonPostToolUseNative::Gemini(ev),
        }
    }

    fn tool_result_view(&self) -> CommonToolResultView<'_> {
        let value = self.tool_result();
        CommonToolResultView {
            value,
            status: infer_status(value),
            stdout: find_string(value, &["stdout", "standardOutput"]),
            stderr: find_string(value, &["stderr", "standardError"]),
            exit_code: find_i64(value, &["exitCode", "exit_code", "code"]),
        }
    }

    fn harness(&self) -> Harness {
        match self {
            Self::Claude(_) => Harness::Claude,
            Self::Codex(_) => Harness::Codex,
            Self::Gemini(_) => Harness::Gemini,
        }
    }

    fn native_tool_use_id(&self) -> Option<&str> {
        match self {
            Self::Claude(ev) => ev.tool_use_id.as_deref(),
            Self::Codex(_) | Self::Gemini(_) => None,
        }
    }

    fn common_extra_str(&self, key: &str) -> Option<&str> {
        match self {
            Self::Claude(ev) => ev.common.extra.get(key),
            Self::Codex(ev) => ev.common.extra.get(key),
            Self::Gemini(ev) => ev.common.extra.get(key),
        }
        .and_then(|value| value.as_str())
    }
}

fn path_candidates_from_parts<'a>(
    tool_name: Option<&'a str>,
    tool_input: Option<&'a serde_json::Value>,
    tool_result: Option<&'a serde_json::Value>,
    cwd: &Path,
    project_root: Option<&Path>,
) -> Vec<PathCandidate<'a>> {
    let mut raw = Vec::new();
    if let Some(input) = tool_input {
        collect_path_candidates(input, SourceKind::ToolInput, tool_name, &mut raw);
    }
    if let Some(result) = tool_result {
        collect_path_candidates(result, SourceKind::ToolResult, tool_name, &mut raw);
    }

    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    for raw_candidate in raw {
        let absolute_path = normalize_path(&absolute_from(Path::new(raw_candidate.path), cwd));
        let dedupe_key = slash_path(&absolute_path);
        if !seen.insert(dedupe_key) {
            continue;
        }

        let project_relative_path = project_root
            .and_then(|root| absolute_path.strip_prefix(root).ok())
            .map(Path::to_path_buf);

        candidates.push(PathCandidate {
            path: raw_candidate.path,
            absolute_path,
            project_relative_path,
            role: raw_candidate.role,
            source: raw_candidate.source,
            confidence: raw_candidate.confidence,
        });
    }
    candidates
}

#[derive(Clone, Copy)]
enum SourceKind {
    ToolInput,
    ToolResult,
}

struct RawPathCandidate<'a> {
    path: &'a str,
    role: PathRole,
    source: DerivedSource,
    confidence: DerivedConfidence,
}

fn collect_path_candidates<'a>(
    value: &'a serde_json::Value,
    source_kind: SourceKind,
    tool_name: Option<&str>,
    out: &mut Vec<RawPathCandidate<'a>>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if let Some(path) = value.as_str().filter(|path| !path.trim().is_empty())
                    && let Some(candidate) = candidate_from_field(key, path, source_kind, tool_name)
                {
                    out.push(candidate);
                }
                collect_path_candidates(value, source_kind, tool_name, out);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                collect_path_candidates(value, source_kind, tool_name, out);
            }
        }
        _ => {}
    }
}

fn candidate_from_field<'a>(
    key: &str,
    path: &'a str,
    source_kind: SourceKind,
    tool_name: Option<&str>,
) -> Option<RawPathCandidate<'a>> {
    // Field names that strongly suggest the tool is targeting a specific file
    // (vs. just listing a path among many). Even so, the file is only
    // *modified* when the tool is known to write — Read also carries
    // `file_path` in its input but doesn't change the file. Without this
    // gate, the post-tool runner would re-format/re-lint files after Read.
    let is_target_field = matches!(
        key,
        "file_path" | "filePath" | "target_file" | "targetFile" | "absolute_path" | "absolutePath"
    );
    let is_path_field = key == "path";

    if !is_target_field && !is_path_field {
        return None;
    }

    let tool_writes = tool_name.is_some_and(is_known_file_writing_tool);
    let role = match (source_kind, is_target_field, is_path_field, tool_writes) {
        // Input from a known writer: targeted file is modified.
        (SourceKind::ToolInput, true, _, true) => PathRole::ModifiedFile,
        // Input from a non-writer (e.g. Read): targeted file is read, not
        // modified. Surface it as ReadFile so downstream filters can ignore
        // it for modify-only purposes while still seeing it for general
        // path-awareness.
        (SourceKind::ToolInput, true, _, false) => PathRole::ReadFile,
        // Plain `path` field in input is too ambiguous on its own; only treat
        // it as a modification when the tool is a known writer.
        (SourceKind::ToolInput, false, true, true) => PathRole::ModifiedFile,
        (SourceKind::ToolInput, false, true, false) => return None,
        // Tool result fields explicitly describe what the tool operated on.
        // For known writers, treat as modified; otherwise treat as a path
        // the tool referenced but did not write.
        (SourceKind::ToolResult, _, _, true) => PathRole::ModifiedFile,
        (SourceKind::ToolResult, true, _, false) => PathRole::ReadFile,
        // Plain `path` in a non-writer's result is too ambiguous.
        (SourceKind::ToolResult, false, true, false) => return None,
        _ => return None,
    };

    Some(RawPathCandidate {
        path,
        role,
        source: match source_kind {
            SourceKind::ToolInput => DerivedSource::ToolInputField(static_path_key(key)),
            SourceKind::ToolResult => DerivedSource::ToolResultField(static_path_key(key)),
        },
        confidence: DerivedConfidence::Exact,
    })
}

fn static_path_key(key: &str) -> &'static str {
    match key {
        "file_path" => "file_path",
        "filePath" => "filePath",
        "target_file" => "target_file",
        "targetFile" => "targetFile",
        "absolute_path" => "absolute_path",
        "absolutePath" => "absolutePath",
        "path" => "path",
        _ => "unknown",
    }
}

fn is_known_file_writing_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" | "write_file" | "replace" | "save_file"
    )
}

fn infer_status(value: Option<&serde_json::Value>) -> ToolExecutionStatus {
    let Some(value) = value else {
        return ToolExecutionStatus::Unknown;
    };

    if let Some(success) = find_bool(Some(value), &["success", "ok"]) {
        return if success {
            ToolExecutionStatus::Success
        } else {
            ToolExecutionStatus::Failure
        };
    }

    if let Some(exit_code) = find_i64(Some(value), &["exitCode", "exit_code"]) {
        return if exit_code == 0 {
            ToolExecutionStatus::Success
        } else {
            ToolExecutionStatus::Failure
        };
    }

    ToolExecutionStatus::Unknown
}

fn find_string<'a>(value: Option<&'a serde_json::Value>, keys: &[&str]) -> Option<&'a str> {
    let value = value?;
    match value {
        serde_json::Value::Object(map) => {
            for key in keys {
                if let Some(s) = map.get(*key).and_then(|value| value.as_str()) {
                    return Some(s);
                }
            }
            map.values()
                .find_map(|value| find_string(Some(value), keys))
        }
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(|value| find_string(Some(value), keys)),
        _ => None,
    }
}

fn find_bool(value: Option<&serde_json::Value>, keys: &[&str]) -> Option<bool> {
    let value = value?;
    match value {
        serde_json::Value::Object(map) => {
            for key in keys {
                if let Some(b) = map.get(*key).and_then(|value| value.as_bool()) {
                    return Some(b);
                }
            }
            map.values().find_map(|value| find_bool(Some(value), keys))
        }
        serde_json::Value::Array(values) => {
            values.iter().find_map(|value| find_bool(Some(value), keys))
        }
        _ => None,
    }
}

fn find_i64(value: Option<&serde_json::Value>, keys: &[&str]) -> Option<i64> {
    let value = value?;
    match value {
        serde_json::Value::Object(map) => {
            for key in keys {
                if let Some(i) = map.get(*key).and_then(|value| value.as_i64()) {
                    return Some(i);
                }
            }
            map.values().find_map(|value| find_i64(Some(value), keys))
        }
        serde_json::Value::Array(values) => {
            values.iter().find_map(|value| find_i64(Some(value), keys))
        }
        _ => None,
    }
}

fn absolute_from(path: &Path, base: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

fn slash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
