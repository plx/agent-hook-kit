use crate::input::{self, ClaudeHookInput};
use crate::output::OutputEnvelope;

fn load_fixture(name: &str) -> serde_json::Value {
    let path = format!(
        "{}/fixtures/claude/{name}",
        env!("CARGO_MANIFEST_DIR").replace("/crates/hookkit-claude", "")
    );
    let data = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {path}: {e}"));
    serde_json::from_str(&data).expect("fixture is not valid JSON")
}

#[test]
fn parse_session_start() {
    let v = load_fixture("session_start.json");
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::SessionStart(_)));
    if let ClaudeHookInput::SessionStart(ev) = input {
        assert_eq!(ev.common.session_id, "abc-123-def");
        assert_eq!(ev.common.hook_event_name, "SessionStart");
        assert_eq!(ev.common.cwd, "/home/user/project");
    }
}

#[test]
fn parse_user_prompt_submit() {
    let v = load_fixture("user_prompt_submit.json");
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::UserPromptSubmit(_)));
    if let ClaudeHookInput::UserPromptSubmit(ev) = input {
        assert_eq!(
            ev.user_prompt.as_deref(),
            Some("Fix the login bug in auth.rs")
        );
    }
}

#[test]
fn parse_pre_tool_use() {
    let v = load_fixture("pre_tool_use.json");
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::PreToolUse(_)));
    if let ClaudeHookInput::PreToolUse(ev) = input {
        assert_eq!(ev.tool_name.as_deref(), Some("Bash"));
    }
}

#[test]
fn parse_post_tool_use() {
    let v = load_fixture("post_tool_use.json");
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::PostToolUse(_)));
    if let ClaudeHookInput::PostToolUse(ev) = input {
        assert_eq!(ev.tool_name.as_deref(), Some("Write"));
        assert!(ev.tool_result.is_some());
    }
}

#[test]
fn parse_stop() {
    let v = load_fixture("stop.json");
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::Stop(_)));
    if let ClaudeHookInput::Stop(ev) = input {
        assert_eq!(ev.stop_reason.as_deref(), Some("end_turn"));
        assert!(ev.last_assistant_message.is_some());
    }
}

#[test]
fn parse_unknown_event() {
    let v = serde_json::json!({
        "sessionId": "test",
        "cwd": "/tmp",
        "hookEventName": "FutureEvent",
        "someField": 42
    });
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::Unknown { .. }));
    if let ClaudeHookInput::Unknown { event_name, raw } = input {
        assert_eq!(event_name, "FutureEvent");
        assert_eq!(raw.as_value().get("someField").unwrap(), 42);
    }
}

#[test]
fn parse_missing_event_name() {
    let v = serde_json::json!({"sessionId": "test", "cwd": "/tmp"});
    let result = input::parse(&v);
    assert!(result.is_err());
}

#[test]
fn output_empty_json() {
    let out = OutputEnvelope::new();
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json, serde_json::json!({}));
}

#[test]
fn output_block() {
    let out = OutputEnvelope::block("not allowed");
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["decision"], "block");
    assert_eq!(json["reason"], "not allowed");
}

#[test]
fn output_pre_tool_deny() {
    let out = OutputEnvelope::pre_tool_deny("dangerous command");
    let json = serde_json::to_value(&out).unwrap();
    let perm = &json["hookSpecificOutput"]["permissionDecision"];
    assert_eq!(perm["decision"], "deny");
    assert_eq!(perm["reason"], "dangerous command");
}

#[test]
fn output_with_context() {
    let out = OutputEnvelope::with_context("formatted file src/main.rs");
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(
        json["hookSpecificOutput"]["additionalContext"],
        "formatted file src/main.rs"
    );
}

#[test]
fn output_stop_continue() {
    let out = OutputEnvelope::stop_continue("task not finished");
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["decision"], "block");
    assert_eq!(json["reason"], "task not finished");
}

#[test]
fn output_allow() {
    let out = OutputEnvelope::allow();
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["decision"], "allow");
}

#[test]
fn unknown_fields_preserved() {
    let v = serde_json::json!({
        "sessionId": "test",
        "cwd": "/tmp",
        "hookEventName": "SessionStart",
        "futureField": "should survive",
        "anotherField": [1, 2, 3]
    });
    let input = input::parse(&v).expect("should parse");
    if let ClaudeHookInput::SessionStart(ev) = input {
        assert_eq!(
            ev.common.extra.get("futureField").unwrap(),
            &serde_json::json!("should survive")
        );
        assert_eq!(
            ev.common.extra.get("anotherField").unwrap(),
            &serde_json::json!([1, 2, 3])
        );
    }
}
