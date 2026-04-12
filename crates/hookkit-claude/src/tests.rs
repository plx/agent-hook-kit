use crate::input::{self, ClaudeHookInput, ClaudeToolInput};
use crate::output::{
    ClaudeEventOutput, ClaudeFileChangedOutput, ClaudePermissionDeniedOutput,
    ClaudePostToolUseOutput, ClaudePreToolUseOutput, ClaudePromptSubmitOutput,
    ClaudeSessionStartOutput, ClaudeStopOutput, ClaudeWorktreeCreateOutput, OutputEnvelope,
};

fn load_fixture(name: &str) -> serde_json::Value {
    let path = format!(
        "{}/fixtures/claude/{name}",
        env!("CARGO_MANIFEST_DIR").replace("/crates/hookkit-claude", "")
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
    assert!(matches!(input, ClaudeHookInput::SessionStart(_)));
    if let ClaudeHookInput::SessionStart(ev) = input {
        assert_eq!(ev.common.session_id, "abc-123-def");
        assert_eq!(ev.common.hook_event_name, "SessionStart");
        assert_eq!(ev.common.cwd, "/home/user/project");
    }
}

#[test]
fn parse_with_snake_case_event_name_alias() {
    let v = serde_json::json!({
        "sessionId": "abc-123-def",
        "cwd": "/home/user/project",
        "hook_event_name": "SessionStart"
    });
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::SessionStart(_)));
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
        let typed = ev.typed_tool_input().unwrap();
        assert!(matches!(typed, ClaudeToolInput::Bash(_)));
        if let ClaudeToolInput::Bash(bash) = typed {
            assert_eq!(bash.command, "rm -rf /tmp/build-artifacts");
        }
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
        let typed = ev.typed_tool_input().unwrap();
        assert!(matches!(typed, ClaudeToolInput::Write(_)));
    }
}

#[test]
fn parse_post_tool_use_failure() {
    let v = serde_json::json!({
        "sessionId": "abc-123",
        "cwd": "/home/user/project",
        "hookEventName": "PostToolUseFailure",
        "toolName": "Bash",
        "toolInput": {"command": "false"},
        "error": "exit code 1"
    });
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::PostToolUseFailure(_)));
    if let ClaudeHookInput::PostToolUseFailure(ev) = input {
        assert_eq!(ev.error.as_deref(), Some("exit code 1"));
    }
}

#[test]
fn parse_permission_denied() {
    let v = serde_json::json!({
        "sessionId": "abc-123",
        "cwd": "/home/user/project",
        "hookEventName": "PermissionDenied",
        "toolName": "Bash",
        "toolInput": {"command": "rm -rf /"},
        "permissionDecision": {"decision": "deny", "reason": "too dangerous"}
    });
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::PermissionDenied(_)));
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
fn parse_notification() {
    let v = serde_json::json!({
        "sessionId": "abc-123",
        "cwd": "/home/user/project",
        "hookEventName": "Notification",
        "message": "Build complete",
        "level": "info"
    });
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::Notification(_)));
    if let ClaudeHookInput::Notification(ev) = input {
        assert_eq!(ev.message.as_deref(), Some("Build complete"));
        assert_eq!(ev.level.as_deref(), Some("info"));
    }
}

#[test]
fn parse_session_end() {
    let v = serde_json::json!({
        "sessionId": "abc-123",
        "cwd": "/home/user/project",
        "hookEventName": "SessionEnd"
    });
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::SessionEnd(_)));
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

// ---- Tool input parsing tests ----

#[test]
fn tool_input_unknown_tool() {
    let v = serde_json::json!({"foo": "bar"});
    let result = input::parse_tool_input("SomeCustomTool", &v);
    assert!(matches!(result, ClaudeToolInput::Unknown(_)));
}

#[test]
fn tool_input_bash_invalid_shape() {
    let v = serde_json::json!({"not_command": 42});
    let result = input::parse_tool_input("Bash", &v);
    // Falls back to Unknown when required fields are missing
    assert!(matches!(result, ClaudeToolInput::Unknown(_)));
}

// ---- Output tests ----

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
fn output_allow() {
    let out = OutputEnvelope::allow();
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["decision"], "allow");
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
fn output_pre_tool_allow() {
    let out = OutputEnvelope::pre_tool_allow();
    let json = serde_json::to_value(&out).unwrap();
    let perm = &json["hookSpecificOutput"]["permissionDecision"];
    assert_eq!(perm["decision"], "allow");
}

#[test]
fn output_pre_tool_ask() {
    let out = OutputEnvelope::pre_tool_ask("needs user confirmation");
    let json = serde_json::to_value(&out).unwrap();
    let perm = &json["hookSpecificOutput"]["permissionDecision"];
    assert_eq!(perm["decision"], "ask");
    assert_eq!(perm["reason"], "needs user confirmation");
}

#[test]
fn output_pre_tool_deny_with_updated_input() {
    let out = OutputEnvelope::pre_tool_deny_with_updated_input(
        "sanitized",
        serde_json::json!({"command": "echo safe"}),
    );
    let json = serde_json::to_value(&out).unwrap();
    let perm = &json["hookSpecificOutput"]["permissionDecision"];
    assert_eq!(perm["decision"], "deny");
    assert_eq!(perm["updatedInput"]["command"], "echo safe");
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
fn output_permission_denied_retry() {
    let out = OutputEnvelope::permission_denied_retry();
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["hookSpecificOutput"]["retry"], true);
}

#[test]
fn output_with_system_message() {
    let out = OutputEnvelope::allow().with_system_message("you are a linter");
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["systemMessage"], "you are a linter");
}

#[test]
fn output_with_suppress() {
    let out = OutputEnvelope::allow().with_suppress_output(true);
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["suppressOutput"], true);
}

#[test]
fn output_no_nulls_for_omitted_fields() {
    let out = OutputEnvelope::allow();
    let json = serde_json::to_string(&out).unwrap();
    assert!(
        !json.contains("null"),
        "JSON should not contain null values"
    );
}

// ---- Phase 4: Advanced Claude event tests ----

fn mk_json(event_name: &str) -> serde_json::Value {
    serde_json::json!({
        "sessionId": "test-session",
        "cwd": "/tmp",
        "hookEventName": event_name
    })
}

fn mk_json_with(event_name: &str, extra: serde_json::Value) -> serde_json::Value {
    let mut v = mk_json(event_name);
    if let (Some(base), Some(ext)) = (v.as_object_mut(), extra.as_object()) {
        for (k, val) in ext {
            base.insert(k.clone(), val.clone());
        }
    }
    v
}

#[test]
fn parse_permission_request() {
    let v = mk_json_with(
        "PermissionRequest",
        serde_json::json!({"toolName": "Bash", "toolInput": {"command": "ls"}}),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::PermissionRequest(_)));
}

#[test]
fn parse_subagent_start() {
    let v = mk_json_with(
        "SubagentStart",
        serde_json::json!({"subagentId": "agent-1", "subagentType": "coder"}),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::SubagentStart(_)));
}

#[test]
fn parse_subagent_stop() {
    let v = mk_json_with("SubagentStop", serde_json::json!({"subagentId": "agent-1"}));
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::SubagentStop(_)));
}

#[test]
fn parse_task_created() {
    let v = mk_json_with(
        "TaskCreated",
        serde_json::json!({"taskId": "t-1", "taskDescription": "fix the bug"}),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::TaskCreated(_)));
}

#[test]
fn parse_task_completed() {
    let v = mk_json_with("TaskCompleted", serde_json::json!({"taskId": "t-1"}));
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::TaskCompleted(_)));
}

#[test]
fn parse_teammate_idle() {
    let v = mk_json_with(
        "TeammateIdle",
        serde_json::json!({"teammateId": "teammate-1"}),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::TeammateIdle(_)));
}

#[test]
fn parse_config_change() {
    let v = mk_json_with(
        "ConfigChange",
        serde_json::json!({"configKey": "model", "configValue": "opus"}),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::ConfigChange(_)));
}

#[test]
fn parse_cwd_changed() {
    let v = mk_json_with(
        "CwdChanged",
        serde_json::json!({"oldCwd": "/old", "newCwd": "/new"}),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::CwdChanged(_)));
}

#[test]
fn parse_file_changed() {
    let v = mk_json_with(
        "FileChanged",
        serde_json::json!({"filePath": "/tmp/test.rs", "changeType": "modified"}),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::FileChanged(_)));
}

#[test]
fn parse_pre_compact() {
    let v = mk_json("PreCompact");
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::PreCompact(_)));
}

#[test]
fn parse_post_compact() {
    let v = mk_json("PostCompact");
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::PostCompact(_)));
}

#[test]
fn parse_instructions_loaded() {
    let v = mk_json_with(
        "InstructionsLoaded",
        serde_json::json!({"instructionsSource": "CLAUDE.md"}),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::InstructionsLoaded(_)));
}

#[test]
fn parse_worktree_create() {
    let v = mk_json_with(
        "WorktreeCreate",
        serde_json::json!({"worktreePath": "/tmp/worktree-1", "branch": "feature-x"}),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::WorktreeCreate(_)));
}

#[test]
fn parse_worktree_remove() {
    let v = mk_json_with(
        "WorktreeRemove",
        serde_json::json!({"worktreePath": "/tmp/worktree-1"}),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::WorktreeRemove(_)));
}

#[test]
fn parse_elicitation() {
    let v = mk_json_with(
        "Elicitation",
        serde_json::json!({"question": "Which option?", "options": ["a", "b"]}),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::Elicitation(_)));
    if let ClaudeHookInput::Elicitation(ev) = input {
        assert_eq!(ev.question.as_deref(), Some("Which option?"));
        assert_eq!(ev.options.as_ref().unwrap().len(), 2);
    }
}

#[test]
fn parse_elicitation_result() {
    let v = mk_json_with(
        "ElicitationResult",
        serde_json::json!({"result": {"choice": "a"}}),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::ElicitationResult(_)));
}

#[test]
fn parse_stop_failure() {
    let v = mk_json_with("StopFailure", serde_json::json!({"error": "timed out"}));
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::StopFailure(_)));
}

// ---- Phase 4 output tests ----

#[test]
fn output_permission_approve() {
    let out = OutputEnvelope::permission_approve();
    let json = serde_json::to_value(&out).unwrap();
    let perm = &json["hookSpecificOutput"]["permissionDecision"];
    assert_eq!(perm["decision"], "allow");
}

#[test]
fn output_permission_deny() {
    let out = OutputEnvelope::permission_deny("too dangerous");
    let json = serde_json::to_value(&out).unwrap();
    let perm = &json["hookSpecificOutput"]["permissionDecision"];
    assert_eq!(perm["decision"], "deny");
    assert_eq!(perm["reason"], "too dangerous");
}

#[test]
fn output_worktree_path() {
    let out = OutputEnvelope::worktree_path("/tmp/worktree-1");
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(
        json["hookSpecificOutput"]["worktreePath"],
        "/tmp/worktree-1"
    );
}

#[test]
fn output_watch_paths() {
    let out = OutputEnvelope::watch_paths(vec!["/tmp/src".to_string(), "/tmp/tests".to_string()]);
    let json = serde_json::to_value(&out).unwrap();
    let paths = json["hookSpecificOutput"]["watchPaths"].as_array().unwrap();
    assert_eq!(paths.len(), 2);
}

#[test]
fn output_session_start_event_scoped() {
    let out: OutputEnvelope = ClaudeSessionStartOutput::new()
        .with_context("session prep context")
        .with_system_message("custom system")
        .with_suppress_output(true)
        .into();
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(
        json["hookSpecificOutput"]["additionalContext"],
        "session prep context"
    );
    assert_eq!(json["systemMessage"], "custom system");
    assert_eq!(json["suppressOutput"], true);
}

#[test]
fn output_prompt_submit_event_scoped() {
    let allow: OutputEnvelope = ClaudePromptSubmitOutput::allow().into();
    let allow_json = serde_json::to_value(&allow).unwrap();
    assert_eq!(allow_json["decision"], "allow");

    let block: OutputEnvelope = ClaudePromptSubmitOutput::block("insufficient context").into();
    let block_json = serde_json::to_value(&block).unwrap();
    assert_eq!(block_json["decision"], "block");
    assert_eq!(block_json["reason"], "insufficient context");
}

#[test]
fn output_pre_tool_use_event_scoped() {
    let allow: OutputEnvelope = ClaudePreToolUseOutput::allow().into();
    let allow_json = serde_json::to_value(&allow).unwrap();
    assert_eq!(
        allow_json["hookSpecificOutput"]["permissionDecision"]["decision"],
        "allow"
    );

    let deny: OutputEnvelope = ClaudePreToolUseOutput::deny("dangerous").into();
    let deny_json = serde_json::to_value(&deny).unwrap();
    let deny_decision = &deny_json["hookSpecificOutput"]["permissionDecision"];
    assert_eq!(deny_decision["decision"], "deny");
    assert_eq!(deny_decision["reason"], "dangerous");

    let ask: OutputEnvelope = ClaudePreToolUseOutput::ask("need approval").into();
    let ask_json = serde_json::to_value(&ask).unwrap();
    let ask_decision = &ask_json["hookSpecificOutput"]["permissionDecision"];
    assert_eq!(ask_decision["decision"], "ask");
    assert_eq!(ask_decision["reason"], "need approval");
}

#[test]
fn output_post_tool_use_event_scoped() {
    let out: OutputEnvelope = ClaudePostToolUseOutput::new()
        .with_context("lint completed")
        .into();
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(
        json["hookSpecificOutput"]["additionalContext"],
        "lint completed"
    );
}

#[test]
fn output_permission_denied_event_scoped() {
    let out: OutputEnvelope = ClaudePermissionDeniedOutput::retry().into();
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["hookSpecificOutput"]["retry"], true);
}

#[test]
fn output_stop_event_scoped() {
    let allow: OutputEnvelope = ClaudeStopOutput::allow_stop().into();
    let allow_json = serde_json::to_value(&allow).unwrap();
    assert_eq!(allow_json, serde_json::json!({}));

    let cont: OutputEnvelope = ClaudeStopOutput::continue_session("still working").into();
    let cont_json = serde_json::to_value(&cont).unwrap();
    assert_eq!(cont_json["decision"], "block");
    assert_eq!(cont_json["reason"], "still working");
}

#[test]
fn output_worktree_create_event_scoped() {
    let out: OutputEnvelope = ClaudeWorktreeCreateOutput::new("/tmp/wt-2").into();
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["hookSpecificOutput"]["worktreePath"], "/tmp/wt-2");
}

#[test]
fn output_file_changed_event_scoped() {
    let out: OutputEnvelope =
        ClaudeFileChangedOutput::new(vec!["/tmp/a".to_string(), "/tmp/b".to_string()]).into();
    let json = serde_json::to_value(&out).unwrap();
    let paths = json["hookSpecificOutput"]["watchPaths"].as_array().unwrap();
    assert_eq!(paths.len(), 2);
    assert_eq!(paths[0], "/tmp/a");
    assert_eq!(paths[1], "/tmp/b");
}

#[test]
fn output_event_enum_dispatch() {
    let out: OutputEnvelope =
        ClaudeEventOutput::Stop(ClaudeStopOutput::continue_session("wait")).into();
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["decision"], "block");
    assert_eq!(json["reason"], "wait");
}
