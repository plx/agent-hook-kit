use crate::input::{self, GeminiHookInput, GeminiToolInput};
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

// ---- Parse tests ----

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
        let typed = ev.typed_tool_input().unwrap();
        assert!(matches!(typed, GeminiToolInput::Shell(_)));
        if let GeminiToolInput::Shell(shell) = typed {
            assert_eq!(shell.command, vec!["rm", "-rf", "/"]);
        }
    }
}

#[test]
fn parse_after_tool() {
    let v = load_fixture("after_tool.json");
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, GeminiHookInput::AfterTool(_)));
    if let GeminiHookInput::AfterTool(ev) = input {
        assert!(ev.tool_response.is_some());
        let typed = ev.typed_tool_input().unwrap();
        assert!(matches!(typed, GeminiToolInput::Shell(_)));
    }
}

#[test]
fn parse_after_agent() {
    let v = load_fixture("after_agent.json");
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, GeminiHookInput::AfterAgent(_)));
    if let GeminiHookInput::AfterAgent(ev) = input {
        assert!(ev.agent_response.is_some());
    }
}

#[test]
fn parse_notification() {
    let v = serde_json::json!({
        "sessionId": "gemini-sess-001",
        "cwd": "/tmp",
        "hookEventName": "Notification",
        "message": "test notification"
    });
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, GeminiHookInput::Notification(_)));
    if let GeminiHookInput::Notification(ev) = input {
        assert_eq!(ev.message.as_deref(), Some("test notification"));
    }
}

#[test]
fn parse_pre_compress() {
    let v = serde_json::json!({
        "sessionId": "gemini-sess-001",
        "cwd": "/tmp",
        "hookEventName": "PreCompress"
    });
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, GeminiHookInput::PreCompress(_)));
}

#[test]
fn parse_session_end() {
    let v = serde_json::json!({
        "sessionId": "gemini-sess-001",
        "cwd": "/tmp",
        "hookEventName": "SessionEnd"
    });
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, GeminiHookInput::SessionEnd(_)));
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

// ---- Output tests ----

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
fn output_replace_tool_result() {
    let out = OutputEnvelope::replace_tool_result(serde_json::json!({"stdout": "replaced"}));
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(
        json["hookSpecificOutput"]["tool_response"]["stdout"],
        "replaced"
    );
}

#[test]
fn output_retry() {
    let out = OutputEnvelope::retry("needs another pass");
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["decision"], "retry");
    assert_eq!(json["reason"], "needs another pass");
}

#[test]
fn output_stop() {
    let out = OutputEnvelope::stop("completed");
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["decision"], "stop");
    assert_eq!(json["reason"], "completed");
}

#[test]
fn output_with_context() {
    let out = OutputEnvelope::with_context("formatted the file");
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(
        json["hookSpecificOutput"]["additionalContext"],
        "formatted the file"
    );
}

#[test]
fn output_no_nulls_for_omitted_fields() {
    let out = OutputEnvelope::deny("test");
    let json = serde_json::to_string(&out).unwrap();
    assert!(
        !json.contains("null"),
        "JSON should not contain null values"
    );
}
