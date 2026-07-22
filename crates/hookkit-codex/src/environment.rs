//! Codex command-hook environment contract.
//!
//! Ordinary Codex hooks currently receive no Codex-specific process variables.
//! Plugin-bundled hooks additionally receive canonical plugin paths plus the
//! two Claude-compatible aliases modeled below.

use hookkit_core::{
    CommandEnvironmentSpec, EnvironmentVariables, EventId, HarnessId, HookkitError, Utf8PathBuf,
};

#[derive(Debug, Clone, PartialEq, Eq)]
/// Canonical paths injected for a hook discovered from a Codex plugin.
pub struct CodexPluginEnvironment {
    /// Root directory containing the installed plugin.
    pub root: Utf8PathBuf,
    /// Plugin-specific persistent data directory.
    pub data: Utf8PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Native process environment available to a Codex command hook.
pub struct CodexCommandEnvironment {
    /// Plugin paths when all canonical and compatibility variables are present
    /// and agree; otherwise `None` for an ordinary hook.
    pub plugin: Option<CodexPluginEnvironment>,
}

impl CodexCommandEnvironment {
    /// Parses Codex's declared hook variables for `event`.
    ///
    /// Partial plugin variable sets are ignored because Codex inherits ambient
    /// process state. A complete set must contain non-empty canonical values
    /// and matching Claude-compatible aliases.
    pub fn from_map(
        event: &EventId,
        variables: &EnvironmentVariables,
    ) -> hookkit_core::Result<Self> {
        <Self as CommandEnvironmentSpec>::from_variables(event, variables)
    }
}

impl CommandEnvironmentSpec for CodexCommandEnvironment {
    const VARIABLE_NAMES: &'static [&'static str] = &[
        "PLUGIN_ROOT",
        "PLUGIN_DATA",
        "CLAUDE_PLUGIN_ROOT",
        "CLAUDE_PLUGIN_DATA",
    ];

    fn from_variables(
        event: &EventId,
        variables: &EnvironmentVariables,
    ) -> hookkit_core::Result<Self> {
        if event.harness() != &HarnessId::CODEX {
            return Err(HookkitError::EventHarnessMismatch {
                harness: HarnessId::CODEX,
                event: event.clone(),
            });
        }

        let values = [
            variables.get("PLUGIN_ROOT"),
            variables.get("PLUGIN_DATA"),
            variables.get("CLAUDE_PLUGIN_ROOT"),
            variables.get("CLAUDE_PLUGIN_DATA"),
        ];
        if values.iter().all(Option::is_none) {
            return Ok(Self::default());
        }
        if values.iter().any(Option::is_none) {
            // Codex inherits the parent process environment for every hook, so a
            // partial set can be unrelated ambient state. Only the complete
            // all-or-none injection identifies a hook discovered from a plugin.
            return Ok(Self::default());
        }

        let [Some(root), Some(data), Some(claude_root), Some(claude_data)] = values else {
            unreachable!("presence was checked above")
        };
        if root.is_empty() || data.is_empty() {
            return Err(invalid(
                event,
                "plugin root and data paths must not be empty",
            ));
        }
        if root != claude_root || data != claude_data {
            return Err(invalid(
                event,
                "Claude-compatible plugin path aliases must match the canonical Codex values",
            ));
        }

        Ok(Self {
            plugin: Some(CodexPluginEnvironment {
                root: root.into(),
                data: data.into(),
            }),
        })
    }
}

fn invalid(event: &EventId, message: impl Into<String>) -> HookkitError {
    HookkitError::InvalidHookEnvironment {
        event: event.clone(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVENTS: &[&str] = &[
        "SessionStart",
        "SubagentStart",
        "PreToolUse",
        "PermissionRequest",
        "PostToolUse",
        "PreCompact",
        "PostCompact",
        "UserPromptSubmit",
        "SubagentStop",
        "Stop",
    ];

    fn event(name: &'static str) -> EventId {
        EventId::builtin(HarnessId::CODEX, name)
    }

    #[test]
    fn ordinary_hooks_have_empty_native_environment_for_every_event() {
        for name in EVENTS {
            assert_eq!(
                CodexCommandEnvironment::from_map(&event(name), &EnvironmentVariables::new())
                    .unwrap(),
                CodexCommandEnvironment::default()
            );
        }
    }

    #[test]
    fn plugin_hooks_parse_canonical_paths_and_compatibility_aliases() {
        let variables = EnvironmentVariables::from_pairs([
            ("PLUGIN_ROOT", "/plugins/demo"),
            ("PLUGIN_DATA", "/data/demo"),
            ("CLAUDE_PLUGIN_ROOT", "/plugins/demo"),
            ("CLAUDE_PLUGIN_DATA", "/data/demo"),
        ]);
        let environment =
            CodexCommandEnvironment::from_map(&event("PreToolUse"), &variables).unwrap();
        assert_eq!(environment.plugin.unwrap().root, "/plugins/demo");
    }

    #[test]
    fn partial_ambient_plugin_state_is_ignored() {
        let names = [
            "PLUGIN_ROOT",
            "PLUGIN_DATA",
            "CLAUDE_PLUGIN_ROOT",
            "CLAUDE_PLUGIN_DATA",
        ];
        for present in 1_u8..0b1111 {
            let variables = EnvironmentVariables::from_pairs(
                names
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| present & (1 << index) != 0)
                    .map(|(_, name)| (*name, "/ambient/value")),
            );
            assert_eq!(
                CodexCommandEnvironment::from_map(&event("Stop"), &variables).unwrap(),
                CodexCommandEnvironment::default(),
            );
        }
    }

    #[test]
    fn complete_conflicting_plugin_state_is_rejected() {
        let conflicting = EnvironmentVariables::from_pairs([
            ("PLUGIN_ROOT", "/plugins/demo"),
            ("PLUGIN_DATA", "/data/demo"),
            ("CLAUDE_PLUGIN_ROOT", "/other"),
            ("CLAUDE_PLUGIN_DATA", "/data/demo"),
        ]);
        assert!(CodexCommandEnvironment::from_map(&event("Stop"), &conflicting).is_err());
    }
}
