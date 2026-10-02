//! Codex command-hook environment contract.
//!
//! Since Codex 0.149.0 a command hook does not inherit Codex's live process
//! environment. Codex starts every hook from an empty environment and replays
//! a snapshot of its own environment, captured when the session's hook
//! registry was created, then applies the handler's variables. Five
//! credential variables are removed, compared case-insensitively, from both
//! sources for every event: `CODEX_EXEC_SERVER_NOISE_AUTH_TOKEN`,
//! `NODE_REPL_AUTH_TOKEN`, `OPENAI_FEDERATION_RULE_ID`,
//! `OPENAI_IDENTITY_TOKEN_FILE`, and `OPENAI_WORKLOAD_IDENTITY_CONTEXT`.
//!
//! Codex hook configuration has no `env` setting; the only handler variables
//! come from plugin discovery. Ordinary hooks therefore receive no
//! Codex-specific variable. Hooks discovered from a plugin, on every event
//! including `Interrupt`, additionally receive `PLUGIN_ROOT` and
//! `PLUGIN_DATA` plus the Claude-compatible aliases `CLAUDE_PLUGIN_ROOT` and
//! `CLAUDE_PLUGIN_DATA`, which Codex sets to the same values. Since Codex
//! 0.154.0 plugin hooks are refreshed per turn, so these paths can change
//! between turns of one session.
//!
//! `CODEX_SESSION_ID` and `CODEX_THREAD_ID` are set for shell-tool and
//! unified-exec processes, not for hooks. They reach a hook only ambiently,
//! through the environment snapshot of a Codex that was itself launched from
//! an outer Codex session, and then describe that outer session; they are not
//! modeled and must not be used as hook identity. The native JSON
//! `session_id` is the hook's session identity.

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
    ///
    /// The paths describe the plugin revision Codex loaded for the current
    /// turn and can differ between turns of the same session.
    pub plugin: Option<CodexPluginEnvironment>,
}

impl CodexCommandEnvironment {
    /// Parses Codex's declared hook variables for `event`.
    ///
    /// A hook's environment replays Codex's own process environment, which
    /// can carry unrelated ambient values, so only a plugin set Codex itself
    /// could have written counts: all four variables, non-empty canonical
    /// values, and Claude-compatible aliases equal to them. Any other set
    /// (partial, empty, or with disagreeing aliases) yields no plugin.
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
    /// Codex writes every plugin variable from `Path::display`, which is
    /// always UTF-8, so a non-UTF-8 value can only be inherited ambient state
    /// (the generic `PLUGIN_*` names especially can come from unrelated
    /// tools). Capture skips such a value, which leaves an incomplete set and
    /// therefore no plugin, instead of failing every Codex hook.
    const LENIENT_VARIABLE_NAMES: &'static [&'static str] = &[
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
        if values.iter().any(Option::is_none) {
            // Codex sets all four variables together, and only for plugin
            // hooks. Every hook replays Codex's own environment snapshot, so an
            // incomplete set is ambient state from an unrelated parent process
            // (for example a Claude plugin hook that launched Codex), not a
            // plugin hook.
            return Ok(Self::default());
        }

        let [Some(root), Some(data), Some(claude_root), Some(claude_data)] = values else {
            unreachable!("presence was checked above")
        };
        if root.is_empty() || data.is_empty() || root != claude_root || data != claude_data {
            // For a plugin hook Codex writes all four variables from the same
            // non-empty `Path::display` strings over the replayed snapshot,
            // so an empty or disagreeing complete set can only be ambient
            // state (for example an outer Codex plugin hook and a Claude Code
            // plugin hook in the launch chain). Failing here would fail every
            // ordinary hook of the session.
            return Ok(Self::default());
        }

        Ok(Self {
            plugin: Some(CodexPluginEnvironment {
                root: root.into(),
                data: data.into(),
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVENTS: &[&str] = &[
        "SessionStart",
        "SessionEnd",
        "SubagentStart",
        "PreToolUse",
        "PermissionRequest",
        "PostToolUse",
        "PreCompact",
        "PostCompact",
        "UserPromptSubmit",
        "SubagentStop",
        "Stop",
        "Interrupt",
    ];

    fn event(name: &'static str) -> EventId {
        EventId::builtin(HarnessId::CODEX, name)
    }

    fn plugin_variables() -> EnvironmentVariables {
        EnvironmentVariables::from_pairs([
            ("PLUGIN_ROOT", "/plugins/demo"),
            ("PLUGIN_DATA", "/data/demo"),
            ("CLAUDE_PLUGIN_ROOT", "/plugins/demo"),
            ("CLAUDE_PLUGIN_DATA", "/data/demo"),
        ])
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
    fn plugin_hooks_parse_canonical_paths_and_compatibility_aliases_for_every_event() {
        for name in EVENTS {
            let environment =
                CodexCommandEnvironment::from_map(&event(name), &plugin_variables()).unwrap();
            assert_eq!(environment.plugin.unwrap().root, "/plugins/demo");
        }
    }

    #[test]
    fn shell_tool_session_variables_are_not_hook_state() {
        for name in ["CODEX_SESSION_ID", "CODEX_THREAD_ID"] {
            assert!(!CodexCommandEnvironment::VARIABLE_NAMES.contains(&name));
        }
        let ambient = EnvironmentVariables::from_pairs([
            ("CODEX_SESSION_ID", "outer-session"),
            ("CODEX_THREAD_ID", "outer-thread"),
        ]);
        assert_eq!(
            CodexCommandEnvironment::from_map(&event("Stop"), &ambient).unwrap(),
            CodexCommandEnvironment::default()
        );
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
    fn complete_but_inconsistent_plugin_state_is_ambient_and_ignored() {
        // Regression: such a set failed every ordinary hook with
        // InvalidHookEnvironment, although Codex never writes one for a
        // plugin hook.
        for (root, data, claude_root, claude_data) in [
            ("/plugins/demo", "/data/demo", "/other", "/data/demo"),
            ("/plugins/demo", "/data/demo", "/plugins/demo", "/other"),
            ("", "", "", ""),
            ("", "/data/demo", "", "/data/demo"),
            ("/plugins/demo", "", "/plugins/demo", ""),
        ] {
            let variables = EnvironmentVariables::from_pairs([
                ("PLUGIN_ROOT", root),
                ("PLUGIN_DATA", data),
                ("CLAUDE_PLUGIN_ROOT", claude_root),
                ("CLAUDE_PLUGIN_DATA", claude_data),
            ]);
            for name in EVENTS {
                assert_eq!(
                    CodexCommandEnvironment::from_map(&event(name), &variables).unwrap(),
                    CodexCommandEnvironment::default(),
                    "{name}: {root:?} {data:?} {claude_root:?} {claude_data:?}"
                );
            }
        }
    }

    #[test]
    fn lenient_names_are_the_declared_plugin_variables() {
        assert_eq!(
            CodexCommandEnvironment::LENIENT_VARIABLE_NAMES,
            CodexCommandEnvironment::VARIABLE_NAMES
        );
    }

    #[test]
    fn other_harness_events_are_rejected() {
        let claude = EventId::builtin(HarnessId::CLAUDE_CODE, "Stop");
        assert!(matches!(
            CodexCommandEnvironment::from_map(&claude, &EnvironmentVariables::new()),
            Err(HookkitError::EventHarnessMismatch { .. })
        ));
    }
}
