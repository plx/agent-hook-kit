use hookkit_core::{HandlerKind, NativeEventDescriptor};
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

#[derive(Serialize)]
struct Registry<'a> {
    format_version: u32,
    events: Vec<Event<'a>>,
}

#[derive(Serialize)]
struct Event<'a> {
    contract: &'a str,
    harness: &'a str,
    event: &'a str,
    native_input: bool,
    native_output: bool,
    bindings: Vec<&'static str>,
    conformance_cases: &'a [&'a str],
}

fn main() {
    let check = std::env::args().any(|arg| arg == "--check");
    let mut descriptors: Vec<_> = [
        hookkit_claude::protocol::EVENTS,
        hookkit_codex::protocol::EVENTS,
        hookkit_gemini::protocol::EVENTS,
        hookkit_antigravity::EVENTS,
    ]
    .into_iter()
    .flatten()
    .collect();
    descriptors.sort_by_key(|descriptor| descriptor.contract_id);
    let registry = Registry {
        format_version: 1,
        events: descriptors.into_iter().map(event).collect(),
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

fn event(descriptor: &NativeEventDescriptor) -> Event<'_> {
    Event {
        contract: descriptor.contract_id,
        harness: descriptor.harness,
        event: descriptor.event,
        native_input: descriptor.native_input,
        native_output: descriptor.native_output,
        bindings: descriptor.bindings.iter().map(binding_name).collect(),
        conformance_cases: descriptor.conformance_cases,
    }
}

fn binding_name(binding: &HandlerKind) -> &'static str {
    match binding {
        HandlerKind::Command => "command",
        HandlerKind::Http => "http",
        HandlerKind::McpTool => "mcp_tool",
        HandlerKind::Prompt => "prompt",
        HandlerKind::Agent => "agent",
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate lives under workspace/crates")
        .to_path_buf()
}
