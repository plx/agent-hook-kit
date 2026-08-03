//! Multi-file config merge.
//!
//! Configs are loaded as a chain (home → project → local), with later configs
//! merged over earlier ones. A config may opt out of earlier state with
//! [`Merge::reset_all`] (drop everything), [`Merge::reset`] (drop specific
//! top-level fields), or [`Merge::reset_tools`] (drop specific tool entries).

use crate::schema::{Merge, MergeResetKey, RunnerConfig, RunnerConfigPatch};

/// Merge `incoming` into `acc` according to `incoming.merge` semantics.
///
/// Order of operations:
/// 1. apply incoming `merge.resetAll` (drop all prior state),
/// 2. apply incoming `merge.reset` per-field resets,
/// 3. apply incoming `merge.resetTools` per-tool resets,
/// 4. overlay incoming `settings`, `tools`, and `run`.
pub fn merge(acc: &mut RunnerConfig, incoming: RunnerConfig) {
    if incoming.merge.reset_all {
        *acc = RunnerConfig::default();
    }

    for key in &incoming.merge.reset {
        match key {
            MergeResetKey::Settings => acc.settings = Default::default(),
            MergeResetKey::Tools => acc.tools.clear(),
            MergeResetKey::Run => acc.run.clear(),
        }
    }

    for id in &incoming.merge.reset_tools {
        acc.tools.remove(id);
    }
    if incoming.merge.reset_deferred_reporting {
        acc.settings.deferred_reporting = Default::default();
    }

    // Settings: a present incoming settings struct wins. Since Pkl always emits
    // a settings object when the field exists, we treat any "non-default"
    // settings as overriding. For simplicity v0 always overwrites settings;
    // future iterations can switch to deep-merge if user need arises.
    acc.settings = incoming.settings;

    for (id, spec) in incoming.tools {
        acc.tools.insert(id, spec);
    }

    if !incoming.run.is_empty() {
        acc.run = incoming.run;
    }

    // merge directives apply only to this load step
    acc.merge = Merge::default();
}

/// Merge one field-preserving Pkl config patch into an accumulated config.
pub fn merge_patch(acc: &mut RunnerConfig, incoming: RunnerConfigPatch) {
    if incoming.merge.reset_all {
        *acc = RunnerConfig::default();
    }

    for key in &incoming.merge.reset {
        match key {
            MergeResetKey::Settings => acc.settings = Default::default(),
            MergeResetKey::Tools => acc.tools.clear(),
            MergeResetKey::Run => acc.run.clear(),
        }
    }

    for id in &incoming.merge.reset_tools {
        acc.tools.remove(id);
    }
    if incoming.merge.reset_deferred_reporting {
        acc.settings.deferred_reporting = Default::default();
    }

    incoming.settings.apply_to(&mut acc.settings);

    for (id, spec) in incoming.tools {
        acc.tools.insert(id, spec);
    }

    if !incoming.run.is_empty() {
        acc.run = incoming.run;
    }

    acc.merge = Merge::default();
}

/// Fold a chain of configs together. The first config is the base; subsequent
/// configs are merged over it in order.
pub fn merge_chain(chain: impl Iterator<Item = RunnerConfig>) -> RunnerConfig {
    let mut acc = RunnerConfig::default();
    for next in chain {
        merge(&mut acc, next);
    }
    acc
}

/// Fold a chain of field-preserving Pkl config patches together.
pub fn merge_patch_chain(chain: impl Iterator<Item = RunnerConfigPatch>) -> RunnerConfig {
    let mut acc = RunnerConfig::default();
    for next in chain {
        merge_patch(&mut acc, next);
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{MergeResetKey, Settings, SettingsPatch, ToolSpec};
    use proptest::prelude::*;

    fn tool(id: &str) -> ToolSpec {
        ToolSpec {
            id: id.into(),
            display_name: id.into(),
            executable: id.into(),
            ..Default::default()
        }
    }

    #[test]
    fn default_merge_overlays_tools() {
        let mut acc = RunnerConfig {
            tools: [("ruff".into(), tool("ruff"))].into_iter().collect(),
            run: vec!["ruff".into()],
            ..Default::default()
        };
        let incoming = RunnerConfig {
            tools: [("prettier".into(), tool("prettier"))]
                .into_iter()
                .collect(),
            run: vec!["ruff".into(), "prettier".into()],
            ..Default::default()
        };
        merge(&mut acc, incoming);

        assert!(acc.tools.contains_key("ruff"));
        assert!(acc.tools.contains_key("prettier"));
        assert_eq!(acc.run, vec!["ruff", "prettier"]);
    }

    #[test]
    fn reset_tools_clears_just_that_section() {
        let mut acc = RunnerConfig {
            tools: [("ruff".into(), tool("ruff"))].into_iter().collect(),
            run: vec!["ruff".into()],
            ..Default::default()
        };
        let incoming = RunnerConfig {
            merge: Merge {
                reset: vec![MergeResetKey::Tools],
                ..Default::default()
            },
            tools: [("prettier".into(), tool("prettier"))]
                .into_iter()
                .collect(),
            run: vec!["prettier".into()],
            ..Default::default()
        };
        merge(&mut acc, incoming);

        assert!(!acc.tools.contains_key("ruff"));
        assert!(acc.tools.contains_key("prettier"));
        assert_eq!(acc.run, vec!["prettier"]);
    }

    #[test]
    fn reset_all_starts_from_scratch() {
        let mut acc = RunnerConfig {
            tools: [("ruff".into(), tool("ruff"))].into_iter().collect(),
            run: vec!["ruff".into()],
            ..Default::default()
        };
        let incoming = RunnerConfig {
            merge: Merge {
                reset_all: true,
                ..Default::default()
            },
            tools: [("biome".into(), tool("biome"))].into_iter().collect(),
            run: vec!["biome".into()],
            ..Default::default()
        };
        merge(&mut acc, incoming);

        assert_eq!(acc.tools.len(), 1);
        assert!(acc.tools.contains_key("biome"));
        assert_eq!(acc.run, vec!["biome"]);
    }

    #[test]
    fn reset_tools_drops_named_tools() {
        let mut acc = RunnerConfig {
            tools: [
                ("ruff".into(), tool("ruff")),
                ("prettier".into(), tool("prettier")),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let incoming = RunnerConfig {
            merge: Merge {
                reset_tools: vec!["ruff".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        merge(&mut acc, incoming);

        assert!(!acc.tools.contains_key("ruff"));
        assert!(acc.tools.contains_key("prettier"));
    }

    fn tool_map(
        ids: impl IntoIterator<Item = String>,
    ) -> std::collections::BTreeMap<String, ToolSpec> {
        ids.into_iter().map(|id| (id.clone(), tool(&id))).collect()
    }

    proptest! {
        /// Property: ordinary merge is a right-biased tool-map union. A later
        /// definition replaces the same identifier, unrelated earlier tools
        /// remain, and an empty incoming run list means “inherit”.
        #[test]
        fn ordinary_merge_is_right_biased_and_inherits_empty_run(
            base_ids in prop::collection::btree_set("[a-z]{1,8}", 0..20),
            incoming_ids in prop::collection::btree_set("[a-z]{1,8}", 0..20),
            base_run in prop::collection::vec("[a-z]{1,8}", 0..20),
            incoming_run in prop::collection::vec("[a-z]{1,8}", 0..20),
        ) {
            let mut base = RunnerConfig {
                tools: tool_map(base_ids.iter().cloned()),
                run: base_run.clone(),
                ..RunnerConfig::default()
            };
            let incoming = RunnerConfig {
                tools: tool_map(incoming_ids.iter().cloned()),
                run: incoming_run.clone(),
                ..RunnerConfig::default()
            };
            merge(&mut base, incoming);

            let expected_ids = base_ids.union(&incoming_ids).cloned().collect::<Vec<_>>();
            prop_assert_eq!(base.tools.keys().cloned().collect::<Vec<_>>(), expected_ids);
            prop_assert_eq!(base.run, if incoming_run.is_empty() { base_run } else { incoming_run });
            prop_assert!(!base.merge.reset_all);
            prop_assert!(base.merge.reset.is_empty());
            prop_assert!(base.merge.reset_tools.is_empty());
        }

        /// Property: an omitted settings patch is an identity operation for
        /// every independently configurable scalar and list field.
        #[test]
        fn empty_settings_patch_preserves_all_settings(
            jobs in any::<u32>(),
            fail_fast in any::<bool>(),
            continue_after_issues in any::<bool>(),
            exclude in prop::collection::vec(any::<String>(), 0..20),
            diagnostics in proptest::option::of(any::<String>()),
        ) {
            let mut settings = Settings {
                jobs,
                fail_fast,
                continue_after_issues,
                exclude: exclude.clone(),
                diagnostics_directory: diagnostics.clone(),
                ..Settings::default()
            };
            SettingsPatch::default().apply_to(&mut settings);

            prop_assert_eq!(settings.jobs, jobs);
            prop_assert_eq!(settings.fail_fast, fail_fast);
            prop_assert_eq!(settings.continue_after_issues, continue_after_issues);
            prop_assert_eq!(settings.exclude, exclude);
            prop_assert_eq!(settings.diagnostics_directory, diagnostics);
        }

        /// Property: folding helpers are exactly repeated single-step merges,
        /// including for a one-element chain. Per-file reset directives are
        /// consumed and never leak into the resolved configuration.
        #[test]
        fn merge_chain_consumes_every_steps_directives(
            ids in prop::collection::vec("[a-z]{1,8}", 0..20),
            reset_all in any::<bool>(),
        ) {
            let config = RunnerConfig {
                merge: Merge { reset_all, reset: vec![MergeResetKey::Run], reset_tools: ids.clone(), reset_deferred_reporting: false },
                tools: tool_map(ids),
                run: vec!["final".into()],
                ..RunnerConfig::default()
            };
            let mut expected = RunnerConfig::default();
            merge(&mut expected, config.clone());
            let actual = merge_chain(std::iter::once(config));

            prop_assert_eq!(actual.tools.keys().collect::<Vec<_>>(), expected.tools.keys().collect::<Vec<_>>());
            prop_assert_eq!(actual.run, expected.run);
            prop_assert!(!actual.merge.reset_all);
            prop_assert!(actual.merge.reset.is_empty());
            prop_assert!(actual.merge.reset_tools.is_empty());
        }
    }
}
