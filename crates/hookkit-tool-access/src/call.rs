use crate::{AccessSource, PathBase, ToolAccessGap, ToolAccessGapReason};
use hookkit_common::{PostToolUseInput, PreToolUseInput};
use hookkit_core::{EventId, HarnessId, Utf8Path, Utf8PathBuf};
use hookkit_shell::{
    CLAUDE_BASH_PROFILE, CODEX_BASH_PROFILE, ShellToolCallExt, ShellToolCallMatch, ToolPhase,
};
use serde_json::{Map, Value};
use std::borrow::Cow;

/// Borrowed native JSON representation without object-to-value cloning.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum JsonRef<'a> {
    /// Any borrowed JSON value, which may or may not be an object.
    Value(&'a Value),
    /// A borrowed JSON object without allocating a wrapping [`Value`].
    Object(&'a Map<String, Value>),
}

impl<'a> JsonRef<'a> {
    /// Returns an object member by key.
    pub fn get(self, key: &str) -> Option<&'a Value> {
        match self {
            Self::Value(value) => value.get(key),
            Self::Object(object) => object.get(key),
        }
    }

    /// Resolves an RFC 6901 JSON Pointer relative to this value or object.
    ///
    /// Both arms follow [`Value::pointer`]: segments are unescaped by
    /// replacing `~1` and then `~0`, and array indices reject signs and
    /// leading zeros. The empty pointer denotes the whole document, which the
    /// `Object` arm cannot lend as a [`Value`], so it returns `None` there;
    /// use [`JsonRef::as_object`] to inspect the whole object instead.
    pub fn pointer(self, pointer: &str) -> Option<&'a Value> {
        match self {
            Self::Value(value) => value.pointer(pointer),
            Self::Object(object) => object_pointer(object, pointer),
        }
    }

    /// Returns the underlying object, if this reference denotes one.
    pub fn as_object(self) -> Option<&'a Map<String, Value>> {
        match self {
            Self::Value(value) => value.as_object(),
            Self::Object(object) => Some(object),
        }
    }

    /// Returns the underlying string, if this reference denotes one.
    pub fn as_str(self) -> Option<&'a str> {
        match self {
            Self::Value(value) => value.as_str(),
            Self::Object(_) => None,
        }
    }
}

/// Lossless borrowed view of an observable native tool call.
#[derive(Debug, Clone)]
pub struct ToolCallRef<'a> {
    /// Native harness event identity.
    pub event: EventId,
    /// Whether the observation precedes or follows tool execution.
    pub phase: ToolPhase,
    /// Native tool name.
    pub tool_name: &'a str,
    /// Native input without flattening or object cloning.
    pub tool_input: JsonRef<'a>,
    /// Working directory used to resolve relative paths, when supplied.
    pub cwd: Option<&'a Utf8Path>,
    /// How relative paths resolved against [`Self::cwd`] are labeled.
    ///
    /// [`PathBase::InvocationCwd`] means `cwd` is the tool's own working
    /// directory. [`PathBase::SessionCwd`] means the harness reports only a
    /// session or workspace directory and the tool may run elsewhere.
    pub cwd_base: PathBase,
    /// Observable workspace roots in native order.
    pub workspace_roots: Cow<'a, [Utf8PathBuf]>,
    /// Native post-tool response, when observable.
    pub response: Option<JsonRef<'a>>,
    /// Harness tool-call correlation identifier, when supplied.
    pub tool_call_id: Option<&'a str>,
    pub(crate) shell_call: ShellToolCallMatch<'a>,
}

impl<'a> ToolCallRef<'a> {
    /// Construct an observable custom tool call that is not pre-classified as
    /// a native shell call. `cwd` is treated as the invocation directory.
    pub fn new(
        event: EventId,
        phase: ToolPhase,
        tool_name: &'a str,
        tool_input: JsonRef<'a>,
        cwd: Option<&'a Utf8Path>,
        workspace_roots: impl Into<Cow<'a, [Utf8PathBuf]>>,
    ) -> Self {
        Self {
            event,
            phase,
            tool_name,
            tool_input,
            cwd,
            cwd_base: PathBase::InvocationCwd,
            workspace_roots: workspace_roots.into(),
            response: None,
            tool_call_id: None,
            shell_call: ShellToolCallMatch::NotShell,
        }
    }

    /// Attaches the native post-tool response.
    pub fn with_response(mut self, response: JsonRef<'a>) -> Self {
        self.response = Some(response);
        self
    }

    /// Attaches the harness tool-call correlation identifier.
    pub fn with_tool_call_id(mut self, tool_call_id: &'a str) -> Self {
        self.tool_call_id = Some(tool_call_id);
        self
    }

    /// Labels relative resolutions against [`Self::cwd`] with `base`, for
    /// example [`PathBase::SessionCwd`] when `cwd` is only a session directory.
    pub fn with_cwd_base(mut self, base: PathBase) -> Self {
        self.cwd_base = base;
        self
    }

    /// Returns the harness that emitted the event.
    pub fn harness(&self) -> &hookkit_core::HarnessId {
        self.event.harness()
    }
}

/// Result of adapting an aligned native event into an observable tool call.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ToolCallObservation<'a> {
    /// An observable tool call that can be analyzed.
    Call(ToolCallRef<'a>),
    /// A typed reason no tool call could be observed.
    Gap(ToolAccessGap),
}

impl<'a> ToolCallObservation<'a> {
    fn gap(reason: ToolAccessGapReason) -> Self {
        Self::Gap(ToolAccessGap {
            source: AccessSource::Structured,
            reason,
        })
    }
}

/// A native or aligned hook input that may carry an originating tool call.
///
/// Implementations borrow the native input, so typed single-harness hooks can
/// be analyzed without cloning them into an aligned wrapper.
pub trait ObservableToolCall {
    /// Borrows the originating tool call, or reports why none is observable.
    fn observe_tool_call(&self) -> ToolCallObservation<'_>;
}

impl ObservableToolCall for PreToolUseInput {
    fn observe_tool_call(&self) -> ToolCallObservation<'_> {
        observe_pre_tool(self)
    }
}

impl ObservableToolCall for PostToolUseInput {
    fn observe_tool_call(&self) -> ToolCallObservation<'_> {
        observe_post_tool(self)
    }
}

/// Claude Code tool events share the generic catalog input shape.
///
/// `PreToolUse` and `PermissionRequest` are observed before execution;
/// `PostToolUse` and `PostToolUseFailure` after it. Claude reports failed
/// calls, such as a Bash command that exits non-zero after writing files,
/// only through `PostToolUseFailure`. Other events yield
/// [`ToolAccessGapReason::MissingToolCall`].
impl ObservableToolCall for hookkit_claude::catalog::CatalogInput {
    fn observe_tool_call(&self) -> ToolCallObservation<'_> {
        let (event, phase) = match self.hook_event_name.as_str() {
            "PreToolUse" => ("PreToolUse", ToolPhase::Pre),
            "PermissionRequest" => ("PermissionRequest", ToolPhase::Pre),
            "PostToolUse" => ("PostToolUse", ToolPhase::Post),
            "PostToolUseFailure" => ("PostToolUseFailure", ToolPhase::Post),
            _ => return ToolCallObservation::gap(ToolAccessGapReason::MissingToolCall),
        };
        let Some(tool_name) = self.field("tool_name").and_then(Value::as_str) else {
            return ToolCallObservation::gap(ToolAccessGapReason::MissingToolName);
        };
        let Some(tool_input) = self.field("tool_input") else {
            return ToolCallObservation::gap(ToolAccessGapReason::MissingToolInput);
        };
        let response = self.field("tool_response");
        let event_id = EventId::builtin(HarnessId::CLAUDE_CODE, event);
        let shell_call = if event == "PreToolUse" {
            self.shell_tool_call()
        } else {
            CLAUDE_BASH_PROFILE.extract_from_value(
                event_id.clone(),
                phase,
                tool_name,
                tool_input,
                Some(&self.cwd),
                response,
            )
        };
        ToolCallObservation::Call(ToolCallRef {
            event: event_id,
            phase,
            tool_name,
            tool_input: JsonRef::Value(tool_input),
            cwd: Some(&self.cwd),
            cwd_base: PathBase::InvocationCwd,
            workspace_roots: Cow::Owned(vec![self.cwd.clone()]),
            response: response.map(JsonRef::Value),
            tool_call_id: self.field("tool_use_id").and_then(Value::as_str),
            shell_call,
        })
    }
}

impl ObservableToolCall for hookkit_claude::protocol::PostToolUseInput {
    fn observe_tool_call(&self) -> ToolCallObservation<'_> {
        ToolCallObservation::Call(ToolCallRef {
            event: EventId::builtin(HarnessId::CLAUDE_CODE, "PostToolUse"),
            phase: ToolPhase::Post,
            tool_name: &self.tool_name,
            tool_input: JsonRef::Value(&self.tool_input),
            cwd: Some(&self.cwd),
            cwd_base: PathBase::InvocationCwd,
            workspace_roots: Cow::Owned(vec![self.cwd.clone()]),
            response: Some(JsonRef::Value(&self.tool_response)),
            tool_call_id: Some(&self.tool_use_id),
            shell_call: self.shell_tool_call(),
        })
    }
}

impl ObservableToolCall for hookkit_codex::protocol::PreToolUseInput {
    fn observe_tool_call(&self) -> ToolCallObservation<'_> {
        ToolCallObservation::Call(ToolCallRef {
            event: EventId::builtin(HarnessId::CODEX, "PreToolUse"),
            phase: ToolPhase::Pre,
            tool_name: &self.tool_name,
            tool_input: JsonRef::Value(&self.tool_input),
            cwd: Some(&self.cwd),
            cwd_base: codex_cwd_base(&self.tool_name),
            workspace_roots: Cow::Owned(vec![self.cwd.clone()]),
            response: None,
            tool_call_id: Some(&self.tool_use_id),
            shell_call: self.shell_tool_call(),
        })
    }
}

impl ObservableToolCall for hookkit_codex::protocol::PostToolUseInput {
    fn observe_tool_call(&self) -> ToolCallObservation<'_> {
        ToolCallObservation::Call(ToolCallRef {
            event: EventId::builtin(HarnessId::CODEX, "PostToolUse"),
            phase: ToolPhase::Post,
            tool_name: &self.tool_name,
            tool_input: JsonRef::Value(&self.tool_input),
            cwd: Some(&self.cwd),
            cwd_base: codex_cwd_base(&self.tool_name),
            workspace_roots: Cow::Owned(vec![self.cwd.clone()]),
            response: Some(JsonRef::Value(&self.tool_response)),
            tool_call_id: Some(&self.tool_use_id),
            shell_call: self.shell_tool_call(),
        })
    }
}

impl ObservableToolCall for hookkit_antigravity::PreToolUseInput {
    fn observe_tool_call(&self) -> ToolCallObservation<'_> {
        ToolCallObservation::Call(ToolCallRef {
            event: EventId::builtin(HarnessId::ANTIGRAVITY, "PreToolUse"),
            phase: ToolPhase::Pre,
            tool_name: &self.tool_call.name,
            tool_input: JsonRef::Object(&self.tool_call.args),
            cwd: self.workspace_paths.first().map(Utf8PathBuf::as_path),
            cwd_base: PathBase::SessionCwd,
            workspace_roots: Cow::Borrowed(&self.workspace_paths),
            response: None,
            tool_call_id: None,
            shell_call: self.shell_tool_call(),
        })
    }
}

impl ObservableToolCall for hookkit_antigravity::PostToolUseInput {
    fn observe_tool_call(&self) -> ToolCallObservation<'_> {
        let Some(tool_call) = &self.tool_call else {
            return ToolCallObservation::gap(ToolAccessGapReason::MissingToolCall);
        };
        ToolCallObservation::Call(ToolCallRef {
            event: EventId::builtin(HarnessId::ANTIGRAVITY, "PostToolUse"),
            phase: ToolPhase::Post,
            tool_name: &tool_call.name,
            tool_input: JsonRef::Object(&tool_call.args),
            cwd: self.workspace_paths.first().map(Utf8PathBuf::as_path),
            cwd_base: PathBase::SessionCwd,
            workspace_roots: Cow::Borrowed(&self.workspace_paths),
            response: None,
            tool_call_id: None,
            shell_call: self.shell_tool_call(),
        })
    }
}

/// Codex hook payloads report the turn working directory. `apply_patch` and
/// structured tools resolve against it, but shell tools may override it with
/// a `workdir` argument that the `Bash` hook payload omits.
fn codex_cwd_base(tool_name: &str) -> PathBase {
    if tool_name == CODEX_BASH_PROFILE.tool_name() {
        PathBase::SessionCwd
    } else {
        PathBase::InvocationCwd
    }
}

/// Adapt an aligned pre-tool input without flattening its native arm.
pub fn observe_pre_tool(input: &PreToolUseInput) -> ToolCallObservation<'_> {
    match input {
        PreToolUseInput::Claude(native) => native.observe_tool_call(),
        PreToolUseInput::Codex(native) => native.observe_tool_call(),
        PreToolUseInput::Antigravity(native) => native.observe_tool_call(),
        _ => ToolCallObservation::gap(ToolAccessGapReason::UnknownInputArm),
    }
}

/// Adapt an aligned post-tool input without flattening its native arm.
pub fn observe_post_tool(input: &PostToolUseInput) -> ToolCallObservation<'_> {
    match input {
        PostToolUseInput::Claude(native) => native.observe_tool_call(),
        PostToolUseInput::Codex(native) => native.observe_tool_call(),
        PostToolUseInput::Antigravity(native) => native.observe_tool_call(),
        _ => ToolCallObservation::gap(ToolAccessGapReason::UnknownInputArm),
    }
}

fn object_pointer<'a>(object: &'a Map<String, Value>, pointer: &str) -> Option<&'a Value> {
    let mut segments = pointer
        .strip_prefix('/')?
        .split('/')
        .map(decode_pointer_segment);
    let mut current = object.get(&segments.next()?)?;
    for segment in segments {
        current = match current {
            Value::Object(map) => map.get(&segment)?,
            Value::Array(values) => values.get(parse_index(&segment)?)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Unescapes one segment exactly as [`Value::pointer`] does.
fn decode_pointer_segment(segment: &str) -> String {
    segment.replace("~1", "/").replace("~0", "~")
}

/// Parses an array index exactly as [`Value::pointer`] does.
fn parse_index(segment: &str) -> Option<usize> {
    if segment.starts_with('+') || (segment.starts_with('0') && segment.len() != 1) {
        return None;
    }
    segment.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_pointer_matches_serde_json_semantics() {
        let value = serde_json::json!({
            "a": [10, 11, 12],
            "b~": 3,
            "c/d": 4,
            "e~1": 5,
        });
        let object = value.as_object().unwrap();
        for pointer in [
            "/a/1", "/a/01", "/a/+1", "/a/-1", "/b~", "/c~1d", "/e~01", "/missing", "a", "/a/9",
        ] {
            assert_eq!(
                JsonRef::Object(object).pointer(pointer),
                JsonRef::Value(&value).pointer(pointer),
                "pointer `{pointer}`"
            );
        }
        assert!(JsonRef::Value(&value).pointer("").is_some());
        assert!(JsonRef::Object(object).pointer("").is_none());
    }
}
