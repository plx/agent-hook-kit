use base64::Engine as _;
use clap::{Parser, Subcommand};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

type Result<T> = std::result::Result<T, String>;

#[derive(Parser)]
#[command(about = "Repository maintenance tasks")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Contracts {
        #[command(subcommand)]
        command: ContractsCommand,
    },
}

#[derive(Subcommand)]
enum ContractsCommand {
    /// Validate catalog metadata, schemas, fixtures, and status coverage.
    Check {
        /// Validate selected current snapshots (the default and only Phase 1 selection).
        #[arg(long, value_parser = ["current"])]
        snapshot: Option<String>,
    },
    /// Render the generated support report.
    Report {
        /// Update contracts/status/support.md.
        #[arg(long, conflicts_with = "check")]
        write: bool,
        /// Fail unless contracts/status/support.md is current.
        #[arg(long)]
        check: bool,
    },
    /// Compare event inventories and content hashes between two snapshot IDs.
    Diff { old: String, new: String },
    /// Verify vendored files against their checked-in SHA-256 manifests.
    VerifyVendor,
    /// Freeze a reviewed snapshot with a deterministic SHA-256 manifest.
    Freeze { harness: String, snapshot: String },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    format_version: u32,
    harnesses: BTreeMap<String, RegistryHarness>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryHarness {
    current: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Harness {
    format_version: u32,
    id: String,
    display_name: String,
    #[serde(default)]
    wire_heritage: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    format_version: u32,
    id: String,
    harness: String,
    state: String,
    retrieved: String,
    sources_file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    manifest_file: Option<String>,
    events: Vec<SnapshotEvent>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SnapshotEvent {
    wire_name: String,
    rust_key: String,
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sources {
    format_version: u32,
    sources: Vec<Source>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    id: String,
    kind: String,
    authority: String,
    url: String,
    retrieved: String,
    #[serde(default)]
    revision: Option<String>,
    #[serde(default)]
    content_sha256: Option<String>,
    #[serde(default)]
    license: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Contract {
    format_version: u32,
    id: String,
    harness: String,
    snapshot: String,
    event: Event,
    schemas: Schemas,
    bindings: BTreeMap<String, Binding>,
    #[serde(default)]
    handler_kinds: Vec<String>,
    fixtures: String,
    #[serde(default)]
    uncertainties: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {
    wire_name: String,
    rust_key: String,
    category: String,
    identification: Identification,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identification {
    inferability: String,
    #[serde(default)]
    discriminator: Option<Discriminator>,
    #[serde(default)]
    overlaps: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Discriminator {
    json_pointer: String,
    r#const: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Schemas {
    input: SchemaClaim,
    #[serde(default)]
    outputs: Vec<OutputSchemaClaim>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemaClaim {
    file: String,
    origin: String,
    sources: Vec<String>,
    assurance: Assurance,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputSchemaClaim {
    id: String,
    file: String,
    origin: String,
    sources: Vec<String>,
    assurance: Assurance,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Assurance {
    confidence: String,
    verification: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    kind: String,
    request: Request,
    outcomes: Vec<Outcome>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    request_content_type: Option<String>,
    #[serde(default)]
    response_content_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    channel: String,
    framing: String,
    content_kind: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Outcome {
    id: String,
    effect: String,
    exit: ExitSelector,
    stdout: Channel,
    stderr: Channel,
    #[serde(default)]
    output_schema: Option<String>,
    sources: Vec<String>,
    assurance: Assurance,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExitSelector {
    #[serde(default)]
    exact: Option<i32>,
    #[serde(default)]
    set: Vec<i32>,
    #[serde(default)]
    range: Option<ExitRange>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExitRange {
    min: i32,
    max: i32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Channel {
    presence: String,
    role: String,
    content_kind: String,
    #[serde(default)]
    encoding: Option<String>,
    #[serde(default)]
    semantic_format: Option<String>,
    #[serde(default)]
    trailing_newline: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixtures {
    format_version: u32,
    input: InputFixtures,
    #[serde(default)]
    output: Vec<OutputFixture>,
    process: Vec<ProcessFixture>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputFixtures {
    positive: Vec<PositiveFixture>,
    negative: Vec<NegativeFixture>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PositiveFixture {
    id: String,
    origin: String,
    sources: Vec<String>,
    value: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NegativeFixture {
    id: String,
    origin: String,
    sources: Vec<String>,
    value: Value,
    expected_pointer: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputFixture {
    id: String,
    schema: String,
    origin: String,
    sources: Vec<String>,
    value: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessFixture {
    id: String,
    binding: String,
    outcome: String,
    exit_code: i32,
    stdout_base64: String,
    stdout_sha256: String,
    stderr_base64: String,
    stderr_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stabilization {
    format_version: u32,
    release: String,
    targets: Vec<Target>,
    #[serde(default)]
    defaults: Vec<TargetDefault>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    contract: String,
    binding: String,
    level: String,
    verification: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetDefault {
    harness: String,
    binding: String,
    level: String,
    verification: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImplementationGaps {
    format_version: u32,
    gaps: Vec<ImplementationGap>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImplementationGap {
    harness: String,
    snapshot: String,
    event: String,
    binding: String,
    assertion: String,
    rationale: String,
    owner: String,
    removal_phase: u8,
    #[serde(default)]
    expiry: Option<String>,
}

struct LoadedContract {
    contract: Contract,
    dir: PathBuf,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("contracts: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let root = workspace_root()?;
    match cli.command {
        Command::Contracts {
            command: ContractsCommand::Check { snapshot },
        } => {
            let _ = snapshot;
            let contracts = check_catalog(&root)?;
            println!("validated {} event contracts", contracts.len());
            Ok(())
        }
        Command::Contracts {
            command: ContractsCommand::Diff { old, new },
        } => diff_snapshots(&root, &old, &new),
        Command::Contracts {
            command: ContractsCommand::VerifyVendor,
        } => {
            let count = verify_vendor(&root)?;
            println!("verified {count} vendored files");
            Ok(())
        }
        Command::Contracts {
            command: ContractsCommand::Freeze { harness, snapshot },
        } => freeze_snapshot(&root, &harness, &snapshot),
        Command::Contracts {
            command: ContractsCommand::Report { write, check },
        } => {
            let contracts = check_catalog(&root)?;
            let report = render_report(&root, &contracts)?;
            let path = root.join("contracts/status/support.md");
            if write {
                fs::write(&path, &report)
                    .map_err(|error| format!("{}: {error}", path.display()))?;
            } else if check {
                let actual = fs::read_to_string(&path)
                    .map_err(|error| format!("{}: {error}", path.display()))?;
                if actual != report {
                    return Err(format!(
                        "{} is stale; run cargo xtask contracts report --write",
                        path.display()
                    ));
                }
            } else {
                print!("{report}");
            }
            Ok(())
        }
    }
}

fn workspace_root() -> Result<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "xtask manifest has no parent".to_string())
}

fn check_catalog(root: &Path) -> Result<Vec<LoadedContract>> {
    let catalog = root.join("contracts");
    let meta = catalog.join("meta/v1");
    let registry_path = catalog.join("registry.yaml");
    validate_yaml_metadata(&registry_path, &meta.join("registry.schema.json"))?;
    let registry: Registry = read_yaml(&registry_path)?;
    require_version(registry.format_version, "registry")?;
    let stabilization_path = catalog.join("status/stabilization-v1.yaml");
    validate_yaml_metadata(&stabilization_path, &meta.join("status.schema.json"))?;
    let stabilization: Stabilization = read_yaml(&stabilization_path)?;
    require_version(stabilization.format_version, "stabilization target")?;
    verify_vendor(root)?;
    let gaps_path = catalog.join("status/implementation-gaps.yaml");
    validate_yaml_metadata(&gaps_path, &meta.join("implementation-gaps.schema.json"))?;
    let gaps: ImplementationGaps = read_yaml(&gaps_path)?;
    require_version(gaps.format_version, "implementation gaps")?;

    let mut contracts = Vec::new();
    let mut contract_ids = BTreeSet::new();
    for (harness, selected) in &registry.harnesses {
        let harness_path = catalog.join("harnesses").join(harness).join("harness.yaml");
        validate_yaml_metadata(&harness_path, &meta.join("harness.schema.json"))?;
        let harness_metadata: Harness = read_yaml(&harness_path)?;
        require_version(harness_metadata.format_version, "harness")?;
        if harness_metadata.id != *harness || harness_metadata.display_name.is_empty() {
            return Err(format!(
                "{}: harness identity mismatch",
                harness_path.display()
            ));
        }
        if harness_metadata
            .wire_heritage
            .as_deref()
            .is_some_and(|heritage| !registry.harnesses.contains_key(heritage))
        {
            return Err(format!("{}: unknown wire heritage", harness_path.display()));
        }
        let snapshot_dir = catalog
            .join("harnesses")
            .join(harness)
            .join("snapshots")
            .join(&selected.current);
        let snapshot_path = snapshot_dir.join("snapshot.yaml");
        validate_yaml_metadata(&snapshot_path, &meta.join("snapshot.schema.json"))?;
        let snapshot: Snapshot = read_yaml(&snapshot_path)?;
        require_version(snapshot.format_version, "snapshot")?;
        if snapshot.id != selected.current || snapshot.harness != *harness {
            return Err(format!(
                "{}: registry/snapshot identity mismatch",
                snapshot_dir.display()
            ));
        }
        if snapshot.state != "draft" && snapshot.state != "frozen" {
            return Err(format!(
                "{}: state must be draft or frozen",
                snapshot_dir.display()
            ));
        }
        if snapshot.state == "frozen" {
            let manifest = snapshot.manifest_file.as_deref().ok_or_else(|| {
                format!(
                    "{}: frozen snapshot needs manifest_file",
                    snapshot_dir.display()
                )
            })?;
            verify_content_manifest(&snapshot_dir, manifest)?;
        } else if snapshot.manifest_file.is_some() {
            return Err(format!(
                "{}: draft snapshot must not have manifest_file",
                snapshot_dir.display()
            ));
        }
        if snapshot.retrieved.is_empty() {
            return Err(format!(
                "{}: retrieval date is required",
                snapshot_dir.display()
            ));
        }

        let sources_path = safe_join(&snapshot_dir, &snapshot.sources_file)?;
        validate_yaml_metadata(&sources_path, &meta.join("sources.schema.json"))?;
        let sources: Sources = read_yaml(&sources_path)?;
        require_version(sources.format_version, "sources")?;
        let source_ids: BTreeSet<_> = sources
            .sources
            .iter()
            .map(|source| source.id.as_str())
            .collect();
        if source_ids.len() != sources.sources.len() {
            return Err(format!("{}: duplicate source id", snapshot_dir.display()));
        }
        for source in &sources.sources {
            if source.kind.is_empty()
                || source.authority.is_empty()
                || source.url.is_empty()
                || source.retrieved.is_empty()
            {
                return Err(format!(
                    "{}: source {} is incomplete",
                    snapshot_dir.display(),
                    source.id
                ));
            }
            if source.revision.is_none() && source.content_sha256.is_none() {
                return Err(format!(
                    "{}: source {} needs a revision or content hash",
                    snapshot_dir.display(),
                    source.id
                ));
            }
            if source
                .content_sha256
                .as_deref()
                .is_some_and(|hash| !valid_sha256(hash))
            {
                return Err(format!(
                    "{}: source {} has invalid SHA-256",
                    snapshot_dir.display(),
                    source.id
                ));
            }
            let _ = &source.license;
        }

        let mut event_names = BTreeSet::new();
        let mut event_keys = BTreeSet::new();
        for indexed in &snapshot.events {
            if !event_names.insert(indexed.wire_name.as_str())
                || !event_keys.insert(indexed.rust_key.as_str())
            {
                return Err(format!(
                    "{}: duplicate event name/key",
                    snapshot_dir.display()
                ));
            }
            let dir = safe_join(&snapshot_dir, &indexed.path)?;
            let contract_path = dir.join("contract.yaml");
            validate_yaml_metadata(&contract_path, &meta.join("event-contract.schema.json"))?;
            let contract: Contract = read_yaml(&contract_path)?;
            validate_contract_identity(&contract, &snapshot, indexed, &dir)?;
            if !contract_ids.insert(contract.id.clone()) {
                return Err(format!("duplicate contract id {}", contract.id));
            }
            validate_contract(&contract, &dir, &source_ids)?;
            contracts.push(LoadedContract { contract, dir });
        }
        let indexed_paths: BTreeSet<_> = snapshot
            .events
            .iter()
            .map(|event| snapshot_dir.join(&event.path).join("contract.yaml"))
            .collect();
        let discovered_paths: BTreeSet<_> = walkdir::WalkDir::new(snapshot_dir.join("events"))
            .into_iter()
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_type().is_file() && entry.file_name() == "contract.yaml")
            .map(|entry| entry.into_path())
            .collect();
        if indexed_paths != discovered_paths {
            return Err(format!(
                "{}: snapshot index and event contract directories differ",
                snapshot_dir.display()
            ));
        }
    }

    let target_keys: BTreeSet<_> = stabilization
        .targets
        .iter()
        .map(|target| (target.contract.as_str(), target.binding.as_str()))
        .collect();
    if target_keys.len() != stabilization.targets.len() {
        return Err("stabilization target contains duplicate contract/binding entries".to_string());
    }
    for loaded in &contracts {
        for binding in loaded.contract.bindings.keys() {
            if !target_keys.contains(&(loaded.contract.id.as_str(), binding.as_str()))
                && !stabilization.defaults.iter().any(|default| {
                    default.harness == loaded.contract.harness && default.binding == *binding
                })
            {
                return Err(format!(
                    "missing stabilization target for {} binding {binding}",
                    loaded.contract.id
                ));
            }
        }
    }
    for target in &stabilization.targets {
        let Some(loaded) = contracts
            .iter()
            .find(|loaded| loaded.contract.id == target.contract)
        else {
            return Err(format!(
                "stabilization target references unknown contract {}",
                target.contract
            ));
        };
        if !loaded.contract.bindings.contains_key(&target.binding) {
            return Err(format!(
                "stabilization target references unknown binding {}/{}",
                target.contract, target.binding
            ));
        }
        if !matches!(
            target.level.as_str(),
            "catalog-only"
                | "native-source-reviewed"
                | "command-runtime-beta"
                | "command-runtime-stable"
                | "unsupported"
        ) {
            return Err(format!("invalid support level {}", target.level));
        }
        if target.verification.is_empty() {
            return Err(format!("{} has empty verification status", target.contract));
        }
    }
    for default in &stabilization.defaults {
        if !registry.harnesses.contains_key(&default.harness)
            || !matches!(
                default.level.as_str(),
                "catalog-only"
                    | "native-source-reviewed"
                    | "command-runtime-beta"
                    | "command-runtime-stable"
                    | "unsupported"
            )
            || default.verification.is_empty()
        {
            return Err(format!(
                "invalid stabilization default for {}/{}",
                default.harness, default.binding
            ));
        }
    }
    validate_implementation_gaps(&gaps, &contracts)?;
    let _ = &stabilization.release;
    Ok(contracts)
}

fn validate_contract_identity(
    contract: &Contract,
    snapshot: &Snapshot,
    indexed: &SnapshotEvent,
    dir: &Path,
) -> Result<()> {
    require_version(contract.format_version, "contract")?;
    if contract.harness != snapshot.harness
        || contract.snapshot != snapshot.id
        || contract.event.wire_name != indexed.wire_name
        || contract.event.rust_key != indexed.rust_key
    {
        return Err(format!(
            "{}: indexed contract identity mismatch",
            dir.display()
        ));
    }
    let expected = format!("{}/{}/{}", snapshot.harness, snapshot.id, indexed.wire_name);
    if contract.id != expected {
        return Err(format!(
            "{}: expected contract id {expected}",
            dir.display()
        ));
    }
    Ok(())
}

fn validate_contract(contract: &Contract, dir: &Path, sources: &BTreeSet<&str>) -> Result<()> {
    let input_schema = read_json(&safe_join(dir, &contract.schemas.input.file)?)?;
    validate_schema_references(&input_schema, dir)?;
    validate_schema_claim(&contract.schemas.input, sources, dir)?;
    let input_validator = compile_schema(&input_schema, dir)?;

    let mut output_validators = BTreeMap::new();
    for claim in &contract.schemas.outputs {
        if output_validators.contains_key(claim.id.as_str()) {
            return Err(format!(
                "{}: duplicate output schema id {}",
                dir.display(),
                claim.id
            ));
        }
        validate_sources(&claim.sources, sources, dir)?;
        validate_assurance(&claim.assurance, dir)?;
        if !matches!(claim.origin.as_str(), "vendored" | "derived" | "authored") {
            return Err(format!(
                "{}: invalid schema origin {}",
                dir.display(),
                claim.origin
            ));
        }
        let schema = read_json(&safe_join(dir, &claim.file)?)?;
        validate_schema_references(&schema, dir)?;
        output_validators.insert(claim.id.as_str(), compile_schema(&schema, dir)?);
    }

    if !matches!(
        contract.event.identification.inferability.as_str(),
        "definitive" | "shape-based" | "ambiguous" | "impossible"
    ) {
        return Err(format!("{}: invalid inferability", dir.display()));
    }
    if contract.event.identification.inferability == "definitive"
        && contract.event.identification.discriminator.is_none()
    {
        return Err(format!(
            "{}: definitive event requires discriminator",
            dir.display()
        ));
    }
    if let Some(discriminator) = &contract.event.identification.discriminator
        && (discriminator.json_pointer.is_empty()
            || discriminator.r#const != contract.event.wire_name)
    {
        return Err(format!("{}: invalid discriminator", dir.display()));
    }
    for kind in &contract.handler_kinds {
        if !matches!(
            kind.as_str(),
            "command" | "http" | "mcp_tool" | "prompt" | "agent"
        ) {
            return Err(format!(
                "{}: invalid inventoried handler kind {kind}",
                dir.display()
            ));
        }
    }
    let _ = (
        &contract.event.category,
        &contract.event.identification.overlaps,
        &contract.uncertainties,
    );

    for (binding_id, binding) in &contract.bindings {
        if binding.kind != "process" && binding.kind != "http" {
            return Err(format!(
                "{}: unsupported binding kind {}",
                dir.display(),
                binding.kind
            ));
        }
        if binding.kind == "http"
            && (binding.method.is_none()
                || binding.request_content_type.is_none()
                || binding.response_content_type.is_none())
        {
            return Err(format!(
                "{}: HTTP binding {binding_id} is transport-incomplete",
                dir.display()
            ));
        }
        if binding.request.channel.is_empty()
            || binding.request.framing.is_empty()
            || binding.request.content_kind.is_empty()
        {
            return Err(format!(
                "{}: binding {binding_id} request is incomplete",
                dir.display()
            ));
        }
        let mut outcomes = BTreeSet::new();
        for outcome in &binding.outcomes {
            if !outcomes.insert(outcome.id.as_str()) {
                return Err(format!(
                    "{}: duplicate outcome {}",
                    dir.display(),
                    outcome.id
                ));
            }
            if !valid_exit_selector(&outcome.exit) || outcome.effect.is_empty() {
                return Err(format!(
                    "{}: outcome {} is incomplete",
                    dir.display(),
                    outcome.id
                ));
            }
            validate_channel(&outcome.stdout, dir)?;
            validate_channel(&outcome.stderr, dir)?;
            validate_sources(&outcome.sources, sources, dir)?;
            validate_assurance(&outcome.assurance, dir)?;
            if let Some(schema) = &outcome.output_schema {
                if !output_validators.contains_key(schema.as_str()) {
                    return Err(format!(
                        "{}: outcome {} references unknown output schema {schema}",
                        dir.display(),
                        outcome.id
                    ));
                }
                if outcome.stdout.content_kind != "json" {
                    return Err(format!(
                        "{}: JSON output schema on non-JSON outcome {}",
                        dir.display(),
                        outcome.id
                    ));
                }
            }
        }
    }

    let fixtures_path = safe_join(dir, &contract.fixtures)?;
    let meta_path = dir
        .ancestors()
        .find(|path| path.file_name().is_some_and(|name| name == "contracts"))
        .expect("event contracts live under contracts")
        .join("meta/v1/fixtures.schema.json");
    validate_yaml_metadata(&fixtures_path, &meta_path)?;
    let fixtures: Fixtures = read_yaml(&fixtures_path)?;
    require_version(fixtures.format_version, "fixtures")?;
    if fixtures.input.positive.len() < 2 || fixtures.input.negative.len() < 2 {
        return Err(format!(
            "{}: needs minimal/representative positive and at least two negative inputs",
            dir.display()
        ));
    }
    for fixture in &fixtures.input.positive {
        validate_fixture_provenance(&fixture.id, &fixture.origin, &fixture.sources, sources, dir)?;
        input_validator.validate(&fixture.value).map_err(|error| {
            format!(
                "{}: positive input {} failed at {}: {error}",
                dir.display(),
                fixture.id,
                error.instance_path()
            )
        })?;
    }
    for fixture in &fixtures.input.negative {
        validate_fixture_provenance(&fixture.id, &fixture.origin, &fixture.sources, sources, dir)?;
        let errors: Vec<_> = input_validator.iter_errors(&fixture.value).collect();
        if errors.is_empty() {
            return Err(format!(
                "{}: negative input {} unexpectedly passed",
                dir.display(),
                fixture.id
            ));
        }
        if !errors
            .iter()
            .any(|error| error.instance_path().as_str() == fixture.expected_pointer)
        {
            return Err(format!(
                "{}: negative input {} did not fail at expected pointer {}",
                dir.display(),
                fixture.id,
                fixture.expected_pointer
            ));
        }
    }
    for fixture in &fixtures.output {
        validate_fixture_provenance(&fixture.id, &fixture.origin, &fixture.sources, sources, dir)?;
        let validator = output_validators
            .get(fixture.schema.as_str())
            .ok_or_else(|| {
                format!(
                    "{}: output fixture {} references unknown schema",
                    dir.display(),
                    fixture.id
                )
            })?;
        validator.validate(&fixture.value).map_err(|error| {
            format!(
                "{}: output fixture {} failed at {}: {error}",
                dir.display(),
                fixture.id,
                error.instance_path()
            )
        })?;
    }
    for fixture in &fixtures.process {
        validate_process_fixture(fixture, contract, &output_validators, dir)?;
    }
    Ok(())
}

fn validate_schema_claim(claim: &SchemaClaim, sources: &BTreeSet<&str>, dir: &Path) -> Result<()> {
    if !matches!(claim.origin.as_str(), "vendored" | "derived" | "authored") {
        return Err(format!(
            "{}: invalid schema origin {}",
            dir.display(),
            claim.origin
        ));
    }
    validate_sources(&claim.sources, sources, dir)?;
    validate_assurance(&claim.assurance, dir)
}

fn validate_assurance(assurance: &Assurance, dir: &Path) -> Result<()> {
    if !matches!(assurance.confidence.as_str(), "high" | "medium" | "low") {
        return Err(format!("{}: invalid assurance confidence", dir.display()));
    }
    if !matches!(
        assurance.verification.as_str(),
        "unverified" | "fixture-validated" | "source-reviewed" | "live-observed"
    ) {
        return Err(format!("{}: invalid assurance verification", dir.display()));
    }
    Ok(())
}

fn validate_sources(claimed: &[String], sources: &BTreeSet<&str>, dir: &Path) -> Result<()> {
    if claimed.is_empty() {
        return Err(format!("{}: claim has no sources", dir.display()));
    }
    for source in claimed {
        if !sources.contains(source.as_str()) {
            return Err(format!("{}: unknown source {source}", dir.display()));
        }
    }
    Ok(())
}

fn validate_fixture_provenance(
    id: &str,
    origin: &str,
    claimed: &[String],
    sources: &BTreeSet<&str>,
    dir: &Path,
) -> Result<()> {
    if id.is_empty()
        || !matches!(
            origin,
            "official" | "sanitized-live" | "synthesized" | "regression"
        )
    {
        return Err(format!(
            "{}: invalid fixture provenance for {id}",
            dir.display()
        ));
    }
    validate_sources(claimed, sources, dir)
}

fn validate_channel(channel: &Channel, dir: &Path) -> Result<()> {
    if !matches!(
        channel.presence.as_str(),
        "required" | "optional" | "forbidden"
    ) || !matches!(
        channel.content_kind.as_str(),
        "empty" | "json" | "text" | "opaque"
    ) || channel.role.is_empty()
    {
        return Err(format!("{}: invalid channel metadata", dir.display()));
    }
    if channel.content_kind == "text" && channel.encoding.as_deref() != Some("utf-8") {
        return Err(format!(
            "{}: text channel must declare UTF-8",
            dir.display()
        ));
    }
    let _ = (&channel.semantic_format, &channel.trailing_newline);
    Ok(())
}

fn validate_process_fixture(
    fixture: &ProcessFixture,
    contract: &Contract,
    validators: &BTreeMap<&str, jsonschema::Validator>,
    dir: &Path,
) -> Result<()> {
    let binding = contract.bindings.get(&fixture.binding).ok_or_else(|| {
        format!(
            "{}: process fixture {} references unknown binding",
            dir.display(),
            fixture.id
        )
    })?;
    let outcome = binding
        .outcomes
        .iter()
        .find(|outcome| outcome.id == fixture.outcome)
        .ok_or_else(|| {
            format!(
                "{}: process fixture {} references unknown outcome",
                dir.display(),
                fixture.id
            )
        })?;
    if !exit_matches(&outcome.exit, fixture.exit_code) {
        return Err(format!(
            "{}: process fixture {} has invalid exit code",
            dir.display(),
            fixture.id
        ));
    }
    let stdout = decode_exact(
        &fixture.stdout_base64,
        &fixture.stdout_sha256,
        dir,
        &fixture.id,
        "stdout",
    )?;
    let stderr = decode_exact(
        &fixture.stderr_base64,
        &fixture.stderr_sha256,
        dir,
        &fixture.id,
        "stderr",
    )?;
    validate_channel_bytes(&outcome.stdout, &stdout, dir, &fixture.id, "stdout")?;
    validate_channel_bytes(&outcome.stderr, &stderr, dir, &fixture.id, "stderr")?;
    if let Some(schema) = &outcome.output_schema {
        let value: Value = serde_json::from_slice(&stdout).map_err(|error| {
            format!(
                "{}: process fixture {} stdout is not JSON: {error}",
                dir.display(),
                fixture.id
            )
        })?;
        validators[schema.as_str()]
            .validate(&value)
            .map_err(|error| {
                format!(
                    "{}: process fixture {} output failed schema: {error}",
                    dir.display(),
                    fixture.id
                )
            })?;
    }
    Ok(())
}

fn decode_exact(
    encoded: &str,
    expected_hash: &str,
    dir: &Path,
    id: &str,
    channel: &str,
) -> Result<Vec<u8>> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| {
            format!(
                "{}: process fixture {id} invalid {channel} base64: {error}",
                dir.display()
            )
        })?;
    let actual = hex_sha256(&bytes);
    if actual != expected_hash {
        return Err(format!(
            "{}: process fixture {id} {channel} SHA-256 mismatch",
            dir.display()
        ));
    }
    Ok(bytes)
}

fn validate_channel_bytes(
    channel: &Channel,
    bytes: &[u8],
    dir: &Path,
    id: &str,
    name: &str,
) -> Result<()> {
    match channel.presence.as_str() {
        "required" if bytes.is_empty() => {
            return Err(format!(
                "{}: process fixture {id} requires {name}",
                dir.display()
            ));
        }
        "forbidden" if !bytes.is_empty() => {
            return Err(format!(
                "{}: process fixture {id} forbids {name}",
                dir.display()
            ));
        }
        _ => {}
    }
    match channel.content_kind.as_str() {
        "empty" if !bytes.is_empty() => {
            return Err(format!(
                "{}: process fixture {id} {name} must be zero bytes",
                dir.display()
            ));
        }
        "json" if !bytes.is_empty() => {
            serde_json::from_slice::<Value>(bytes).map_err(|error| {
                format!(
                    "{}: process fixture {id} invalid {name} JSON: {error}",
                    dir.display()
                )
            })?;
        }
        "text" if std::str::from_utf8(bytes).is_err() => {
            return Err(format!(
                "{}: process fixture {id} {name} is not UTF-8",
                dir.display()
            ));
        }
        _ => {}
    }
    if channel.trailing_newline.as_deref() == Some("forbidden") && bytes.ends_with(b"\n") {
        return Err(format!(
            "{}: process fixture {id} {name} has forbidden newline",
            dir.display()
        ));
    }
    Ok(())
}

fn compile_schema(schema: &Value, dir: &Path) -> Result<jsonschema::Validator> {
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .build(schema)
        .map_err(|error| format!("{}: invalid JSON Schema: {error}", dir.display()))
}

fn render_report(root: &Path, contracts: &[LoadedContract]) -> Result<String> {
    let status: Stabilization = read_yaml(&root.join("contracts/status/stabilization-v1.yaml"))?;
    let mut rows = Vec::new();
    for loaded in contracts {
        for binding in loaded.contract.bindings.keys() {
            let target = status
                .targets
                .iter()
                .find(|target| target.contract == loaded.contract.id && target.binding == *binding)
                .map(|target| (target.level.as_str(), target.verification.as_str()))
                .or_else(|| {
                    status
                        .defaults
                        .iter()
                        .find(|default| {
                            default.harness == loaded.contract.harness
                                && default.binding == *binding
                        })
                        .map(|default| (default.level.as_str(), default.verification.as_str()))
                })
                .expect("checked target");
            rows.push((
                loaded.contract.harness.as_str(),
                loaded.contract.event.wire_name.as_str(),
                binding.as_str(),
                target.0,
                target.1,
            ));
        }
        let _ = &loaded.dir;
    }
    rows.sort_unstable();
    let mut output = format!(
        "# Generated support report\n\nRelease target: `{}`. Generated by `cargo xtask contracts report`.\n\n",
        status.release
    );
    output.push_str("| Harness | Event | Binding | Inventoried | Input schema | Output/process | Native input | Native output | Command runtime | Other runtime | Hermetic conformance | Live verified | Release target | Verification |\n| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n");
    for (harness, event, binding, level, verification) in rows {
        let (command, other) = if binding == "command" {
            ("legacy/unknown", "n/a")
        } else {
            ("n/a", "legacy/unknown")
        };
        output.push_str(&format!(
            "| {harness} | `{event}` | `{binding}` | yes | yes | yes | legacy/unknown | legacy/unknown | {command} | {other} | not yet measured | no | `{level}` | `{verification}` |\n"
        ));
    }
    Ok(output)
}

fn freeze_snapshot(root: &Path, harness: &str, snapshot_id: &str) -> Result<()> {
    let directory = root
        .join("contracts/harnesses")
        .join(harness)
        .join("snapshots")
        .join(snapshot_id);
    let snapshot_path = directory.join("snapshot.yaml");
    let mut snapshot: Snapshot = read_yaml(&snapshot_path)?;
    if snapshot.harness != harness || snapshot.id != snapshot_id {
        return Err(format!(
            "{}: snapshot identity mismatch",
            directory.display()
        ));
    }
    if snapshot.state != "draft" {
        return Err(format!(
            "{}: only draft snapshots can be frozen",
            directory.display()
        ));
    }
    let manifest_name = "MANIFEST.sha256";
    let manifest = content_manifest(&directory, manifest_name)?;
    fs::write(directory.join(manifest_name), manifest)
        .map_err(|error| format!("{}: {error}", directory.display()))?;
    snapshot.state = "frozen".to_string();
    snapshot.manifest_file = Some(manifest_name.to_string());
    let yaml = serde_yaml_ng::to_string(&snapshot)
        .map_err(|error| format!("{}: {error}", snapshot_path.display()))?;
    fs::write(&snapshot_path, yaml)
        .map_err(|error| format!("{}: {error}", snapshot_path.display()))?;
    verify_content_manifest(&directory, manifest_name)?;
    println!("froze {harness}/{snapshot_id}");
    Ok(())
}

fn content_manifest(directory: &Path, manifest_name: &str) -> Result<String> {
    let mut paths: Vec<_> = walkdir::WalkDir::new(directory)
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| format!("{}: {error}", directory.display()))?
        .into_iter()
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .filter(|path| {
            path.file_name()
                .is_none_or(|name| name != "snapshot.yaml" && name != manifest_name)
        })
        .collect();
    paths.sort();
    let mut output = String::new();
    for path in paths {
        let relative = path
            .strip_prefix(directory)
            .expect("walked path below snapshot")
            .to_string_lossy();
        let bytes = fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        writeln!(&mut output, "{}  {relative}", hex_sha256(&bytes))
            .expect("writing to String cannot fail");
    }
    Ok(output)
}

fn verify_content_manifest(directory: &Path, manifest_name: &str) -> Result<()> {
    let manifest_path = safe_join(directory, manifest_name)?;
    let actual = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("{}: {error}", manifest_path.display()))?;
    let expected = content_manifest(directory, manifest_name)?;
    if actual != expected {
        return Err(format!(
            "{}: frozen snapshot content differs from deterministic manifest",
            directory.display()
        ));
    }
    Ok(())
}

fn validate_implementation_gaps(
    gaps: &ImplementationGaps,
    contracts: &[LoadedContract],
) -> Result<()> {
    let mut keys = BTreeSet::new();
    for gap in &gaps.gaps {
        let key = (
            gap.harness.as_str(),
            gap.snapshot.as_str(),
            gap.event.as_str(),
            gap.binding.as_str(),
            gap.assertion.as_str(),
        );
        if !keys.insert(key) {
            return Err(format!(
                "duplicate implementation gap for {}/{}/{}/{} assertion {}",
                gap.harness, gap.snapshot, gap.event, gap.binding, gap.assertion
            ));
        }
        let contract_id = format!("{}/{}/{}", gap.harness, gap.snapshot, gap.event);
        let Some(contract) = contracts
            .iter()
            .find(|loaded| loaded.contract.id == contract_id)
        else {
            return Err(format!(
                "implementation gap references unknown {contract_id}"
            ));
        };
        if !contract.contract.bindings.contains_key(&gap.binding) {
            return Err(format!(
                "implementation gap references unknown binding {contract_id}/{}",
                gap.binding
            ));
        }
        if gap.assertion.is_empty()
            || gap.rationale.is_empty()
            || gap.owner.is_empty()
            || !(2..=6).contains(&gap.removal_phase)
        {
            return Err(format!("implementation gap {contract_id} is incomplete"));
        }
        if gap.expiry.as_deref().is_some_and(|date| {
            date.len() != 10
                || date.as_bytes()[4] != b'-'
                || date.as_bytes()[7] != b'-'
                || !date
                    .bytes()
                    .enumerate()
                    .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
        }) {
            return Err(format!(
                "implementation gap {contract_id} has invalid expiry"
            ));
        }
    }
    Ok(())
}

fn safe_join(root: &Path, relative: impl AsRef<Path>) -> Result<PathBuf> {
    let relative = relative.as_ref();
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(format!("unsafe catalog path {}", relative.display()));
    }
    let joined = root.join(relative);
    if joined.exists() {
        let canonical_root = root
            .canonicalize()
            .map_err(|error| format!("{}: {error}", root.display()))?;
        let canonical = joined
            .canonicalize()
            .map_err(|error| format!("{}: {error}", joined.display()))?;
        if !canonical.starts_with(&canonical_root) {
            return Err(format!("catalog path escapes root: {}", joined.display()));
        }
    }
    Ok(joined)
}

fn validate_schema_references(schema: &Value, directory: &Path) -> Result<()> {
    match schema {
        Value::Object(object) => {
            if let Some(Value::String(reference)) = object.get("$ref")
                && !reference.starts_with('#')
            {
                return Err(format!(
                    "{}: non-fragment $ref is not registered for offline resolution: {reference}",
                    directory.display()
                ));
            }
            for value in object.values() {
                validate_schema_references(value, directory)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                validate_schema_references(value, directory)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn verify_vendor(root: &Path) -> Result<usize> {
    let vendor = root.join("contracts/vendor");
    if !vendor.exists() {
        return Ok(0);
    }
    let manifests: Vec<_> = walkdir::WalkDir::new(&vendor)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file() && entry.file_name() == "MANIFEST.sha256")
        .map(|entry| entry.into_path())
        .collect();
    let mut count = 0;
    for manifest in manifests {
        let directory = manifest.parent().expect("manifest parent");
        let text = fs::read_to_string(&manifest)
            .map_err(|error| format!("{}: {error}", manifest.display()))?;
        for (line_number, line) in text.lines().enumerate() {
            let (expected, relative) = line.split_once("  ").ok_or_else(|| {
                format!(
                    "{}:{}: expected '<sha256>  <file>'",
                    manifest.display(),
                    line_number + 1
                )
            })?;
            if !valid_sha256(expected)
                || relative.contains("..")
                || Path::new(relative).is_absolute()
            {
                return Err(format!(
                    "{}:{}: invalid vendor manifest entry",
                    manifest.display(),
                    line_number + 1
                ));
            }
            let path = safe_join(directory, relative)?;
            let bytes = fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
            if hex_sha256(&bytes) != expected {
                return Err(format!("{}: vendored checksum mismatch", path.display()));
            }
            count += 1;
        }
    }
    Ok(count)
}

fn diff_snapshots(root: &Path, old: &str, new: &str) -> Result<()> {
    let snapshots = root.join("contracts/harnesses");
    let old_dir = find_snapshot(&snapshots, old)?;
    let new_dir = find_snapshot(&snapshots, new)?;
    let old_snapshot: Snapshot = read_yaml(&old_dir.join("snapshot.yaml"))?;
    let new_snapshot: Snapshot = read_yaml(&new_dir.join("snapshot.yaml"))?;
    let old_events: BTreeSet<_> = old_snapshot
        .events
        .iter()
        .map(|event| event.wire_name.as_str())
        .collect();
    let new_events: BTreeSet<_> = new_snapshot
        .events
        .iter()
        .map(|event| event.wire_name.as_str())
        .collect();
    println!(
        "old: {}/{} ({} events)",
        old_snapshot.harness,
        old_snapshot.id,
        old_events.len()
    );
    println!(
        "new: {}/{} ({} events)",
        new_snapshot.harness,
        new_snapshot.id,
        new_events.len()
    );
    for event in new_events.difference(&old_events) {
        println!("+ {event}");
    }
    for event in old_events.difference(&new_events) {
        println!("- {event}");
    }
    let old_hash = hash_tree(&old_dir)?;
    let new_hash = hash_tree(&new_dir)?;
    println!(
        "content: {}",
        if old_hash == new_hash {
            "identical"
        } else {
            "changed"
        }
    );
    Ok(())
}

fn find_snapshot(root: &Path, id: &str) -> Result<PathBuf> {
    let matches: Vec<_> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_dir() && entry.file_name() == id)
        .map(|entry| entry.into_path())
        .collect();
    match matches.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err(format!("snapshot {id} not found")),
        _ => Err(format!("snapshot id {id} is ambiguous across harnesses")),
    }
}

fn hash_tree(root: &Path) -> Result<String> {
    let mut files: Vec<_> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .collect();
    files.sort();
    let mut digest = Sha256::new();
    for path in files {
        let relative = path.strip_prefix(root).expect("walked path below root");
        digest.update(relative.to_string_lossy().as_bytes());
        digest.update([0]);
        digest.update(fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?);
        digest.update([0]);
    }
    let bytes = digest.finalize();
    let mut output = String::with_capacity(64);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(output)
}

fn read_yaml<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if bytes.windows(2).any(|window| window == b"<<") {
        return Err(format!("{}: YAML merge keys are forbidden", path.display()));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| format!("{}: YAML must be UTF-8: {error}", path.display()))?;
    for (index, line) in text.lines().enumerate() {
        let content = line.split('#').next().unwrap_or_default();
        if content.contains(": &")
            || content.trim_start().starts_with('&')
            || content.trim_start().starts_with('*')
            || content.contains(": *")
        {
            return Err(format!(
                "{}:{}: YAML anchors and aliases are forbidden",
                path.display(),
                index + 1
            ));
        }
    }
    serde_yaml_ng::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))
}

fn validate_yaml_metadata(path: &Path, meta_schema: &Path) -> Result<()> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let yaml: serde_yaml_ng::Value = serde_yaml_ng::from_slice(&bytes)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let value = serde_json::to_value(yaml)
        .map_err(|error| format!("{}: cannot normalize YAML: {error}", path.display()))?;
    let schema = read_json(meta_schema)?;
    compile_schema(&schema, meta_schema)?
        .validate(&value)
        .map_err(|error| {
            format!(
                "{}: metadata failed {} at {}: {error}",
                path.display(),
                meta_schema.display(),
                error.instance_path()
            )
        })
}

fn read_json(path: &Path) -> Result<Value> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))
}

fn require_version(version: u32, kind: &str) -> Result<()> {
    if version == 1 {
        Ok(())
    } else {
        Err(format!("unsupported {kind} format version {version}"))
    }
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_exit_selector(selector: &ExitSelector) -> bool {
    let forms = usize::from(selector.exact.is_some())
        + usize::from(!selector.set.is_empty())
        + usize::from(selector.range.is_some());
    forms == 1
        && selector
            .range
            .as_ref()
            .is_none_or(|range| range.min <= range.max)
}

fn exit_matches(selector: &ExitSelector, code: i32) -> bool {
    selector.exact == Some(code)
        || selector.set.contains(&code)
        || selector
            .range
            .as_ref()
            .is_some_and(|range| (range.min..=range.max).contains(&code))
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str, contents: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "hookkit-xtask-{name}-{}-{}.yaml",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        fs::write(&path, contents).expect("write temporary YAML");
        path
    }

    #[test]
    fn duplicate_yaml_keys_are_rejected() {
        let path = temp_file(
            "duplicate",
            "format_version: 1\nformat_version: 1\nharnesses: {}\n",
        );
        let error = read_yaml::<Registry>(&path).expect_err("duplicate key must fail");
        assert!(error.contains("duplicate field") || error.contains("duplicate entry"));
        fs::remove_file(path).expect("remove temporary YAML");
    }

    #[test]
    fn yaml_anchors_are_rejected() {
        let path = temp_file("anchor", "format_version: 1\nharnesses: &shared {}\n");
        let error = read_yaml::<Registry>(&path).expect_err("anchor must fail");
        assert!(error.contains("anchors and aliases are forbidden"));
        fs::remove_file(path).expect("remove temporary YAML");
    }

    #[test]
    fn exit_selectors_are_exclusive_and_support_ranges() {
        assert!(valid_exit_selector(&ExitSelector {
            exact: Some(0),
            set: vec![],
            range: None,
        }));
        assert!(exit_matches(
            &ExitSelector {
                exact: None,
                set: vec![],
                range: Some(ExitRange { min: 1, max: 255 }),
            },
            127
        ));
        assert!(!valid_exit_selector(&ExitSelector {
            exact: Some(0),
            set: vec![0],
            range: None,
        }));
    }

    #[test]
    fn catalog_paths_cannot_escape_their_root() {
        let root = std::env::temp_dir();
        assert!(safe_join(&root, "nested/file.json").is_ok());
        assert!(safe_join(&root, "../secret").is_err());
        assert!(safe_join(&root, "/etc/passwd").is_err());
    }
}
