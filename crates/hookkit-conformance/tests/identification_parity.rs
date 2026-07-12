use hookkit_core::{IdentificationDescriptor, IdentificationStrength};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
struct Registry {
    harnesses: BTreeMap<String, SelectedSnapshot>,
}

#[derive(Deserialize)]
struct SelectedSnapshot {
    current: String,
}

#[derive(Deserialize)]
struct Contract {
    id: String,
    harness: String,
    snapshot: String,
    event: Event,
}

#[derive(Deserialize)]
struct Event {
    wire_name: String,
    identification: Identification,
}

#[derive(Deserialize)]
struct Identification {
    inferability: String,
    #[serde(default)]
    discriminator: Option<Discriminator>,
    #[serde(default)]
    overlaps: Vec<String>,
}

#[derive(Deserialize)]
struct Discriminator {
    json_pointer: String,
    r#const: String,
}

#[test]
fn production_identification_descriptors_match_selected_catalog_metadata() {
    let catalog = selected_contracts();
    let descriptors = production_descriptors();
    assert_eq!(descriptors.len(), catalog.len());
    assert_eq!(
        descriptors
            .iter()
            .map(|descriptor| descriptor.contract().to_string())
            .collect::<BTreeSet<_>>(),
        catalog.keys().cloned().collect::<BTreeSet<_>>()
    );

    for descriptor in descriptors {
        let contract = catalog
            .get(descriptor.contract().as_str())
            .unwrap_or_else(|| panic!("missing catalog contract {}", descriptor.contract()));
        assert_eq!(descriptor.event().harness().as_str(), contract.harness);
        assert_eq!(descriptor.event().name(), contract.event.wire_name);
        assert_eq!(descriptor.snapshot().as_str(), contract.snapshot);
        let expected_strength = match contract.event.identification.inferability.as_str() {
            "definitive" => IdentificationStrength::Definitive,
            "shape-based" => IdentificationStrength::SoundShape,
            "ambiguous" => IdentificationStrength::Ambiguous,
            "impossible" => IdentificationStrength::Impossible,
            other => panic!("unknown inferability {other}"),
        };
        assert_eq!(descriptor.strength(), expected_strength);
        let expected_discriminator = contract
            .event
            .identification
            .discriminator
            .as_ref()
            .map(|value| (value.json_pointer.as_str(), value.r#const.as_str()));
        assert_eq!(descriptor.discriminator(), expected_discriminator);
        assert_eq!(
            descriptor.overlaps(),
            contract
                .event
                .identification
                .overlaps
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
    }
}

fn production_descriptors() -> Vec<IdentificationDescriptor> {
    let mut descriptors = Vec::new();
    descriptors.extend(hookkit_claude::protocol::identification_descriptors());
    descriptors.extend(hookkit_codex::protocol::identification_descriptors());
    descriptors.extend(hookkit_gemini::protocol::identification_descriptors());
    descriptors.extend(hookkit_antigravity::identification_descriptors());
    descriptors
}

fn selected_contracts() -> BTreeMap<String, Contract> {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap();
    let registry: Registry = serde_yaml_ng::from_slice(
        &std::fs::read(workspace.join("contracts/registry.yaml")).unwrap(),
    )
    .unwrap();
    let mut contracts = BTreeMap::new();
    for (harness, selected) in registry.harnesses {
        let snapshot = workspace
            .join("contracts/harnesses")
            .join(harness)
            .join("snapshots")
            .join(selected.current);
        for entry in walkdir::WalkDir::new(snapshot.join("events"))
            .into_iter()
            .map(Result::unwrap)
            .filter(|entry| entry.file_type().is_file() && entry.file_name() == "contract.yaml")
        {
            let contract: Contract =
                serde_yaml_ng::from_slice(&std::fs::read(entry.path()).unwrap()).unwrap();
            assert!(contracts.insert(contract.id.clone(), contract).is_none());
        }
    }
    contracts
}
