use crate::input::{self, GeminiHookInput};
use crate::output::OutputEnvelope;

fn load_fixture(name: &str) -> serde_json::Value {
    let path = format!(
        "{}/fixtures/gemini/{name}",
        env!("CARGO_MANIFEST_DIR").replace("/crates/hookkit-gemini", "")
    );
    let data = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {path}: {e}"));
    serde_json::from_str(&data).expect("fixture is not valid JSON")
}

#[test]
fn parse_session_start() {
    let v = load_fixture("session_start.json");
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, GeminiHookInput::SessionStart(_)));
    if let GeminiHookInput::SessionStart(ev) = input {
        assert_eq!(ev.common.session_id, "gemini-sess-001");
    }
}

#[test]
fn parse_before_agent() {
    let v = load_fixture("before_agent.json");
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, GeminiHookInput::BeforeAgent(_)));
    if let GeminiHookInput::BeforeAgent(ev) = input {
        assert_eq!(
            ev.user_prompt.as_deref(),
            Some("Update the README with new API docs")
        );
    }
}

#[test]
fn parse_before_tool() {
    let v = load_fixture("before_tool.json");
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, GeminiHookInput::BeforeTool(_)));
    if let GeminiHookInput::BeforeTool(ev) = input {
        assert_eq!(ev.tool_name.as_deref(), Some("shell"));
    }
}

#[test]
fn parse_after_tool() {
    let v = load_fixture("after_tool.json");
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, GeminiHookInput::AfterTool(_)));
    if let GeminiHookInput::AfterTool(ev) = input {
        assert!(ev.tool_response.is_some());
    }
}

#[test]
fn parse_after_agent() {
    let v = load_fixture("after_agent.json");
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, GeminiHookInput::AfterAgent(_)));
}

#[test]
fn parse_unknown_event() {
    let v = serde_json::json!({
        "sessionId": "test",
        "cwd": "/tmp",
        "hookEventName": "BeforeModel"
    });
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, GeminiHookInput::Unknown { .. }));
}

#[test]
fn output_deny() {
    let out = OutputEnvelope::deny("blocked");
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["decision"], "deny");
    assert_eq!(json["reason"], "blocked");
}

#[test]
fn output_rewrite_tool_input() {
    let out = OutputEnvelope::rewrite_tool_input(serde_json::json!({"command": ["ls"]}));
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(
        json["hookSpecificOutput"]["tool_input"]["command"],
        serde_json::json!(["ls"])
    );
}

#[test]
fn output_retry() {
    let out = OutputEnvelope::retry("needs another pass");
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["decision"], "retry");
    assert_eq!(json["reason"], "needs another pass");
}
