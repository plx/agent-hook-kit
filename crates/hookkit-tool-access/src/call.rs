use crate::{AccessSource, ToolAccessGap, ToolAccessGapReason};
use hookkit_common::{PostToolUseInput, PreToolUseInput};
use hookkit_core::{EventId, HarnessId, Utf8Path, Utf8PathBuf};
use hookkit_shell::{ShellToolCallExt, ShellToolCallMatch, ToolPhase};
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
    /// Invocation working directory, when supplied by the harness.
    pub cwd: Option<&'a Utf8Path>,
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
    /// a native shell call.
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

/// Adapt an aligned pre-tool input without flattening its native arm.
pub fn observe_pre_tool(input: &PreToolUseInput) -> ToolCallObservation<'_> {
    match input {
        PreToolUseInput::Claude(native) => {
            let Some(tool_name) = native
                .field("tool_name")
                .and_then(serde_json::Value::as_str)
            else {
                return ToolCallObservation::gap(ToolAccessGapReason::MissingToolName);
            };
            let Some(tool_input) = native.field("tool_input") else {
                return ToolCallObservation::gap(ToolAccessGapReason::MissingToolInput);
            };
            ToolCallObservation::Call(ToolCallRef {
                event: EventId::builtin(HarnessId::CLAUDE_CODE, "PreToolUse"),
                phase: ToolPhase::Pre,
                tool_name,
                tool_input: JsonRef::Value(tool_input),
                cwd: Some(&native.cwd),
                workspace_roots: Cow::Owned(vec![native.cwd.clone()]),
                response: None,
                tool_call_id: native
                    .field("tool_use_id")
                    .and_then(serde_json::Value::as_str),
                shell_call: native.shell_tool_call(),
            })
        }
        PreToolUseInput::Codex(native) => ToolCallObservation::Call(ToolCallRef {
            event: EventId::builtin(HarnessId::CODEX, "PreToolUse"),
            phase: ToolPhase::Pre,
            tool_name: &native.tool_name,
            tool_input: JsonRef::Value(&native.tool_input),
            cwd: Some(&native.cwd),
            workspace_roots: Cow::Owned(vec![native.cwd.clone()]),
            response: None,
            tool_call_id: Some(&native.tool_use_id),
            shell_call: native.shell_tool_call(),
        }),
        PreToolUseInput::Gemini(native) => ToolCallObservation::Call(ToolCallRef {
            event: EventId::builtin(HarnessId::GEMINI_CLI, "BeforeTool"),
            phase: ToolPhase::Pre,
            tool_name: &native.tool_name,
            tool_input: JsonRef::Object(&native.tool_input),
            cwd: Some(&native.cwd),
            workspace_roots: Cow::Owned(vec![native.cwd.clone()]),
            response: None,
            tool_call_id: None,
            shell_call: native.shell_tool_call(),
        }),
        PreToolUseInput::Antigravity(native) => ToolCallObservation::Call(ToolCallRef {
            event: EventId::builtin(HarnessId::ANTIGRAVITY, "PreToolUse"),
            phase: ToolPhase::Pre,
            tool_name: &native.tool_call.name,
            tool_input: JsonRef::Object(&native.tool_call.args),
            cwd: native.workspace_paths.first().map(Utf8PathBuf::as_path),
            workspace_roots: Cow::Borrowed(&native.workspace_paths),
            response: None,
            tool_call_id: None,
            shell_call: native.shell_tool_call(),
        }),
        _ => ToolCallObservation::gap(ToolAccessGapReason::UnknownInputArm),
    }
}

/// Adapt an aligned post-tool input. Antigravity's current post-tool contract
/// intentionally yields a typed gap because it omits the originating call.
pub fn observe_post_tool(input: &PostToolUseInput) -> ToolCallObservation<'_> {
    match input {
        PostToolUseInput::Claude(native) => ToolCallObservation::Call(ToolCallRef {
            event: EventId::builtin(HarnessId::CLAUDE_CODE, "PostToolUse"),
            phase: ToolPhase::Post,
            tool_name: &native.tool_name,
            tool_input: JsonRef::Value(&native.tool_input),
            cwd: Some(&native.cwd),
            workspace_roots: Cow::Owned(vec![native.cwd.clone()]),
            response: Some(JsonRef::Value(&native.tool_response)),
            tool_call_id: Some(&native.tool_use_id),
            shell_call: native.shell_tool_call(),
        }),
        PostToolUseInput::Codex(native) => ToolCallObservation::Call(ToolCallRef {
            event: EventId::builtin(HarnessId::CODEX, "PostToolUse"),
            phase: ToolPhase::Post,
            tool_name: &native.tool_name,
            tool_input: JsonRef::Value(&native.tool_input),
            cwd: Some(&native.cwd),
            workspace_roots: Cow::Owned(vec![native.cwd.clone()]),
            response: Some(JsonRef::Value(&native.tool_response)),
            tool_call_id: Some(&native.tool_use_id),
            shell_call: native.shell_tool_call(),
        }),
        PostToolUseInput::Gemini(native) => ToolCallObservation::Call(ToolCallRef {
            event: EventId::builtin(HarnessId::GEMINI_CLI, "AfterTool"),
            phase: ToolPhase::Post,
            tool_name: &native.tool_name,
            tool_input: JsonRef::Object(&native.tool_input),
            cwd: Some(&native.cwd),
            workspace_roots: Cow::Owned(vec![native.cwd.clone()]),
            response: Some(JsonRef::Object(&native.tool_response)),
            tool_call_id: None,
            shell_call: native.shell_tool_call(),
        }),
        PostToolUseInput::Antigravity(_) => {
            ToolCallObservation::gap(ToolAccessGapReason::MissingToolCall)
        }
        _ => ToolCallObservation::gap(ToolAccessGapReason::UnknownInputArm),
    }
}

fn object_pointer<'a>(object: &'a Map<String, Value>, pointer: &str) -> Option<&'a Value> {
    if pointer.is_empty() {
        return None;
    }
    let mut segments = pointer.strip_prefix('/')?.split('/');
    let first = decode_pointer_segment(segments.next()?).ok()?;
    let mut current = object.get(&first)?;
    for segment in segments {
        let segment = decode_pointer_segment(segment).ok()?;
        current = match current {
            Value::Object(map) => map.get(&segment)?,
            Value::Array(values) => values.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

fn decode_pointer_segment(segment: &str) -> Result<String, ()> {
    let mut decoded = String::with_capacity(segment.len());
    let mut chars = segment.chars();
    while let Some(character) = chars.next() {
        if character != '~' {
            decoded.push(character);
            continue;
        }
        match chars.next() {
            Some('0') => decoded.push('~'),
            Some('1') => decoded.push('/'),
            _ => return Err(()),
        }
    }
    Ok(decoded)
}
