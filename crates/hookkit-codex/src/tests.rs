use crate::input::{self, CodexHookInput};
use crate::output::OutputEnvelope;

fn load_fixture(name: &str) -> serde_json::Value {
    let path = format!(
        "{}/fixtures/codex/{name}",
        env!("CARGO_MANIFEST_DIR").replace("/crates/hookkit-codex", "")
    );
    let data = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {path}: {e}"));
    serde_json::from_str(&data).expect("fixture is not valid JSON")
}

#[test]
fn parse_session_start() {
    let v = load_fixture("session_start.json");
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, CodexHookInput::SessionStart(_)));
    if let CodexHookInput::SessionStart(ev) = input {
        assert_eq!(ev.common.session_id, "codex-sess-001");
    }
}

#[test]
fn parse_pre_tool_use() {
    let v = load_fixture("pre_tool_use.json");
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, CodexHookInput::PreToolUse(_)));
    if let CodexHookInput::PreToolUse(ev) = input {
        assert_eq!(ev.tool_name.as_deref(), Some("Bash"));
    }
}

#[test]
fn parse_post_tool_use() {
    let v = load_fixture("post_tool_use.json");
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, CodexHookInput::PostToolUse(_)));
}

#[test]
fn parse_user_prompt_submit() {
    let v = load_fixture("user_prompt_submit.json");
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, CodexHookInput::UserPromptSubmit(_)));
    if let CodexHookInput::UserPromptSubmit(ev) = input {
        assert_eq!(
            ev.user_prompt.as_deref(),
            Some("Refactor the database module")
        );
    }
}

#[test]
fn parse_stop() {
    let v = load_fixture("stop.json");
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, CodexHookInput::Stop(_)));
    if let CodexHookInput::Stop(ev) = input {
        assert_eq!(ev.stop_reason.as_deref(), Some("end_turn"));
    }
}

#[test]
fn parse_unknown_event() {
    let v = serde_json::json!({
        "sessionId": "test",
        "cwd": "/tmp",
        "hookEventName": "NewEvent"
    });
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, CodexHookInput::Unknown { .. }));
}

#[test]
fn output_deny() {
    let out = OutputEnvelope::deny("force push not allowed");
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["decision"], "deny");
    assert_eq!(json["reason"], "force push not allowed");
}

#[test]
fn output_empty() {
    let out = OutputEnvelope::new();
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json, serde_json::json!({}));
}

#[test]
fn output_stop_continue() {
    let out = OutputEnvelope::stop_continue("not done yet");
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["decision"], "block");
    assert_eq!(json["reason"], "not done yet");
}
