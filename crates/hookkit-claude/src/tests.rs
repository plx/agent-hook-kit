use crate::input::{self, ClaudeHookInput, ClaudeToolInput};
use crate::output::{
    ClaudeElicitationOutput, ClaudeElicitationResultOutput, ClaudeEventOutput,
    ClaudePermissionDeniedOutput, ClaudePermissionRequestOutput, ClaudePostToolBatchOutput,
    ClaudePostToolUseFailureOutput, ClaudePostToolUseOutput, ClaudePreToolUseOutput,
    ClaudePromptExpansionOutput, ClaudePromptSubmitOutput, ClaudeSessionStartOutput,
    ClaudeStopOutput, ClaudeWatchPathsOutput, ClaudeWorktreeCreateOutput, OutputEnvelope,
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
        assert_eq!(ev.source.as_deref(), Some("startup"));
        assert_eq!(ev.model.as_deref(), Some("claude-sonnet-4-6"));
    }
}

#[test]
fn parse_with_camel_case_event_name_alias() {
    let v = serde_json::json!({
        "session_id": "abc-123-def",
        "cwd": "/home/user/project",
        "hookEventName": "SessionStart"
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
        assert_eq!(ev.prompt.as_deref(), Some("Fix the login bug in auth.rs"));
    }
}

#[test]
fn parse_user_prompt_expansion() {
    let v = load_fixture("user_prompt_expansion.json");
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::UserPromptExpansion(_)));
    if let ClaudeHookInput::UserPromptExpansion(ev) = input {
        assert_eq!(ev.command_name.as_deref(), Some("review-skill"));
        assert_eq!(ev.command_source.as_deref(), Some("plugin"));
        assert_eq!(ev.prompt.as_deref(), Some("/review-skill payments"));
    }
}

#[test]
fn parse_pre_tool_use() {
    let v = load_fixture("pre_tool_use.json");
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::PreToolUse(_)));
    if let ClaudeHookInput::PreToolUse(ev) = input {
        assert_eq!(ev.tool_name.as_deref(), Some("Bash"));
        assert_eq!(ev.tool_use_id.as_deref(), Some("toolu_01PRETOOL"));
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
        assert!(ev.tool_response.is_some());
        assert_eq!(ev.tool_use_id.as_deref(), Some("toolu_01POSTTOOL"));
        let typed = ev.typed_tool_input().unwrap();
        assert!(matches!(typed, ClaudeToolInput::Write(_)));
    }
}

#[test]
fn parse_post_tool_use_failure() {
    let v = serde_json::json!({
        "session_id": "abc-123",
        "cwd": "/home/user/project",
        "hook_event_name": "PostToolUseFailure",
        "tool_name": "Bash",
        "tool_input": {"command": "false"},
        "tool_use_id": "toolu_01FAIL",
        "error": "exit code 1"
    });
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::PostToolUseFailure(_)));
    if let ClaudeHookInput::PostToolUseFailure(ev) = input {
        assert_eq!(ev.error.as_deref(), Some("exit code 1"));
    }
}

#[test]
fn parse_post_tool_batch() {
    let v = load_fixture("post_tool_batch.json");
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::PostToolBatch(_)));
    if let ClaudeHookInput::PostToolBatch(ev) = input {
        let calls = ev.tool_calls.expect("tool calls");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].tool_name.as_deref(), Some("Read"));
    }
}

#[test]
fn parse_permission_denied() {
    let v = serde_json::json!({
        "session_id": "abc-123",
        "cwd": "/home/user/project",
        "hook_event_name": "PermissionDenied",
        "tool_name": "Bash",
        "tool_input": {"command": "rm -rf /"},
        "tool_use_id": "toolu_01DENIED",
        "reason": "too dangerous"
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
        assert_eq!(ev.stop_hook_active, Some(false));
        assert!(ev.last_assistant_message.is_some());
    }
}

#[test]
fn parse_notification() {
    let v = serde_json::json!({
        "session_id": "abc-123",
        "cwd": "/home/user/project",
        "hook_event_name": "Notification",
        "message": "Build complete",
        "title": "Build finished",
        "notification_type": "idle_prompt"
    });
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::Notification(_)));
    if let ClaudeHookInput::Notification(ev) = input {
        assert_eq!(ev.message.as_deref(), Some("Build complete"));
        assert_eq!(ev.title.as_deref(), Some("Build finished"));
        assert_eq!(ev.notification_type.as_deref(), Some("idle_prompt"));
    }
}

#[test]
fn parse_session_end() {
    let v = serde_json::json!({
        "session_id": "abc-123",
        "cwd": "/home/user/project",
        "hook_event_name": "SessionEnd",
        "reason": "other"
    });
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::SessionEnd(_)));
}

#[test]
fn parse_unknown_event() {
    let v = serde_json::json!({
        "session_id": "test",
        "cwd": "/tmp",
        "hook_event_name": "FutureEvent",
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
    let v = serde_json::json!({"session_id": "test", "cwd": "/tmp"});
    let result = input::parse(&v);
    assert!(result.is_err());
}

#[test]
fn unknown_fields_preserved() {
    let v = serde_json::json!({
        "session_id": "test",
        "cwd": "/tmp",
        "hook_event_name": "SessionStart",
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
    assert_eq!(json, serde_json::json!({}));
}

#[test]
fn output_pre_tool_deny() {
    let out = OutputEnvelope::pre_tool_deny("dangerous command");
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    assert_eq!(json["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(
        json["hookSpecificOutput"]["permissionDecisionReason"],
        "dangerous command"
    );
}

#[test]
fn output_pre_tool_allow() {
    let out = OutputEnvelope::pre_tool_allow();
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["hookSpecificOutput"]["permissionDecision"], "allow");
}

#[test]
fn output_pre_tool_ask() {
    let out = OutputEnvelope::pre_tool_ask("needs user confirmation");
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["hookSpecificOutput"]["permissionDecision"], "ask");
    assert_eq!(
        json["hookSpecificOutput"]["permissionDecisionReason"],
        "needs user confirmation"
    );
}

#[test]
fn output_pre_tool_deny_with_updated_input() {
    let out = OutputEnvelope::pre_tool_deny_with_updated_input(
        "sanitized",
        serde_json::json!({"command": "echo safe"}),
    );
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(
        json["hookSpecificOutput"]["updatedInput"]["command"],
        "echo safe"
    );
}

#[test]
fn output_with_context() {
    let out = OutputEnvelope::with_context("formatted file src/main.rs");
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["hookSpecificOutput"]["hookEventName"], "PostToolUse");
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
        "session_id": "test-session",
        "cwd": "/tmp",
        "hook_event_name": event_name
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
        serde_json::json!({
            "tool_name": "Bash",
            "tool_input": {"command": "ls"},
            "permission_suggestions": [{"type": "addRules"}]
        }),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::PermissionRequest(_)));
}

#[test]
fn parse_subagent_start() {
    let v = mk_json_with(
        "SubagentStart",
        serde_json::json!({"agent_id": "agent-1", "agent_type": "coder"}),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::SubagentStart(_)));
}

#[test]
fn parse_subagent_stop() {
    let v = mk_json_with(
        "SubagentStop",
        serde_json::json!({
            "agent_id": "agent-1",
            "agent_type": "coder",
            "stop_hook_active": false,
            "agent_transcript_path": "/tmp/subagents/agent-1.jsonl"
        }),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::SubagentStop(_)));
}

#[test]
fn parse_task_created() {
    let v = mk_json_with(
        "TaskCreated",
        serde_json::json!({
            "task_id": "t-1",
            "task_subject": "fix the bug",
            "task_description": "investigate auth flow"
        }),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::TaskCreated(_)));
}

#[test]
fn parse_task_completed() {
    let v = mk_json_with(
        "TaskCompleted",
        serde_json::json!({"task_id": "t-1", "task_subject": "fix the bug"}),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::TaskCompleted(_)));
}

#[test]
fn parse_teammate_idle() {
    let v = mk_json_with(
        "TeammateIdle",
        serde_json::json!({"teammate_name": "teammate-1", "team_name": "alpha"}),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::TeammateIdle(_)));
}

#[test]
fn parse_config_change() {
    let v = mk_json_with(
        "ConfigChange",
        serde_json::json!({"source": "project_settings", "file_path": "/tmp/.claude/settings.json"}),
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
        serde_json::json!({"file_path": "/tmp/test.rs", "event": "change"}),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::FileChanged(_)));
}

#[test]
fn parse_pre_compact() {
    let v = mk_json_with(
        "PreCompact",
        serde_json::json!({"trigger": "manual", "custom_instructions": ""}),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::PreCompact(_)));
}

#[test]
fn parse_post_compact() {
    let v = mk_json_with(
        "PostCompact",
        serde_json::json!({"trigger": "manual", "compact_summary": "summary"}),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::PostCompact(_)));
}

#[test]
fn parse_instructions_loaded() {
    let v = mk_json_with(
        "InstructionsLoaded",
        serde_json::json!({
            "file_path": "/tmp/CLAUDE.md",
            "memory_type": "Project",
            "load_reason": "session_start"
        }),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::InstructionsLoaded(_)));
}

#[test]
fn parse_worktree_create() {
    let v = mk_json_with("WorktreeCreate", serde_json::json!({"name": "feature-x"}));
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
        serde_json::json!({
            "mcp_server_name": "my-mcp-server",
            "message": "Which option?",
            "mode": "form",
            "requested_schema": {"type": "object"}
        }),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::Elicitation(_)));
    if let ClaudeHookInput::Elicitation(ev) = input {
        assert_eq!(ev.message.as_deref(), Some("Which option?"));
        assert_eq!(ev.mode.as_deref(), Some("form"));
    }
}

#[test]
fn parse_elicitation_result() {
    let v = mk_json_with(
        "ElicitationResult",
        serde_json::json!({
            "mcp_server_name": "my-mcp-server",
            "action": "accept",
            "content": {"choice": "a"}
        }),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::ElicitationResult(_)));
}

#[test]
fn parse_stop_failure() {
    let v = mk_json_with(
        "StopFailure",
        serde_json::json!({"error": "rate_limit", "error_details": {"message": "timed out"}}),
    );
    let input = input::parse(&v).expect("should parse");
    assert!(matches!(input, ClaudeHookInput::StopFailure(_)));
}

// ---- Phase 4 output tests ----

#[test]
fn output_permission_approve() {
    let out = OutputEnvelope::permission_approve();
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(
        json["hookSpecificOutput"]["hookEventName"],
        "PermissionRequest"
    );
    assert_eq!(json["hookSpecificOutput"]["decision"]["behavior"], "allow");
}

#[test]
fn output_permission_deny() {
    let out = OutputEnvelope::permission_deny("too dangerous");
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["hookSpecificOutput"]["decision"]["behavior"], "deny");
    assert_eq!(
        json["hookSpecificOutput"]["decision"]["message"],
        "too dangerous"
    );
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
    assert_eq!(json["hookSpecificOutput"]["hookEventName"], "SessionStart");
    assert_eq!(
        json["hookSpecificOutput"]["additionalContext"],
        "session prep context"
    );
    assert_eq!(json["systemMessage"], "custom system");
    assert_eq!(json["suppressOutput"], true);
}

#[test]
fn output_prompt_submit_event_scoped() {
    let allow: OutputEnvelope = ClaudePromptSubmitOutput::allow()
        .with_context("team policy")
        .with_session_title("Payments review")
        .into();
    let allow_json = serde_json::to_value(&allow).unwrap();
    assert_eq!(allow_json["decision"], serde_json::Value::Null);
    assert_eq!(
        allow_json["hookSpecificOutput"]["hookEventName"],
        "UserPromptSubmit"
    );
    assert_eq!(
        allow_json["hookSpecificOutput"]["sessionTitle"],
        "Payments review"
    );

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
        allow_json["hookSpecificOutput"]["permissionDecision"],
        "allow"
    );

    let deny: OutputEnvelope = ClaudePreToolUseOutput::deny("dangerous").into();
    let deny_json = serde_json::to_value(&deny).unwrap();
    assert_eq!(
        deny_json["hookSpecificOutput"]["permissionDecision"],
        "deny"
    );
    assert_eq!(
        deny_json["hookSpecificOutput"]["permissionDecisionReason"],
        "dangerous"
    );

    let ask: OutputEnvelope = ClaudePreToolUseOutput::ask("need approval").into();
    let ask_json = serde_json::to_value(&ask).unwrap();
    assert_eq!(ask_json["hookSpecificOutput"]["permissionDecision"], "ask");
    assert_eq!(
        ask_json["hookSpecificOutput"]["permissionDecisionReason"],
        "need approval"
    );
}

#[test]
fn output_prompt_expansion_event_scoped() {
    let out: OutputEnvelope = ClaudePromptExpansionOutput::block("disabled")
        .with_context("use /review instead")
        .into();
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["decision"], "block");
    assert_eq!(
        json["hookSpecificOutput"]["hookEventName"],
        "UserPromptExpansion"
    );
}

#[test]
fn output_post_tool_use_event_scoped() {
    let out: OutputEnvelope = ClaudePostToolUseOutput::new()
        .with_context("lint completed")
        .into();
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["hookSpecificOutput"]["hookEventName"], "PostToolUse");
    assert_eq!(
        json["hookSpecificOutput"]["additionalContext"],
        "lint completed"
    );
}

#[test]
fn output_post_tool_use_failure_event_scoped() {
    let out: OutputEnvelope = ClaudePostToolUseFailureOutput::new()
        .with_context("retry with smaller input")
        .into();
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(
        json["hookSpecificOutput"]["hookEventName"],
        "PostToolUseFailure"
    );
    assert_eq!(
        json["hookSpecificOutput"]["additionalContext"],
        "retry with smaller input"
    );
}

#[test]
fn output_post_tool_batch_event_scoped() {
    let out: OutputEnvelope = ClaudePostToolBatchOutput::new()
        .with_context("these reads came from the ledger module")
        .into();
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["hookSpecificOutput"]["hookEventName"], "PostToolBatch");
}

#[test]
fn output_permission_request_event_scoped() {
    let out: OutputEnvelope = ClaudePermissionRequestOutput::allow()
        .with_updated_input(serde_json::json!({"command": "npm run lint"}))
        .into();
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["hookSpecificOutput"]["decision"]["behavior"], "allow");
    assert_eq!(
        json["hookSpecificOutput"]["decision"]["updatedInput"]["command"],
        "npm run lint"
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
        ClaudeWatchPathsOutput::new(vec!["/tmp/a".to_string(), "/tmp/b".to_string()]).into();
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

#[test]
fn output_elicitation_event_scoped() {
    let out: OutputEnvelope = ClaudeElicitationOutput::accept(serde_json::json!({
        "username": "alice"
    }))
    .into();
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(json["hookSpecificOutput"]["hookEventName"], "Elicitation");
    assert_eq!(json["hookSpecificOutput"]["action"], "accept");
}

#[test]
fn output_elicitation_result_event_scoped() {
    let out: OutputEnvelope = ClaudeElicitationResultOutput::decline().into();
    let json = serde_json::to_value(&out).unwrap();
    assert_eq!(
        json["hookSpecificOutput"]["hookEventName"],
        "ElicitationResult"
    );
    assert_eq!(json["hookSpecificOutput"]["action"], "decline");
}
