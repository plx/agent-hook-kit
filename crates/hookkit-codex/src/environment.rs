//! Codex command-hook environment contract.
//!
//! Ordinary Codex hooks currently receive no Codex-specific process variables.
//! Plugin-bundled hooks additionally receive canonical plugin paths plus the
//! two Claude-compatible aliases modeled below.

use hookkit_core::{
    CommandEnvironmentSpec, EnvironmentVariables, EventId, HarnessId, HookkitError, Utf8PathBuf,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexPluginEnvironment {
    pub root: Utf8PathBuf,
    pub data: Utf8PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodexCommandEnvironment {
    pub plugin: Option<CodexPluginEnvironment>,
}

impl CodexCommandEnvironment {
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
            return Err(invalid(
                event,
                "plugin hooks require PLUGIN_ROOT, PLUGIN_DATA, CLAUDE_PLUGIN_ROOT, and CLAUDE_PLUGIN_DATA together",
            ));
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
    fn partial_or_conflicting_plugin_state_is_rejected() {
        let partial = EnvironmentVariables::from_pairs([("PLUGIN_ROOT", "/plugins/demo")]);
        assert!(matches!(
            CodexCommandEnvironment::from_map(&event("Stop"), &partial),
            Err(HookkitError::InvalidHookEnvironment { .. })
        ));

        let conflicting = EnvironmentVariables::from_pairs([
            ("PLUGIN_ROOT", "/plugins/demo"),
            ("PLUGIN_DATA", "/data/demo"),
            ("CLAUDE_PLUGIN_ROOT", "/other"),
            ("CLAUDE_PLUGIN_DATA", "/data/demo"),
        ]);
        assert!(CodexCommandEnvironment::from_map(&event("Stop"), &conflicting).is_err());
    }
}
