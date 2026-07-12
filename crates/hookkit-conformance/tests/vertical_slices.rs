use base64::Engine as _;
use hookkit_core::{EventSpec, RawInvocation};

fn fixture(harness: &str, snapshot: &str, event: &str) -> serde_yaml_ng::Value {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let path = root
        .join("contracts/harnesses")
        .join(harness)
        .join("snapshots")
        .join(snapshot)
        .join("events")
        .join(event)
        .join("fixtures.yaml");
    serde_yaml_ng::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn positive_value(document: &serde_yaml_ng::Value, id: &str) -> serde_json::Value {
    let fixtures = document["input"]["positive"].as_sequence().unwrap();
    let value = fixtures
        .iter()
        .find(|fixture| fixture["id"].as_str() == Some(id))
        .unwrap()["value"]
        .clone();
    serde_json::to_value(value).unwrap()
}

fn process_case(document: &serde_yaml_ng::Value, id: &str) -> (Vec<u8>, Vec<u8>, u8) {
    let cases = document["process"].as_sequence().unwrap();
    let case = cases
        .iter()
        .find(|case| case["id"].as_str() == Some(id))
        .unwrap();
    let decode = |field: &str| {
        base64::engine::general_purpose::STANDARD
            .decode(case[field].as_str().unwrap())
            .unwrap()
    };
    (
        decode("stdout_base64"),
        decode("stderr_base64"),
        case["exit_code"].as_u64().unwrap() as u8,
    )
}

fn assert_emission(actual: &hookkit_core::ProcessEmission, expected: (Vec<u8>, Vec<u8>, u8)) {
    assert_eq!(actual.stdout(), expected.0);
    assert_eq!(actual.stderr(), expected.1);
    assert_eq!(actual.exit_code(), expected.2);
}

fn assert_json_emission(actual: &hookkit_core::ProcessEmission, expected: (Vec<u8>, Vec<u8>, u8)) {
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(actual.stdout()).unwrap(),
        serde_json::from_slice::<serde_json::Value>(&expected.0).unwrap()
    );
    assert_eq!(actual.stderr(), expected.1);
    assert_eq!(actual.exit_code(), expected.2);
    assert!(!actual.stdout().ends_with(b"\n"));
}

#[test]
fn catalog_inputs_reach_typed_vertical_slice_fields() {
    let claude = fixture("claude-code", "docs-2026-07-12-r1", "session-start");
    let raw = RawInvocation::parse(
        serde_json::to_vec(&positive_value(&claude, "representative")).unwrap(),
    )
    .unwrap();
    let input = hookkit_claude::protocol::SessionStart::parse(&raw).unwrap();
    assert!(!input.session_id.is_empty());
    assert_eq!(input.hook_event_name, "SessionStart");

    let antigravity = fixture("antigravity", "docs-2026-07-12-r1", "pre-invocation");
    let raw = RawInvocation::parse(
        serde_json::to_vec(&positive_value(&antigravity, "representative")).unwrap(),
    )
    .unwrap();
    let input = hookkit_antigravity::PreInvocation::parse(&raw).unwrap();
    assert!(input.extra.contains_key("futureField"));
    assert_eq!(input.workspace_paths.len(), 2);
}

#[test]
fn exact_non_json_and_structured_emissions_match_contract_cases() {
    let session = fixture("claude-code", "docs-2026-07-12-r1", "session-start");
    let output = hookkit_claude::protocol::SessionStartOutput::structured(
        Some("Read conventions.".into()),
        Some(true),
        Some("Review".into()),
        vec!["/repo/.env".into()],
    );
    let emission = hookkit_claude::protocol::SessionStart::emit(output).unwrap();
    assert_json_emission(&emission, process_case(&session, "command-structured"));
    let output =
        hookkit_claude::protocol::SessionStartOutput::text_context("Hook-provided context.");
    let emission = hookkit_claude::protocol::SessionStart::emit(output).unwrap();
    assert_emission(&emission, process_case(&session, "command-text"));

    let worktree = fixture("claude-code", "docs-2026-07-12-r1", "worktree-create");
    let path = hookkit_claude::protocol::WorktreeCreateOutput::path_with_newline(
        "/tmp/hookkit-worktree".into(),
    )
    .unwrap();
    let emission = hookkit_claude::protocol::WorktreeCreate::emit(path).unwrap();
    assert_emission(&emission, process_case(&worktree, "command-created"));
    let failure = process_case(&worktree, "command-failed");
    let output = hookkit_claude::protocol::WorktreeCreateOutput::failed(
        String::from_utf8(failure.1.clone()).unwrap(),
        failure.2,
    )
    .unwrap();
    let emission = hookkit_claude::protocol::WorktreeCreate::emit(output).unwrap();
    assert_emission(&emission, failure);

    let codex = fixture("codex", "commit-9e552e9-r1", "pre-tool-use");
    let emission = hookkit_codex::protocol::PreToolUse::emit(
        hookkit_codex::protocol::PreToolUseOutput::no_op(),
    )
    .unwrap();
    assert_emission(&emission, process_case(&codex, "no-op"));
    let emission = hookkit_codex::protocol::PreToolUse::emit(
        hookkit_codex::protocol::PreToolUseOutput::deny(Some("blocked".into())),
    )
    .unwrap();
    assert_json_emission(&emission, process_case(&codex, "deny-json"));
    let emission = hookkit_codex::protocol::PreToolUse::emit(
        hookkit_codex::protocol::PreToolUseOutput::deny_stderr("blocked"),
    )
    .unwrap();
    assert_emission(&emission, process_case(&codex, "deny-stderr"));

    let gemini = fixture("gemini-cli", "commit-f354eeb-r1", "before-tool-selection");
    let emission = hookkit_gemini::protocol::BeforeToolSelection::emit(
        hookkit_gemini::protocol::BeforeToolSelectionOutput::no_op(),
    )
    .unwrap();
    assert_json_emission(&emission, process_case(&gemini, "no-op"));
    let output = hookkit_gemini::protocol::BeforeToolSelectionOutput::configure(
        Some(hookkit_gemini::protocol::ToolMode::None),
        Vec::new(),
    )
    .unwrap();
    let emission = hookkit_gemini::protocol::BeforeToolSelection::emit(output).unwrap();
    assert_json_emission(&emission, process_case(&gemini, "disable-tools"));

    let antigravity = fixture("antigravity", "docs-2026-07-12-r1", "pre-invocation");
    let output = hookkit_antigravity::PreInvocationOutput::inject(
        hookkit_antigravity::InjectStep::EphemeralMessage {
            ephemeral_message: "Remember to lint".into(),
        },
    );
    let emission = hookkit_antigravity::PreInvocation::emit(output).unwrap();
    assert_json_emission(&emission, process_case(&antigravity, "inject-reminder"));
}
