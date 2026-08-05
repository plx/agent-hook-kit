//! Claude Code command-hook environment contract.
//!
//! This module models only variables Claude Code documents as hook subprocess
//! state. Inherited provider settings and undocumented implementation details
//! are intentionally excluded.

use hookkit_core::{
    CommandEnvironmentSpec, EnvironmentVariables, EventId, HarnessId, HookkitError, NativeContext,
    SessionId, Utf8PathBuf,
};
use std::collections::BTreeMap;
use std::fmt;

/// Native events for which Claude exposes a required `CLAUDE_ENV_FILE` path.
pub const ENVIRONMENT_FILE_EVENTS: &[&str] =
    &["SessionStart", "Setup", "CwdChanged", "FileChanged"];

#[derive(Debug, Clone, PartialEq, Eq)]
/// Whether the hook runs in a local or Claude-hosted session.
pub enum ClaudeExecutionLocation {
    /// A local Claude Code process.
    Local {
        /// Remote-control bridge session, when the local process is controlled
        /// from another client.
        remote_control_session_id: Option<String>,
    },
    /// A Claude-hosted remote process.
    Cloud {
        /// Native cloud session identifier.
        remote_session_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
/// Native process environment supplied to a Claude Code command hook.
pub struct ClaudeCommandEnvironment {
    /// Project root from `CLAUDE_PROJECT_DIR`.
    pub project_dir: Utf8PathBuf,
    /// Native session identity from `CLAUDE_CODE_SESSION_ID`.
    pub session_id: SessionId,
    /// Writable environment file exposed only for
    /// [`ENVIRONMENT_FILE_EVENTS`].
    pub environment_file: Option<Utf8PathBuf>,
    /// Local, remote-control, or cloud execution context.
    pub execution_location: ClaudeExecutionLocation,
    /// Plugin paths and options when the hook belongs to a plugin.
    pub plugin: Option<ClaudePluginEnvironment>,
    /// Optional effort level, preserving forward-compatible unknown values.
    pub effort: Option<ClaudeEffort>,
    /// Optional W3C trace context supplied by Claude.
    pub traceparent: Option<String>,
}

impl ClaudeCommandEnvironment {
    /// Parses Claude's declared hook variables for `event`.
    ///
    /// Fixed marker values and conditional all-or-none profiles for cloud,
    /// remote-control, environment-file, and plugin state are validated.
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

        let environment_file = if ENVIRONMENT_FILE_EVENTS.contains(&event.name()) {
            Some(Utf8PathBuf::from(required(
                event,
                variables,
                "CLAUDE_ENV_FILE",
            )?))
        } else {
            None
        };

        let execution_location = match variables.get("CLAUDE_CODE_REMOTE") {
            None => {
                if variables.contains_key("CLAUDE_CODE_REMOTE_SESSION_ID") {
                    return Err(invalid(
                        event,
                        "CLAUDE_CODE_REMOTE_SESSION_ID requires CLAUDE_CODE_REMOTE=true",
                    ));
                }
                ClaudeExecutionLocation::Local {
                    remote_control_session_id: variables
                        .get("CLAUDE_CODE_BRIDGE_SESSION_ID")
                        .map(|_| {
                            required(event, variables, "CLAUDE_CODE_BRIDGE_SESSION_ID")
                                .map(str::to_owned)
                        })
                        .transpose()?,
                }
            }
            Some("true") => {
                if variables.contains_key("CLAUDE_CODE_BRIDGE_SESSION_ID") {
                    return Err(invalid(
                        event,
                        "cloud hooks must use CLAUDE_CODE_REMOTE_SESSION_ID, not the local bridge id",
                    ));
                }
                ClaudeExecutionLocation::Cloud {
                    remote_session_id: required(event, variables, "CLAUDE_CODE_REMOTE_SESSION_ID")?
                        .to_owned(),
                }
            }
            Some(_) => {
                return Err(invalid(
                    event,
                    "CLAUDE_CODE_REMOTE must be exactly true when present",
                ));
            }
        };

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

        let root = variables.get("CLAUDE_PLUGIN_ROOT");
        let data = variables.get("CLAUDE_PLUGIN_DATA");
        let plugin = match (root, data) {
            (None, None) if options.is_empty() => None,
            (Some(root), Some(data)) if !root.is_empty() && !data.is_empty() => {
                Some(ClaudePluginEnvironment {
                    root: root.into(),
                    data: data.into(),
                    options: ClaudePluginOptions(options),
                })
            }
            (None, None) => {
                return Err(invalid(
                    event,
                    "plugin options require CLAUDE_PLUGIN_ROOT and CLAUDE_PLUGIN_DATA",
                ));
            }
            _ => {
                return Err(invalid(
                    event,
                    "CLAUDE_PLUGIN_ROOT and CLAUDE_PLUGIN_DATA must be non-empty and appear together",
                ));
            }
        };

        let effort = variables
            .get("CLAUDE_EFFORT")
            .map(|value| {
                if value.is_empty() {
                    return Err(invalid(event, "CLAUDE_EFFORT must not be empty"));
                }
                Ok(match value {
                    "low" => ClaudeEffort::Low,
                    "medium" => ClaudeEffort::Medium,
                    "high" => ClaudeEffort::High,
                    "xhigh" => ClaudeEffort::Xhigh,
                    "max" => ClaudeEffort::Max,
                    value => ClaudeEffort::Unknown(value.to_owned()),
                })
            })
            .transpose()?;

        Ok(Self {
            project_dir,
            session_id,
            environment_file,
            execution_location,
            plugin,
            effort,
            traceparent: variables.get("TRACEPARENT").map(str::to_owned),
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

    #[test]
    fn every_documented_event_parses_the_baseline_environment() {
        for name in EVENTS {
            let environment =
                ClaudeCommandEnvironment::from_map(&event(name), &variables_for(name)).unwrap();
            assert_eq!(environment.session_id.as_str(), "session-1");
            assert_eq!(environment.project_dir, "/repo");
        }
    }

    #[test]
    fn environment_file_is_exposed_only_to_its_four_events() {
        for name in EVENTS {
            let environment =
                ClaudeCommandEnvironment::from_map(&event(name), &variables_for(name)).unwrap();
            assert_eq!(
                environment.environment_file.is_some(),
                ENVIRONMENT_FILE_EVENTS.contains(name)
            );
        }

        for name in ENVIRONMENT_FILE_EVENTS {
            assert!(matches!(
                ClaudeCommandEnvironment::from_map(&event(name), &baseline()),
                Err(HookkitError::InvalidHookEnvironment { .. })
            ));
            let mut empty = baseline();
            empty.insert("CLAUDE_ENV_FILE", "");
            assert!(ClaudeCommandEnvironment::from_map(&event(name), &empty).is_err());
        }

        let mut irrelevant = baseline();
        irrelevant.insert("CLAUDE_ENV_FILE", "/tmp/inherited");
        assert!(
            ClaudeCommandEnvironment::from_map(&event("PostToolUse"), &irrelevant)
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
            ClaudeCommandEnvironment::from_map(&event("Stop"), &local)
                .unwrap()
                .execution_location,
            ClaudeExecutionLocation::Local {
                remote_control_session_id: Some(_)
            }
        ));

        let mut cloud = baseline();
        cloud.insert("CLAUDE_CODE_REMOTE", "true");
        cloud.insert("CLAUDE_CODE_REMOTE_SESSION_ID", "cse_cloud");
        assert!(matches!(
            ClaudeCommandEnvironment::from_map(&event("Stop"), &cloud)
                .unwrap()
                .execution_location,
            ClaudeExecutionLocation::Cloud { .. }
        ));

        let mut empty_bridge = baseline();
        empty_bridge.insert("CLAUDE_CODE_BRIDGE_SESSION_ID", "");
        assert!(ClaudeCommandEnvironment::from_map(&event("Stop"), &empty_bridge).is_err());
    }

    #[test]
    fn plugin_options_are_parsed_but_redacted_from_debug() {
        let mut variables = baseline();
        variables.insert("CLAUDE_PLUGIN_ROOT", "/plugins/demo");
        variables.insert("CLAUDE_PLUGIN_DATA", "/data/demo");
        variables.insert("CLAUDE_PLUGIN_OPTION_TOKEN", "super-secret");
        let plugin = ClaudeCommandEnvironment::from_map(&event("PostToolUse"), &variables)
            .unwrap()
            .plugin
            .unwrap();
        assert_eq!(plugin.options.get("TOKEN"), Some("super-secret"));
        let rendered = format!("{:?}", plugin.options);
        assert!(rendered.contains("TOKEN"));
        assert!(!rendered.contains("super-secret"));
    }

    #[test]
    fn fixed_markers_and_redundant_input_state_are_validated() {
        let mut variables = baseline();
        variables.insert("CLAUDE_CODE_CHILD_SESSION", "true");
        assert!(matches!(
            ClaudeCommandEnvironment::from_map(&event("PostToolUse"), &variables),
            Err(HookkitError::InvalidHookEnvironment { .. })
        ));

        let mut variables = baseline();
        variables.insert("CLAUDE_EFFORT", "high");
        let environment =
            ClaudeCommandEnvironment::from_map(&event("PostToolUse"), &variables).unwrap();
        assert!(matches!(
            environment.validate_input(&event("PostToolUse"), "session-1", Some("low")),
            Err(HookkitError::EnvironmentContextMismatch { .. })
        ));

        let mut empty_effort = baseline();
        empty_effort.insert("CLAUDE_EFFORT", "");
        assert!(ClaudeCommandEnvironment::from_map(&event("PostToolUse"), &empty_effort).is_err());
    }

    #[test]
    fn incomplete_conditional_profiles_are_rejected() {
        let mut remote_without_marker = baseline();
        remote_without_marker.insert("CLAUDE_CODE_REMOTE_SESSION_ID", "cse_cloud");
        assert!(
            ClaudeCommandEnvironment::from_map(&event("Stop"), &remote_without_marker).is_err()
        );

        let mut marker_without_remote = baseline();
        marker_without_remote.insert("CLAUDE_CODE_REMOTE", "true");
        assert!(
            ClaudeCommandEnvironment::from_map(&event("Stop"), &marker_without_remote).is_err()
        );

        let mut mixed_remote_modes = baseline();
        mixed_remote_modes.insert("CLAUDE_CODE_REMOTE", "true");
        mixed_remote_modes.insert("CLAUDE_CODE_REMOTE_SESSION_ID", "cse_cloud");
        mixed_remote_modes.insert("CLAUDE_CODE_BRIDGE_SESSION_ID", "session_bridge");
        assert!(ClaudeCommandEnvironment::from_map(&event("Stop"), &mixed_remote_modes).is_err());

        let mut partial_plugin = baseline();
        partial_plugin.insert("CLAUDE_PLUGIN_ROOT", "/plugins/demo");
        assert!(ClaudeCommandEnvironment::from_map(&event("Stop"), &partial_plugin).is_err());

        let mut option_without_plugin = baseline();
        option_without_plugin.insert("CLAUDE_PLUGIN_OPTION_TOKEN", "secret");
        assert!(
            ClaudeCommandEnvironment::from_map(&event("Stop"), &option_without_plugin).is_err()
        );
    }
}
