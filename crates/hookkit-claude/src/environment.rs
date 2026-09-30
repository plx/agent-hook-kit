//! Claude Code command-hook environment contract.
//!
//! This module models only variables Claude Code documents as hook subprocess
//! state. Inherited provider settings and undocumented implementation details
//! are intentionally excluded.
//!
//! | Variables | Presence | Parsed as |
//! | :- | :- | :- |
//! | `CLAUDECODE`, `CLAUDE_CODE_CHILD_SESSION` | every event | required, exactly `1` |
//! | `CLAUDE_CODE_SESSION_ID` | every event | required; must equal the input `session_id` |
//! | `CLAUDE_PROJECT_DIR` | every event | required path |
//! | `CLAUDE_ENV_FILE` | [`ENVIRONMENT_FILE_EVENTS`] only | optional path |
//! | `CLAUDE_EFFORT` | when the model supports effort | optional; compared with the input `effort.level` |
//! | `TRACEPARENT` | when trace context propagates | optional, may be empty |
//! | `CLAUDE_PID` | every event, Claude Code v2.1.214 or later | optional positive integer |
//! | `CLAUDE_CODE_REMOTE`, `CLAUDE_CODE_REMOTE_SESSION_ID` | cloud sessions | the marker `true` requires the id |
//! | `CLAUDE_CODE_BRIDGE_SESSION_ID` | local sessions with Remote Control | optional |
//! | `CLAUDE_CODE_MESSAGING_SOCKET`, `CLAUDE_CODE_MESSAGING_TOKEN` | sessions with an inbox socket | optional; the token only with the socket |
//! | `CLAUDE_PLUGIN_ROOT`, `CLAUDE_PLUGIN_DATA`, `CLAUDE_PLUGIN_OPTION_*` | plugin hooks | the complete pair, then its options |
//!
//! Hook processes inherit the parent environment, so a variable can be
//! present without Claude Code having set it for this hook (for example in a
//! Claude Code session started from another session's shell, or a value the
//! user exported). Parsing therefore fails only on state Claude Code itself
//! would never produce for a hook: a missing or wrong baseline variable, a
//! cloud-session marker without its session id, an empty value in a complete
//! plugin pair or an empty plugin option name, and a malformed `CLAUDE_PID`.
//! Other optional state that is empty or incomplete is treated as absent: an
//! empty `CLAUDE_ENV_FILE`, `CLAUDE_EFFORT`, bridge id, or messaging
//! variable, a `CLAUDE_CODE_REMOTE` other than `true`, a lone plugin
//! variable, plugin options without the plugin pair, and a messaging token
//! without a socket.

use hookkit_core::{
    CommandEnvironmentSpec, EnvironmentVariables, EventId, HarnessId, HookkitError, NativeContext,
    SessionId, Utf8PathBuf,
};
use std::collections::BTreeMap;
use std::fmt;

/// Native events for which Claude exposes a `CLAUDE_ENV_FILE` path.
///
/// The hooks reference documents `CLAUDE_ENV_FILE` for exactly these events
/// ("available for SessionStart, Setup, CwdChanged, and FileChanged hooks";
/// the environment-variable reference says the same). No sentence promises
/// it is always set, and the reference examples guard it with
/// `[ -n "$CLAUDE_ENV_FILE" ]`, so it is parsed as optional: an absent or
/// empty value yields `None` rather than failing the hook. It is ignored on
/// every other event, including `PreModelSwitch` and `PostModelSwitch`.
///
/// Variables a `CwdChanged` or `FileChanged` hook writes to the file persist
/// into later Bash commands only until the next `CwdChanged` event, when
/// Claude Code clears them; `SessionStart` and `Setup` exports persist for
/// the session.
pub const ENVIRONMENT_FILE_EVENTS: &[&str] =
    &["SessionStart", "Setup", "CwdChanged", "FileChanged"];

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
/// Whether the hook runs in a local or a cloud Claude Code session.
pub enum ClaudeExecutionLocation {
    /// A local Claude Code process.
    Local {
        /// Remote-control bridge session, when the local process is controlled
        /// from another client.
        remote_control_session_id: Option<String>,
    },
    /// A Claude Code cloud session, marked by `CLAUDE_CODE_REMOTE=true`: an
    /// Anthropic-hosted session on the web or, since Claude Code v2.1.224, a
    /// session on a self-hosted runner.
    Cloud {
        /// Native cloud session identifier from
        /// `CLAUDE_CODE_REMOTE_SESSION_ID`.
        remote_session_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
/// Effort level supplied by Claude's hook environment.
pub enum ClaudeEffort {
    /// Low effort.
    Low,
    /// Medium effort.
    Medium,
    /// High effort.
    High,
    /// Extra-high effort.
    Xhigh,
    /// Maximum effort.
    Max,
    /// Forward-compatible value not known to this snapshot.
    Unknown(String),
}

impl ClaudeEffort {
    /// Returns the exact environment string for this effort level.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
            Self::Unknown(value) => value,
        }
    }
}

/// Raw plugin option strings keyed by their exported, uppercased suffix.
/// Values are intentionally redacted from `Debug`.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ClaudePluginOptions(BTreeMap<String, String>);

impl ClaudePluginOptions {
    /// Returns the raw value for an uppercased option suffix.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    /// Iterates through option names and values in lexicographic name order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }

    /// Iterates through option names in lexicographic order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }

    /// Reports whether no plugin options were exported.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for ClaudePluginOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClaudePluginOptions")
            .field("names", &self.0.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Paths and options injected for a plugin-provided Claude hook.
pub struct ClaudePluginEnvironment {
    /// Root directory containing the installed plugin.
    pub root: Utf8PathBuf,
    /// Plugin-specific persistent data directory.
    pub data: Utf8PathBuf,
    /// Raw plugin options with values redacted from `Debug`.
    pub options: ClaudePluginOptions,
}

/// Per-session cross-session messaging token. The value is a secret and is
/// redacted from `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct ClaudeMessagingToken(String);

impl ClaudeMessagingToken {
    /// Returns the raw token, for example to send it as the socket's first
    /// `{"type":"auth","token":...}` line.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ClaudeMessagingToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ClaudeMessagingToken(<redacted>)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
/// Cross-session messaging state exported in sessions that bind an inbox
/// socket.
pub struct ClaudeMessagingEnvironment {
    /// Inbox socket path from `CLAUDE_CODE_MESSAGING_SOCKET` (Claude Code
    /// v2.1.224 or later).
    pub socket: Utf8PathBuf,
    /// Per-session token from `CLAUDE_CODE_MESSAGING_TOKEN` (Claude Code
    /// v2.1.228 or later). `None` on v2.1.224 through v2.1.227, which export
    /// only the socket.
    pub token: Option<ClaudeMessagingToken>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
/// Native process environment supplied to a Claude Code command hook.
pub struct ClaudeCommandEnvironment {
    /// Project root from `CLAUDE_PROJECT_DIR`: where the session started. It
    /// does not follow Claude into a worktree; the input `cwd` does.
    pub project_dir: Utf8PathBuf,
    /// Native session identity from `CLAUDE_CODE_SESSION_ID`.
    pub session_id: SessionId,
    /// Writable environment file from `CLAUDE_ENV_FILE`, exposed only for
    /// [`ENVIRONMENT_FILE_EVENTS`]. `None` off that list, and when the
    /// variable is absent or empty.
    pub environment_file: Option<Utf8PathBuf>,
    /// Local, remote-control, or cloud execution context.
    pub execution_location: ClaudeExecutionLocation,
    /// Plugin paths and options when the hook belongs to a plugin.
    pub plugin: Option<ClaudePluginEnvironment>,
    /// Optional effort level, preserving forward-compatible unknown values.
    pub effort: Option<ClaudeEffort>,
    /// Optional W3C trace context supplied by Claude.
    pub traceparent: Option<String>,
    /// Claude Code's own process ID from `CLAUDE_PID`, set for every hook
    /// command since Claude Code v2.1.214. `None` on older releases.
    pub claude_pid: Option<u32>,
    /// Cross-session messaging socket and token, present only in sessions
    /// that bind an inbox socket. Boxed because it is rarely present, which
    /// keeps the environment small inside harness sum types.
    pub messaging: Option<Box<ClaudeMessagingEnvironment>>,
}

impl ClaudeCommandEnvironment {
    /// Parses Claude's declared hook variables for `event`.
    ///
    /// Fixed marker values, the cloud-session pair, a complete plugin pair,
    /// and `CLAUDE_PID` are validated; see the module documentation for how
    /// partial optional state is treated.
    pub fn from_map(
        event: &EventId,
        variables: &EnvironmentVariables,
    ) -> hookkit_core::Result<Self> {
        <Self as CommandEnvironmentSpec>::from_variables(event, variables)
    }

    /// Cross-checks session and effort values duplicated in native JSON input.
    ///
    /// Session identity must always agree. Effort is compared only when both
    /// the environment and event payload supply it.
    pub fn validate_input(
        &self,
        event: &EventId,
        session_id: &str,
        effort: Option<&str>,
    ) -> hookkit_core::Result<()> {
        if self.session_id.as_str() != session_id {
            return Err(mismatch(
                event,
                "CLAUDE_CODE_SESSION_ID does not match input session_id",
            ));
        }
        if let (Some(environment), Some(input)) = (self.effort.as_ref(), effort) {
            if environment.as_str() != input {
                return Err(mismatch(
                    event,
                    "CLAUDE_EFFORT does not match input effort.level",
                ));
            }
        }
        Ok(())
    }
}

impl CommandEnvironmentSpec for ClaudeCommandEnvironment {
    const VARIABLE_NAMES: &'static [&'static str] = &[
        "CLAUDECODE",
        "CLAUDE_CODE_CHILD_SESSION",
        "CLAUDE_CODE_SESSION_ID",
        "CLAUDE_PROJECT_DIR",
        "CLAUDE_ENV_FILE",
        "CLAUDE_EFFORT",
        "TRACEPARENT",
        "CLAUDE_CODE_REMOTE",
        "CLAUDE_CODE_REMOTE_SESSION_ID",
        "CLAUDE_CODE_BRIDGE_SESSION_ID",
        "CLAUDE_PLUGIN_ROOT",
        "CLAUDE_PLUGIN_DATA",
        "CLAUDE_PID",
        "CLAUDE_CODE_MESSAGING_SOCKET",
        "CLAUDE_CODE_MESSAGING_TOKEN",
    ];
    const VARIABLE_PREFIXES: &'static [&'static str] = &["CLAUDE_PLUGIN_OPTION_"];

    fn from_variables(
        event: &EventId,
        variables: &EnvironmentVariables,
    ) -> hookkit_core::Result<Self> {
        if event.harness() != &HarnessId::CLAUDE_CODE {
            return Err(HookkitError::EventHarnessMismatch {
                harness: HarnessId::CLAUDE_CODE,
                event: event.clone(),
            });
        }

        require_marker(event, variables, "CLAUDECODE", "1")?;
        require_marker(event, variables, "CLAUDE_CODE_CHILD_SESSION", "1")?;
        let session_id = SessionId::new(required(event, variables, "CLAUDE_CODE_SESSION_ID")?)?;
        let project_dir = Utf8PathBuf::from(required(event, variables, "CLAUDE_PROJECT_DIR")?);

        // Documented for exactly these events, but never promised to be
        // present; an empty value is the documented "unset" guard's case.
        let environment_file = if ENVIRONMENT_FILE_EVENTS.contains(&event.name()) {
            non_empty(variables, "CLAUDE_ENV_FILE").map(Utf8PathBuf::from)
        } else {
            None
        };

        // Claude Code sets CLAUDE_CODE_REMOTE to exactly "true" in cloud
        // sessions and leaves it unset locally. Any other value, and a cloud
        // session id without the marker, is inherited ambient state.
        let execution_location = if variables.get("CLAUDE_CODE_REMOTE") == Some("true") {
            // A cloud session is never controlled through the local bridge;
            // an inherited bridge id is ignored.
            ClaudeExecutionLocation::Cloud {
                remote_session_id: required(event, variables, "CLAUDE_CODE_REMOTE_SESSION_ID")?
                    .to_owned(),
            }
        } else {
            ClaudeExecutionLocation::Local {
                remote_control_session_id: non_empty(variables, "CLAUDE_CODE_BRIDGE_SESSION_ID")
                    .map(str::to_owned),
            }
        };

        // Only a complete, non-empty CLAUDE_PLUGIN_ROOT + CLAUDE_PLUGIN_DATA pair
        // identifies a genuine plugin hook. A lone value (with or without stray
        // CLAUDE_PLUGIN_OPTION_* entries) is treated as ambient state inherited
        // from an unrelated parent process — for example, a Claude Code session
        // that recursively invokes another one — rather than a hard failure,
        // mirroring how hookkit-codex already treats a partial Claude-compatible
        // alias set as ambient noise instead of an error. Plugin options are
        // validated only for a genuine plugin hook.
        let plugin = match (
            variables.get("CLAUDE_PLUGIN_ROOT"),
            variables.get("CLAUDE_PLUGIN_DATA"),
        ) {
            (Some(root), Some(data)) => {
                if root.is_empty() || data.is_empty() {
                    return Err(invalid(
                        event,
                        "CLAUDE_PLUGIN_ROOT and CLAUDE_PLUGIN_DATA must be non-empty when both are present",
                    ));
                }
                let mut options = BTreeMap::new();
                for (name, value) in variables.iter() {
                    let Some(key) = name.strip_prefix("CLAUDE_PLUGIN_OPTION_") else {
                        continue;
                    };
                    if key.is_empty() {
                        return Err(invalid(event, "plugin option name must not be empty"));
                    }
                    options.insert(key.to_owned(), value.to_owned());
                }
                Some(ClaudePluginEnvironment {
                    root: root.into(),
                    data: data.into(),
                    options: ClaudePluginOptions(options),
                })
            }
            _ => None,
        };

        // Claude Code sets CLAUDE_EFFORT only when the model supports effort,
        // and never empty; an empty value is inherited state.
        let effort = non_empty(variables, "CLAUDE_EFFORT").map(|value| match value {
            "low" => ClaudeEffort::Low,
            "medium" => ClaudeEffort::Medium,
            "high" => ClaudeEffort::High,
            "xhigh" => ClaudeEffort::Xhigh,
            "max" => ClaudeEffort::Max,
            value => ClaudeEffort::Unknown(value.to_owned()),
        });

        let claude_pid = non_empty(variables, "CLAUDE_PID")
            .map(|value| parse_pid(event, value))
            .transpose()?;

        // Claude Code exports the socket when it binds one and the token
        // alongside it (v2.1.228+). A token without a socket, or an empty
        // value, is inherited ambient state.
        let messaging = non_empty(variables, "CLAUDE_CODE_MESSAGING_SOCKET").map(|socket| {
            Box::new(ClaudeMessagingEnvironment {
                socket: socket.into(),
                token: non_empty(variables, "CLAUDE_CODE_MESSAGING_TOKEN")
                    .map(|token| ClaudeMessagingToken(token.to_owned())),
            })
        });

        Ok(Self {
            project_dir,
            session_id,
            environment_file,
            execution_location,
            plugin,
            effort,
            traceparent: variables.get("TRACEPARENT").map(str::to_owned),
            claude_pid,
            messaging,
        })
    }

    fn validate_context(
        &self,
        event: &EventId,
        context: &NativeContext,
    ) -> hookkit_core::Result<()> {
        if context.session_id.as_ref() == Some(&self.session_id) {
            Ok(())
        } else {
            Err(mismatch(
                event,
                "CLAUDE_CODE_SESSION_ID does not match input session_id",
            ))
        }
    }
}

fn non_empty<'a>(variables: &'a EnvironmentVariables, name: &str) -> Option<&'a str> {
    variables.get(name).filter(|value| !value.is_empty())
}

fn parse_pid(event: &EventId, value: &str) -> hookkit_core::Result<u32> {
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid(event, "CLAUDE_PID must be a decimal process id"));
    }
    match value.parse::<u32>() {
        Ok(pid) if pid > 0 => Ok(pid),
        _ => Err(invalid(event, "CLAUDE_PID must be a positive process id")),
    }
}

fn required<'a>(
    event: &EventId,
    variables: &'a EnvironmentVariables,
    name: &'static str,
) -> hookkit_core::Result<&'a str> {
    match variables.get(name) {
        Some(value) if !value.is_empty() => Ok(value),
        Some(_) => Err(invalid(event, format!("{name} must not be empty"))),
        None => Err(invalid(event, format!("missing required {name}"))),
    }
}

fn require_marker(
    event: &EventId,
    variables: &EnvironmentVariables,
    name: &'static str,
    expected: &'static str,
) -> hookkit_core::Result<()> {
    match variables.get(name) {
        Some(actual) if actual == expected => Ok(()),
        Some(_) => Err(invalid(event, format!("{name} must be exactly {expected}"))),
        None => Err(invalid(event, format!("missing required {name}"))),
    }
}

fn invalid(event: &EventId, message: impl Into<String>) -> HookkitError {
    HookkitError::InvalidHookEnvironment {
        event: event.clone(),
        message: message.into(),
    }
}

fn mismatch(event: &EventId, message: impl Into<String>) -> HookkitError {
    HookkitError::EnvironmentContextMismatch {
        event: event.clone(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVENTS: &[&str] = &[
        "SessionStart",
        "Setup",
        "InstructionsLoaded",
        "UserPromptSubmit",
        "UserPromptExpansion",
        "MessageDisplay",
        "PreToolUse",
        "PermissionRequest",
        "PostToolUse",
        "PostToolUseFailure",
        "PostToolBatch",
        "PermissionDenied",
        "Notification",
        "SubagentStart",
        "SubagentStop",
        "TaskCreated",
        "TaskCompleted",
        "Stop",
        "StopFailure",
        "TeammateIdle",
        "ConfigChange",
        "CwdChanged",
        "DirectoryAdded",
        "FileChanged",
        "WorktreeCreate",
        "WorktreeRemove",
        "PreCompact",
        "PostCompact",
        "PreModelSwitch",
        "PostModelSwitch",
        "SessionEnd",
        "Elicitation",
        "ElicitationResult",
    ];

    fn event(name: &'static str) -> EventId {
        EventId::builtin(HarnessId::CLAUDE_CODE, name)
    }

    fn baseline() -> EnvironmentVariables {
        EnvironmentVariables::from_pairs([
            ("CLAUDECODE", "1"),
            ("CLAUDE_CODE_CHILD_SESSION", "1"),
            ("CLAUDE_CODE_SESSION_ID", "session-1"),
            ("CLAUDE_PROJECT_DIR", "/repo"),
        ])
    }

    fn variables_for(name: &str) -> EnvironmentVariables {
        let mut variables = baseline();
        if ENVIRONMENT_FILE_EVENTS.contains(&name) {
            variables.insert("CLAUDE_ENV_FILE", "/tmp/claude-env");
        }
        variables
    }

    fn parse(
        name: &'static str,
        variables: &EnvironmentVariables,
    ) -> hookkit_core::Result<ClaudeCommandEnvironment> {
        ClaudeCommandEnvironment::from_map(&event(name), variables)
    }

    #[test]
    fn every_documented_event_parses_the_baseline_environment() {
        for name in EVENTS {
            let environment = parse(name, &variables_for(name)).unwrap();
            assert_eq!(environment.session_id.as_str(), "session-1");
            assert_eq!(environment.project_dir, "/repo");
            assert_eq!(environment.claude_pid, None);
            assert_eq!(environment.messaging, None);
        }
    }

    #[test]
    fn environment_file_is_exposed_only_to_its_four_events() {
        for name in EVENTS {
            let environment = parse(name, &variables_for(name)).unwrap();
            assert_eq!(
                environment.environment_file.is_some(),
                ENVIRONMENT_FILE_EVENTS.contains(name)
            );
        }
        assert!(!ENVIRONMENT_FILE_EVENTS.contains(&"PreModelSwitch"));
        assert!(!ENVIRONMENT_FILE_EVENTS.contains(&"PostModelSwitch"));

        for name in ENVIRONMENT_FILE_EVENTS {
            // A version or configuration that never sets CLAUDE_ENV_FILE for
            // these events must still run the hook, just without a path.
            assert_eq!(parse(name, &baseline()).unwrap().environment_file, None);

            // An empty value is the documented `[ -n "$CLAUDE_ENV_FILE" ]`
            // guard's "unavailable" case, not a hook failure.
            let mut empty = baseline();
            empty.insert("CLAUDE_ENV_FILE", "");
            assert_eq!(parse(name, &empty).unwrap().environment_file, None);
        }

        let mut irrelevant = baseline();
        irrelevant.insert("CLAUDE_ENV_FILE", "/tmp/inherited");
        assert!(
            parse("PostToolUse", &irrelevant)
                .unwrap()
                .environment_file
                .is_none()
        );
    }

    #[test]
    fn local_remote_control_and_cloud_sessions_are_distinct() {
        let mut local = baseline();
        local.insert("CLAUDE_CODE_BRIDGE_SESSION_ID", "session_bridge");
        assert!(matches!(
            parse("Stop", &local).unwrap().execution_location,
            ClaudeExecutionLocation::Local {
                remote_control_session_id: Some(_)
            }
        ));

        let mut cloud = baseline();
        cloud.insert("CLAUDE_CODE_REMOTE", "true");
        cloud.insert("CLAUDE_CODE_REMOTE_SESSION_ID", "cse_cloud");
        assert_eq!(
            parse("Stop", &cloud).unwrap().execution_location,
            ClaudeExecutionLocation::Cloud {
                remote_session_id: "cse_cloud".into()
            }
        );

        // Claude Code never exports an empty bridge id; an empty value is
        // inherited state and means no Remote Control connection.
        let mut empty_bridge = baseline();
        empty_bridge.insert("CLAUDE_CODE_BRIDGE_SESSION_ID", "");
        assert_eq!(
            parse("Stop", &empty_bridge).unwrap().execution_location,
            ClaudeExecutionLocation::Local {
                remote_control_session_id: None
            }
        );
    }

    #[test]
    fn ambient_remote_state_does_not_fail_the_hook() {
        // Claude Code sets CLAUDE_CODE_REMOTE only to "true"; anything else
        // is inherited or user-set and means a local session.
        for value in ["false", "1", ""] {
            let mut variables = baseline();
            variables.insert("CLAUDE_CODE_REMOTE", value);
            assert!(matches!(
                parse("PreToolUse", &variables).unwrap().execution_location,
                ClaudeExecutionLocation::Local { .. }
            ));
        }

        // A cloud session id without the marker is inherited state.
        let mut id_without_marker = baseline();
        id_without_marker.insert("CLAUDE_CODE_REMOTE_SESSION_ID", "cse_cloud");
        assert_eq!(
            parse("Stop", &id_without_marker)
                .unwrap()
                .execution_location,
            ClaudeExecutionLocation::Local {
                remote_control_session_id: None
            }
        );

        // A cloud session is never bridged; an inherited bridge id is ignored.
        let mut mixed = baseline();
        mixed.insert("CLAUDE_CODE_REMOTE", "true");
        mixed.insert("CLAUDE_CODE_REMOTE_SESSION_ID", "cse_cloud");
        mixed.insert("CLAUDE_CODE_BRIDGE_SESSION_ID", "session_bridge");
        assert!(matches!(
            parse("Stop", &mixed).unwrap().execution_location,
            ClaudeExecutionLocation::Cloud { .. }
        ));
    }

    #[test]
    fn plugin_options_are_parsed_but_redacted_from_debug() {
        let mut variables = baseline();
        variables.insert("CLAUDE_PLUGIN_ROOT", "/plugins/demo");
        variables.insert("CLAUDE_PLUGIN_DATA", "/data/demo");
        variables.insert("CLAUDE_PLUGIN_OPTION_TOKEN", "super-secret");
        let plugin = parse("PostToolUse", &variables).unwrap().plugin.unwrap();
        assert_eq!(plugin.options.get("TOKEN"), Some("super-secret"));
        let rendered = format!("{:?}", plugin.options);
        assert!(rendered.contains("TOKEN"));
        assert!(!rendered.contains("super-secret"));
    }

    #[test]
    fn empty_plugin_option_suffix_fails_only_for_a_plugin_hook() {
        let mut ambient = baseline();
        ambient.insert("CLAUDE_PLUGIN_OPTION_", "x");
        assert_eq!(parse("Stop", &ambient).unwrap().plugin, None);

        let mut plugin = ambient.clone();
        plugin.insert("CLAUDE_PLUGIN_ROOT", "/plugins/demo");
        plugin.insert("CLAUDE_PLUGIN_DATA", "/data/demo");
        assert!(parse("Stop", &plugin).is_err());
    }

    #[test]
    fn claude_pid_is_an_optional_positive_integer() {
        let mut variables = baseline();
        variables.insert("CLAUDE_PID", "4242");
        assert_eq!(
            parse("PreModelSwitch", &variables).unwrap().claude_pid,
            Some(4242)
        );

        let mut empty = baseline();
        empty.insert("CLAUDE_PID", "");
        assert_eq!(parse("Stop", &empty).unwrap().claude_pid, None);

        for invalid in ["0", "-1", "+7", "abc", "4294967296", " 12"] {
            let mut variables = baseline();
            variables.insert("CLAUDE_PID", invalid);
            assert!(
                matches!(
                    parse("Stop", &variables),
                    Err(HookkitError::InvalidHookEnvironment { .. })
                ),
                "{invalid:?} should be rejected"
            );
        }
    }

    #[test]
    fn messaging_socket_and_token_are_optional_and_redacted() {
        let mut both = baseline();
        both.insert("CLAUDE_CODE_MESSAGING_SOCKET", "/tmp/claude-inbox.sock");
        both.insert("CLAUDE_CODE_MESSAGING_TOKEN", "tok-secret");
        let messaging = parse("SessionStart", &both).unwrap().messaging.unwrap();
        assert_eq!(messaging.socket, "/tmp/claude-inbox.sock");
        assert_eq!(
            messaging
                .token
                .as_ref()
                .map(ClaudeMessagingToken::expose_secret),
            Some("tok-secret")
        );
        let rendered = format!("{messaging:?}");
        assert!(!rendered.contains("tok-secret"));

        // v2.1.224 through v2.1.227 export only the socket.
        let mut socket_only = baseline();
        socket_only.insert("CLAUDE_CODE_MESSAGING_SOCKET", "/tmp/claude-inbox.sock");
        assert_eq!(
            parse("Stop", &socket_only)
                .unwrap()
                .messaging
                .unwrap()
                .token,
            None
        );

        // A token without a socket is inherited ambient state.
        let mut token_only = baseline();
        token_only.insert("CLAUDE_CODE_MESSAGING_TOKEN", "tok-secret");
        assert_eq!(parse("Stop", &token_only).unwrap().messaging, None);
    }

    #[test]
    fn fixed_markers_and_redundant_input_state_are_validated() {
        let mut variables = baseline();
        variables.insert("CLAUDE_CODE_CHILD_SESSION", "true");
        assert!(matches!(
            parse("PostToolUse", &variables),
            Err(HookkitError::InvalidHookEnvironment { .. })
        ));

        let mut variables = baseline();
        variables.insert("CLAUDE_EFFORT", "high");
        let environment = parse("PostToolUse", &variables).unwrap();
        assert!(matches!(
            environment.validate_input(&event("PostToolUse"), "session-1", Some("low")),
            Err(HookkitError::EnvironmentContextMismatch { .. })
        ));

        // An empty CLAUDE_EFFORT is inherited state, not a hook failure.
        let mut empty_effort = baseline();
        empty_effort.insert("CLAUDE_EFFORT", "");
        let environment = parse("PostToolUse", &empty_effort).unwrap();
        assert_eq!(environment.effort, None);
        environment
            .validate_input(&event("PostToolUse"), "session-1", Some("high"))
            .unwrap();

        let mut future_effort = baseline();
        future_effort.insert("CLAUDE_EFFORT", "ultra");
        let environment = parse("PostToolUse", &future_effort).unwrap();
        assert_eq!(
            environment.effort,
            Some(ClaudeEffort::Unknown("ultra".into()))
        );
        environment
            .validate_input(&event("PostToolUse"), "session-1", Some("ultra"))
            .unwrap();
    }

    #[test]
    fn incomplete_conditional_profiles_are_rejected_or_ignored() {
        let mut marker_without_remote = baseline();
        marker_without_remote.insert("CLAUDE_CODE_REMOTE", "true");
        assert!(parse("Stop", &marker_without_remote).is_err());

        // A lone plugin variable (or a stray option with neither) is ambient
        // noise from an unrelated parent process, not a hard failure — see
        // the rationale comment on the plugin-detection match above.
        let mut partial_plugin = baseline();
        partial_plugin.insert("CLAUDE_PLUGIN_ROOT", "/plugins/demo");
        assert_eq!(parse("Stop", &partial_plugin).unwrap().plugin, None);

        let mut option_without_plugin = baseline();
        option_without_plugin.insert("CLAUDE_PLUGIN_OPTION_TOKEN", "secret");
        assert_eq!(parse("Stop", &option_without_plugin).unwrap().plugin, None);

        let mut both_present_but_empty = baseline();
        both_present_but_empty.insert("CLAUDE_PLUGIN_ROOT", "");
        both_present_but_empty.insert("CLAUDE_PLUGIN_DATA", "/data/demo");
        assert!(parse("Stop", &both_present_but_empty).is_err());
    }
}
