//! Exact, profile-driven extraction of shell commands from native tool calls.
//!
//! Extraction intentionally does not probe alternate field names: a profile
//! either matches its documented tool name and JSON path or returns a typed
//! mismatch/error.

use std::{borrow::Cow, error::Error, fmt};

use hookkit_core::{EventId, HarnessId, Utf8Path, Utf8PathBuf, normalize_utf8_path};
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

/// Shell language a native tool may run its command under.
///
/// The bundled analyzer parses Bash. Harness shell tools commonly run the
/// user's shell instead: Claude Code and Codex use zsh when it is the user's
/// shell (the macOS default), and Codex uses PowerShell on Windows. The native
/// payloads do not identify the shell, so the bundled adapters report
/// [`Self::Unknown`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ShellDialect {
    /// GNU Bash.
    Bash,
    /// zsh, whose redirection and expansion syntax differs from Bash in ways
    /// that can change file effects (`>!`, `2>&file`, a bare `< file`).
    Zsh,
    /// PowerShell, which the Bash analyzer cannot model.
    PowerShell,
    /// The shell is not identified. Bash-compatible analysis is best effort,
    /// and dialect-dependent syntax is reported as unresolved.
    #[default]
    Unknown,
}

/// Where a shell call's working directory came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ShellCwdOrigin {
    /// An absolute path read from the tool input's own working-directory
    /// field.
    ToolInput,
    /// A relative tool-input working directory joined onto the fallback cwd.
    /// Only [`ShellToolCallRef::effective_cwd`] exposes the joined path.
    ToolInputRelativeToFallback,
    /// The caller-supplied fallback (the hook event cwd or a workspace root),
    /// which the harness documents as the command's working directory.
    Fallback,
    /// The caller-supplied fallback is only the default working directory:
    /// the native tool accepts a working-directory argument that the hook
    /// payload does not expose, so the command may run elsewhere. Relative
    /// paths resolved against it are best effort.
    UnverifiedFallback,
    /// No absolute working directory is available.
    Unavailable,
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
    fallback_cwd_verified: bool,
    dialect: ShellDialect,
}

impl ShellToolProfile {
    #[cfg(any(feature = "claude", feature = "codex", feature = "antigravity"))]
    pub(crate) const fn builtin(
        tool_name: &'static str,
        command_pointer: &'static str,
        cwd_pointer: Option<&'static str>,
        fallback_cwd_verified: bool,
    ) -> Self {
        Self {
            tool_name: Cow::Borrowed(tool_name),
            command_pointer: Cow::Borrowed(command_pointer),
            cwd_pointer: match cwd_pointer {
                Some(pointer) => Some(Cow::Borrowed(pointer)),
                None => None,
            },
            fallback_cwd_verified,
            dialect: ShellDialect::Unknown,
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
            fallback_cwd_verified: true,
            dialect: ShellDialect::Unknown,
        })
    }

    /// Adds an optional JSON Pointer from which a command-specific cwd is read.
    ///
    /// An absolute value overrides the fallback cwd. A relative value is joined
    /// onto an absolute fallback and exposed only through
    /// [`ShellToolCallRef::effective_cwd`]; an empty or `null` value is treated
    /// as absent.
    pub fn with_cwd_pointer(
        mut self,
        cwd_pointer: impl Into<Cow<'static, str>>,
    ) -> Result<Self, ShellToolProfileError> {
        let cwd_pointer = cwd_pointer.into();
        validate_pointer(&cwd_pointer)?;
        self.cwd_pointer = Some(cwd_pointer);
        Ok(self)
    }

    /// Marks the fallback cwd as only the tool's default working directory,
    /// for tools that accept a working-directory argument the hook payload
    /// does not expose. Matched calls then report
    /// [`ShellCwdOrigin::UnverifiedFallback`].
    pub fn with_unverified_fallback_cwd(mut self) -> Self {
        self.fallback_cwd_verified = false;
        self
    }

    /// Sets the shell dialect reported on matched calls.
    pub fn with_dialect(mut self, dialect: ShellDialect) -> Self {
        self.dialect = dialect;
        self
    }

    /// Returns whether the fallback cwd is documented as the command's
    /// working directory.
    pub fn fallback_cwd_verified(&self) -> bool {
        self.fallback_cwd_verified
    }

    /// Returns the shell dialect reported on matched calls.
    pub fn dialect(&self) -> ShellDialect {
        self.dialect
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

        // Only an absolute directory is a usable base for relative paths.
        let fallback = default_cwd.filter(|cwd| cwd.is_absolute());
        let fallback_origin = match fallback {
            Some(_) if self.fallback_cwd_verified => ShellCwdOrigin::Fallback,
            Some(_) => ShellCwdOrigin::UnverifiedFallback,
            None => ShellCwdOrigin::Unavailable,
        };
        let mut joined_cwd = None;
        let (cwd, cwd_origin) = match self.cwd_pointer.as_deref() {
            Some(pointer) => match tool_input.pointer(pointer) {
                Some(Value::String(cwd)) if cwd.is_empty() => (fallback, fallback_origin),
                Some(Value::String(cwd)) if Utf8Path::new(cwd).is_absolute() => {
                    (Some(Utf8Path::new(cwd)), ShellCwdOrigin::ToolInput)
                }
                Some(Value::String(cwd)) => match fallback {
                    Some(base) => {
                        joined_cwd = Some(normalize_utf8_path(base.join(cwd)));
                        (None, ShellCwdOrigin::ToolInputRelativeToFallback)
                    }
                    None => (None, ShellCwdOrigin::Unavailable),
                },
                Some(Value::Null) | None => (fallback, fallback_origin),
                Some(_) => {
                    return ShellToolCallMatch::Malformed(ShellToolCallError {
                        harness,
                        event,
                        tool_name: tool_name.to_owned(),
                        pointer: pointer.to_owned(),
                        kind: ShellToolCallErrorKind::CwdNotString,
                    });
                }
            },
            None => (fallback, fallback_origin),
        };

        ShellToolCallMatch::Matched(ShellToolCallRef {
            harness,
            event,
            phase,
            tool_name,
            command,
            cwd,
            cwd_origin,
            dialect: self.dialect,
            tool_input,
            response,
            joined_cwd,
        })
    }
}

/// A borrowed, normalized shell call with its native JSON retained losslessly.
#[derive(Debug, Clone)]
#[non_exhaustive]
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
    /// Absolute command-specific or fallback working directory, if available.
    ///
    /// This is `None` when the tool input supplied a relative working
    /// directory; [`Self::effective_cwd`] also covers that case. Check
    /// [`Self::cwd_origin`] before treating the value as authoritative.
    pub cwd: Option<&'a Utf8Path>,
    /// Where [`Self::cwd`] or [`Self::effective_cwd`] came from.
    pub cwd_origin: ShellCwdOrigin,
    /// Shell language the native tool may run the command under.
    pub dialect: ShellDialect,
    /// Complete native tool input.
    pub tool_input: JsonRef<'a>,
    /// Complete native response for post-tool observations, if supplied.
    pub response: Option<JsonRef<'a>>,
    joined_cwd: Option<Utf8PathBuf>,
}

impl ShellToolCallRef<'_> {
    /// Returns the best available working directory, including a relative
    /// tool-input directory joined onto the fallback cwd.
    pub fn effective_cwd(&self) -> Option<&Utf8Path> {
        self.cwd.or(self.joined_cwd.as_deref())
    }
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
    /// The native input's event is not a tool event the adapter supports, so
    /// whether it describes a shell call is unknown.
    UnsupportedEvent,
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
    let tokens = pointer.strip_prefix('/')?;
    match tokens.split_once('/') {
        // `/a/` addresses `object["a"][""]`, so an empty remainder is still a
        // token to resolve.
        Some((first, rest)) => object
            .get(unescape_pointer_token(first)?.as_ref())?
            .pointer(&format!("/{rest}")),
        None => object.get(unescape_pointer_token(tokens)?.as_ref()),
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
    fn object_pointers_resolve_a_trailing_empty_token() {
        let profile = ShellToolProfile::new("shell", "/args/").unwrap();
        let input = serde_json::json!({"args": {"": "cmd"}});
        let event = EventId::builtin(HarnessId::builtin("custom"), "BeforeTool");

        for matched in [
            profile.extract_from_value(event.clone(), ToolPhase::Pre, "shell", &input, None, None),
            profile.extract_from_object(
                event,
                ToolPhase::Pre,
                "shell",
                input.as_object().unwrap(),
                None,
                None,
            ),
        ] {
            let ShellToolCallMatch::Matched(call) = matched else {
                panic!("expected a shell call");
            };
            assert_eq!(call.command, "cmd");
        }
    }

    #[test]
    fn cwd_origin_distinguishes_tool_input_fallback_and_relative_values() {
        let profile = ShellToolProfile::new("shell", "/command")
            .unwrap()
            .with_cwd_pointer("/cwd")
            .unwrap();
        let event = EventId::builtin(HarnessId::builtin("custom"), "BeforeTool");
        let fallback = Utf8Path::new("/repo");
        let call = |input: serde_json::Value, default: Option<&'static str>| {
            let input = Box::leak(Box::new(input));
            match profile.extract_from_value(
                event.clone(),
                ToolPhase::Pre,
                "shell",
                input,
                default.map(Utf8Path::new),
                None,
            ) {
                ShellToolCallMatch::Matched(call) => call,
                other => panic!("expected a shell call, got {other:?}"),
            }
        };

        let absolute = call(
            serde_json::json!({"command": "x", "cwd": "/work"}),
            Some("/repo"),
        );
        assert_eq!(absolute.cwd, Some(Utf8Path::new("/work")));
        assert_eq!(absolute.cwd_origin, ShellCwdOrigin::ToolInput);

        let relative = call(
            serde_json::json!({"command": "x", "cwd": "sub/.."}),
            Some("/repo"),
        );
        assert_eq!(relative.cwd, None);
        assert_eq!(relative.effective_cwd(), Some(fallback));
        assert_eq!(
            relative.cwd_origin,
            ShellCwdOrigin::ToolInputRelativeToFallback
        );

        for input in [
            serde_json::json!({"command": "x"}),
            serde_json::json!({"command": "x", "cwd": ""}),
            serde_json::json!({"command": "x", "cwd": null}),
        ] {
            let fallen_back = call(input, Some("/repo"));
            assert_eq!(fallen_back.cwd, Some(fallback));
            assert_eq!(fallen_back.cwd_origin, ShellCwdOrigin::Fallback);
        }

        let relative_fallback = call(serde_json::json!({"command": "x"}), Some("repo"));
        assert_eq!(relative_fallback.cwd, None);
        assert_eq!(relative_fallback.cwd_origin, ShellCwdOrigin::Unavailable);

        let unverified = ShellToolProfile::new("shell", "/command")
            .unwrap()
            .with_unverified_fallback_cwd()
            .with_dialect(ShellDialect::Zsh);
        let input = serde_json::json!({"command": "x"});
        let ShellToolCallMatch::Matched(call) = unverified.extract_from_value(
            event.clone(),
            ToolPhase::Pre,
            "shell",
            &input,
            Some(fallback),
            None,
        ) else {
            panic!("expected a shell call");
        };
        assert_eq!(call.cwd_origin, ShellCwdOrigin::UnverifiedFallback);
        assert_eq!(call.dialect, ShellDialect::Zsh);
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
