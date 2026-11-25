//! Antigravity command-hook environment contract.
//!
//! The hooks reference defines all native invocation state in the JSON
//! payload and documents no environment variable. CLI 1.5.3 and later set
//! `PLUGIN_ROOT` and `PLUGIN_DATA` for hook commands that come from a
//! plugin, pointing to the plugin's folder and its persistent data folder,
//! according to the CLI release notes. The plugin authoring guide documents
//! the two variables only for plugin MCP servers, says plugin hooks run with
//! the plugin root as their working directory, and steers hook commands to
//! relative paths instead (CLI 1.5.6 warns that older CLI versions do not set
//! `PLUGIN_ROOT`). Whether the Antigravity App or IDE sets them is
//! undocumented, and ordinary workspace and global hooks receive neither.
//!
//! Unlike Codex, Antigravity sets no `CLAUDE_*` aliases next to the two
//! variables, so a `PLUGIN_ROOT`/`PLUGIN_DATA` pair inherited from an outer
//! process, such as a Codex plugin hook (Codex sets the same two names) that
//! launched Antigravity, cannot be told apart from one Antigravity set.
//! Treat [`AntigravityCommandEnvironment::plugin`] as a low-confidence hint.

use hookkit_core::{
    CommandEnvironmentSpec, EnvironmentVariables, EventId, HarnessId, HookkitError, Utf8PathBuf,
};

#[derive(Debug, Clone, PartialEq, Eq)]
/// Paths Antigravity sets for a hook command that comes from a plugin.
pub struct AntigravityPluginEnvironment {
    /// `PLUGIN_ROOT`: the plugin's folder.
    pub root: Utf8PathBuf,
    /// `PLUGIN_DATA`: the plugin's persistent data folder.
    pub data: Utf8PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
/// Native process environment available to an Antigravity command hook.
///
/// Construction validates that the selected event belongs to the Antigravity
/// harness. Every other ambient variable is ignored, because the documented
/// protocol supplies all invocation state in JSON. The struct is
/// `#[non_exhaustive]` so it can model further variables additively; build
/// it with [`AntigravityCommandEnvironment::from_map`] or `Default`.
pub struct AntigravityCommandEnvironment {
    /// Plugin paths when both `PLUGIN_ROOT` and `PLUGIN_DATA` are set to
    /// non-empty values; otherwise `None`, as for an ordinary hook.
    ///
    /// Only CLI 1.5.3 and later set them, and an inherited pair is
    /// indistinguishable from Antigravity's own; see the
    /// [module documentation](self).
    pub plugin: Option<AntigravityPluginEnvironment>,
}

impl AntigravityCommandEnvironment {
    /// Parses Antigravity's declared hook variables for `event`.
    ///
    /// Any partial or empty `PLUGIN_ROOT`/`PLUGIN_DATA` set yields no plugin,
    /// since Antigravity sets both together and only for plugin hooks.
    pub fn from_map(
        event: &EventId,
        variables: &EnvironmentVariables,
    ) -> hookkit_core::Result<Self> {
        <Self as CommandEnvironmentSpec>::from_variables(event, variables)
    }
}

impl CommandEnvironmentSpec for AntigravityCommandEnvironment {
    const VARIABLE_NAMES: &'static [&'static str] = &["PLUGIN_ROOT", "PLUGIN_DATA"];
    /// Antigravity writes both variables as UTF-8 paths, so a non-UTF-8 value
    /// can only be inherited ambient state. Capture skips such a value, which
    /// leaves an incomplete set and therefore no plugin, instead of failing
    /// every Antigravity hook.
    const LENIENT_VARIABLE_NAMES: &'static [&'static str] = &["PLUGIN_ROOT", "PLUGIN_DATA"];

    fn from_variables(
        event: &EventId,
        variables: &EnvironmentVariables,
    ) -> hookkit_core::Result<Self> {
        if event.harness() != &HarnessId::ANTIGRAVITY {
            return Err(HookkitError::EventHarnessMismatch {
                harness: HarnessId::ANTIGRAVITY,
                event: event.clone(),
            });
        }
        let plugin = match (variables.get("PLUGIN_ROOT"), variables.get("PLUGIN_DATA")) {
            (Some(root), Some(data)) if !root.is_empty() && !data.is_empty() => {
                Some(AntigravityPluginEnvironment {
                    root: root.into(),
                    data: data.into(),
                })
            }
            // A partial or empty set is not one Antigravity writes for a
            // plugin hook; failing here would fail every ordinary hook.
            _ => None,
        };
        Ok(Self { plugin })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVENTS: &[&str] = &[
        "SessionStart",
        "PreToolUse",
        "PostToolUse",
        "PreInvocation",
        "PostInvocation",
        "Stop",
    ];

    fn event(name: &'static str) -> EventId {
        EventId::builtin(HarnessId::ANTIGRAVITY, name)
    }

    #[test]
    fn ordinary_hooks_have_no_plugin_for_every_documented_event() {
        for name in EVENTS {
            for variables in [
                EnvironmentVariables::new(),
                EnvironmentVariables::from_pairs([("PATH", "/bin")]),
            ] {
                assert_eq!(
                    AntigravityCommandEnvironment::from_map(&event(name), &variables).unwrap(),
                    AntigravityCommandEnvironment::default(),
                    "{name}"
                );
            }
        }
    }

    #[test]
    fn plugin_hooks_parse_both_paths_for_every_documented_event() {
        let variables =
            EnvironmentVariables::from_pairs([("PLUGIN_ROOT", "/p"), ("PLUGIN_DATA", "/d")]);
        for name in EVENTS {
            let plugin = AntigravityCommandEnvironment::from_map(&event(name), &variables)
                .unwrap()
                .plugin
                .unwrap();
            assert_eq!(plugin.root, "/p", "{name}");
            assert_eq!(plugin.data, "/d", "{name}");
        }
    }

    #[test]
    fn partial_or_empty_plugin_state_is_ignored() {
        for pairs in [
            vec![("PLUGIN_ROOT", "/p")],
            vec![("PLUGIN_DATA", "/d")],
            vec![("PLUGIN_ROOT", "/p"), ("PLUGIN_DATA", "")],
            vec![("PLUGIN_ROOT", ""), ("PLUGIN_DATA", "/d")],
            vec![("PLUGIN_ROOT", ""), ("PLUGIN_DATA", "")],
        ] {
            let variables = EnvironmentVariables::from_pairs(pairs.clone());
            assert_eq!(
                AntigravityCommandEnvironment::from_map(&event("Stop"), &variables).unwrap(),
                AntigravityCommandEnvironment::default(),
                "{pairs:?}"
            );
        }
    }

    #[test]
    fn lenient_names_are_the_declared_plugin_variables() {
        assert_eq!(
            AntigravityCommandEnvironment::LENIENT_VARIABLE_NAMES,
            AntigravityCommandEnvironment::VARIABLE_NAMES
        );
    }

    #[test]
    fn other_harness_events_are_rejected() {
        for harness in [HarnessId::CLAUDE_CODE, HarnessId::CODEX] {
            let other = EventId::builtin(harness, "Stop");
            assert!(matches!(
                AntigravityCommandEnvironment::from_map(&other, &EnvironmentVariables::new()),
                Err(HookkitError::EventHarnessMismatch { .. })
            ));
        }
    }
}
