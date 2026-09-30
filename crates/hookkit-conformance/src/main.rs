use hookkit_core::{HandlerKind, NativeEventDescriptor};
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

#[derive(Serialize)]
struct Registry {
    format_version: u32,
    events: Vec<Event>,
}

#[derive(Serialize)]
struct Event {
    contract: String,
    harness: String,
    event: String,
    native_input: bool,
    native_output: bool,
    bindings: Vec<&'static str>,
    conformance_cases: Vec<&'static str>,
}

fn main() {
    let check = std::env::args().any(|arg| arg == "--check");
    let mut descriptors = hookkit_conformance::verified_descriptors().unwrap_or_else(|error| {
        eprintln!("conformance failed: {error}");
        std::process::exit(1);
    });
    descriptors.sort_by_key(|descriptor| descriptor.contract().as_str());
    let events = descriptors
        .iter()
        .map(event)
        .collect::<Result<_, _>>()
        .unwrap_or_else(|error| {
            eprintln!("conformance failed: {error}");
            std::process::exit(1);
        });
    let registry = Registry {
        format_version: 1,
        events,
    };
    let rendered = format!(
        "{}\n",
        serde_json::to_string_pretty(&registry).expect("registry is serializable")
    );
    let path = workspace_root().join("contracts/status/implementation/registry.json");
    if check {
        let actual = fs::read_to_string(&path).expect("implementation registry exists");
        if actual != rendered {
            eprintln!("{} is stale", path.display());
            std::process::exit(1);
        }
    } else {
        fs::create_dir_all(path.parent().expect("registry parent")).expect("create status dir");
        fs::write(path, rendered).expect("write implementation registry");
    }
}

fn event(descriptor: &NativeEventDescriptor) -> Result<Event, String> {
    Ok(Event {
        contract: descriptor.contract().to_string(),
        harness: descriptor.event().harness().to_string(),
        event: descriptor.event().name().to_string(),
        // A descriptor can only be built from an `EventSpec`, which supplies
        // both the native parser and the native emitter.
        native_input: true,
        native_output: true,
        bindings: descriptor
            .bindings()
            .iter()
            .map(binding_name)
            .collect::<Result<_, _>>()?,
        conformance_cases: descriptor.conformance_cases().to_vec(),
    })
}

fn binding_name(binding: &HandlerKind) -> Result<&'static str, String> {
    match binding {
        HandlerKind::Command => Ok("command"),
        HandlerKind::Http => Ok("http"),
        HandlerKind::McpTool => Ok("mcp_tool"),
        HandlerKind::Prompt => Ok("prompt"),
        HandlerKind::Agent => Ok("agent"),
        other => Err(format!(
            "handler binding {other:?} has no implementation-registry name"
        )),
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate lives under workspace/crates")
        .to_path_buf()
}
