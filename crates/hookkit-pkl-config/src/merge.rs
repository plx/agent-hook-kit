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
pub fn merge_chain(mut chain: impl Iterator<Item = RunnerConfig>) -> RunnerConfig {
    let mut acc = chain.next().unwrap_or_default();
    for next in chain {
        merge(&mut acc, next);
    }
    acc
}

/// Fold a chain of field-preserving Pkl config patches together.
pub fn merge_patch_chain(mut chain: impl Iterator<Item = RunnerConfigPatch>) -> RunnerConfig {
    let mut acc = chain
        .next()
        .map(RunnerConfigPatch::into_config)
        .unwrap_or_default();
    for next in chain {
        merge_patch(&mut acc, next);
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{MergeResetKey, ToolSpec};

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
}
