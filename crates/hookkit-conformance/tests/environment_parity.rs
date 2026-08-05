use hookkit_core::CommandEnvironmentSpec;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
struct Registry {
    supplements: RegistrySupplements,
}

#[derive(Deserialize)]
struct RegistrySupplements {
    command_environments: String,
}

#[derive(Deserialize)]
struct Supplement {
    harnesses: BTreeMap<String, HarnessEnvironment>,
}

#[derive(Deserialize)]
struct HarnessEnvironment {
    profiles: Vec<EnvironmentProfile>,
}

#[derive(Deserialize)]
struct EnvironmentProfile {
    variables: Vec<EnvironmentVariable>,
}

#[derive(Deserialize)]
struct EnvironmentVariable {
    name: Option<String>,
    prefix: Option<String>,
}

#[test]
fn production_environment_selectors_match_the_selected_supplement() {
    let expected = selected_supplement_selectors();
    let actual = BTreeMap::from([
        (
            "antigravity".to_string(),
            selectors::<hookkit_antigravity::AntigravityCommandEnvironment>(),
        ),
        (
            "claude-code".to_string(),
            selectors::<hookkit_claude::ClaudeCommandEnvironment>(),
        ),
        (
            "codex".to_string(),
            selectors::<hookkit_codex::CodexCommandEnvironment>(),
        ),
    ]);

    assert_eq!(actual, expected);
}

fn selectors<E: CommandEnvironmentSpec>() -> (BTreeSet<String>, BTreeSet<String>) {
    (
        E::VARIABLE_NAMES
            .iter()
            .map(|name| (*name).to_string())
            .collect(),
        E::VARIABLE_PREFIXES
            .iter()
            .map(|prefix| (*prefix).to_string())
            .collect(),
    )
}

fn selected_supplement_selectors() -> BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap();
    let registry: Registry = serde_yaml_ng::from_slice(
        &std::fs::read(workspace.join("contracts/registry.yaml")).unwrap(),
    )
    .unwrap();
    let supplement_path = workspace
        .join("contracts/supplements/command-environments")
        .join(registry.supplements.command_environments)
        .join("supplement.yaml");
    let supplement: Supplement =
        serde_yaml_ng::from_slice(&std::fs::read(supplement_path).unwrap()).unwrap();

    supplement
        .harnesses
        .into_iter()
        .map(|(harness, environment)| {
            let mut names = BTreeSet::new();
            let mut prefixes = BTreeSet::new();
            for variable in environment
                .profiles
                .into_iter()
                .flat_map(|profile| profile.variables)
            {
                if let Some(name) = variable.name {
                    names.insert(name);
                }
                if let Some(prefix) = variable.prefix {
                    prefixes.insert(prefix);
                }
            }
            (harness, (names, prefixes))
        })
        .collect()
}
