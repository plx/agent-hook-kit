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
    /// Validate or synchronize the Copier event and alignment catalog.
    TemplateCatalog {
        #[command(subcommand)]
        command: TemplateCatalogCommand,
    },
}

#[derive(Subcommand)]
enum TemplateCatalogCommand {
    /// Validate registry coverage, alignment members, and generated question data.
    Check,
    /// Validate the canonical catalog and refresh its generated Copier question data.
    Sync,
}

#[derive(Subcommand)]
enum ContractsCommand {
    /// Validate every snapshot's metadata, schemas, and fixtures plus the
    /// registry-selected status overlays.
    Check,
    /// Render the generated support report.
    Report {
        /// Update contracts/status/support.md.
        #[arg(long, conflicts_with = "check")]
        write: bool,
        /// Fail unless contracts/status/support.md is current.
        #[arg(long)]
        check: bool,
    },
    /// Compare event inventories and per-event contract content between two
    /// snapshot IDs, ignoring the snapshot-ID tokens every file embeds.
    ///
    /// Qualify IDs shared by multiple harnesses as `harness/snapshot`.
    Diff { old: String, new: String },
    /// Verify vendored files against their checked-in SHA-256 manifests.
    VerifyVendor,
    /// Print the registry-selected snapshots' upstream sources as tab-separated
    /// rows for scripts/check-upstream-contract-drift.sh.
    UpstreamSources,
    /// Freeze a reviewed snapshot with a deterministic SHA-256 manifest.
    Freeze { harness: String, snapshot: String },
    /// Freeze a reviewed command-environment supplement.
    FreezeCommandEnvironments { supplement: String },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    format_version: u32,
    harnesses: BTreeMap<String, RegistryHarness>,
    supplements: RegistrySupplements,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryHarness {
    current: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistrySupplements {
    command_environments: String,
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

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CommandEnvironmentSupplement {
    format_version: u32,
    id: String,
    state: String,
    retrieved: String,
    sources_file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    manifest_file: Option<String>,
    harnesses: BTreeMap<String, CommandEnvironmentHarness>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CommandEnvironmentHarness {
    snapshot: String,
    sources: Vec<String>,
    assurance: Assurance,
    profiles: Vec<CommandEnvironmentProfile>,
    event_profiles: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CommandEnvironmentProfile {
    id: String,
    activation: CommandEnvironmentActivation,
    completeness: String,
    variables: Vec<CommandEnvironmentVariable>,
    sources: Vec<String>,
    assurance: Assurance,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CommandEnvironmentActivation {
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    condition: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CommandEnvironmentVariable {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prefix: Option<String>,
    value: CommandEnvironmentValue,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CommandEnvironmentValue {
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    literal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    allow_empty: Option<bool>,
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
    #[serde(default)]
    reproducibility: Option<String>,
    #[serde(default)]
    limitations: Vec<String>,
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

#[derive(Debug, Deserialize, Serialize)]
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
    #[serde(default)]
    output_negative: Vec<NegativeOutputFixture>,
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
    #[serde(default)]
    expected_keyword: Option<String>,
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
struct NegativeOutputFixture {
    id: String,
    schema: String,
    origin: String,
    sources: Vec<String>,
    value: Value,
    expected_pointer: String,
    expected_keyword: String,
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImplementationRegistry {
    format_version: u32,
    events: Vec<ImplementationEvent>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ImplementationEvent {
    contract: String,
    harness: String,
    event: String,
    native_input: bool,
    native_output: bool,
    bindings: Vec<String>,
    conformance_cases: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EventScaffoldCatalog {
    format_version: u32,
    events: Vec<EventScaffold>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EventScaffold {
    contract: String,
    harness: String,
    snapshot: String,
    wire_event: String,
    stable_id: String,
    display_name: String,
    category: String,
    order: u16,
    rust_event: String,
    rust_output: String,
    selector: ScaffoldSelector,
    starter: ScaffoldStarter,
    fixture: ScaffoldFixture,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    aligned_family: Option<String>,
    capabilities: ScaffoldCapabilities,
    support: ScaffoldSupport,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ScaffoldSelector {
    rust_type: String,
    variant: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ScaffoldStarter {
    strategy: String,
    expression: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ScaffoldFixture {
    source: String,
    case: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    environment: Option<String>,
    /// Representative input materialized only in generated Copier data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ScaffoldCapabilities {
    input_rewrite: bool,
    true_pre_action_block: bool,
    post_action_feedback: bool,
    output_ignored: bool,
    output_advisory: bool,
    tool_result: bool,
    separate_user_agent_messages: bool,
    session_boundary: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ScaffoldSupport {
    status: String,
    note: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AlignmentCatalog {
    format_version: u32,
    alignments: Vec<AlignmentFamily>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AlignmentFamily {
    stable_id: String,
    display_name: String,
    order: u16,
    runtime_marker: String,
    supported_harnesses: Vec<String>,
    native_members: BTreeMap<String, String>,
    portable_floor: PortableFloor,
    capabilities: AlignmentCapabilities,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PortableFloor {
    input: String,
    output: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AlignmentCapabilities {
    input_rewrite: bool,
    true_pre_action_block: bool,
    post_action_feedback: bool,
    tool_result: bool,
    separate_user_agent_messages: bool,
    exact_session_boundary: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ArchetypeCatalog {
    format_version: u32,
    archetypes: Vec<Archetype>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Archetype {
    stable_id: String,
    display_name: String,
    order: u16,
    default_mode: String,
    supported_modes: Vec<String>,
    supported_harnesses: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    default_harnesses: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_harness: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    required_hooks: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    default_cross_hooks: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    default_single_hooks: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    required_state: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runner: Option<ArchetypeRunner>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ArchetypeRunner {
    kind: String,
    default_lowering_policy: String,
    default_quality_profile: String,
    default_quality_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_reconciliation_posture: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CompatibilityCatalog {
    format_version: u32,
    template_version: String,
    copier_version: String,
    rust_msrv: String,
    pkl_version: String,
    hookkit: HookkitCompatibility,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HookkitCompatibility {
    repository: String,
    git_revision: String,
    crates_io_version: String,
}

#[derive(Serialize)]
struct GeneratedQuestionCatalog<'a> {
    event_catalog: HiddenCatalogQuestion<'a, EventScaffoldCatalog>,
    alignment_catalog: HiddenCatalogQuestion<'a, AlignmentCatalog>,
    archetype_catalog: HiddenCatalogQuestion<'a, ArchetypeCatalog>,
    compatibility: HiddenCatalogQuestion<'a, CompatibilityCatalog>,
}

#[derive(Serialize)]
struct HiddenCatalogQuestion<'a, T> {
    r#type: &'static str,
    when: bool,
    default: &'a T,
}

struct LoadedContract {
    contract: Contract,
    dir: PathBuf,
    process_cases: BTreeMap<String, String>,
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
            command: ContractsCommand::Check,
        } => {
            let contracts = check_catalog(&root)?;
            println!(
                "validated {} selected event contracts, all catalog snapshots, and command-environment supplements",
                contracts.len()
            );
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
            command: ContractsCommand::UpstreamSources,
        } => {
            print!("{}", render_upstream_sources(&root)?);
            Ok(())
        }
        Command::Contracts {
            command: ContractsCommand::Freeze { harness, snapshot },
        } => freeze_snapshot(&root, &harness, &snapshot),
        Command::Contracts {
            command: ContractsCommand::FreezeCommandEnvironments { supplement },
        } => freeze_command_environment_supplement(&root, &supplement),
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
        Command::TemplateCatalog {
            command: TemplateCatalogCommand::Check,
        } => {
            let (events, alignments, archetypes, compatibility) = check_template_catalog(&root)?;
            check_generated_question_catalog(
                &root,
                &events,
                &alignments,
                &archetypes,
                &compatibility,
            )?;
            check_generated_fixture_templates(&root, &events)?;
            check_generated_handler_templates(&root, &events, &alignments)?;
            println!(
                "validated {} event scaffolds, {} alignment families, and {} archetypes",
                events.events.len(),
                alignments.alignments.len(),
                archetypes.archetypes.len()
            );
            Ok(())
        }
        Command::TemplateCatalog {
            command: TemplateCatalogCommand::Sync,
        } => {
            let (events, alignments, archetypes, compatibility) = check_template_catalog(&root)?;
            write_generated_question_catalog(
                &root,
                &events,
                &alignments,
                &archetypes,
                &compatibility,
            )?;
            write_generated_fixture_templates(&root, &events)?;
            write_generated_handler_templates(&root, &events, &alignments)?;
            println!(
                "synchronized {} event scaffolds, {} alignment families, and {} archetypes",
                events.events.len(),
                alignments.alignments.len(),
                archetypes.archetypes.len()
            );
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

fn check_template_catalog(
    root: &Path,
) -> Result<(
    EventScaffoldCatalog,
    AlignmentCatalog,
    ArchetypeCatalog,
    CompatibilityCatalog,
)> {
    let catalog_root = root.join("templates/hook-project/catalog");
    let events_path = catalog_root.join("event-scaffolds.yml");
    let alignments_path = catalog_root.join("alignment-families.yml");
    let archetypes_path = catalog_root.join("archetypes.yml");
    let compatibility_path = catalog_root.join("compatibility.yml");
    let implementation_path = root.join("contracts/status/implementation/registry.json");

    let mut events: EventScaffoldCatalog = read_yaml(&events_path)?;
    let alignments: AlignmentCatalog = read_yaml(&alignments_path)?;
    let archetypes: ArchetypeCatalog = read_yaml(&archetypes_path)?;
    let compatibility: CompatibilityCatalog = read_yaml(&compatibility_path)?;
    let implementation: ImplementationRegistry =
        serde_json::from_value(read_json(&implementation_path)?)
            .map_err(|error| format!("invalid implementation registry: {error}"))?;

    validate_template_catalog(
        root,
        &events,
        &alignments,
        &archetypes,
        &compatibility,
        &implementation,
    )?;
    check_toolchain_versions(root, &compatibility)?;
    hydrate_template_fixture_values(root, &mut events)?;
    Ok((events, alignments, archetypes, compatibility))
}

/// Check that toolchain versions repeated as literals elsewhere in the
/// repository agree with the template compatibility catalog, their single
/// source of truth. Scripts that can read the catalog at run time do so and
/// need no literal; `required` rules guard pins that must stay literal.
fn check_toolchain_versions(root: &Path, compatibility: &CompatibilityCatalog) -> Result<()> {
    let rules = [
        (
            "copier.yml",
            "_min_copier_version: \"",
            &compatibility.copier_version,
            true,
        ),
        (
            "templates/hook-project/tests/run.sh",
            "copier==",
            &compatibility.copier_version,
            true,
        ),
        (
            "Cargo.toml",
            "rust-version = \"",
            &compatibility.rust_msrv,
            true,
        ),
        (
            ".github/workflows/ci.yml",
            "dtolnay/rust-toolchain@",
            &compatibility.rust_msrv,
            true,
        ),
        (
            ".github/workflows/ci.yml",
            "pkl/releases/download/",
            &compatibility.pkl_version,
            false,
        ),
        (
            "scripts/release-check.sh",
            "cargo +",
            &compatibility.rust_msrv,
            false,
        ),
    ];
    for (relative, prefix, expected, required) in rules {
        let path = root.join(relative);
        let text =
            fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        let versions = pinned_versions(&text, prefix);
        if required && versions.is_empty() {
            return Err(format!(
                "{}: expected a `{prefix}<version>` pin matching the template compatibility catalog",
                path.display()
            ));
        }
        if let Some(version) = versions
            .iter()
            .find(|version| !same_version(version, expected))
        {
            return Err(format!(
                "{}: `{prefix}{version}` disagrees with {expected} in templates/hook-project/catalog/compatibility.yml",
                path.display()
            ));
        }
    }
    Ok(())
}

/// Numeric versions immediately following each occurrence of `prefix`;
/// non-numeric references such as `@stable` or `${version}` are skipped.
fn pinned_versions<'a>(text: &'a str, prefix: &str) -> Vec<&'a str> {
    text.match_indices(prefix)
        .filter_map(|(index, _)| {
            let rest = &text[index + prefix.len()..];
            let end = rest
                .find(|character: char| !(character.is_ascii_digit() || character == '.'))
                .unwrap_or(rest.len());
            let version = rest[..end].trim_end_matches('.');
            (!version.is_empty()).then_some(version)
        })
        .collect()
}

/// Whether `actual` names `expected`, allowing Cargo's `1.85` shorthand for
/// `1.85.0`.
fn same_version(actual: &str, expected: &str) -> bool {
    actual == expected || expected.strip_suffix(".0") == Some(actual)
}

fn hydrate_template_fixture_values(root: &Path, events: &mut EventScaffoldCatalog) -> Result<()> {
    for event in &mut events.events {
        let fixtures: Fixtures = read_yaml(&root.join(&event.fixture.source))?;
        let value = fixtures
            .input
            .positive
            .into_iter()
            .find(|case| case.id == event.fixture.case)
            .map(|case| case.value)
            .ok_or_else(|| {
                format!(
                    "event scaffold {} fixture source has no positive case {}",
                    event.contract, event.fixture.case
                )
            })?;
        event.fixture.value = Some(value);
    }
    Ok(())
}

fn validate_template_catalog(
    root: &Path,
    events: &EventScaffoldCatalog,
    alignments: &AlignmentCatalog,
    archetypes: &ArchetypeCatalog,
    compatibility: &CompatibilityCatalog,
    implementation: &ImplementationRegistry,
) -> Result<()> {
    require_version(events.format_version, "event scaffold catalog")?;
    require_version(alignments.format_version, "alignment catalog")?;
    require_version(archetypes.format_version, "archetype catalog")?;
    require_version(
        compatibility.format_version,
        "template compatibility catalog",
    )?;
    require_version(
        implementation.format_version,
        "implementation registry used by template catalog",
    )?;

    let implemented: BTreeMap<_, _> = implementation
        .events
        .iter()
        .filter(|event| event.bindings.iter().any(|binding| binding == "command"))
        .map(|event| (event.contract.as_str(), event))
        .collect();
    let mut scaffold_by_contract = BTreeMap::new();
    let mut order_keys = BTreeSet::new();
    for event in &events.events {
        if scaffold_by_contract
            .insert(event.contract.as_str(), event)
            .is_some()
        {
            return Err(format!(
                "event scaffold catalog contains duplicate contract {}",
                event.contract
            ));
        }
        if !order_keys.insert((event.harness.as_str(), event.order)) {
            return Err(format!(
                "event scaffold catalog reuses order {} for harness {}",
                event.order, event.harness
            ));
        }
        let Some(registered) = implemented.get(event.contract.as_str()) else {
            return Err(format!(
                "event scaffold {} is stale or is not an implemented command contract",
                event.contract
            ));
        };
        if registered.harness != event.harness || registered.event != event.wire_event {
            return Err(format!(
                "event scaffold {} does not match implementation registry identity",
                event.contract
            ));
        }
        let expected_prefix = format!("{}/{}/", event.harness, event.snapshot);
        if !event.contract.starts_with(&expected_prefix)
            || event.contract.strip_prefix(&expected_prefix) != Some(event.wire_event.as_str())
        {
            return Err(format!(
                "event scaffold {} has inconsistent harness, snapshot, or wire event",
                event.contract
            ));
        }
        if !registered.native_input || !registered.native_output {
            return Err(format!(
                "event scaffold {} requires native input and output implementations",
                event.contract
            ));
        }
        if !valid_stable_id(&event.stable_id)
            || event.display_name.trim().is_empty()
            || event.category.trim().is_empty()
            || !valid_rust_path(&event.rust_event)
            || !valid_rust_path(&event.rust_output)
            || !valid_rust_path(&event.selector.rust_type)
            || event.selector.variant != event.wire_event
        {
            return Err(format!(
                "event scaffold {} has invalid presentation or Rust metadata",
                event.contract
            ));
        }
        if !matches!(
            event.starter.strategy.as_str(),
            "no_op" | "allow" | "example" | "must_implement"
        ) || event.starter.expression.trim().is_empty()
        {
            return Err(format!(
                "event scaffold {} has an invalid starter",
                event.contract
            ));
        }
        if !matches!(event.support.status.as_str(), "scaffolded" | "unsupported")
            || event.support.note.trim().is_empty()
            || !matches!(
                event.capabilities.session_boundary.as_str(),
                "none" | "exact" | "inferred"
            )
        {
            return Err(format!(
                "event scaffold {} has invalid support or capability metadata",
                event.contract
            ));
        }
        let fixture = root.join(&event.fixture.source);
        if event.fixture.source.starts_with('/')
            || event.fixture.source.split('/').any(|part| part == "..")
            || !fixture.is_file()
            || event.fixture.case.trim().is_empty()
        {
            return Err(format!(
                "event scaffold {} references an invalid fixture",
                event.contract
            ));
        }
        let fixtures: Fixtures = read_yaml(&fixture)?;
        if !fixtures
            .input
            .positive
            .iter()
            .any(|case| case.id == event.fixture.case)
        {
            return Err(format!(
                "event scaffold {} fixture source has no positive case {}",
                event.contract, event.fixture.case
            ));
        }
        if event
            .fixture
            .environment
            .as_deref()
            .is_some_and(str::is_empty)
        {
            return Err(format!(
                "event scaffold {} has an empty environment fixture ID",
                event.contract
            ));
        }
    }

    let implemented_contracts: BTreeSet<_> = implemented.keys().copied().collect();
    let scaffold_contracts: BTreeSet<_> = scaffold_by_contract.keys().copied().collect();
    if scaffold_contracts != implemented_contracts {
        let missing: Vec<_> = implemented_contracts
            .difference(&scaffold_contracts)
            .copied()
            .collect();
        let stale: Vec<_> = scaffold_contracts
            .difference(&implemented_contracts)
            .copied()
            .collect();
        return Err(format!(
            "event scaffold coverage differs from implementation registry (missing: {}; stale: {})",
            missing.join(", "),
            stale.join(", ")
        ));
    }

    let implemented_markers = implemented_alignment_markers();
    let mut families = BTreeMap::new();
    let mut marker_names = BTreeSet::new();
    let mut alignment_orders = BTreeSet::new();
    for family in &alignments.alignments {
        if !valid_stable_id(&family.stable_id)
            || family.display_name.trim().is_empty()
            || family.portable_floor.input.trim().is_empty()
            || family.portable_floor.output.trim().is_empty()
        {
            return Err(format!(
                "alignment {} has invalid identity or portable-floor metadata",
                family.stable_id
            ));
        }
        if families.insert(family.stable_id.as_str(), family).is_some()
            || !marker_names.insert(family.runtime_marker.as_str())
            || !alignment_orders.insert(family.order)
        {
            return Err(format!(
                "alignment {} duplicates an ID, marker, or display order",
                family.stable_id
            ));
        }
        if !implemented_markers.contains(family.runtime_marker.as_str()) {
            return Err(format!(
                "alignment {} references unimplemented runtime marker {}",
                family.stable_id, family.runtime_marker
            ));
        }
        let supported: BTreeSet<_> = family
            .supported_harnesses
            .iter()
            .map(String::as_str)
            .collect();
        let members: BTreeSet<_> = family.native_members.keys().map(String::as_str).collect();
        if supported.len() != family.supported_harnesses.len()
            || supported != members
            || supported.is_empty()
            || !supported
                .iter()
                .all(|harness| matches!(*harness, "claude-code" | "codex" | "antigravity"))
        {
            return Err(format!(
                "alignment {} has inconsistent supported harnesses and native members",
                family.stable_id
            ));
        }

        let mut member_scaffolds = Vec::new();
        for (harness, contract) in &family.native_members {
            let Some(scaffold) = scaffold_by_contract.get(contract.as_str()).copied() else {
                return Err(format!(
                    "alignment {} references stale native member {}",
                    family.stable_id, contract
                ));
            };
            if scaffold.harness != *harness
                || scaffold.aligned_family.as_deref() != Some(family.stable_id.as_str())
            {
                return Err(format!(
                    "alignment {} member {} has inconsistent harness or reverse mapping",
                    family.stable_id, contract
                ));
            }
            member_scaffolds.push(scaffold);
        }
        validate_alignment_capability_floor(family, &member_scaffolds)?;
    }

    for event in &events.events {
        let Some(family_id) = event.aligned_family.as_deref() else {
            continue;
        };
        let Some(family) = families.get(family_id) else {
            return Err(format!(
                "event scaffold {} references unknown alignment {}",
                event.contract, family_id
            ));
        };
        if family.native_members.get(&event.harness) != Some(&event.contract) {
            return Err(format!(
                "event scaffold {} is not the registered {} member for its harness",
                event.contract, family_id
            ));
        }
    }

    let known_harnesses = BTreeSet::from(["claude-code", "codex", "antigravity"]);
    let known_state = BTreeSet::from([
        "session_metadata",
        "claim_once",
        "inspectable_set",
        "record_queue",
        "run_artifacts",
        "custom_aggregate",
        "file_activity",
    ]);
    let mut archetype_ids = BTreeSet::new();
    let mut archetype_orders = BTreeSet::new();
    for archetype in &archetypes.archetypes {
        let modes: BTreeSet<_> = archetype
            .supported_modes
            .iter()
            .map(String::as_str)
            .collect();
        let supported: BTreeSet<_> = archetype
            .supported_harnesses
            .iter()
            .map(String::as_str)
            .collect();
        if !valid_stable_id(&archetype.stable_id)
            || archetype.display_name.trim().is_empty()
            || !archetype_ids.insert(archetype.stable_id.as_str())
            || !archetype_orders.insert(archetype.order)
            || modes.len() != archetype.supported_modes.len()
            || modes.is_empty()
            || !modes.iter().all(|mode| matches!(*mode, "cross" | "single"))
            || !modes.contains(archetype.default_mode.as_str())
            || supported.len() != archetype.supported_harnesses.len()
            || supported.is_empty()
            || !supported.is_subset(&known_harnesses)
        {
            return Err(format!(
                "archetype {} has invalid identity, mode, harness, or order metadata",
                archetype.stable_id
            ));
        }
        if modes.contains("cross") {
            let defaults: BTreeSet<_> = archetype
                .default_harnesses
                .iter()
                .map(String::as_str)
                .collect();
            if defaults.len() != archetype.default_harnesses.len()
                || defaults.len() < 2
                || !defaults.is_subset(&supported)
            {
                return Err(format!(
                    "archetype {} has invalid default cross-harness selection",
                    archetype.stable_id
                ));
            }
        } else if !archetype.default_harnesses.is_empty() {
            return Err(format!(
                "archetype {} declares cross defaults without cross support",
                archetype.stable_id
            ));
        }
        if modes.contains("single") {
            if archetype
                .default_harness
                .as_deref()
                .is_none_or(|harness| !supported.contains(harness))
            {
                return Err(format!(
                    "archetype {} has invalid default single harness",
                    archetype.stable_id
                ));
            }
        } else if archetype.default_harness.is_some() {
            return Err(format!(
                "archetype {} declares a single default without single support",
                archetype.stable_id
            ));
        }
        let required_hooks: BTreeSet<_> = archetype
            .required_hooks
            .iter()
            .map(String::as_str)
            .collect();
        let required_state: BTreeSet<_> = archetype
            .required_state
            .iter()
            .map(String::as_str)
            .collect();
        if required_hooks.len() != archetype.required_hooks.len()
            || required_state.len() != archetype.required_state.len()
            || !required_state.is_subset(&known_state)
        {
            return Err(format!(
                "archetype {} has duplicate or unknown requirements",
                archetype.stable_id
            ));
        }
        let cross_hooks = if archetype.default_cross_hooks.is_empty() {
            &archetype.required_hooks
        } else {
            &archetype.default_cross_hooks
        };
        let single_hooks = if archetype.default_single_hooks.is_empty() {
            &archetype.required_hooks
        } else {
            &archetype.default_single_hooks
        };
        if !archetype
            .required_hooks
            .iter()
            .all(|hook| cross_hooks.contains(hook) || single_hooks.contains(hook))
        {
            return Err(format!(
                "archetype {} defaults omit a required hook",
                archetype.stable_id
            ));
        }
        if modes.contains("cross") {
            for hook in cross_hooks {
                let Some(family) = families.get(hook.as_str()) else {
                    return Err(format!(
                        "archetype {} requires unknown aligned hook {}",
                        archetype.stable_id, hook
                    ));
                };
                if !archetype
                    .default_harnesses
                    .iter()
                    .all(|harness| family.supported_harnesses.contains(harness))
                {
                    return Err(format!(
                        "archetype {} requires {} outside its default harness tier",
                        archetype.stable_id, hook
                    ));
                }
            }
        } else if !archetype.default_cross_hooks.is_empty() {
            return Err(format!(
                "archetype {} declares cross hook defaults without cross support",
                archetype.stable_id
            ));
        }
        if modes.contains("single") {
            for hook in single_hooks {
                if !archetype.supported_harnesses.iter().all(|harness| {
                    events
                        .events
                        .iter()
                        .any(|event| event.harness == *harness && event.stable_id == *hook)
                }) {
                    return Err(format!(
                        "archetype {} requires native hook {} absent from a supported harness",
                        archetype.stable_id, hook
                    ));
                }
            }
        } else if !archetype.default_single_hooks.is_empty() {
            return Err(format!(
                "archetype {} declares single hook defaults without single support",
                archetype.stable_id
            ));
        }
        if let Some(runner) = &archetype.runner {
            let valid_reconciliation = match runner.kind.as_str() {
                "immediate" => runner.default_reconciliation_posture.is_none(),
                "deferred" => matches!(
                    runner.default_reconciliation_posture.as_deref(),
                    Some("best-effort" | "strict")
                ),
                _ => false,
            };
            if !valid_reconciliation
                || !matches!(
                    runner.default_lowering_policy.as_str(),
                    "strict" | "best-effort" | "best-effort-with-warnings"
                )
                || runner.default_quality_profile.trim().is_empty()
                || runner.default_quality_tools.is_empty()
                || runner
                    .default_quality_tools
                    .iter()
                    .any(|tool| tool.trim().is_empty())
            {
                return Err(format!(
                    "archetype {} has invalid runner defaults",
                    archetype.stable_id
                ));
            }
        }
    }
    if !archetype_ids.contains("custom") {
        return Err("archetype catalog must define the custom starter".to_string());
    }

    if !valid_version(&compatibility.template_version, true)
        || !valid_version(&compatibility.copier_version, false)
        || !valid_version(&compatibility.rust_msrv, false)
        || !valid_version(&compatibility.pkl_version, false)
        || !valid_version(&compatibility.hookkit.crates_io_version, false)
        || !compatibility.hookkit.repository.starts_with("https://")
        || compatibility.hookkit.git_revision.len() != 40
        || !compatibility
            .hookkit
            .git_revision
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(
            "template compatibility catalog has invalid versions, repository, or Git revision"
                .to_string(),
        );
    }
    Ok(())
}

fn validate_alignment_capability_floor(
    family: &AlignmentFamily,
    members: &[&EventScaffold],
) -> Result<()> {
    let all = |capability: fn(&ScaffoldCapabilities) -> bool| {
        members
            .iter()
            .all(|member| capability(&member.capabilities))
    };
    let invalid = family.capabilities.input_rewrite && !all(|caps| caps.input_rewrite)
        || family.capabilities.true_pre_action_block && !all(|caps| caps.true_pre_action_block)
        || family.capabilities.post_action_feedback && !all(|caps| caps.post_action_feedback)
        || family.capabilities.tool_result && !all(|caps| caps.tool_result)
        || family.capabilities.separate_user_agent_messages
            && !all(|caps| caps.separate_user_agent_messages)
        || family.capabilities.exact_session_boundary
            && !members
                .iter()
                .all(|member| member.capabilities.session_boundary == "exact");
    if invalid {
        return Err(format!(
            "alignment {} advertises a capability absent from one or more native members",
            family.stable_id
        ));
    }
    Ok(())
}

fn implemented_alignment_markers() -> BTreeSet<&'static str> {
    fn register<K: hookkit_runtime::aligned::AlignedEventSpec>(
        markers: &mut BTreeSet<&'static str>,
        path: &'static str,
    ) {
        let _ = std::marker::PhantomData::<K>;
        markers.insert(path);
    }

    let mut markers = BTreeSet::new();
    register::<hookkit_runtime::aligned::PreToolUse>(
        &mut markers,
        "hookkit_runtime::aligned::PreToolUse",
    );
    register::<hookkit_runtime::aligned::PostToolUse>(
        &mut markers,
        "hookkit_runtime::aligned::PostToolUse",
    );
    register::<hookkit_runtime::aligned::TurnCompletion>(
        &mut markers,
        "hookkit_runtime::aligned::TurnCompletion",
    );
    register::<hookkit_runtime::aligned::PermissionRequest>(
        &mut markers,
        "hookkit_runtime::aligned::PermissionRequest",
    );
    register::<hookkit_runtime::aligned::PreCompact>(
        &mut markers,
        "hookkit_runtime::aligned::PreCompact",
    );
    register::<hookkit_runtime::aligned::PostCompact>(
        &mut markers,
        "hookkit_runtime::aligned::PostCompact",
    );
    register::<hookkit_runtime::aligned::SessionStart>(
        &mut markers,
        "hookkit_runtime::aligned::SessionStart",
    );
    register::<hookkit_runtime::aligned::SessionEnd>(
        &mut markers,
        "hookkit_runtime::aligned::SessionEnd",
    );
    register::<hookkit_runtime::aligned::SubagentStart>(
        &mut markers,
        "hookkit_runtime::aligned::SubagentStart",
    );
    register::<hookkit_runtime::aligned::SubagentStop>(
        &mut markers,
        "hookkit_runtime::aligned::SubagentStop",
    );
    register::<hookkit_runtime::aligned::UserPromptSubmit>(
        &mut markers,
        "hookkit_runtime::aligned::UserPromptSubmit",
    );
    markers
}

fn valid_stable_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        && value.as_bytes()[0].is_ascii_lowercase()
}

fn valid_version(value: &str, allow_prerelease: bool) -> bool {
    let (core, prerelease) = value
        .split_once('-')
        .map_or((value, None), |(core, tail)| (core, Some(tail)));
    let components: Vec<_> = core.split('.').collect();
    components.len() == 3
        && components.iter().all(|component| {
            !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit())
        })
        && match prerelease {
            None => true,
            Some(tail) => {
                allow_prerelease
                    && !tail.is_empty()
                    && tail
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
            }
        }
}

fn valid_rust_path(value: &str) -> bool {
    let mut segments = value.split("::");
    let valid_segment = |segment: &str| {
        !segment.is_empty()
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            && (segment.as_bytes()[0].is_ascii_alphabetic() || segment.as_bytes()[0] == b'_')
    };
    segments.all(valid_segment)
}

fn render_generated_question_catalog(
    events: &EventScaffoldCatalog,
    alignments: &AlignmentCatalog,
    archetypes: &ArchetypeCatalog,
    compatibility: &CompatibilityCatalog,
) -> Result<String> {
    let generated = GeneratedQuestionCatalog {
        event_catalog: HiddenCatalogQuestion {
            r#type: "yaml",
            when: false,
            default: events,
        },
        alignment_catalog: HiddenCatalogQuestion {
            r#type: "yaml",
            when: false,
            default: alignments,
        },
        archetype_catalog: HiddenCatalogQuestion {
            r#type: "yaml",
            when: false,
            default: archetypes,
        },
        compatibility: HiddenCatalogQuestion {
            r#type: "yaml",
            when: false,
            default: compatibility,
        },
    };
    let yaml = serde_yaml_ng::to_string(&generated)
        .map_err(|error| format!("cannot serialize generated template catalog: {error}"))?;
    Ok(format!(
        "# @generated by `cargo xtask template-catalog sync`; do not edit.\n{yaml}"
    ))
}

fn generated_question_catalog_path(root: &Path) -> PathBuf {
    root.join("templates/hook-project/questions/catalog.yml")
}

fn write_generated_question_catalog(
    root: &Path,
    events: &EventScaffoldCatalog,
    alignments: &AlignmentCatalog,
    archetypes: &ArchetypeCatalog,
    compatibility: &CompatibilityCatalog,
) -> Result<()> {
    let path = generated_question_catalog_path(root);
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent", path.display()))?;
    fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    fs::write(
        &path,
        render_generated_question_catalog(events, alignments, archetypes, compatibility)?,
    )
    .map_err(|error| format!("{}: {error}", path.display()))
}

fn check_generated_question_catalog(
    root: &Path,
    events: &EventScaffoldCatalog,
    alignments: &AlignmentCatalog,
    archetypes: &ArchetypeCatalog,
    compatibility: &CompatibilityCatalog,
) -> Result<()> {
    let path = generated_question_catalog_path(root);
    let actual = fs::read_to_string(&path).map_err(|error| {
        format!(
            "{}: {error}; run cargo xtask template-catalog sync",
            path.display()
        )
    })?;
    let expected =
        render_generated_question_catalog(events, alignments, archetypes, compatibility)?;
    if actual != expected {
        return Err(format!(
            "{} is stale; run cargo xtask template-catalog sync",
            path.display()
        ));
    }
    Ok(())
}

fn generated_fixture_template_root(root: &Path) -> PathBuf {
    root.join("templates/hook-project/template/{{ crate_path }}/fixtures")
}

fn render_generated_fixture_templates(
    events: &EventScaffoldCatalog,
) -> Result<BTreeMap<PathBuf, String>> {
    let mut rendered = BTreeMap::new();
    for event in &events.events {
        let condition = format!(
            "'{}:{}' in effective_fixture_ids",
            event.harness, event.stable_id
        );
        let filename = format!(
            "{{% if {condition} %}}{}.json{{% endif %}}.jinja",
            event.stable_id
        );
        let value = event.fixture.value.as_ref().ok_or_else(|| {
            format!(
                "generated fixture value was not hydrated for {}",
                event.contract
            )
        })?;
        let mut contents = serde_json::to_string_pretty(value)
            .map_err(|error| format!("cannot serialize fixture {}: {error}", event.contract))?;
        contents.push('\n');
        rendered.insert(PathBuf::from(&event.harness).join(filename), contents);
    }
    Ok(rendered)
}

fn write_generated_fixture_templates(root: &Path, events: &EventScaffoldCatalog) -> Result<()> {
    let fixture_root = generated_fixture_template_root(root);
    if fixture_root.exists() {
        fs::remove_dir_all(&fixture_root)
            .map_err(|error| format!("{}: {error}", fixture_root.display()))?;
    }
    for (relative, contents) in render_generated_fixture_templates(events)? {
        let path = fixture_root.join(relative);
        let parent = path
            .parent()
            .ok_or_else(|| format!("{} has no parent", path.display()))?;
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
        fs::write(&path, contents).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    Ok(())
}

fn check_generated_fixture_templates(root: &Path, events: &EventScaffoldCatalog) -> Result<()> {
    let fixture_root = generated_fixture_template_root(root);
    let expected = render_generated_fixture_templates(events)?;
    let mut actual = BTreeMap::new();
    if fixture_root.is_dir() {
        for entry in walkdir::WalkDir::new(&fixture_root) {
            let entry = entry.map_err(|error| {
                format!(
                    "cannot enumerate generated fixture templates under {}: {error}",
                    fixture_root.display()
                )
            })?;
            if !entry.file_type().is_file() {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(&fixture_root)
                .map_err(|error| error.to_string())?
                .to_path_buf();
            let contents = fs::read_to_string(entry.path())
                .map_err(|error| format!("{}: {error}", entry.path().display()))?;
            actual.insert(relative, contents);
        }
    }
    if actual != expected {
        return Err(format!(
            "{} is stale; run cargo xtask template-catalog sync",
            fixture_root.display()
        ));
    }
    Ok(())
}

fn generated_handler_template_root(root: &Path) -> PathBuf {
    root.join("templates/hook-project/template/{{ crate_path }}/src/hooks")
}

fn harness_module_prefix(harness: &str) -> String {
    harness.replace('-', "_")
}

fn render_native_handler_template(event: &EventScaffold) -> String {
    let expression = if event.starter.strategy == "must_implement" {
        event
            .starter
            .expression
            .strip_prefix("return ")
            .unwrap_or(&event.starter.expression)
            .to_owned()
    } else {
        format!("Ok({})", event.starter.expression)
    };
    format!(
        concat!(
            "//! @generated by `cargo xtask template-catalog sync`; customize after copying.\n",
            "//! Native `{wire_event}` handler for `{contract}`.\n",
            "\n",
            "#[rustfmt::skip]\n",
            "pub fn handle(\n",
            "    _input: <{rust_event} as hookkit_core::EventSpec>::Input,\n",
            "    _environment: &<{rust_event} as hookkit_core::EventSpec>::CommandEnvironment,\n",
            "    _context: &hookkit_core::RuntimeContext<'_>,\n",
            "    _state_dir: Option<&std::path::Path>,\n",
            ") -> hookkit_core::Result<\n",
            "    <{rust_event} as hookkit_core::EventSpec>::CommandOutput,\n",
            "> {{\n",
            "    {expression}\n",
            "}}\n",
        ),
        wire_event = event.wire_event,
        contract = event.contract,
        rust_event = event.rust_event,
        expression = expression,
    )
}

fn render_aligned_handler_template(family: &AlignmentFamily) -> Result<String> {
    let base = family.runtime_marker.rsplit("::").next().ok_or_else(|| {
        format!(
            "alignment {} has an invalid runtime marker",
            family.stable_id
        )
    })?;
    let body = match family.stable_id.as_str() {
        "post_tool" => {
            "    hookkit_common::PostToolUseOutput::no_op(context.harness())\n".to_owned()
        }
        "turn_completion" => {
            "    hookkit_common::TurnCompletionOutput::allow(context.harness())\n".to_owned()
        }
        // Every other family, including `permission_request`, starts from the
        // aligned no-op. For PermissionRequest that leaves the dialog to the
        // user; `PermissionRequestOutput::allow` would answer it for them.
        _ => format!("    hookkit_common::{base}Output::no_op(context.harness())\n"),
    };
    Ok(format!(
        concat!(
            "//! @generated by `cargo xtask template-catalog sync`; customize after copying.\n",
            "//! Portable `{stable_id}` handler.\n",
            "\n",
            "#[rustfmt::skip]\n",
            "pub fn handle(\n",
            "    _input: hookkit_common::{base}Input,\n",
            "    _environment: &hookkit_common::{base}CommandEnvironment,\n",
            "    context: &hookkit_core::RuntimeContext<'_>,\n",
            "    _state_dir: Option<&std::path::Path>,\n",
            ") -> hookkit_core::Result<hookkit_common::{base}Output> {{\n",
            "{body}",
            "}}\n",
        ),
        stable_id = family.stable_id,
        base = base,
        body = body,
    ))
}

fn render_generated_handler_templates(
    events: &EventScaffoldCatalog,
    alignments: &AlignmentCatalog,
) -> Result<BTreeMap<PathBuf, String>> {
    let mut rendered = BTreeMap::new();
    for event in &events.events {
        let condition = if event.stable_id == "pre_tool_use" {
            format!(
                "harness_mode == 'single' and starter == 'custom' and harness == '{}' and '{}' in native_hooks",
                event.harness, event.stable_id
            )
        } else {
            format!(
                "harness_mode == 'single' and harness == '{}' and '{}' in native_hooks",
                event.harness, event.stable_id
            )
        };
        let filename = format!(
            "{{% if {condition} %}}{}_{}.rs{{% endif %}}.jinja",
            harness_module_prefix(&event.harness),
            event.stable_id
        );
        rendered.insert(
            PathBuf::from("native").join(filename),
            render_native_handler_template(event),
        );
    }
    for family in &alignments.alignments {
        if family.stable_id == "pre_tool" {
            continue;
        }
        let condition = format!(
            "harness_mode == 'cross' and '{}' in aligned_hooks",
            family.stable_id
        );
        let filename = format!(
            "{{% if {condition} %}}{}.rs{{% endif %}}.jinja",
            family.stable_id
        );
        rendered.insert(
            PathBuf::from("aligned").join(filename),
            render_aligned_handler_template(family)?,
        );
    }
    Ok(rendered)
}

fn write_generated_handler_templates(
    root: &Path,
    events: &EventScaffoldCatalog,
    alignments: &AlignmentCatalog,
) -> Result<()> {
    let handler_root = generated_handler_template_root(root);
    for directory in ["native", "aligned"] {
        let path = handler_root.join(directory);
        if path.exists() {
            fs::remove_dir_all(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        }
    }
    for (relative, contents) in render_generated_handler_templates(events, alignments)? {
        let path = handler_root.join(relative);
        let parent = path
            .parent()
            .ok_or_else(|| format!("{} has no parent", path.display()))?;
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
        fs::write(&path, contents).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    Ok(())
}

fn check_generated_handler_templates(
    root: &Path,
    events: &EventScaffoldCatalog,
    alignments: &AlignmentCatalog,
) -> Result<()> {
    let handler_root = generated_handler_template_root(root);
    let expected = render_generated_handler_templates(events, alignments)?;
    let mut actual = BTreeMap::new();
    for directory in ["native", "aligned"] {
        let path = handler_root.join(directory);
        if !path.is_dir() {
            continue;
        }
        for entry in walkdir::WalkDir::new(&path) {
            let entry = entry.map_err(|error| {
                format!(
                    "cannot enumerate generated handler templates under {}: {error}",
                    path.display()
                )
            })?;
            if !entry.file_type().is_file() {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(&handler_root)
                .map_err(|error| error.to_string())?
                .to_path_buf();
            let contents = fs::read_to_string(entry.path())
                .map_err(|error| format!("{}: {error}", entry.path().display()))?;
            actual.insert(relative, contents);
        }
    }
    if actual != expected {
        return Err(format!(
            "generated handler templates under {} are stale; run cargo xtask template-catalog sync",
            handler_root.display()
        ));
    }
    Ok(())
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

    for harness in registry.harnesses.keys() {
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
    }

    let mut snapshot_paths: Vec<_> = walkdir::WalkDir::new(catalog.join("harnesses"))
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to enumerate contract snapshots: {error}"))?
        .into_iter()
        .filter(|entry| entry.file_type().is_file() && entry.file_name() == "snapshot.yaml")
        .map(|entry| entry.into_path())
        .collect();
    snapshot_paths.sort();

    let mut contracts = Vec::new();
    let mut contract_ids = BTreeSet::new();
    let mut selected_snapshots = BTreeMap::new();
    let mut draft_snapshots = Vec::new();
    let mut unselected_frozen = Vec::new();
    for snapshot_path in snapshot_paths {
        let snapshot_dir = snapshot_path
            .parent()
            .expect("snapshot.yaml has a parent directory");
        let (snapshot, loaded) =
            validate_snapshot(&catalog, &meta, snapshot_dir, &registry, &mut contract_ids)?;
        let selected = registry
            .harnesses
            .get(&snapshot.harness)
            .is_some_and(|selected| selected.current == snapshot.id);
        if selected {
            selected_snapshots.insert(snapshot.harness.clone(), snapshot.retrieved.clone());
            contracts.extend(loaded);
        }
        if snapshot.state != "frozen" {
            draft_snapshots.push((snapshot_dir.to_path_buf(), snapshot));
        } else if !selected {
            unselected_frozen.push(snapshot_path.clone());
        }
    }
    if selected_snapshots.len() != registry.harnesses.len() {
        return Err("one or more registry-selected snapshots were not found".to_string());
    }
    validate_snapshot_states(&registry, &selected_snapshots, &draft_snapshots)?;
    unselected_frozen.extend(validate_command_environment_supplements(
        &catalog, &meta, &registry,
    )?);
    validate_frozen_ledger(&catalog, FROZEN_LEDGER, &unselected_frozen)?;
    selected_content_hash_acknowledgements(&selected_sources(root)?)?;

    let target_keys: BTreeSet<_> = stabilization
        .targets
        .iter()
        .map(|target| (target.contract.as_str(), target.binding.as_str()))
        .collect();
    if target_keys.len() != stabilization.targets.len() {
        return Err("stabilization target contains duplicate contract/binding entries".to_string());
    }
    let default_keys: BTreeSet<_> = stabilization
        .defaults
        .iter()
        .map(|default| (default.harness.as_str(), default.binding.as_str()))
        .collect();
    if default_keys.len() != stabilization.defaults.len() {
        return Err("stabilization target contains duplicate harness/binding defaults".to_string());
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
        if !valid_support_level(&target.level) {
            return Err(format!("invalid support level {}", target.level));
        }
        if !valid_verification(&target.verification) {
            return Err(format!("{} has empty verification status", target.contract));
        }
        if release_claim_requires_observation(&target.level, &target.verification) {
            return Err(format!(
                "{} requests stable/live support without a validated observation overlay",
                target.contract
            ));
        }
    }
    for default in &stabilization.defaults {
        if !registry.harnesses.contains_key(&default.harness)
            || !valid_support_level(&default.level)
            || !valid_verification(&default.verification)
        {
            return Err(format!(
                "invalid stabilization default for {}/{}",
                default.harness, default.binding
            ));
        }
        if release_claim_requires_observation(&default.level, &default.verification) {
            return Err(format!(
                "stabilization default for {}/{} requests stable/live support without a validated observation overlay",
                default.harness, default.binding
            ));
        }
    }
    let implementation_path = catalog.join("status/implementation/registry.json");
    validate_json_metadata(
        &implementation_path,
        &meta.join("implementation-registry.schema.json"),
    )?;
    let implementation: ImplementationRegistry =
        serde_json::from_value(read_json(&implementation_path)?)
            .map_err(|error| format!("invalid implementation registry: {error}"))?;
    validate_implementation_registry(&implementation, &contracts)?;
    validate_target_implementation(&stabilization, &implementation, &contracts)?;
    validate_implementation_gaps(&gaps, &contracts, &implementation)?;
    let _ = &stabilization.release;
    Ok(contracts)
}

/// Lifecycle files of every frozen snapshot and command-environment
/// supplement, relative to `contracts/`, with their SHA-256.
///
/// `snapshot.yaml` carries a snapshot's frozen marker outside its own
/// manifest, and deleting a supplement's manifest would unfreeze it, so a
/// hand edit could otherwise demote published evidence to an unverified draft
/// and then change it. `contracts check` requires each listed file to be
/// unchanged, which also keeps it frozen, and requires every frozen snapshot
/// or supplement that the registry does not select to be listed. Add an entry
/// when freezing (`contracts freeze` prints it); never edit one.
const FROZEN_LEDGER: &[(&str, &str)] = &[
    (
        "harnesses/antigravity/snapshots/docs-2026-07-12-r1/snapshot.yaml",
        "90d1defa0051e2bfd7d6484f0d7ba16ec71b957177640ce459f6f86ca5f35f9e",
    ),
    (
        "harnesses/antigravity/snapshots/docs-2026-07-12-r2/snapshot.yaml",
        "2176615fdda5fcbf9ec3ddafbb1a2d21120e9f93b9385ac942fa75b8fd1a1f2a",
    ),
    (
        "harnesses/antigravity/snapshots/docs-2026-08-04-r1/snapshot.yaml",
        "162731e956e1d6789b03f73437fc3384e47265db6f78f75ac1e84b2dc4d9d7a9",
    ),
    (
        "harnesses/antigravity/snapshots/docs-2026-09-29-r1/snapshot.yaml",
        "a3b88e4d942f8a81dfe1b69cd1562666a23b109d89df0c80fbe08a2328ce8347",
    ),
    (
        "harnesses/claude-code/snapshots/docs-2026-07-12-r1/snapshot.yaml",
        "91fcb7739acb1500afaeabded55591f4353beb5c21017b5f719a19ba2c2cc4d9",
    ),
    (
        "harnesses/claude-code/snapshots/docs-2026-07-12-r2/snapshot.yaml",
        "df4815f1f6853f1d1b788192be8bd816fc21a00ea889cd3eaf443bd1aa7c0ce8",
    ),
    (
        "harnesses/claude-code/snapshots/docs-2026-08-05-r1/snapshot.yaml",
        "004f2dff8cd7fe8f0d541ac33bf89300eaa43dea11a8a77fb5549c3090399548",
    ),
    (
        "harnesses/claude-code/snapshots/docs-2026-09-29-r1/snapshot.yaml",
        "bda26b5eb6ce813aaf20e0e21ba172cd68c23553d5c760601e455cc973d5ee41",
    ),
    (
        "harnesses/codex/snapshots/commit-1e59dc5-r1/snapshot.yaml",
        "9d0a961c29057e9e3c4777a4b262503665ff9f29c304fbb409ec05ab5b39c6f4",
    ),
    (
        "harnesses/codex/snapshots/commit-9e552e9-r1/snapshot.yaml",
        "cac3c4ba21f2b03f8d6a5c3831ac8bc79482101fe301d21aa8b3eed6c7758ff4",
    ),
    (
        "harnesses/codex/snapshots/commit-9e552e9-r2/snapshot.yaml",
        "a394123d5dbbbd810ecc629c5312e00ab8fbf919ba51949d0f6fc46bcdf5b85e",
    ),
    (
        "harnesses/codex/snapshots/commit-ff6aec9-r1/snapshot.yaml",
        "733f64e3cea297648cccc272dd51df6678ca4791fc321c2907ac97421f7e63e4",
    ),
    (
        "supplements/command-environments/command-environments-2026-08-05-r2/supplement.yaml",
        "5fb7bf64a67d5e579b59cc277e222b0318c4c196735474ecf8f58419860dfc63",
    ),
    (
        "supplements/command-environments/command-environments-2026-08-05-r3/supplement.yaml",
        "96eec7fb712f5e299b0539db6219ed872ee03b1ed32014b04f111ae1a52e1970",
    ),
    (
        "supplements/command-environments/command-environments-2026-08-05-r4/supplement.yaml",
        "992c28b42d758c7852e1d625e29681e1f418565a3c91a26860dab820fdad6fff",
    ),
    (
        "supplements/command-environments/command-environments-2026-09-29-r1/supplement.yaml",
        "08e691f305123d43e4d2a6fc2240b743dccb82ab26a88586997f05a2cf0a6b31",
    ),
    (
        "supplements/command-environments/command-environments-2026-09-30-r1/supplement.yaml",
        "0a876cbbe682fd37dadeaf82012041294bb597c54fbf942e0477cfa54cc7d0c4",
    ),
];

/// Check the frozen ledger against the catalog under `catalog`.
///
/// Every ledger file must exist with its recorded digest, and every lifecycle
/// file in `unselected_frozen` (frozen but not registry-selected) must be
/// listed.
fn validate_frozen_ledger(
    catalog: &Path,
    ledger: &[(&str, &str)],
    unselected_frozen: &[PathBuf],
) -> Result<()> {
    let mut listed = BTreeSet::new();
    for (relative, digest) in ledger {
        let path = safe_join(catalog, relative)?;
        let bytes = fs::read(&path).map_err(|error| {
            format!(
                "{}: lifecycle file recorded in the frozen ledger cannot be read: {error}",
                path.display()
            )
        })?;
        if hex_sha256(&bytes) != *digest {
            return Err(format!(
                "{}: frozen lifecycle file changed; frozen snapshots and supplements are immutable, so create a successor instead",
                path.display()
            ));
        }
        listed.insert(path);
    }
    for path in unselected_frozen {
        if !listed.contains(path) {
            let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
            let relative = path.strip_prefix(catalog).unwrap_or(path);
            return Err(format!(
                "{}: frozen but not registry-selected, so it must be recorded in FROZEN_LEDGER in xtask/src/main.rs as (\"{}\", \"{}\")",
                path.display(),
                relative.display(),
                hex_sha256(&bytes)
            ));
        }
    }
    Ok(())
}

/// Enforce the snapshot lifecycle that content manifests alone cannot.
///
/// `snapshot.yaml` carries the frozen marker outside its own manifest, so a
/// hand edit could otherwise demote a published snapshot to an unverified
/// draft. Registry-selected snapshots must therefore be frozen, like selected
/// supplements, and a draft may only be a successor candidate: one retrieved
/// no earlier than its harness's selected snapshot. [`FROZEN_LEDGER`] pins
/// every superseded snapshot's lifecycle file, including its `retrieved`
/// date.
fn validate_snapshot_states(
    registry: &Registry,
    selected_retrieved: &BTreeMap<String, String>,
    drafts: &[(PathBuf, Snapshot)],
) -> Result<()> {
    for (directory, snapshot) in drafts {
        if registry
            .harnesses
            .get(&snapshot.harness)
            .is_some_and(|selected| selected.current == snapshot.id)
        {
            return Err(format!(
                "{}: registry-selected snapshot must be frozen",
                directory.display()
            ));
        }
        if selected_retrieved
            .get(&snapshot.harness)
            .is_some_and(|selected| snapshot.retrieved.as_str() < selected.as_str())
        {
            return Err(format!(
                "{}: draft snapshot predates the registry-selected snapshot; superseded snapshots must stay frozen",
                directory.display()
            ));
        }
    }
    Ok(())
}

fn validate_snapshot(
    catalog: &Path,
    meta: &Path,
    snapshot_dir: &Path,
    registry: &Registry,
    contract_ids: &mut BTreeSet<String>,
) -> Result<(Snapshot, Vec<LoadedContract>)> {
    let snapshots_dir = snapshot_dir
        .parent()
        .ok_or_else(|| format!("{}: snapshot has no parent", snapshot_dir.display()))?;
    if snapshots_dir.file_name().and_then(|name| name.to_str()) != Some("snapshots") {
        return Err(format!(
            "{}: snapshot is outside a harness snapshots directory",
            snapshot_dir.display()
        ));
    }
    let harness = snapshots_dir
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("{}: cannot determine harness", snapshot_dir.display()))?;
    if !registry.harnesses.contains_key(harness) {
        return Err(format!(
            "{}: snapshot belongs to unregistered harness {harness}",
            snapshot_dir.display()
        ));
    }

    let snapshot_path = snapshot_dir.join("snapshot.yaml");
    validate_yaml_metadata(&snapshot_path, &meta.join("snapshot.schema.json"))?;
    let snapshot: Snapshot = read_yaml(&snapshot_path)?;
    require_version(snapshot.format_version, "snapshot")?;
    let directory_id = snapshot_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            format!(
                "{}: invalid snapshot directory name",
                snapshot_dir.display()
            )
        })?;
    if snapshot.id != directory_id || snapshot.harness != harness {
        return Err(format!(
            "{}: snapshot identity mismatch",
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
        verify_content_manifest(snapshot_dir, manifest, Some(SNAPSHOT_METADATA_FILE))?;
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

    let strict_successor = !uses_legacy_snapshot_semantics(&snapshot.harness, &snapshot.id);
    let fetchable_hashes = requires_fetchable_content_hashes(&snapshot.state, &snapshot.retrieved);
    let sources_path = safe_join(snapshot_dir, &snapshot.sources_file)?;
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
        validate_source(source, snapshot_dir, strict_successor, fetchable_hashes)?;
        if source.reproducibility.as_deref() == Some("vendored") {
            let revision = source.revision.as_deref().unwrap_or_default();
            let vendored = safe_join(&catalog.join("vendor").join(harness), revision)?;
            if !vendored.is_dir() {
                return Err(format!(
                    "{}: vendored source {} has no evidence at {}",
                    snapshot_dir.display(),
                    source.id,
                    vendored.display()
                ));
            }
        }
    }

    let mut loaded = Vec::new();
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
        let dir = safe_join(snapshot_dir, &indexed.path)?;
        let contract_path = dir.join("contract.yaml");
        validate_yaml_metadata(&contract_path, &meta.join("event-contract.schema.json"))?;
        let contract: Contract = read_yaml(&contract_path)?;
        validate_contract_identity(&contract, &snapshot, indexed, &dir)?;
        if !contract_ids.insert(contract.id.clone()) {
            return Err(format!("duplicate contract id {}", contract.id));
        }
        let process_cases = validate_contract(&contract, &dir, &source_ids, strict_successor)?;
        loaded.push(LoadedContract {
            contract,
            dir,
            process_cases,
        });
    }
    let indexed_paths: BTreeSet<_> = snapshot
        .events
        .iter()
        .map(|event| snapshot_dir.join(&event.path).join("contract.yaml"))
        .collect();
    let discovered_paths: BTreeSet<_> = walkdir::WalkDir::new(snapshot_dir.join("events"))
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| format!("{}: {error}", snapshot_dir.display()))?
        .into_iter()
        .filter(|entry| entry.file_type().is_file() && entry.file_name() == "contract.yaml")
        .map(|entry| entry.into_path())
        .collect();
    if indexed_paths != discovered_paths {
        return Err(format!(
            "{}: snapshot index and event contract directories differ",
            snapshot_dir.display()
        ));
    }

    // Retain this path relationship as an explicit invariant rather than relying
    // only on the directory-name extraction above.
    if !snapshot_dir.starts_with(catalog.join("harnesses").join(harness).join("snapshots")) {
        return Err(format!(
            "{}: invalid snapshot location",
            snapshot_dir.display()
        ));
    }
    Ok((snapshot, loaded))
}

/// Validate every command-environment supplement and return the lifecycle
/// files of the frozen supplements the registry does not select.
fn validate_command_environment_supplements(
    catalog: &Path,
    meta: &Path,
    registry: &Registry,
) -> Result<Vec<PathBuf>> {
    let root = catalog.join("supplements/command-environments");
    let mut paths: Vec<_> = walkdir::WalkDir::new(&root)
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to enumerate command-environment supplements: {error}"))?
        .into_iter()
        .filter(|entry| entry.file_type().is_file() && entry.file_name() == "supplement.yaml")
        .map(|entry| entry.into_path())
        .collect();
    paths.sort();

    let mut ids = BTreeSet::new();
    let mut selected_found = false;
    let mut unselected_frozen = Vec::new();
    for path in paths {
        let directory = path
            .parent()
            .expect("supplement.yaml has a parent directory");
        validate_yaml_metadata(
            &path,
            &meta.join("command-environment-supplement.schema.json"),
        )?;
        let supplement: CommandEnvironmentSupplement = read_yaml(&path)?;
        require_version(supplement.format_version, "command-environment supplement")?;
        let directory_id = directory
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("{}: invalid supplement directory", directory.display()))?;
        if supplement.id != directory_id || !ids.insert(supplement.id.clone()) {
            return Err(format!(
                "{}: duplicate or mismatched supplement identity",
                directory.display()
            ));
        }
        let selected = supplement.id == registry.supplements.command_environments;
        selected_found |= selected;
        if selected && supplement.state != "frozen" {
            return Err(format!(
                "{}: registry-selected supplement must be frozen",
                directory.display()
            ));
        }
        if supplement.state == "frozen" {
            let manifest = supplement.manifest_file.as_deref().ok_or_else(|| {
                format!(
                    "{}: frozen supplement needs manifest_file",
                    directory.display()
                )
            })?;
            verify_content_manifest(directory, manifest, None)?;
            if !selected {
                unselected_frozen.push(path.clone());
            }
        } else if supplement.state == "draft" {
            if supplement.manifest_file.is_some() {
                return Err(format!(
                    "{}: draft supplement must not have manifest_file",
                    directory.display()
                ));
            }
        } else {
            return Err(format!(
                "{}: state must be draft or frozen",
                directory.display()
            ));
        }
        if supplement.retrieved.is_empty() {
            return Err(format!(
                "{}: retrieval date is required",
                directory.display()
            ));
        }

        let sources_path = safe_join(directory, &supplement.sources_file)?;
        validate_yaml_metadata(&sources_path, &meta.join("sources.schema.json"))?;
        let sources: Sources = read_yaml(&sources_path)?;
        require_version(sources.format_version, "command-environment sources")?;
        let source_ids: BTreeSet<_> = sources
            .sources
            .iter()
            .map(|source| source.id.as_str())
            .collect();
        if source_ids.len() != sources.sources.len() {
            return Err(format!("{}: duplicate source id", directory.display()));
        }
        let fetchable_hashes =
            requires_fetchable_content_hashes(&supplement.state, &supplement.retrieved);
        for source in &sources.sources {
            validate_source(source, directory, true, fetchable_hashes)?;
        }

        if selected {
            let expected_harnesses: BTreeSet<_> =
                registry.harnesses.keys().map(String::as_str).collect();
            let actual_harnesses: BTreeSet<_> =
                supplement.harnesses.keys().map(String::as_str).collect();
            if actual_harnesses != expected_harnesses {
                return Err(format!(
                    "{}: selected supplement harness inventory differs from registry",
                    directory.display()
                ));
            }
        }
        for (harness_id, harness) in &supplement.harnesses {
            if selected
                && registry
                    .harnesses
                    .get(harness_id)
                    .is_none_or(|entry| entry.current != harness.snapshot)
            {
                return Err(format!(
                    "{}: selected supplement targets non-current snapshot {harness_id}/{}",
                    directory.display(),
                    harness.snapshot
                ));
            }
            validate_sources(&harness.sources, &source_ids, directory)?;
            validate_assurance(&harness.assurance, directory)?;
            let snapshot_path = catalog
                .join("harnesses")
                .join(harness_id)
                .join("snapshots")
                .join(&harness.snapshot)
                .join("snapshot.yaml");
            let snapshot: Snapshot = read_yaml(&snapshot_path)?;
            if snapshot.harness != *harness_id || snapshot.id != harness.snapshot {
                return Err(format!(
                    "{}: supplement base snapshot identity mismatch for {harness_id}",
                    directory.display()
                ));
            }
            let expected_events: BTreeSet<_> = snapshot
                .events
                .iter()
                .map(|event| event.wire_name.as_str())
                .collect();
            validate_command_environment_harness(
                harness,
                &expected_events,
                &source_ids,
                directory,
            )?;
        }
    }
    if !selected_found {
        return Err(format!(
            "registry-selected command-environment supplement {} was not found",
            registry.supplements.command_environments
        ));
    }
    Ok(unselected_frozen)
}

fn validate_command_environment_harness(
    harness: &CommandEnvironmentHarness,
    expected_events: &BTreeSet<&str>,
    sources: &BTreeSet<&str>,
    directory: &Path,
) -> Result<()> {
    let actual_events: BTreeSet<_> = harness.event_profiles.keys().map(String::as_str).collect();
    if &actual_events != expected_events {
        return Err(format!(
            "{}: command-environment event coverage differs from base snapshot",
            directory.display()
        ));
    }

    let exact_names: BTreeSet<_> = harness
        .profiles
        .iter()
        .flat_map(|profile| profile.variables.iter())
        .filter_map(|variable| variable.name.as_deref())
        .collect();
    let mut profile_ids = BTreeSet::new();
    let mut selectors = BTreeSet::new();
    for profile in &harness.profiles {
        if !profile_ids.insert(profile.id.as_str()) {
            return Err(format!(
                "{}: duplicate command-environment profile {}",
                directory.display(),
                profile.id
            ));
        }
        validate_sources(&profile.sources, sources, directory)?;
        validate_assurance(&profile.assurance, directory)?;
        if !matches!(profile.completeness.as_str(), "all-or-none" | "independent") {
            return Err(format!(
                "{}: invalid completeness for profile {}",
                directory.display(),
                profile.id
            ));
        }
        match (
            profile.activation.kind.as_str(),
            &profile.activation.condition,
        ) {
            ("always" | "optional", None) => {}
            ("conditional", Some(condition)) if !condition.is_empty() => {}
            _ => {
                return Err(format!(
                    "{}: invalid activation for profile {}",
                    directory.display(),
                    profile.id
                ));
            }
        }
        for variable in &profile.variables {
            let selector = match (&variable.name, &variable.prefix) {
                (Some(name), None) => format!("name:{name}"),
                (None, Some(prefix)) => format!("prefix:{prefix}"),
                _ => {
                    return Err(format!(
                        "{}: profile {} variable needs exactly one selector",
                        directory.display(),
                        profile.id
                    ));
                }
            };
            if !selectors.insert(selector) {
                return Err(format!(
                    "{}: environment selector is claimed by multiple profiles",
                    directory.display()
                ));
            }
            let value = &variable.value;
            match value.kind.as_str() {
                "literal" if value.literal.is_some() && value.reference.is_none() => {}
                "input-field"
                    if value.literal.is_none()
                        && value
                            .reference
                            .as_deref()
                            .is_some_and(|value| value.starts_with('/')) => {}
                "environment-variable"
                    if value.literal.is_none()
                        && value
                            .reference
                            .as_deref()
                            .is_some_and(|value| exact_names.contains(value)) => {}
                "opaque" | "path" if value.literal.is_none() && value.reference.is_none() => {}
                _ => {
                    return Err(format!(
                        "{}: invalid value contract for profile {}",
                        directory.display(),
                        profile.id
                    ));
                }
            }
            let _ = value.allow_empty;
        }
    }

    let mut referenced_profiles = BTreeSet::new();
    for (event, profiles) in &harness.event_profiles {
        let mut unique = BTreeSet::new();
        for profile in profiles {
            if !unique.insert(profile.as_str()) || !profile_ids.contains(profile.as_str()) {
                return Err(format!(
                    "{}: event {event} has a duplicate or unknown environment profile {profile}",
                    directory.display()
                ));
            }
            referenced_profiles.insert(profile.as_str());
        }
    }
    if referenced_profiles != profile_ids {
        return Err(format!(
            "{}: one or more command-environment profiles are unused",
            directory.display()
        ));
    }
    Ok(())
}

fn uses_legacy_snapshot_semantics(harness: &str, snapshot: &str) -> bool {
    matches!(
        (harness, snapshot),
        ("antigravity", "docs-2026-07-12-r1")
            | ("claude-code", "docs-2026-07-12-r1")
            | ("codex", "commit-9e552e9-r1")
    )
}

/// First retrieval date whose recorded content hashes must be reproducible by
/// fetching the source URL itself. Frozen snapshots and supplements retrieved
/// earlier keep the hashes they were frozen with.
const FETCHABLE_CONTENT_HASHES_FROM: &str = "2026-09-30";

/// Whether a snapshot or supplement in `state`, retrieved on `retrieved`,
/// must record content hashes only for URLs whose body they cover.
fn requires_fetchable_content_hashes(state: &str, retrieved: &str) -> bool {
    state != "frozen" || retrieved >= FETCHABLE_CONTENT_HASHES_FROM
}

fn validate_source(
    source: &Source,
    snapshot_dir: &Path,
    strict_successor: bool,
    fetchable_hashes: bool,
) -> Result<()> {
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
    // github.com serves an HTML page with per-request content for a file, so
    // a hash of the file itself belongs to its raw.githubusercontent.com URL,
    // which is what the drift check fetches.
    if fetchable_hashes
        && source.content_sha256.is_some()
        && source.url.starts_with("https://github.com/")
    {
        return Err(format!(
            "{}: source {} records a content hash for a github.com page; record the raw.githubusercontent.com URL whose body the hash covers",
            snapshot_dir.display(),
            source.id
        ));
    }
    let Some(reproducibility) = source.reproducibility.as_deref() else {
        if strict_successor {
            return Err(format!(
                "{}: source {} must classify reproducibility",
                snapshot_dir.display(),
                source.id
            ));
        }
        return Ok(());
    };
    if !matches!(
        reproducibility,
        "vendored" | "pinned-revision" | "content-hash-only" | "unreproducible"
    ) {
        return Err(format!(
            "{}: source {} has invalid reproducibility",
            snapshot_dir.display(),
            source.id
        ));
    }
    if reproducibility == "vendored" && source.revision.is_none()
        || reproducibility == "pinned-revision" && source.revision.is_none()
        || reproducibility == "content-hash-only" && source.content_sha256.is_none()
        || reproducibility == "unreproducible" && source.limitations.is_empty()
        || source
            .limitations
            .iter()
            .any(|limitation| limitation.is_empty())
    {
        return Err(format!(
            "{}: source {} reproducibility evidence is incomplete",
            snapshot_dir.display(),
            source.id
        ));
    }
    let _ = &source.license;
    Ok(())
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

fn validate_contract(
    contract: &Contract,
    dir: &Path,
    sources: &BTreeSet<&str>,
    strict_successor: bool,
) -> Result<BTreeMap<String, String>> {
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
    match &contract.event.identification.discriminator {
        Some(discriminator)
            if discriminator.json_pointer.is_empty()
                || discriminator.r#const != contract.event.wire_name =>
        {
            return Err(format!("{}: invalid discriminator", dir.display()));
        }
        _ => {}
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
            if !output_schema_direction_valid(
                &outcome.stdout.content_kind,
                outcome.output_schema.as_deref(),
            ) {
                return Err(format!(
                    "{}: outcome {} must reference a schema exactly when its response is JSON",
                    dir.display(),
                    outcome.id
                ));
            }
            match outcome.output_schema.as_deref() {
                Some(schema) if !output_validators.contains_key(schema) => {
                    return Err(format!(
                        "{}: outcome {} references unknown output schema {schema}",
                        dir.display(),
                        outcome.id
                    ));
                }
                _ => {}
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
    let minimal = fixtures
        .input
        .positive
        .iter()
        .find(|fixture| fixture.id == "minimal")
        .ok_or_else(|| format!("{}: missing minimal positive input", dir.display()))?;
    let representative = fixtures
        .input
        .positive
        .iter()
        .find(|fixture| fixture.id == "representative")
        .ok_or_else(|| format!("{}: missing representative positive input", dir.display()))?;
    if strict_successor && minimal.value == representative.value {
        return Err(format!(
            "{}: minimal and representative positive inputs are identical",
            dir.display()
        ));
    }
    let mut positive_ids = BTreeSet::new();
    for fixture in &fixtures.input.positive {
        if !positive_ids.insert(fixture.id.as_str()) {
            return Err(format!(
                "{}: duplicate positive input fixture {}",
                dir.display(),
                fixture.id
            ));
        }
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
    let mut negative_ids = BTreeSet::new();
    for fixture in &fixtures.input.negative {
        if !negative_ids.insert(fixture.id.as_str()) {
            return Err(format!(
                "{}: duplicate negative input fixture {}",
                dir.display(),
                fixture.id
            ));
        }
        validate_fixture_provenance(&fixture.id, &fixture.origin, &fixture.sources, sources, dir)?;
        if strict_successor && fixture.expected_keyword.is_none() {
            return Err(format!(
                "{}: negative input {} must name its expected validation keyword",
                dir.display(),
                fixture.id
            ));
        }
        validate_expected_failure(
            &input_validator,
            &fixture.value,
            &fixture.id,
            &fixture.expected_pointer,
            fixture.expected_keyword.as_deref(),
            "negative input",
            dir,
        )?;
    }
    let mut output_ids = BTreeSet::new();
    for fixture in &fixtures.output {
        if !output_ids.insert(fixture.id.as_str()) {
            return Err(format!(
                "{}: duplicate output fixture {}",
                dir.display(),
                fixture.id
            ));
        }
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
    let mut negative_output_ids = BTreeSet::new();
    for fixture in &fixtures.output_negative {
        if !negative_output_ids.insert(fixture.id.as_str()) {
            return Err(format!(
                "{}: duplicate negative output fixture {}",
                dir.display(),
                fixture.id
            ));
        }
        validate_fixture_provenance(&fixture.id, &fixture.origin, &fixture.sources, sources, dir)?;
        let validator = output_validators
            .get(fixture.schema.as_str())
            .ok_or_else(|| {
                format!(
                    "{}: negative output fixture {} references unknown schema",
                    dir.display(),
                    fixture.id
                )
            })?;
        validate_expected_failure(
            validator,
            &fixture.value,
            &fixture.id,
            &fixture.expected_pointer,
            Some(&fixture.expected_keyword),
            "negative output",
            dir,
        )?;
    }
    let mut process_cases = BTreeMap::new();
    for fixture in &fixtures.process {
        if process_cases
            .insert(fixture.id.clone(), fixture.binding.clone())
            .is_some()
        {
            return Err(format!(
                "{}: duplicate process fixture {}",
                dir.display(),
                fixture.id
            ));
        }
        validate_process_fixture(fixture, contract, &output_validators, dir)?;
    }
    Ok(process_cases)
}

fn validate_expected_failure(
    validator: &jsonschema::Validator,
    value: &Value,
    id: &str,
    expected_pointer: &str,
    expected_keyword: Option<&str>,
    fixture_kind: &str,
    dir: &Path,
) -> Result<()> {
    let errors: Vec<_> = validator.iter_errors(value).collect();
    if errors.is_empty() {
        return Err(format!(
            "{}: {fixture_kind} {id} unexpectedly passed",
            dir.display()
        ));
    }
    if errors.iter().any(|error| {
        error.instance_path().as_str() == expected_pointer
            && expected_keyword.is_none_or(|keyword| error.kind().keyword() == keyword)
    }) {
        return Ok(());
    }
    let observed = errors
        .iter()
        .map(|error| {
            format!(
                "{}:{}",
                error.instance_path().as_str(),
                error.kind().keyword()
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "{}: {fixture_kind} {id} did not fail at expected pointer {expected_pointer:?} with keyword {expected_keyword:?}; observed {observed}",
        dir.display()
    ))
}

fn output_schema_direction_valid(content_kind: &str, output_schema: Option<&str>) -> bool {
    (content_kind == "json") == output_schema.is_some()
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

fn valid_support_level(level: &str) -> bool {
    matches!(
        level,
        "catalog-only"
            | "native-source-reviewed"
            | "command-runtime-beta"
            | "command-runtime-stable"
            | "other-runtime-beta"
            | "other-runtime-stable"
            | "unsupported"
    )
}

fn valid_verification(verification: &str) -> bool {
    matches!(
        verification,
        "unverified" | "fixture-validated" | "source-reviewed" | "live-observed"
    )
}

fn release_claim_requires_observation(level: &str, verification: &str) -> bool {
    level.ends_with("-stable") || verification == "live-observed"
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
    let implementation: ImplementationRegistry = serde_json::from_value(read_json(
        &root.join("contracts/status/implementation/registry.json"),
    )?)
    .map_err(|error| format!("invalid implementation registry: {error}"))?;
    let mut rows = Vec::new();
    for loaded in contracts {
        for binding in loaded.contract.bindings.keys() {
            let target = effective_target(&status, &loaded.contract, binding)
                .expect("checked target/default coverage");
            let implemented = implementation
                .events
                .iter()
                .find(|event| event.contract == loaded.contract.id);
            let binding_implemented = implemented
                .is_some_and(|event| event.bindings.iter().any(|value| value == binding));
            rows.push((
                loaded.contract.harness.as_str(),
                loaded.contract.event.wire_name.as_str(),
                binding.as_str(),
                target.0,
                target.1,
                implemented.is_some_and(|event| event.native_input),
                implemented.is_some_and(|event| event.native_output),
                binding_implemented,
                binding_implemented
                    && implemented
                        .is_some_and(|event| binding_has_conformance(event, loaded, binding)),
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
    for (
        harness,
        event,
        binding,
        level,
        verification,
        native_input,
        native_output,
        binding_implemented,
        conformance,
    ) in rows
    {
        let (command, other) = if binding == "command" {
            (if binding_implemented { "yes" } else { "no" }, "n/a")
        } else {
            ("n/a", if binding_implemented { "yes" } else { "no" })
        };
        output.push_str(&format!(
            "| {harness} | `{event}` | `{binding}` | yes | yes | yes | {} | {} | {command} | {other} | {} | {} | `{level}` | `{verification}` |\n",
            if native_input { "yes" } else { "no" },
            if native_output { "yes" } else { "no" },
            if conformance { "yes" } else { "no" },
            if verification == "live-observed" { "yes" } else { "no" },
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
    let manifest = content_manifest(&directory, manifest_name, Some(SNAPSHOT_METADATA_FILE))?;
    fs::write(directory.join(manifest_name), manifest)
        .map_err(|error| format!("{}: {error}", directory.display()))?;
    snapshot.state = "frozen".to_string();
    snapshot.manifest_file = Some(manifest_name.to_string());
    let yaml = serde_yaml_ng::to_string(&snapshot)
        .map_err(|error| format!("{}: {error}", snapshot_path.display()))?;
    fs::write(&snapshot_path, &yaml)
        .map_err(|error| format!("{}: {error}", snapshot_path.display()))?;
    verify_content_manifest(&directory, manifest_name, Some(SNAPSHOT_METADATA_FILE))?;
    println!("froze {harness}/{snapshot_id}");
    print_frozen_ledger_entry(
        &format!("harnesses/{harness}/snapshots/{snapshot_id}/{SNAPSHOT_METADATA_FILE}"),
        yaml.as_bytes(),
    );
    Ok(())
}

/// Print the [`FROZEN_LEDGER`] entry for a freshly frozen lifecycle file.
fn print_frozen_ledger_entry(relative: &str, bytes: &[u8]) {
    println!(
        "record it in FROZEN_LEDGER in xtask/src/main.rs, which `contracts check` requires once the registry does not select it:\n    (\"{relative}\", \"{}\"),",
        hex_sha256(bytes)
    );
}

fn freeze_command_environment_supplement(root: &Path, supplement_id: &str) -> Result<()> {
    let directory = root
        .join("contracts/supplements/command-environments")
        .join(supplement_id);
    let supplement_path = directory.join("supplement.yaml");
    let mut supplement: CommandEnvironmentSupplement = read_yaml(&supplement_path)?;
    if supplement.id != supplement_id {
        return Err(format!(
            "{}: supplement identity mismatch",
            directory.display()
        ));
    }
    if supplement.state != "draft" {
        return Err(format!(
            "{}: only draft supplements can be frozen",
            directory.display()
        ));
    }
    let manifest_name = "MANIFEST.sha256";
    supplement.state = "frozen".to_string();
    supplement.manifest_file = Some(manifest_name.to_string());
    let yaml = serde_yaml_ng::to_string(&supplement)
        .map_err(|error| format!("{}: {error}", supplement_path.display()))?;
    fs::write(&supplement_path, &yaml)
        .map_err(|error| format!("{}: {error}", supplement_path.display()))?;
    let manifest = content_manifest(&directory, manifest_name, None)?;
    fs::write(directory.join(manifest_name), manifest)
        .map_err(|error| format!("{}: {error}", directory.display()))?;
    verify_content_manifest(&directory, manifest_name, None)?;
    println!("froze command-environment supplement {supplement_id}");
    print_frozen_ledger_entry(
        &format!("supplements/command-environments/{supplement_id}/supplement.yaml"),
        yaml.as_bytes(),
    );
    Ok(())
}

/// Root-level file that records a snapshot's frozen state. It is rewritten
/// after the manifest is computed, so the manifest cannot cover it; the
/// lifecycle rules in [`validate_snapshot_states`] guard it instead.
const SNAPSHOT_METADATA_FILE: &str = "snapshot.yaml";

/// Render the deterministic SHA-256 manifest of every file below `directory`.
///
/// Only the root-level manifest itself and, for snapshots, the root-level
/// `lifecycle_file` are excluded; identically named files deeper in the tree
/// are ordinary content.
fn content_manifest(
    directory: &Path,
    manifest_name: &str,
    lifecycle_file: Option<&str>,
) -> Result<String> {
    let excluded: Vec<PathBuf> = std::iter::once(manifest_name)
        .chain(lifecycle_file)
        .map(|name| directory.join(name))
        .collect();
    let mut paths: Vec<_> = walkdir::WalkDir::new(directory)
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| format!("{}: {error}", directory.display()))?
        .into_iter()
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .filter(|path| !excluded.contains(path))
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

fn verify_content_manifest(
    directory: &Path,
    manifest_name: &str,
    lifecycle_file: Option<&str>,
) -> Result<()> {
    let manifest_path = safe_join(directory, manifest_name)?;
    let actual = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("{}: {error}", manifest_path.display()))?;
    let expected = content_manifest(directory, manifest_name, lifecycle_file)?;
    if actual != expected {
        return Err(format!(
            "{}: frozen catalog content differs from deterministic manifest",
            directory.display()
        ));
    }
    Ok(())
}

fn validate_implementation_gaps(
    gaps: &ImplementationGaps,
    contracts: &[LoadedContract],
    registry: &ImplementationRegistry,
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
        if implementation_assertion_satisfied(gap, contract, registry)? {
            return Err(format!(
                "implementation gap {contract_id}/{} assertion {} is stale because it now passes",
                gap.binding, gap.assertion
            ));
        }
    }
    Ok(())
}

fn implementation_assertion_satisfied(
    gap: &ImplementationGap,
    contract: &LoadedContract,
    registry: &ImplementationRegistry,
) -> Result<bool> {
    let event = registry
        .events
        .iter()
        .find(|event| event.contract == contract.contract.id);
    implementation_assertion_value(&gap.assertion, &gap.binding, event, &contract.process_cases)
        .map_err(|error| format!("{}: {error}", contract.contract.id))
}

fn implementation_assertion_value(
    assertion: &str,
    binding: &str,
    event: Option<&ImplementationEvent>,
    process_cases: &BTreeMap<String, String>,
) -> Result<bool> {
    Ok(match assertion {
        "native-input" => event.is_some_and(|event| event.native_input),
        "native-output" => event.is_some_and(|event| event.native_output),
        "binding-implemented" => {
            event.is_some_and(|event| event.bindings.iter().any(|value| value == binding))
        }
        assertion if assertion.starts_with("conformance-case:") => {
            let case = assertion
                .strip_prefix("conformance-case:")
                .expect("prefix checked");
            let Some(case_binding) = process_cases.get(case) else {
                return Err(format!("unknown conformance case {case}"));
            };
            if case_binding != binding {
                return Err(format!(
                    "conformance case {case} belongs to binding {case_binding}, not {binding}"
                ));
            }
            event.is_some_and(|event| {
                event.bindings.iter().any(|value| value == binding)
                    && event.conformance_cases.iter().any(|value| value == case)
            })
        }
        _ => {
            return Err(format!("unknown implementation assertion {assertion}"));
        }
    })
}

fn validate_implementation_registry(
    registry: &ImplementationRegistry,
    contracts: &[LoadedContract],
) -> Result<()> {
    require_version(registry.format_version, "implementation registry")?;
    let mut ids = BTreeSet::new();
    for event in &registry.events {
        if !ids.insert(event.contract.as_str()) {
            return Err(format!(
                "duplicate implementation registry entry {}",
                event.contract
            ));
        }
        let Some(contract) = contracts
            .iter()
            .find(|loaded| loaded.contract.id == event.contract)
        else {
            return Err(format!(
                "implementation registry references unknown contract {}",
                event.contract
            ));
        };
        if event.harness != contract.contract.harness
            || event.event != contract.contract.event.wire_name
        {
            return Err(format!(
                "implementation registry identity mismatch for {}",
                event.contract
            ));
        }
        let output_without_input = event.native_output && !event.native_input;
        let binding_without_native =
            !(event.bindings.is_empty() || event.native_input && event.native_output);
        let conformance_without_binding =
            !event.conformance_cases.is_empty() && event.bindings.is_empty();
        if output_without_input || binding_without_native || conformance_without_binding {
            return Err(format!(
                "implementation registry has inconsistent coverage for {}",
                event.contract
            ));
        }
        for binding in &event.bindings {
            if !contract.contract.bindings.contains_key(binding) {
                return Err(format!(
                    "implementation registry references unknown binding {}/{}",
                    event.contract, binding
                ));
            }
        }
        if event.bindings.iter().collect::<BTreeSet<_>>().len() != event.bindings.len()
            || event
                .conformance_cases
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != event.conformance_cases.len()
        {
            return Err(format!(
                "implementation registry has duplicate coverage for {}",
                event.contract
            ));
        }
        for case in &event.conformance_cases {
            let Some(binding) = contract.process_cases.get(case) else {
                return Err(format!(
                    "implementation registry references unknown process case {}/{}",
                    event.contract, case
                ));
            };
            if !event.bindings.contains(binding) {
                return Err(format!(
                    "implementation registry conformance case {}/{} belongs to unimplemented binding {}",
                    event.contract, case, binding
                ));
            }
        }
    }
    Ok(())
}

fn effective_target<'a>(
    status: &'a Stabilization,
    contract: &Contract,
    binding: &str,
) -> Option<(&'a str, &'a str)> {
    status
        .targets
        .iter()
        .find(|target| target.contract == contract.id && target.binding == binding)
        .map(|target| (target.level.as_str(), target.verification.as_str()))
        .or_else(|| {
            status
                .defaults
                .iter()
                .find(|default| default.harness == contract.harness && default.binding == binding)
                .map(|default| (default.level.as_str(), default.verification.as_str()))
        })
}

fn binding_has_conformance(
    event: &ImplementationEvent,
    contract: &LoadedContract,
    binding: &str,
) -> bool {
    event.conformance_cases.iter().any(|case| {
        contract
            .process_cases
            .get(case)
            .is_some_and(|case_binding| case_binding == binding)
    })
}

fn validate_target_implementation(
    status: &Stabilization,
    registry: &ImplementationRegistry,
    contracts: &[LoadedContract],
) -> Result<()> {
    for contract in contracts {
        for binding in contract.contract.bindings.keys() {
            let (level, _) = effective_target(status, &contract.contract, binding)
                .expect("target/default coverage checked");
            if matches!(level, "catalog-only" | "unsupported") {
                continue;
            }
            let implemented = registry
                .events
                .iter()
                .find(|event| event.contract == contract.contract.id);
            let native = implemented.is_some_and(|event| event.native_input && event.native_output);
            let runtime = implemented.is_some_and(|event| {
                event.bindings.contains(binding)
                    && binding_has_conformance(event, contract, binding)
            });
            let supported = support_level_backed(level, binding, native, runtime);
            if !supported {
                return Err(format!(
                    "stabilization target claims {level} for {}/{} without matching native implementation and executed conformance",
                    contract.contract.id, binding
                ));
            }
        }
    }
    Ok(())
}

fn support_level_backed(level: &str, binding: &str, native: bool, runtime: bool) -> bool {
    match level {
        "catalog-only" | "unsupported" => true,
        "native-source-reviewed" => native,
        "command-runtime-beta" | "command-runtime-stable" => {
            binding == "command" && native && runtime
        }
        "other-runtime-beta" | "other-runtime-stable" => binding != "command" && native && runtime,
        _ => false,
    }
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
            match object.get("$ref") {
                Some(Value::String(reference)) if !reference.starts_with('#') => {
                    return Err(format!(
                        "{}: non-fragment $ref is not registered for offline resolution: {reference}",
                        directory.display()
                    ));
                }
                _ => {}
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

/// Verify every vendored file against the `MANIFEST.sha256` in its directory
/// tree and return the number of verified files.
///
/// Every regular file below `contracts/vendor` must be a manifest or be listed
/// by exactly one manifest, and every manifest must list at least one file, so
/// unlisted additions and emptied manifests cannot pass silently.
fn verify_vendor(root: &Path) -> Result<usize> {
    let vendor = root.join("contracts/vendor");
    if !vendor.exists() {
        return Ok(0);
    }
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(&vendor) {
        let entry = entry.map_err(|error| format!("{}: {error}", vendor.display()))?;
        if entry.file_type().is_symlink() {
            return Err(format!(
                "{}: vendored evidence must not be a symbolic link",
                entry.path().display()
            ));
        }
        if entry.file_type().is_file() {
            files.push(entry.into_path());
        }
    }
    files.sort();
    let mut listed = BTreeSet::new();
    let mut count = 0;
    for manifest in files.iter().filter(|path| {
        path.file_name()
            .is_some_and(|name| name == "MANIFEST.sha256")
    }) {
        let directory = manifest.parent().expect("manifest parent");
        let text = fs::read_to_string(manifest)
            .map_err(|error| format!("{}: {error}", manifest.display()))?;
        if text.lines().next().is_none() {
            return Err(format!(
                "{}: vendor manifest lists no files",
                manifest.display()
            ));
        }
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
            if !listed.insert(path.clone()) {
                return Err(format!(
                    "{}:{}: {} is listed more than once",
                    manifest.display(),
                    line_number + 1,
                    path.display()
                ));
            }
            let bytes = fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
            if hex_sha256(&bytes) != expected {
                return Err(format!("{}: vendored checksum mismatch", path.display()));
            }
            count += 1;
        }
    }
    if let Some(unlisted) = files.iter().find(|path| {
        path.file_name()
            .is_none_or(|name| name != "MANIFEST.sha256")
            && !listed.contains(*path)
    }) {
        return Err(format!(
            "{}: vendored file is not listed in a MANIFEST.sha256",
            unlisted.display()
        ));
    }
    Ok(count)
}

fn diff_snapshots(root: &Path, old: &str, new: &str) -> Result<()> {
    print!(
        "{}",
        render_snapshot_diff(&root.join("contracts/harnesses"), old, new)?
    );
    Ok(())
}

/// Describe how two snapshots differ, event by event.
///
/// Every snapshot file embeds its own snapshot ID (contract IDs, schema `$id`s,
/// fixture provenance), so each side's ID is replaced with a placeholder before
/// comparing. What remains are substantive schema, fixture, contract, and
/// provenance differences, which the maintenance workflow classifies.
fn render_snapshot_diff(snapshots: &Path, old: &str, new: &str) -> Result<String> {
    let old_dir = find_snapshot(snapshots, old)?;
    let new_dir = find_snapshot(snapshots, new)?;
    let old_snapshot: Snapshot = read_yaml(&old_dir.join(SNAPSHOT_METADATA_FILE))?;
    let new_snapshot: Snapshot = read_yaml(&new_dir.join(SNAPSHOT_METADATA_FILE))?;
    let old_events: BTreeMap<_, _> = old_snapshot
        .events
        .iter()
        .map(|event| (event.wire_name.as_str(), event))
        .collect();
    let new_events: BTreeMap<_, _> = new_snapshot
        .events
        .iter()
        .map(|event| (event.wire_name.as_str(), event))
        .collect();

    let mut output = String::new();
    for (label, snapshot) in [("old", &old_snapshot), ("new", &new_snapshot)] {
        writeln!(
            &mut output,
            "{label}: {}/{} ({} events)",
            snapshot.harness,
            snapshot.id,
            snapshot.events.len()
        )
        .expect("writing to String cannot fail");
    }
    let (mut added, mut removed, mut changed, mut unchanged) = (0, 0, 0, 0);
    for event in new_events
        .keys()
        .filter(|event| !old_events.contains_key(*event))
    {
        writeln!(&mut output, "+ {event}").expect("writing to String cannot fail");
        added += 1;
    }
    for event in old_events
        .keys()
        .filter(|event| !new_events.contains_key(*event))
    {
        writeln!(&mut output, "- {event}").expect("writing to String cannot fail");
        removed += 1;
    }
    for (event, old_event) in &old_events {
        let Some(new_event) = new_events.get(event) else {
            continue;
        };
        let differences = differing_files(
            &normalized_tree(
                &safe_join(&old_dir, &old_event.path)?,
                &old_snapshot.id,
                false,
            )?,
            &normalized_tree(
                &safe_join(&new_dir, &new_event.path)?,
                &new_snapshot.id,
                false,
            )?,
        );
        if differences.is_empty() {
            writeln!(&mut output, "= {event}").expect("writing to String cannot fail");
            unchanged += 1;
        } else {
            writeln!(&mut output, "~ {event}: {}", differences.join(", "))
                .expect("writing to String cannot fail");
            changed += 1;
        }
    }
    let snapshot_differences = differing_files(
        &normalized_tree(&old_dir, &old_snapshot.id, true)?,
        &normalized_tree(&new_dir, &new_snapshot.id, true)?,
    );
    if !snapshot_differences.is_empty() {
        writeln!(
            &mut output,
            "~ snapshot files: {}",
            snapshot_differences.join(", ")
        )
        .expect("writing to String cannot fail");
    }
    writeln!(
        &mut output,
        "events: {added} added, {removed} removed, {changed} changed, {unchanged} unchanged"
    )
    .expect("writing to String cannot fail");
    writeln!(
        &mut output,
        "content: {}",
        if added + removed + changed == 0 && snapshot_differences.is_empty() {
            "identical"
        } else {
            "changed"
        }
    )
    .expect("writing to String cannot fail");
    Ok(output)
}

/// Read every file below `directory`, keyed by relative path, with each
/// occurrence of `snapshot_id` replaced by a placeholder.
///
/// With `snapshot_root`, the snapshot's lifecycle metadata, its manifest, and
/// the per-event trees (compared separately) are skipped.
fn normalized_tree(
    directory: &Path,
    snapshot_id: &str,
    snapshot_root: bool,
) -> Result<BTreeMap<String, Vec<u8>>> {
    let skipped: Vec<PathBuf> = if snapshot_root {
        [SNAPSHOT_METADATA_FILE, "MANIFEST.sha256", "events"]
            .iter()
            .map(|name| directory.join(name))
            .collect()
    } else {
        Vec::new()
    };
    let mut files = BTreeMap::new();
    let mut walker = walkdir::WalkDir::new(directory).into_iter();
    while let Some(entry) = walker.next() {
        let entry = entry.map_err(|error| format!("{}: {error}", directory.display()))?;
        if skipped.iter().any(|skip| entry.path() == skip) {
            if entry.file_type().is_dir() {
                walker.skip_current_dir();
            }
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(directory)
            .expect("walked path below directory")
            .to_string_lossy()
            .into_owned();
        let bytes = fs::read(entry.path())
            .map_err(|error| format!("{}: {error}", entry.path().display()))?;
        let normalized = match String::from_utf8(bytes) {
            Ok(text) => text.replace(snapshot_id, "<snapshot>").into_bytes(),
            Err(error) => error.into_bytes(),
        };
        files.insert(relative, normalized);
    }
    Ok(files)
}

/// List relative paths whose normalized content differs, marking files that
/// exist on only one side with `+` (new) or `-` (old).
fn differing_files(
    old: &BTreeMap<String, Vec<u8>>,
    new: &BTreeMap<String, Vec<u8>>,
) -> Vec<String> {
    let paths: BTreeSet<_> = old.keys().chain(new.keys()).collect();
    paths
        .into_iter()
        .filter_map(|path| match (old.get(path), new.get(path)) {
            (Some(before), Some(after)) if before == after => None,
            (Some(_), Some(_)) => Some(path.clone()),
            (None, _) => Some(format!("+{path}")),
            (_, None) => Some(format!("-{path}")),
        })
        .collect()
}

fn find_snapshot(root: &Path, id: &str) -> Result<PathBuf> {
    if let Some((harness, snapshot)) = id.split_once('/') {
        let valid_component = |component: &str| {
            !component.is_empty()
                && component != "."
                && component != ".."
                && !component.contains(['/', '\\'])
        };
        if valid_component(harness) && valid_component(snapshot) {
            let qualified = root.join(harness).join("snapshots").join(snapshot);
            if qualified.is_dir() {
                return Ok(qualified);
            }
        }
        return Err(format!("snapshot {id} not found"));
    }

    let mut matches = Vec::new();
    for entry in walkdir::WalkDir::new(root) {
        let entry = entry.map_err(|error| format!("{}: {error}", root.display()))?;
        if entry.file_type().is_dir() && entry.file_name() == id {
            matches.push(entry.into_path());
        }
    }
    match matches.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err(format!("snapshot {id} not found")),
        _ => Err(format!("snapshot id {id} is ambiguous across harnesses")),
    }
}

/// Kind of Git object a GitHub page URL names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitHubObject {
    /// A directory, `https://github.com/<owner>/<repository>/tree/...`.
    Tree,
    /// A file, `https://github.com/<owner>/<repository>/blob/...`.
    Blob,
}

/// Upstream GitHub tree or file named by a source URL of the form
/// `https://github.com/<owner>/<repository>/{tree,blob}/<revision>/<path>`.
#[derive(Debug, PartialEq, Eq)]
struct GitHubPath<'a> {
    owner: &'a str,
    repository: &'a str,
    object: GitHubObject,
    revision: &'a str,
    path: &'a str,
}

impl GitHubPath<'_> {
    /// Anonymous clone URL for the repository.
    fn clone_url(&self) -> String {
        format!("https://github.com/{}/{}.git", self.owner, self.repository)
    }

    /// URL that serves a file's bytes rather than GitHub's HTML page for it.
    fn raw_url(&self) -> Option<String> {
        (self.object == GitHubObject::Blob).then(|| {
            format!(
                "https://raw.githubusercontent.com/{}/{}/{}/{}",
                self.owner, self.repository, self.revision, self.path
            )
        })
    }
}

fn parse_github_path(url: &str) -> Option<GitHubPath<'_>> {
    let mut parts = url.strip_prefix("https://github.com/")?.splitn(5, '/');
    let owner = parts.next()?;
    let repository = parts.next()?;
    let object = match parts.next()? {
        "tree" => GitHubObject::Tree,
        "blob" => GitHubObject::Blob,
        _ => return None,
    };
    let revision = parts.next()?;
    let path = parts.next()?.trim_end_matches('/');
    [owner, repository, revision, path]
        .iter()
        .all(|part| !part.is_empty() && !part.split('/').any(|segment| segment == ".."))
        .then_some(GitHubPath {
            owner,
            repository,
            object,
            revision,
            path,
        })
}

/// URL whose body a recorded `content_sha256` covers: the raw file for a
/// GitHub file page, which itself serves HTML with per-request content, and
/// the source URL otherwise.
fn content_hash_url(url: &str) -> String {
    parse_github_path(url)
        .and_then(|path| path.raw_url())
        .unwrap_or_else(|| url.to_string())
}

/// One source of a registry-selected snapshot or of the registry-selected
/// command-environment supplement.
struct SelectedSource {
    /// Harness (or [`SUPPLEMENT_SOURCE_GROUP`]) and snapshot or supplement ID.
    identity: [String; 2],
    /// Snapshot or supplement directory.
    directory: PathBuf,
    /// Harness whose vendor directory holds a vendored source's evidence.
    vendor_harness: Option<String>,
    source: Source,
}

impl SelectedSource {
    fn label(&self) -> String {
        format!(
            "{}/{}/{}",
            self.identity[0], self.identity[1], self.source.id
        )
    }
}

/// Every source of the registry-selected snapshots, then of the
/// registry-selected command-environment supplement.
fn selected_sources(root: &Path) -> Result<Vec<SelectedSource>> {
    let catalog = root.join("contracts");
    let registry: Registry = read_yaml(&catalog.join("registry.yaml"))?;
    require_version(registry.format_version, "registry")?;
    let mut selected_sources = Vec::new();
    for (harness, selected) in &registry.harnesses {
        let snapshot_dir = safe_join(
            &catalog.join("harnesses"),
            Path::new(harness).join("snapshots").join(&selected.current),
        )?;
        let snapshot: Snapshot = read_yaml(&snapshot_dir.join(SNAPSHOT_METADATA_FILE))?;
        let sources: Sources = read_yaml(&safe_join(&snapshot_dir, &snapshot.sources_file)?)?;
        for source in sources.sources {
            selected_sources.push(SelectedSource {
                identity: [harness.clone(), selected.current.clone()],
                directory: snapshot_dir.clone(),
                vendor_harness: Some(harness.clone()),
                source,
            });
        }
    }

    let supplement_id = &registry.supplements.command_environments;
    let supplement_dir = safe_join(
        &catalog.join("supplements/command-environments"),
        Path::new(supplement_id),
    )?;
    let supplement: CommandEnvironmentSupplement =
        read_yaml(&supplement_dir.join("supplement.yaml"))?;
    let sources: Sources = read_yaml(&safe_join(&supplement_dir, &supplement.sources_file)?)?;
    for source in sources.sources {
        // A vendored supplement source is stored under the one harness that
        // cites it.
        let mut citing = supplement
            .harnesses
            .iter()
            .filter(|(_, harness)| harness.sources.contains(&source.id))
            .map(|(harness, _)| harness.clone());
        let vendor_harness = match (citing.next(), citing.next()) {
            (Some(harness), None) => Some(harness),
            _ => None,
        };
        selected_sources.push(SelectedSource {
            identity: [SUPPLEMENT_SOURCE_GROUP.to_string(), supplement_id.clone()],
            directory: supplement_dir.clone(),
            vendor_harness,
            source,
        });
    }
    Ok(selected_sources)
}

/// For each selected source, the newer content hash that a later selected
/// retrieval of the same URL records after reviewing the difference.
///
/// Two selected sources that pin one URL to different hashes can never both
/// match upstream, so the drift check would fail on one of them forever. The
/// later retrieval (by its `retrieved` date) must acknowledge the earlier one
/// by citing the earlier hash in full in its limitations; the earlier source
/// then accepts the later hash as reviewed drift. Same-day retrievals are
/// ordered by which one cites the other. An unacknowledged or ambiguous
/// disagreement is an error.
fn selected_content_hash_acknowledgements(sources: &[SelectedSource]) -> Result<Vec<Option<&str>>> {
    let cites = |source: &SelectedSource, hash: &str| {
        source
            .source
            .limitations
            .iter()
            .any(|limitation| limitation.contains(hash))
    };
    let mut acknowledged: Vec<Option<&str>> = vec![None; sources.len()];
    for (first_index, first) in sources.iter().enumerate() {
        let Some(first_hash) = first.source.content_sha256.as_deref() else {
            continue;
        };
        let url = content_hash_url(&first.source.url);
        for (second_index, second) in sources.iter().enumerate().skip(first_index + 1) {
            let Some(second_hash) = second.source.content_sha256.as_deref() else {
                continue;
            };
            if second_hash == first_hash || content_hash_url(&second.source.url) != url {
                continue;
            }
            let first_is_older = match first.source.retrieved.cmp(&second.source.retrieved) {
                std::cmp::Ordering::Less => true,
                std::cmp::Ordering::Greater => false,
                std::cmp::Ordering::Equal => {
                    match (cites(second, first_hash), cites(first, second_hash)) {
                        (true, false) => true,
                        (false, true) => false,
                        _ => {
                            return Err(format!(
                                "{url} is pinned to {first_hash} by {} and to {second_hash} by {}, both retrieved {}, and neither is the reviewed successor of the other; cut successors that agree",
                                first.label(),
                                second.label(),
                                first.source.retrieved
                            ));
                        }
                    }
                }
            };
            let ((older_index, older, older_hash), (newer, newer_hash)) = if first_is_older {
                ((first_index, first, first_hash), (second, second_hash))
            } else {
                ((second_index, second, second_hash), (first, first_hash))
            };
            if !cites(newer, older_hash) {
                return Err(format!(
                    "{url} is pinned to {older_hash} by {} and to {newer_hash} by the later retrieval {}, so one of them can never match upstream; after reviewing the difference, cite the earlier hash in the later source's limitations, or cut successors that agree",
                    older.label(),
                    newer.label()
                ));
            }
            match acknowledged[older_index] {
                Some(previous) if previous != newer_hash => {
                    return Err(format!(
                        "{url}: {} is superseded by later selected retrievals with different hashes {previous} and {newer_hash}; cut successors that agree",
                        older.label()
                    ));
                }
                _ => acknowledged[older_index] = Some(newer_hash),
            }
        }
    }
    Ok(acknowledged)
}

/// Render the upstream inputs of every registry-selected snapshot, and of the
/// registry-selected command-environment supplement, for
/// `scripts/check-upstream-contract-drift.sh`.
///
/// Each row holds tab-separated fields, with `-` for an absent value:
/// harness, snapshot, source ID, reproducibility, URL, pinned revision,
/// recorded content SHA-256, Git clone URL, upstream path, the vendored
/// directory relative to the workspace root, and an acknowledged newer
/// content SHA-256. The URL is the one whose body the recorded hash covers:
/// the raw.githubusercontent.com URL for a GitHub file page. GitHub tree and
/// file pages also carry their revision, clone URL, and path. The
/// acknowledged hash is one that another selected source records for the
/// same URL after reviewing the difference; see
/// [`selected_content_hash_acknowledgements`]. Supplement rows use
/// [`SUPPLEMENT_SOURCE_GROUP`] as the harness and the supplement ID as the
/// snapshot, so documentation pages that only the supplement cites (such as
/// the Claude Code environment-variable reference) are drift-checked too.
fn render_upstream_sources(root: &Path) -> Result<String> {
    let sources = selected_sources(root)?;
    let acknowledged = selected_content_hash_acknowledgements(&sources)?;
    let mut output = String::new();
    for (selected, acknowledged) in sources.iter().zip(acknowledged) {
        render_source_row(&mut output, root, selected, acknowledged)?;
    }
    Ok(output)
}

/// Harness column of the upstream-source rows for the registry-selected
/// command-environment supplement.
const SUPPLEMENT_SOURCE_GROUP: &str = "command-environments";

/// Appends one upstream-source row for `selected`, whose recorded content hash
/// another selected source may have `acknowledged` as superseded.
fn render_source_row(
    output: &mut String,
    root: &Path,
    selected: &SelectedSource,
    acknowledged: Option<&str>,
) -> Result<()> {
    let SelectedSource {
        identity,
        directory,
        vendor_harness,
        source,
    } = selected;
    let tree = parse_github_path(&source.url);
    let mismatch = tree
        .as_ref()
        .zip(source.revision.as_deref())
        .filter(|(tree, revision)| tree.revision != *revision);
    if let Some((tree, revision)) = mismatch {
        return Err(format!(
            "{}: source {} URL names revision {} but records {revision}",
            directory.display(),
            source.id,
            tree.revision
        ));
    }
    let vendored = match (&tree, source.reproducibility.as_deref()) {
        (Some(tree), Some("vendored")) if tree.object == GitHubObject::Tree => {
            let harness = vendor_harness.as_deref().ok_or_else(|| {
                format!(
                    "{}: vendored source {} is not cited by exactly one harness",
                    directory.display(),
                    source.id
                )
            })?;
            let leaf = tree.path.rsplit('/').next().unwrap_or(tree.path);
            let relative = format!("contracts/vendor/{harness}/{}/{leaf}", tree.revision);
            if !safe_join(root, &relative)?
                .join("MANIFEST.sha256")
                .is_file()
            {
                return Err(format!(
                    "{}: vendored source {} has no manifest under {relative}",
                    directory.display(),
                    source.id
                ));
            }
            Some(relative)
        }
        _ => None,
    };
    let clone_url = tree.as_ref().map(GitHubPath::clone_url);
    // Only a Git tree's or file's revision can be fetched. Other sources may
    // record a free-form revision label, such as a documentation
    // site's build ETag, which the drift check never uses.
    let revision = tree.as_ref().and(source.revision.as_deref());
    let url = content_hash_url(&source.url);
    let fields = [
        Some(identity[0].as_str()),
        Some(identity[1].as_str()),
        Some(source.id.as_str()),
        source.reproducibility.as_deref(),
        Some(url.as_str()),
        revision,
        source.content_sha256.as_deref(),
        clone_url.as_deref(),
        tree.as_ref().map(|tree| tree.path),
        vendored.as_deref(),
        acknowledged,
    ];
    let mut row = Vec::with_capacity(fields.len());
    for field in fields {
        let field = field.unwrap_or("-");
        if field.is_empty() || field.contains(char::is_whitespace) {
            return Err(format!(
                "{}: source {} has an empty or whitespace-bearing field {field:?}",
                directory.display(),
                source.id
            ));
        }
        row.push(field);
    }
    writeln!(output, "{}", row.join("\t")).expect("writing to String cannot fail");
    Ok(())
}

fn read_yaml<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| format!("{}: YAML must be UTF-8: {error}", path.display()))?;
    reject_yaml_references(text).map_err(|error| format!("{}:{error}", path.display()))?;
    serde_yaml_ng::from_str(text).map_err(|error| format!("{}: {error}", path.display()))
}

/// Reject YAML anchors, aliases, and merge keys, returning a `line: message`
/// error suffix.
///
/// serde_yaml_ng resolves aliases before callers see a value and exposes no
/// parser events, so candidate `&name` and `*name` tokens are found lexically
/// and then confirmed with the parser itself. Renaming a real alias leaves it
/// dangling (a parse error); renaming a real anchor dangles its aliases or,
/// when unused, leaves the parsed document unchanged. Renaming scalar content,
/// such as a block-scalar `*** Begin Patch` line or quoted text, changes the
/// parsed document instead, so such content is accepted.
fn reject_yaml_references(text: &str) -> std::result::Result<(), String> {
    // Syntax errors are left for the typed parse to report precisely.
    let Ok(parsed) = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(text) else {
        return Ok(());
    };
    if let Some(line) = merge_key_line(text, &parsed) {
        return Err(format!("{line}: YAML merge keys are forbidden"));
    }
    for (offset, line, token) in yaml_reference_candidates(text) {
        let (indicator, name) = token.split_at(1);
        let mutated = format!(
            "{}{indicator}hookkit-renamed-{name}{}",
            &text[..offset],
            &text[offset + token.len()..]
        );
        let is_reference = match serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&mutated) {
            Ok(value) => value == parsed,
            Err(_) => true,
        };
        if is_reference {
            return Err(format!("{line}: YAML anchors and aliases are forbidden"));
        }
    }
    Ok(())
}

/// Locate a `<<` mapping key, which serde_yaml_ng keeps as an ordinary key.
fn merge_key_line(text: &str, value: &serde_yaml_ng::Value) -> Option<usize> {
    fn contains_merge_key(value: &serde_yaml_ng::Value) -> bool {
        match value {
            serde_yaml_ng::Value::Mapping(mapping) => mapping.iter().any(|(key, value)| {
                key.as_str() == Some("<<") || contains_merge_key(key) || contains_merge_key(value)
            }),
            serde_yaml_ng::Value::Sequence(items) => items.iter().any(contains_merge_key),
            serde_yaml_ng::Value::Tagged(tagged) => contains_merge_key(&tagged.value),
            _ => false,
        }
    }
    contains_merge_key(value).then(|| {
        text.lines()
            .position(|line| line.contains("<<"))
            .map_or(1, |index| index + 1)
    })
}

/// Find `&name` and `*name` tokens that start a node position lexically:
/// at the start of a line's content or after whitespace or a flow indicator,
/// outside comments. Returns byte offset, 1-based line number, and token.
fn yaml_reference_candidates(text: &str) -> Vec<(usize, usize, &str)> {
    let mut candidates = Vec::new();
    let mut line_start = 0;
    for (index, line) in text.split_inclusive('\n').enumerate() {
        let bytes = line.as_bytes();
        let mut position = 0;
        while position < bytes.len() {
            let byte = bytes[position];
            let boundary =
                position == 0 || matches!(bytes[position - 1], b' ' | b'\t' | b'[' | b'{' | b',');
            if byte == b'#' && (position == 0 || bytes[position - 1].is_ascii_whitespace()) {
                break;
            }
            if boundary && matches!(byte, b'&' | b'*') {
                let length = bytes[position + 1..]
                    .iter()
                    .position(|next| next.is_ascii_whitespace() || b",[]{}".contains(next))
                    .unwrap_or(bytes.len() - position - 1);
                if length > 0 {
                    candidates.push((
                        line_start + position,
                        index + 1,
                        &line[position..=position + length],
                    ));
                }
                position += length + 1;
                continue;
            }
            position += 1;
        }
        line_start += line.len();
    }
    candidates
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

fn validate_json_metadata(path: &Path, meta_schema: &Path) -> Result<()> {
    let value = read_json(path)?;
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
    fn qualified_snapshot_ids_disambiguate_shared_names() {
        let root = std::env::temp_dir().join(format!(
            "hookkit-xtask-qualified-snapshot-{}",
            std::process::id()
        ));
        let alpha = root.join("alpha/snapshots/shared");
        let beta = root.join("beta/snapshots/shared");
        fs::create_dir_all(&alpha).expect("create alpha snapshot");
        fs::create_dir_all(&beta).expect("create beta snapshot");

        assert_eq!(find_snapshot(&root, "alpha/shared").unwrap(), alpha);
        assert!(
            find_snapshot(&root, "shared")
                .unwrap_err()
                .contains("ambiguous")
        );
        assert!(find_snapshot(&root, "../shared").is_err());

        fs::remove_dir_all(root).expect("remove temporary snapshot tree");
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

    #[test]
    fn negative_fixtures_match_pointer_and_keyword_together() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["kind"],
            "properties": {"kind": {"const": "expected"}}
        });
        let validator = compile_schema(&schema, Path::new("test-schema")).unwrap();
        validate_expected_failure(
            &validator,
            &serde_json::json!({"kind": "wrong"}),
            "wrong-kind",
            "/kind",
            Some("const"),
            "negative input",
            Path::new("test-fixtures"),
        )
        .unwrap();
        let error = validate_expected_failure(
            &validator,
            &serde_json::json!({"kind": "wrong"}),
            "wrong-kind",
            "/kind",
            Some("type"),
            "negative input",
            Path::new("test-fixtures"),
        )
        .unwrap_err();
        assert!(error.contains("observed /kind:const"));
    }

    #[test]
    fn json_response_schema_direction_is_bidirectional() {
        assert!(output_schema_direction_valid("json", Some("response")));
        assert!(output_schema_direction_valid("text", None));
        assert!(!output_schema_direction_valid("json", None));
        assert!(!output_schema_direction_valid("text", Some("response")));
    }

    #[test]
    fn runtime_targets_require_native_and_conformance_coverage() {
        assert!(support_level_backed(
            "catalog-only",
            "command",
            false,
            false
        ));
        assert!(support_level_backed(
            "native-source-reviewed",
            "command",
            true,
            false
        ));
        assert!(!support_level_backed(
            "command-runtime-beta",
            "command",
            true,
            false
        ));
        assert!(support_level_backed(
            "command-runtime-beta",
            "command",
            true,
            true
        ));
        assert!(!support_level_backed(
            "command-runtime-beta",
            "http",
            true,
            true
        ));
    }

    #[test]
    fn stable_and_live_claims_require_an_observation_overlay() {
        assert!(!release_claim_requires_observation(
            "command-runtime-beta",
            "source-reviewed"
        ));
        assert!(release_claim_requires_observation(
            "command-runtime-stable",
            "source-reviewed"
        ));
        assert!(release_claim_requires_observation(
            "command-runtime-beta",
            "live-observed"
        ));
    }

    #[test]
    fn only_the_original_frozen_snapshots_use_legacy_v1_semantics() {
        assert!(uses_legacy_snapshot_semantics(
            "claude-code",
            "docs-2026-07-12-r1"
        ));
        assert!(!uses_legacy_snapshot_semantics(
            "claude-code",
            "docs-2026-07-12-r2"
        ));
        assert!(!uses_legacy_snapshot_semantics(
            "claude-code",
            "docs-2026-07-12-r3"
        ));
    }

    #[test]
    fn implementation_gap_assertions_track_real_registry_coverage() {
        let event = ImplementationEvent {
            contract: "h/s/Event".into(),
            harness: "h".into(),
            event: "Event".into(),
            native_input: true,
            native_output: true,
            bindings: vec!["command".into()],
            conformance_cases: vec!["structured".into()],
        };
        let process_cases = BTreeMap::from([("structured".into(), "command".into())]);
        assert!(
            implementation_assertion_value("native-input", "command", Some(&event), &process_cases)
                .unwrap()
        );
        assert!(
            implementation_assertion_value(
                "conformance-case:structured",
                "command",
                Some(&event),
                &process_cases
            )
            .unwrap()
        );
        assert!(
            !implementation_assertion_value(
                "conformance-case:structured",
                "command",
                None,
                &process_cases
            )
            .unwrap()
        );
        assert!(
            implementation_assertion_value(
                "conformance-case:missing",
                "command",
                Some(&event),
                &process_cases
            )
            .is_err()
        );
    }

    #[test]
    fn command_environment_profiles_require_exact_event_coverage() {
        let mut harness = CommandEnvironmentHarness {
            snapshot: "snapshot".into(),
            sources: vec!["source".into()],
            assurance: Assurance {
                confidence: "low".into(),
                verification: "source-reviewed".into(),
            },
            profiles: vec![],
            event_profiles: BTreeMap::from([("OnlyEvent".into(), vec![])]),
        };
        let events = BTreeSet::from(["OnlyEvent"]);
        let sources = BTreeSet::from(["source"]);
        validate_command_environment_harness(&harness, &events, &sources, Path::new("supplement"))
            .expect("an explicit empty environment covers the event");

        harness.event_profiles.clear();
        let error = validate_command_environment_harness(
            &harness,
            &events,
            &sources,
            Path::new("supplement"),
        )
        .expect_err("omitting an event must fail");
        assert!(error.contains("event coverage differs"));
    }

    fn current_template_catalog_parts() -> (
        PathBuf,
        EventScaffoldCatalog,
        AlignmentCatalog,
        ArchetypeCatalog,
        CompatibilityCatalog,
        ImplementationRegistry,
    ) {
        let root = workspace_root().expect("workspace root");
        let catalog = root.join("templates/hook-project/catalog");
        let events = read_yaml(&catalog.join("event-scaffolds.yml")).expect("event catalog");
        let alignments =
            read_yaml(&catalog.join("alignment-families.yml")).expect("alignment catalog");
        let archetypes = read_yaml(&catalog.join("archetypes.yml")).expect("archetype catalog");
        let compatibility =
            read_yaml(&catalog.join("compatibility.yml")).expect("compatibility catalog");
        let implementation = serde_json::from_value(
            read_json(&root.join("contracts/status/implementation/registry.json"))
                .expect("implementation registry"),
        )
        .expect("typed implementation registry");
        (
            root,
            events,
            alignments,
            archetypes,
            compatibility,
            implementation,
        )
    }

    #[test]
    fn current_template_catalog_covers_all_implemented_command_events() {
        let root = workspace_root().expect("workspace root");
        let (events, alignments, archetypes, compatibility) =
            check_template_catalog(&root).expect("current template catalog should be valid");
        check_generated_question_catalog(&root, &events, &alignments, &archetypes, &compatibility)
            .expect("generated Copier data should be current");
        // 33 Claude Code, 12 Codex, and 5 Antigravity events.
        assert_eq!(events.events.len(), 50);
        assert_eq!(alignments.alignments.len(), 11);
    }

    #[test]
    fn template_catalog_rejects_missing_and_duplicate_scaffolds() {
        let (root, events, alignments, archetypes, compatibility, implementation) =
            current_template_catalog_parts();
        let mut missing = events.clone();
        missing.events.pop();
        assert!(
            validate_template_catalog(
                &root,
                &missing,
                &alignments,
                &archetypes,
                &compatibility,
                &implementation,
            )
            .expect_err("missing scaffold must fail")
            .contains("coverage differs")
        );

        let mut duplicate = events.clone();
        duplicate.events.push(events.events[0].clone());
        assert!(
            validate_template_catalog(
                &root,
                &duplicate,
                &alignments,
                &archetypes,
                &compatibility,
                &implementation,
            )
            .expect_err("duplicate scaffold must fail")
            .contains("duplicate contract")
        );
    }

    #[test]
    fn template_catalog_rejects_stale_alignment_members_and_markers() {
        let (root, events, alignments, archetypes, compatibility, implementation) =
            current_template_catalog_parts();
        let mut stale_member = alignments.clone();
        stale_member.alignments[0]
            .native_members
            .insert("codex".into(), "codex/obsolete/PreToolUse".into());
        assert!(
            validate_template_catalog(
                &root,
                &events,
                &stale_member,
                &archetypes,
                &compatibility,
                &implementation,
            )
            .expect_err("stale member must fail")
            .contains("stale native member")
        );

        let mut unknown_marker = alignments.clone();
        unknown_marker.alignments[0].runtime_marker = "hookkit_runtime::aligned::Invented".into();
        assert!(
            validate_template_catalog(
                &root,
                &events,
                &unknown_marker,
                &archetypes,
                &compatibility,
                &implementation,
            )
            .expect_err("unknown marker must fail")
            .contains("unimplemented runtime marker")
        );
    }

    #[test]
    fn template_catalog_rejects_invalid_archetypes_and_compatibility() {
        let (root, events, alignments, archetypes, compatibility, implementation) =
            current_template_catalog_parts();
        let mut invalid_archetypes = archetypes.clone();
        invalid_archetypes.archetypes[0]
            .supported_harnesses
            .push("invented".into());
        assert!(
            validate_template_catalog(
                &root,
                &events,
                &alignments,
                &invalid_archetypes,
                &compatibility,
                &implementation,
            )
            .expect_err("unknown archetype harness must fail")
            .contains("invalid identity, mode, harness, or order")
        );

        let mut invalid_compatibility = compatibility.clone();
        invalid_compatibility.hookkit.git_revision = "main".into();
        assert!(
            validate_template_catalog(
                &root,
                &events,
                &alignments,
                &archetypes,
                &invalid_compatibility,
                &implementation,
            )
            .expect_err("mutable compatibility revision must fail")
            .contains("compatibility catalog")
        );
    }

    #[test]
    fn refreshed_event_regressions_are_explicit_in_the_scaffold_overlay() {
        let (_, events, _, _, _, _) = current_template_catalog_parts();
        let find = |contract: &str| {
            events
                .events
                .iter()
                .find(|event| event.contract == contract)
                .expect("regression scaffold")
        };

        assert!(
            find("claude-code/docs-2026-09-29-r1/DirectoryAdded")
                .support
                .note
                .contains("source")
        );
        assert!(
            find("claude-code/docs-2026-09-29-r1/SessionStart")
                .support
                .note
                .contains("source=fork")
        );
        // Claude Code discards the JSON output of these events.
        for event in [
            "Setup",
            "InstructionsLoaded",
            "Notification",
            "StopFailure",
            "SessionEnd",
            "PostCompact",
            "WorktreeRemove",
        ] {
            let scaffold = find(&format!("claude-code/docs-2026-09-29-r1/{event}"));
            assert!(scaffold.capabilities.output_ignored, "{event}");
            assert!(!scaffold.capabilities.post_action_feedback, "{event}");
        }
        // A WorktreeRemove hook that exits 0 counts the worktree as removed
        // and replaces Claude Code's `git worktree remove` fallback, so a
        // starter that deletes nothing must not report success.
        let worktree_remove = find("claude-code/docs-2026-09-29-r1/WorktreeRemove");
        assert_eq!(worktree_remove.starter.strategy, "must_implement");
        assert!(
            worktree_remove
                .starter
                .expression
                .contains("implement WorktreeRemove cleanup")
        );
        assert!(
            find("claude-code/docs-2026-09-29-r1/PreModelSwitch")
                .capabilities
                .true_pre_action_block
        );
        assert!(
            !find("claude-code/docs-2026-09-29-r1/PostModelSwitch")
                .capabilities
                .true_pre_action_block
        );
        let codex_session_end = find("codex/commit-ff6aec9-r1/SessionEnd");
        assert!(codex_session_end.capabilities.output_ignored);
        assert!(codex_session_end.starter.expression.ends_with("::no_op()"));
        let codex_interrupt = find("codex/commit-ff6aec9-r1/Interrupt");
        assert!(!codex_interrupt.capabilities.true_pre_action_block);
        assert!(codex_interrupt.starter.expression.ends_with("::no_op()"));
        let antigravity_post_tool = find("antigravity/docs-2026-09-29-r1/PostToolUse");
        assert!(antigravity_post_tool.support.note.contains("typed"));
        assert!(!antigravity_post_tool.capabilities.tool_result);
        assert!(
            find("antigravity/docs-2026-09-29-r1/Stop")
                .starter
                .expression
                .ends_with("::allow_stop()")
        );
    }

    fn temp_tree(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "hookkit-xtask-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock follows the Unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create temporary tree");
        root
    }

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().expect("file has a parent")).expect("create parent");
        fs::write(path, contents).expect("write temporary file");
    }

    #[test]
    fn yaml_scalar_content_resembling_references_is_accepted() {
        let text = concat!(
            "cases:\n",
            "  - id: patch # an &anchor-like comment\n",
            "    command: |\n",
            "      *** Begin Patch\n",
            "      *** Update File: src/main.rs\n",
            "      apply_patch <<'PATCH'\n",
            "      *** End Patch\n",
            "    quoted: \"*not-an-alias and &not-an-anchor\"\n",
            "    plain: emphasis *inside* prose\n",
        );
        assert_eq!(reject_yaml_references(text), Ok(()));
    }

    #[test]
    fn yaml_references_are_rejected_in_every_position() {
        for (text, expected) in [
            ("items:\n  - &item one\n  - two\n", "anchors and aliases"),
            ("items:\n  - &item one\n  - *item\n", "anchors and aliases"),
            (
                "base: &base {a: 1}\nflow: {b: *base}\n",
                "anchors and aliases",
            ),
            ("list: [&first 1, *first]\n", "anchors and aliases"),
            ("base: {a: 1}\nchild:\n  <<: {a: 2}\n", "merge keys"),
        ] {
            let error = reject_yaml_references(text).expect_err(text);
            assert!(error.contains(expected), "{text:?} produced {error}");
        }
    }

    #[test]
    fn selected_and_superseded_snapshots_must_stay_frozen() {
        let registry = Registry {
            format_version: 1,
            harnesses: BTreeMap::from([(
                "codex".to_string(),
                RegistryHarness {
                    current: "selected".to_string(),
                },
            )]),
            supplements: RegistrySupplements {
                command_environments: "supplement".to_string(),
            },
        };
        let selected = BTreeMap::from([("codex".to_string(), "2026-08-04".to_string())]);
        let draft = |id: &str, retrieved: &str| {
            (
                PathBuf::from(id),
                Snapshot {
                    format_version: 1,
                    id: id.to_string(),
                    harness: "codex".to_string(),
                    state: "draft".to_string(),
                    retrieved: retrieved.to_string(),
                    sources_file: "sources.yaml".to_string(),
                    manifest_file: None,
                    events: Vec::new(),
                },
            )
        };

        let error =
            validate_snapshot_states(&registry, &selected, &[draft("selected", "2026-08-04")])
                .expect_err("a selected draft must fail");
        assert!(error.contains("registry-selected snapshot must be frozen"));
        let error = validate_snapshot_states(&registry, &selected, &[draft("old", "2026-07-12")])
            .expect_err("an unfrozen historical snapshot must fail");
        assert!(error.contains("superseded snapshots must stay frozen"));
        validate_snapshot_states(&registry, &selected, &[draft("successor", "2026-09-29")])
            .expect("a successor draft is a legitimate candidate");
    }

    #[test]
    fn content_manifests_exclude_only_root_lifecycle_files() {
        let root = temp_tree("manifest");
        write_file(&root.join("snapshot.yaml"), "state: frozen\n");
        write_file(&root.join("events/nested/snapshot.yaml"), "content\n");
        write_file(&root.join("events/nested/MANIFEST.sha256"), "content\n");
        let manifest = content_manifest(&root, "MANIFEST.sha256", Some(SNAPSHOT_METADATA_FILE))
            .expect("manifest renders");
        assert!(manifest.contains("  events/nested/snapshot.yaml\n"));
        assert!(manifest.contains("  events/nested/MANIFEST.sha256\n"));
        assert!(!manifest.contains("  snapshot.yaml\n"));
        fs::remove_dir_all(root).expect("remove temporary tree");
    }

    #[test]
    fn vendor_verification_rejects_unlisted_files_and_empty_manifests() {
        let root = temp_tree("vendor");
        let generated = root.join("contracts/vendor/codex/0123/generated");
        write_file(&generated.join("a.json"), "{}\n");
        write_file(
            &generated.join("MANIFEST.sha256"),
            &format!("{}  a.json\n", hex_sha256(b"{}\n")),
        );
        assert_eq!(verify_vendor(&root), Ok(1));

        write_file(&generated.join("unlisted.json"), "{}\n");
        let error = verify_vendor(&root).expect_err("an unlisted file must fail");
        assert!(error.contains("not listed in a MANIFEST.sha256"));
        fs::remove_file(generated.join("unlisted.json")).expect("remove unlisted file");

        write_file(&generated.join("MANIFEST.sha256"), "");
        let error = verify_vendor(&root).expect_err("an empty manifest must fail");
        assert!(error.contains("lists no files"));
        fs::remove_dir_all(root).expect("remove temporary tree");
    }

    #[test]
    fn snapshot_diff_ignores_snapshot_ids_and_reports_changed_events() {
        let root = temp_tree("diff");
        let write_snapshot = |id: &str, schema: &str, retrieved: &str| {
            let directory = root.join("codex/snapshots").join(id);
            write_file(
                &directory.join("snapshot.yaml"),
                &format!(
                    "format_version: 1\nid: {id}\nharness: codex\nstate: draft\nretrieved: {retrieved}\nsources_file: sources.yaml\nevents:\n- wire_name: Stop\n  rust_key: stop\n  path: events/stop\n- wire_name: PreToolUse\n  rust_key: pre_tool_use\n  path: events/pre-tool-use\n"
                ),
            );
            write_file(
                &directory.join("sources.yaml"),
                &format!("retrieved: {retrieved}\n"),
            );
            write_file(
                &directory.join("events/stop/contract.yaml"),
                &format!("id: codex/{id}/Stop\n"),
            );
            write_file(
                &directory.join("events/pre-tool-use/input.schema.json"),
                &format!("{{\"$id\": \"urn:{id}\", {schema}}}\n"),
            );
        };
        write_snapshot("old-r1", "\"type\": \"object\"", "2026-08-04");
        write_snapshot("same-r2", "\"type\": \"object\"", "2026-08-04");
        write_snapshot("new-r3", "\"type\": \"array\"", "2026-09-29");

        let identical = render_snapshot_diff(&root, "old-r1", "same-r2").expect("diff renders");
        assert!(identical.contains("= PreToolUse\n"), "{identical}");
        assert!(identical.ends_with("content: identical\n"), "{identical}");

        let changed = render_snapshot_diff(&root, "old-r1", "new-r3").expect("diff renders");
        assert!(changed.contains("= Stop\n"), "{changed}");
        assert!(
            changed.contains("~ PreToolUse: input.schema.json\n"),
            "{changed}"
        );
        assert!(
            changed.contains("~ snapshot files: sources.yaml\n"),
            "{changed}"
        );
        assert!(changed.ends_with("content: changed\n"), "{changed}");
        fs::remove_dir_all(root).expect("remove temporary tree");
    }

    #[test]
    fn github_tree_and_file_urls_parse_into_clone_url_revision_and_path() {
        let tree = parse_github_path(
            "https://github.com/openai/codex/tree/1e59dc5/codex-rs/hooks/schema/generated/",
        )
        .expect("tree URL parses");
        assert_eq!(tree.object, GitHubObject::Tree);
        assert_eq!(tree.revision, "1e59dc5");
        assert_eq!(tree.path, "codex-rs/hooks/schema/generated");
        assert_eq!(tree.clone_url(), "https://github.com/openai/codex.git");
        assert_eq!(tree.raw_url(), None);

        let blob = parse_github_path("https://github.com/openai/codex/blob/1e59dc5/README.md")
            .expect("file URL parses");
        assert_eq!(blob.object, GitHubObject::Blob);
        assert_eq!(blob.path, "README.md");
        assert_eq!(
            blob.raw_url().as_deref(),
            Some("https://raw.githubusercontent.com/openai/codex/1e59dc5/README.md")
        );
        // A recorded content hash covers the raw file, never GitHub's page.
        assert_eq!(
            content_hash_url("https://github.com/openai/codex/blob/1e59dc5/README.md"),
            "https://raw.githubusercontent.com/openai/codex/1e59dc5/README.md"
        );
        assert_eq!(
            content_hash_url("https://learn.chatgpt.com/docs/hooks.md"),
            "https://learn.chatgpt.com/docs/hooks.md"
        );

        assert!(parse_github_path("https://learn.chatgpt.com/docs/hooks").is_none());
        assert!(
            parse_github_path("https://github.com/openai/codex/commits/1e59dc5/README.md")
                .is_none()
        );
        assert!(
            parse_github_path("https://github.com/openai/codex/tree/1e59dc5/../escape").is_none()
        );
    }

    fn documentation_source(url: &str, hash: &str, limitations: &[&str]) -> Source {
        Source {
            id: "page".to_string(),
            kind: "official-documentation".to_string(),
            authority: "primary".to_string(),
            url: url.to_string(),
            retrieved: "2026-09-30".to_string(),
            revision: None,
            content_sha256: Some(hash.to_string()),
            license: None,
            reproducibility: Some("content-hash-only".to_string()),
            limitations: limitations.iter().map(ToString::to_string).collect(),
        }
    }

    #[test]
    fn new_retrievals_cannot_hash_github_pages() {
        let hash = "a".repeat(64);
        let blob = documentation_source(
            "https://github.com/google-antigravity/antigravity-cli/blob/eaf9e06/CHANGELOG.md",
            &hash,
            &[],
        );
        let error = validate_source(&blob, Path::new("snapshot"), true, true)
            .expect_err("a hash of a github.com page is irreproducible");
        assert!(error.contains("raw.githubusercontent.com"), "{error}");
        // Frozen snapshots retrieved before the rule keep their evidence.
        validate_source(&blob, Path::new("snapshot"), true, false)
            .expect("grandfathered retrievals are accepted");
        let raw = documentation_source(
            "https://raw.githubusercontent.com/google-antigravity/antigravity-cli/eaf9e06/CHANGELOG.md",
            &hash,
            &[],
        );
        validate_source(&raw, Path::new("snapshot"), true, true)
            .expect("the raw file URL is what the hash covers");

        assert!(requires_fetchable_content_hashes("draft", "2026-07-12"));
        assert!(requires_fetchable_content_hashes("frozen", "2026-09-30"));
        assert!(!requires_fetchable_content_hashes("frozen", "2026-09-29"));
    }

    #[test]
    fn selected_sources_must_agree_on_a_url_hash_or_acknowledge_the_difference() {
        let older_hash = "c".repeat(64);
        let newer_hash = "5".repeat(64);
        let selected = |group: &str, retrieved: &str, source: Source| SelectedSource {
            identity: [group.to_string(), "snapshot".to_string()],
            directory: PathBuf::from(group),
            vendor_harness: None,
            source: Source {
                retrieved: retrieved.to_string(),
                ..source
            },
        };
        let url = "https://code.claude.com/docs/en/hooks.md";
        let cites_older = format!("It differs from the older retrieval (sha256 {older_hash}).");
        let cites_newer = format!("A later retrieval (sha256 {newer_hash}) is not pinned.");
        let older = |limitations: &[&str]| {
            selected(
                "claude-code",
                "2026-09-29",
                documentation_source(url, &older_hash, limitations),
            )
        };
        let newer = |limitations: &[&str]| {
            selected(
                "supplement",
                "2026-09-30",
                documentation_source(url, &newer_hash, limitations),
            )
        };

        let error = selected_content_hash_acknowledgements(&[older(&[]), newer(&[])])
            .expect_err("an unreviewed disagreement must fail");
        assert!(error.contains("can never match upstream"), "{error}");
        // Only the later retrieval can review the difference.
        let error =
            selected_content_hash_acknowledgements(&[older(&[cites_newer.as_str()]), newer(&[])])
                .expect_err("the earlier source cannot acknowledge a later one");
        assert!(error.contains("can never match upstream"), "{error}");

        // The later retrieval's citation makes the later hash acceptable for
        // the earlier source, never the reverse, whatever the source order or
        // whether the earlier source also mentions the later hash.
        let reviewed = [older(&[]), newer(&[cites_older.as_str()])];
        assert_eq!(
            selected_content_hash_acknowledgements(&reviewed).expect("reviewed drift"),
            vec![Some(newer_hash.as_str()), None]
        );
        let mutual = [
            newer(&[cites_older.as_str()]),
            older(&[cites_newer.as_str()]),
        ];
        assert_eq!(
            selected_content_hash_acknowledgements(&mutual).expect("reviewed drift"),
            vec![None, Some(newer_hash.as_str())]
        );

        // Same-day retrievals are ordered by which one reviewed the other.
        let same_day = [
            selected(
                "a",
                "2026-09-30",
                documentation_source(url, &older_hash, &[]),
            ),
            selected(
                "b",
                "2026-09-30",
                documentation_source(url, &newer_hash, &[cites_older.as_str()]),
            ),
        ];
        assert_eq!(
            selected_content_hash_acknowledgements(&same_day).expect("reviewed same-day drift"),
            vec![Some(newer_hash.as_str()), None]
        );
        let ambiguous = [
            selected(
                "a",
                "2026-09-30",
                documentation_source(url, &older_hash, &[]),
            ),
            selected(
                "b",
                "2026-09-30",
                documentation_source(url, &newer_hash, &[]),
            ),
        ];
        let error = selected_content_hash_acknowledgements(&ambiguous)
            .expect_err("same-day retrievals that disagree need a reviewed successor");
        assert!(
            error.contains("neither is the reviewed successor"),
            "{error}"
        );

        // The same body under GitHub's page and raw URLs is one URL.
        let page = "https://github.com/o/r/blob/0123/CHANGELOG.md";
        let raw = "https://raw.githubusercontent.com/o/r/0123/CHANGELOG.md";
        let aliases = [
            selected(
                "a",
                "2026-09-29",
                documentation_source(page, &older_hash, &[]),
            ),
            selected(
                "b",
                "2026-09-30",
                documentation_source(raw, &newer_hash, &[]),
            ),
        ];
        assert!(selected_content_hash_acknowledgements(&aliases).is_err());
    }

    #[test]
    fn frozen_ledger_pins_lifecycle_files_and_requires_superseded_entries() {
        let catalog = temp_tree("ledger");
        let old = catalog.join("harnesses/codex/snapshots/old/snapshot.yaml");
        let unlisted = catalog.join("harnesses/codex/snapshots/unlisted/snapshot.yaml");
        write_file(&old, "state: frozen\nretrieved: 2026-08-04\n");
        write_file(&unlisted, "state: frozen\nretrieved: 2026-08-05\n");
        let digest = hex_sha256(b"state: frozen\nretrieved: 2026-08-04\n");
        let ledger = [(
            "harnesses/codex/snapshots/old/snapshot.yaml",
            digest.as_str(),
        )];

        validate_frozen_ledger(&catalog, &ledger, std::slice::from_ref(&old))
            .expect("an unchanged superseded snapshot is valid");
        let error = validate_frozen_ledger(&catalog, &ledger, &[old.clone(), unlisted.clone()])
            .expect_err("a superseded snapshot must be recorded");
        assert!(error.contains("FROZEN_LEDGER"), "{error}");
        assert!(
            error.contains("harnesses/codex/snapshots/unlisted/snapshot.yaml"),
            "{error}"
        );

        // Demoting a recorded snapshot to a later-dated draft is detected even
        // though its manifest was removed with it.
        write_file(&old, "state: draft\nretrieved: 2026-12-01\n");
        let error = validate_frozen_ledger(&catalog, &ledger, &[])
            .expect_err("a demoted snapshot must fail");
        assert!(error.contains("frozen lifecycle file changed"), "{error}");
        fs::remove_file(&old).expect("remove lifecycle file");
        let error = validate_frozen_ledger(&catalog, &ledger, &[])
            .expect_err("a removed snapshot must fail");
        assert!(error.contains("cannot be read"), "{error}");
        fs::remove_dir_all(catalog).expect("remove temporary tree");
    }

    #[test]
    fn frozen_ledger_matches_the_checked_in_catalog() {
        let root = workspace_root().expect("workspace root");
        let catalog = root.join("contracts");
        validate_frozen_ledger(&catalog, FROZEN_LEDGER, &[])
            .expect("every recorded lifecycle file is unchanged");
        for (relative, _) in FROZEN_LEDGER {
            let text = fs::read_to_string(catalog.join(relative)).expect("lifecycle file");
            assert!(text.contains("state: frozen"), "{relative}");
        }
    }

    #[test]
    fn upstream_sources_follow_the_registry_selection() {
        let root = workspace_root().expect("workspace root");
        let registry: Registry =
            read_yaml(&root.join("contracts/registry.yaml")).expect("registry parses");
        let rendered = render_upstream_sources(&root).expect("selected sources render");
        let rows: Vec<Vec<&str>> = rendered
            .lines()
            .map(|line| line.split('\t').collect())
            .collect();
        assert!(rows.iter().all(|row| row.len() == 11));
        for (harness, selected) in &registry.harnesses {
            assert!(
                rows.iter()
                    .any(|row| row[0] == harness && row[1] == selected.current),
                "{harness} has no selected source row"
            );
        }
        // Every source of the selected command-environment supplement is
        // drift-checked, including pages no event snapshot cites.
        let supplement_id = &registry.supplements.command_environments;
        let supplement_dir = root
            .join("contracts/supplements/command-environments")
            .join(supplement_id);
        let supplement: CommandEnvironmentSupplement =
            read_yaml(&supplement_dir.join("supplement.yaml")).expect("supplement parses");
        let sources: Sources = read_yaml(&supplement_dir.join(&supplement.sources_file))
            .expect("supplement sources parse");
        for source in &sources.sources {
            let row = rows
                .iter()
                .find(|row| {
                    row[0] == SUPPLEMENT_SOURCE_GROUP
                        && row[1] == supplement_id.as_str()
                        && row[2] == source.id
                })
                .unwrap_or_else(|| panic!("supplement source {} has no row", source.id));
            assert_eq!(row[4], content_hash_url(&source.url));
            assert_eq!(row[6], source.content_sha256.as_deref().unwrap_or("-"));
        }
        // A GitHub file page is hashed through its raw URL and compared with
        // upstream HEAD through its pinned revision and path.
        for row in rows
            .iter()
            .filter(|row| row[4].starts_with("https://github.com/"))
        {
            assert_eq!(row[6], "-", "{row:?} hashes a github.com page");
        }
        let selected = selected_sources(&root).expect("selected sources load");
        assert_eq!(selected.len(), rows.len());
        for (selected, row) in selected.iter().zip(&rows) {
            let Some(file) = parse_github_path(&selected.source.url)
                .filter(|path| path.object == GitHubObject::Blob)
            else {
                continue;
            };
            assert_eq!(Some(row[4].to_string()), file.raw_url(), "{row:?}");
            assert_eq!(row[5], file.revision, "{row:?}");
            assert_eq!(row[7], file.clone_url(), "{row:?}");
            assert_eq!(row[8], file.path, "{row:?}");
        }
        // A hash that another selected source reviewed and superseded is
        // acknowledged, so the drift check does not fail on it forever.
        for row in rows.iter().filter(|row| row[10] != "-") {
            assert!(valid_sha256(row[10]), "{row:?}");
            assert_ne!(row[10], row[6], "{row:?}");
            assert!(
                rows.iter()
                    .any(|other| other[4] == row[4] && other[6] == row[10]),
                "{row:?} acknowledges a hash no selected source records"
            );
        }
        assert!(
            rows.iter().any(|row| row[0] == SUPPLEMENT_SOURCE_GROUP
                && row[4] == "https://code.claude.com/docs/en/env-vars.md"),
            "the environment-variable reference is drift-checked"
        );
        let vendored = rows
            .iter()
            .find(|row| row[3] == "vendored")
            .expect("the Codex generated schemas are vendored");
        assert!(vendored[9].starts_with(&format!(
            "contracts/vendor/{}/{}/",
            vendored[0], vendored[5]
        )));
        assert!(root.join(vendored[9]).join("MANIFEST.sha256").is_file());
    }

    #[test]
    fn claude_and_codex_pre_tool_starters_pass_through() {
        // An explicit "allow" skips Claude Code's permission prompt, so the
        // starters' default must keep the harness's normal permission flow.
        let (_, events, _, _, _, _) = current_template_catalog_parts();
        let starters: Vec<_> = events
            .events
            .iter()
            .filter(|event| event.stable_id == "pre_tool_use" && event.harness != "antigravity")
            .collect();
        assert_eq!(starters.len(), 2);
        for event in starters {
            assert_eq!(event.starter.strategy, "no_op", "{}", event.contract);
            assert!(
                event
                    .starter
                    .expression
                    .ends_with("PreToolUseOutput::no_op()"),
                "{}",
                event.contract
            );
        }
    }

    #[test]
    fn toolchain_pins_are_parsed_after_their_prefix() {
        assert_eq!(
            pinned_versions(
                "uses: dtolnay/rust-toolchain@1.85.0\nuses: dtolnay/rust-toolchain@stable\n",
                "dtolnay/rust-toolchain@"
            ),
            vec!["1.85.0"]
        );
        assert_eq!(
            pinned_versions("run: cargo +1.85.0 check.", "cargo +"),
            vec!["1.85.0"]
        );
        assert!(same_version("1.85", "1.85.0"));
        assert!(same_version("9.17.1", "9.17.1"));
        assert!(!same_version("1.86", "1.85.0"));
    }
}
