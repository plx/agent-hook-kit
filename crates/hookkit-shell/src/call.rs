//! Exact, profile-driven extraction of shell commands from native tool calls.
//!
//! Extraction intentionally does not probe alternate field names: a profile
//! either matches its documented tool name and JSON path or returns a typed
//! mismatch/error.

use std::{borrow::Cow, error::Error, fmt};

use hookkit_core::{EventId, HarnessId, Utf8Path};
use serde_json::{Map, Value};

/// Whether a shell call was observed before or after tool execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ToolPhase {
    /// Input observed before tool execution.
    Pre,
    /// Input and optional response observed after tool execution.
    Post,
}

/// Borrowed JSON supplied by a native hook contract.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum JsonRef<'a> {
    /// Borrowed arbitrary JSON value.
    Value(&'a Value),
    /// Borrowed JSON object guaranteed by a native contract.
    Object(&'a Map<String, Value>),
}

impl<'a> JsonRef<'a> {
    fn pointer(self, pointer: &str) -> Option<&'a Value> {
        match self {
            Self::Value(value) => value.pointer(pointer),
            Self::Object(object) => object_pointer(object, pointer),
        }
    }
}

/// An exact shell-tool shape, suitable for bundled or custom harnesses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellToolProfile {
    tool_name: Cow<'static, str>,
    command_pointer: Cow<'static, str>,
    cwd_pointer: Option<Cow<'static, str>>,
}

impl ShellToolProfile {
    #[cfg(any(feature = "claude", feature = "codex", feature = "antigravity"))]
    pub(crate) const fn builtin(
        tool_name: &'static str,
        command_pointer: &'static str,
        cwd_pointer: Option<&'static str>,
    ) -> Self {
        Self {
            tool_name: Cow::Borrowed(tool_name),
            command_pointer: Cow::Borrowed(command_pointer),
            cwd_pointer: match cwd_pointer {
                Some(pointer) => Some(Cow::Borrowed(pointer)),
                None => None,
            },
        }
    }

    /// Defines a profile with an exact tool name and nonempty JSON Pointer to
    /// its command field.
    pub fn new(
        tool_name: impl Into<Cow<'static, str>>,
        command_pointer: impl Into<Cow<'static, str>>,
    ) -> Result<Self, ShellToolProfileError> {
        let tool_name = tool_name.into();
        let command_pointer = command_pointer.into();
        if tool_name.is_empty() {
            return Err(ShellToolProfileError::EmptyToolName);
        }
        validate_pointer(&command_pointer)?;
        Ok(Self {
            tool_name,
            command_pointer,
            cwd_pointer: None,
        })
    }

    /// Adds an optional JSON Pointer from which a command-specific cwd is read.
    pub fn with_cwd_pointer(
        mut self,
        cwd_pointer: impl Into<Cow<'static, str>>,
    ) -> Result<Self, ShellToolProfileError> {
        let cwd_pointer = cwd_pointer.into();
        validate_pointer(&cwd_pointer)?;
        self.cwd_pointer = Some(cwd_pointer);
        Ok(self)
    }

    /// Returns the exact tool name matched by this profile.
    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    /// Returns the JSON Pointer locating the shell command string.
    pub fn command_pointer(&self) -> &str {
        &self.command_pointer
    }

    /// Returns the optional JSON Pointer locating a command-specific cwd.
    pub fn cwd_pointer(&self) -> Option<&str> {
        self.cwd_pointer.as_deref()
    }

    /// Extracts a call from a generic JSON value using this exact profile.
    pub fn extract_from_value<'a>(
        &self,
        event: EventId,
        phase: ToolPhase,
        tool_name: &'a str,
        tool_input: &'a Value,
        default_cwd: Option<&'a Utf8Path>,
        response: Option<&'a Value>,
    ) -> ShellToolCallMatch<'a> {
        self.extract(
            event,
            phase,
            tool_name,
            JsonRef::Value(tool_input),
            default_cwd,
            response.map(JsonRef::Value),
        )
    }

    /// Extracts a call from a generic JSON object using this exact profile.
    pub fn extract_from_object<'a>(
        &self,
        event: EventId,
        phase: ToolPhase,
        tool_name: &'a str,
        tool_input: &'a Map<String, Value>,
        default_cwd: Option<&'a Utf8Path>,
        response: Option<JsonRef<'a>>,
    ) -> ShellToolCallMatch<'a> {
        self.extract(
            event,
            phase,
            tool_name,
            JsonRef::Object(tool_input),
            default_cwd,
            response,
        )
    }

    fn extract<'a>(
        &self,
        event: EventId,
        phase: ToolPhase,
        tool_name: &'a str,
        tool_input: JsonRef<'a>,
        default_cwd: Option<&'a Utf8Path>,
        response: Option<JsonRef<'a>>,
    ) -> ShellToolCallMatch<'a> {
        if tool_name != self.tool_name {
            return ShellToolCallMatch::NotShell;
        }

        let harness = event.harness().clone();
        let Some(command_value) = tool_input.pointer(&self.command_pointer) else {
            return ShellToolCallMatch::Malformed(ShellToolCallError {
                harness,
                event,
                tool_name: tool_name.to_owned(),
                pointer: self.command_pointer.to_string(),
                kind: ShellToolCallErrorKind::MissingCommand,
            });
        };
        let Some(command) = command_value.as_str() else {
            return ShellToolCallMatch::Malformed(ShellToolCallError {
                harness,
                event,
                tool_name: tool_name.to_owned(),
                pointer: self.command_pointer.to_string(),
                kind: ShellToolCallErrorKind::CommandNotString,
            });
        };

        let cwd = match self.cwd_pointer.as_deref() {
            Some(pointer) => match tool_input.pointer(pointer) {
                Some(Value::String(cwd)) => Some(Utf8Path::new(cwd)),
                Some(_) => {
                    return ShellToolCallMatch::Malformed(ShellToolCallError {
                        harness,
                        event,
                        tool_name: tool_name.to_owned(),
                        pointer: pointer.to_owned(),
                        kind: ShellToolCallErrorKind::CwdNotString,
                    });
                }
                None => default_cwd,
            },
            None => default_cwd,
        };

        ShellToolCallMatch::Matched(ShellToolCallRef {
            harness,
            event,
            phase,
            tool_name,
            command,
            cwd,
            tool_input,
            response,
        })
    }
}

/// A borrowed, normalized shell call with its native JSON retained losslessly.
#[derive(Debug, Clone)]
pub struct ShellToolCallRef<'a> {
    /// Harness that owns the native event.
    pub harness: HarnessId,
    /// Exact native event identity.
    pub event: EventId,
    /// Whether the call was observed before or after execution.
    pub phase: ToolPhase,
    /// Exact harness-native tool name.
    pub tool_name: &'a str,
    /// Borrowed command source.
    pub command: &'a str,
    /// Command-specific or native-event working directory, if supplied.
    pub cwd: Option<&'a Utf8Path>,
    /// Complete native tool input.
    pub tool_input: JsonRef<'a>,
    /// Complete native response for post-tool observations, if supplied.
    pub response: Option<JsonRef<'a>>,
}

/// Result of matching a native hook input against its exact shell-tool profile.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ShellToolCallMatch<'a> {
    /// The native tool name does not match the profile.
    NotShell,
    /// A well-formed shell call matched the profile.
    Matched(ShellToolCallRef<'a>),
    /// The tool name matched but a required field was missing or malformed.
    Malformed(ShellToolCallError),
}

/// Extension implemented for supported native hook input types.
pub trait ShellToolCallExt {
    /// Attempts exact shell-tool extraction without guessing aliases or fields.
    fn shell_tool_call(&self) -> ShellToolCallMatch<'_>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Details of a malformed native tool call whose name matched a shell profile.
pub struct ShellToolCallError {
    /// Harness that owns the native event.
    pub harness: HarnessId,
    /// Exact native event identity.
    pub event: EventId,
    /// Matched harness-native tool name.
    pub tool_name: String,
    /// JSON Pointer at which extraction failed.
    pub pointer: String,
    /// Kind of malformed field.
    pub kind: ShellToolCallErrorKind,
}

/// Classification of a malformed shell-tool field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShellToolCallErrorKind {
    /// The configured command field was absent.
    MissingCommand,
    /// The configured command field existed but was not a string.
    CommandNotString,
    /// The configured cwd field existed but was not a string.
    CwdNotString,
}

impl fmt::Display for ShellToolCallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let problem = match self.kind {
            ShellToolCallErrorKind::MissingCommand => "command field is missing",
            ShellToolCallErrorKind::CommandNotString => "command field is not a string",
            ShellToolCallErrorKind::CwdNotString => "cwd field is not a string",
        };
        write!(
            formatter,
            "malformed {} shell tool {} at {}: {problem}",
            self.event, self.tool_name, self.pointer
        )
    }
}

impl Error for ShellToolCallError {}

impl ShellToolCallError {
    #[cfg(feature = "claude")]
    pub(crate) fn missing_command(event: EventId, tool_name: &str, pointer: &str) -> Self {
        Self {
            harness: event.harness().clone(),
            event,
            tool_name: tool_name.to_owned(),
            pointer: pointer.to_owned(),
            kind: ShellToolCallErrorKind::MissingCommand,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
/// Invalid [`ShellToolProfile`] definition.
pub enum ShellToolProfileError {
    /// The exact tool name was empty.
    EmptyToolName,
    /// A command or cwd pointer was not a valid nonempty JSON Pointer.
    InvalidJsonPointer(String),
}

impl fmt::Display for ShellToolProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyToolName => formatter.write_str("shell tool name must not be empty"),
            Self::InvalidJsonPointer(pointer) => {
                write!(formatter, "invalid JSON Pointer {pointer:?}")
            }
        }
    }
}

impl Error for ShellToolProfileError {}

fn validate_pointer(pointer: &str) -> Result<(), ShellToolProfileError> {
    if !pointer.starts_with('/') || pointer.split('/').skip(1).any(invalid_pointer_token) {
        return Err(ShellToolProfileError::InvalidJsonPointer(
            pointer.to_owned(),
        ));
    }
    Ok(())
}

fn invalid_pointer_token(token: &str) -> bool {
    let mut characters = token.chars();
    while let Some(character) = characters.next() {
        if character == '~' && !matches!(characters.next(), Some('0' | '1')) {
            return true;
        }
    }
    false
}

fn object_pointer<'a>(object: &'a Map<String, Value>, pointer: &str) -> Option<&'a Value> {
    let (first, rest) = pointer
        .strip_prefix('/')?
        .split_once('/')
        .unwrap_or((pointer.strip_prefix('/')?, ""));
    let first = unescape_pointer_token(first)?;
    let value = object.get(first.as_ref())?;
    if rest.is_empty() {
        Some(value)
    } else {
        value.pointer(&format!("/{rest}"))
    }
}

fn unescape_pointer_token(token: &str) -> Option<Cow<'_, str>> {
    if !token.contains('~') {
        return Some(Cow::Borrowed(token));
    }
    let mut output = String::with_capacity(token.len());
    let mut characters = token.chars();
    while let Some(character) = characters.next() {
        if character != '~' {
            output.push(character);
            continue;
        }
        output.push(match characters.next()? {
            '0' => '~',
            '1' => '/',
            _ => return None,
        });
    }
    Some(Cow::Owned(output))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn exact_profile_distinguishes_non_shell_and_malformed_calls() {
        let profile = ShellToolProfile::new("Bash", "/command").unwrap();
        let event = EventId::builtin(HarnessId::CODEX, "PreToolUse");
        let input = serde_json::json!({"command": 42});

        assert!(matches!(
            profile.extract_from_value(event.clone(), ToolPhase::Pre, "Read", &input, None, None,),
            ShellToolCallMatch::NotShell
        ));
        assert!(matches!(
            profile.extract_from_value(event, ToolPhase::Pre, "Bash", &input, None, None),
            ShellToolCallMatch::Malformed(ShellToolCallError {
                kind: ShellToolCallErrorKind::CommandNotString,
                ..
            })
        ));
    }

    #[test]
    fn object_profile_supports_nested_json_pointers_and_cwd_fallback() {
        let profile = ShellToolProfile::new("shell", "/request/command")
            .unwrap()
            .with_cwd_pointer("/request/cwd")
            .unwrap();
        let input = serde_json::json!({"request": {"command": "pwd"}});
        let object = input.as_object().unwrap();
        let event = EventId::builtin(HarnessId::builtin("custom"), "BeforeTool");
        let fallback = Utf8Path::new("/repo");

        let ShellToolCallMatch::Matched(call) = profile.extract_from_object(
            event,
            ToolPhase::Pre,
            "shell",
            object,
            Some(fallback),
            None,
        ) else {
            panic!("expected a shell call");
        };
        assert_eq!(call.command, "pwd");
        assert_eq!(call.cwd, Some(fallback));
    }

    #[test]
    fn profile_rejects_invalid_json_pointers() {
        assert!(matches!(
            ShellToolProfile::new("shell", "command"),
            Err(ShellToolProfileError::InvalidJsonPointer(_))
        ));
        assert!(matches!(
            ShellToolProfile::new("shell", "/bad~2token"),
            Err(ShellToolProfileError::InvalidJsonPointer(_))
        ));
    }

    proptest! {
        /// Property: profile extraction follows RFC 6901 rather than treating
        /// `/` and `~` inside object keys as path syntax. Value-backed and
        /// object-backed extraction must agree on the borrowed command.
        #[test]
        fn profiles_resolve_escaped_top_level_keys(
            key in "[a-z]{1,6}[/~][a-z]{0,6}",
            command in any::<String>(),
        ) {
            let pointer = format!("/{}", key.replace('~', "~0").replace('/', "~1"));
            let profile = ShellToolProfile::new("shell", pointer).unwrap();
            let input = serde_json::json!({key: command.clone()});
            let event = EventId::builtin(HarnessId::builtin("property"), "BeforeTool");

            let ShellToolCallMatch::Matched(from_value) = profile.extract_from_value(
                event.clone(), ToolPhase::Pre, "shell", &input, None, None,
            ) else {
                return Err(TestCaseError::fail("value-backed profile did not match"));
            };
            let ShellToolCallMatch::Matched(from_object) = profile.extract_from_object(
                event, ToolPhase::Pre, "shell", input.as_object().unwrap(), None, None,
            ) else {
                return Err(TestCaseError::fail("object-backed profile did not match"));
            };

            prop_assert_eq!(from_value.command, command.as_str());
            prop_assert_eq!(from_object.command, command.as_str());
        }

        /// Property: changing only the native tool name changes a valid shell
        /// payload from matched to NotShell, never to malformed.
        #[test]
        fn tool_name_matching_is_exact(command in any::<String>(), other in any::<String>()) {
            let profile = ShellToolProfile::new("shell", "/command").unwrap();
            let input = serde_json::json!({"command": command});
            let event = EventId::builtin(HarnessId::builtin("property"), "BeforeTool");
            let result = profile.extract_from_value(event, ToolPhase::Pre, &other, &input, None, None);

            if other == "shell" {
                prop_assert!(matches!(result, ShellToolCallMatch::Matched(_)));
            } else {
                prop_assert!(matches!(result, ShellToolCallMatch::NotShell));
            }
        }
    }
}
