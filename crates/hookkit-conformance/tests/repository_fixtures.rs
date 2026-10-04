//! Keeps the repository-level `fixtures/` tree honest.
//!
//! Every input fixture must decode, through its harness adapter, as the event
//! its file name spells, and every golden output must be exactly what a typed
//! constructor emits for the registry-selected snapshots. A golden file with
//! no constructor below fails the test, so no fixture can silently describe a
//! shape the harness rejects.

use hookkit_core::{EventId, HarnessSpec, ProcessEmission, RawInvocation};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn json_files(directory: &str) -> Vec<PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(fixtures_root().join(directory))
        .unwrap_or_else(|error| panic!("fixtures/{directory}: {error}"))
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    files.sort();
    files
}

/// `pre_tool_use` names the `PreToolUse` wire event.
fn wire_name(stem: &str) -> String {
    stem.split('_')
        .map(|word| {
            let mut characters = word.chars();
            characters
                .next()
                .map(|first| first.to_ascii_uppercase().to_string() + characters.as_str())
                .unwrap_or_default()
        })
        .collect()
}

/// Decodes every input fixture under `fixtures/<directory>` and returns the
/// events they cover.
fn decode_inputs<H: HarnessSpec>(directory: &str) -> Vec<String> {
    let mut events = Vec::new();
    for path in json_files(directory) {
        let stem = path.file_stem().unwrap().to_str().unwrap();
        let event = EventId::new(H::ID, wire_name(stem)).unwrap();
        let raw = RawInvocation::parse(std::fs::read(&path).unwrap())
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        if let Some(discriminator) = raw.json().get("hook_event_name") {
            assert_eq!(
                discriminator.as_str(),
                Some(event.name()),
                "{}",
                path.display()
            );
        }
        let input = H::decode(&event, &raw)
            .unwrap_or_else(|error| panic!("{} failed native decode: {error}", path.display()));
        assert_eq!(H::input_event(&input), event, "{}", path.display());
        events.push(event.name().to_owned());
    }
    events
}

#[test]
fn every_input_fixture_decodes_as_the_event_its_file_names() {
    let claude = decode_inputs::<hookkit_claude::protocol::ClaudeCode>("claude");
    assert!(claude.iter().any(|event| event == "PreModelSwitch"));
    assert!(claude.iter().any(|event| event == "PostModelSwitch"));
    let codex = decode_inputs::<hookkit_codex::protocol::Codex>("codex");
    assert!(codex.iter().any(|event| event == "Interrupt"));
    let antigravity = decode_inputs::<hookkit_antigravity::Antigravity>("antigravity");
    assert!(!antigravity.is_empty());
}

fn stdout(emission: hookkit_core::Result<ProcessEmission>) -> serde_json::Value {
    let emission = emission.expect("golden constructor emits");
    assert_eq!(emission.exit_code(), 0);
    assert!(emission.stderr().is_empty());
    serde_json::from_slice(emission.stdout()).expect("golden output is JSON")
}

#[test]
fn every_golden_output_is_what_a_typed_constructor_emits() {
    use hookkit_core::EventSpec;

    let expected: BTreeMap<&str, serde_json::Value> = BTreeMap::from([
        (
            "claude_post_model_switch_context.json",
            stdout(hookkit_claude::events::PostModelSwitch::emit(
                hookkit_claude::events::PostModelSwitchOutput::with_context(
                    "On Opus, delegate implementation work to subagents.",
                ),
            )),
        ),
        (
            "claude_post_tool_context.json",
            stdout(hookkit_claude::events::PostToolUse::emit(
                hookkit_claude::events::PostToolUseOutput::with_context(
                    "formatted 3 files with rustfmt",
                ),
            )),
        ),
        (
            "claude_pre_model_switch_block.json",
            stdout(hookkit_claude::events::PreModelSwitch::emit(
                hookkit_claude::events::PreModelSwitchOutput::block(
                    "Opus 4.6 is retired for this project.",
                ),
            )),
        ),
        (
            "claude_pre_tool_deny.json",
            stdout(hookkit_claude::events::PreToolUse::emit(
                hookkit_claude::events::PreToolUseOutput::deny("destructive command blocked"),
            )),
        ),
        (
            "claude_stop_continue.json",
            stdout(hookkit_claude::events::Stop::emit(
                hookkit_claude::events::StopOutput::block("tests still failing, keep going"),
            )),
        ),
        (
            "codex_interrupt_notice.json",
            stdout(hookkit_codex::catalog::Interrupt::emit(
                hookkit_codex::catalog::InterruptOutput::system_message(
                    "Saved the interrupted turn to the local audit log.",
                ),
            )),
        ),
        (
            // Codex reads a PreToolUse denial only from `hookSpecificOutput`;
            // its top-level `decision` accepts only approve or block.
            "codex_pre_tool_deny.json",
            stdout(hookkit_codex::protocol::PreToolUse::emit(
                hookkit_codex::protocol::PreToolUseOutput::deny("force push to main not allowed"),
            )),
        ),
        (
            "codex_stop_continue.json",
            stdout(hookkit_codex::catalog::Stop::emit(
                hookkit_codex::catalog::StopOutput::block("task incomplete"),
            )),
        ),
    ]);

    let files = json_files("golden");
    let names: Vec<_> = files
        .iter()
        .map(|path| path.file_name().unwrap().to_str().unwrap())
        .collect();
    assert_eq!(
        names,
        expected.keys().copied().collect::<Vec<_>>(),
        "every golden file needs a typed constructor here"
    );
    for path in &files {
        let actual: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(actual, expected[name], "{name}");
    }
}
