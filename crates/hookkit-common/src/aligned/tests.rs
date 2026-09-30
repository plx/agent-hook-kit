//! Unit tests for the aligned accessors and the exact native lowering of the
//! portable output helpers.

use super::*;
use hookkit_core::{CommandEnvironmentSpec, EnvironmentVariables, ProcessEmission, RawInvocation};
use serde_json::{Value, json};

const HARNESSES: [HarnessId; 3] = [
    HarnessId::CLAUDE_CODE,
    HarnessId::CODEX,
    HarnessId::ANTIGRAVITY,
];

fn parse<E: EventSpec>(value: Value) -> E::Input {
    E::parse(&RawInvocation::parse(serde_json::to_vec(&value).unwrap()).unwrap()).unwrap()
}

fn stdout_json(emission: &ProcessEmission) -> Value {
    serde_json::from_slice(emission.stdout()).unwrap()
}

fn emit_pre_tool(output: PreToolUseOutput) -> ProcessEmission {
    match output {
        PreToolUseOutput::Claude(output) => hookkit_claude::catalog::PreToolUse::emit(output),
        PreToolUseOutput::Codex(output) => hookkit_codex::protocol::PreToolUse::emit(output),
        PreToolUseOutput::Antigravity(output) => hookkit_antigravity::PreToolUse::emit(output),
    }
    .unwrap()
}

fn claude_environment(
    event: &EventId,
    project_dir: &str,
) -> hookkit_claude::ClaudeCommandEnvironment {
    hookkit_claude::ClaudeCommandEnvironment::from_variables(
        event,
        &EnvironmentVariables::from_pairs([
            ("CLAUDECODE", "1"),
            ("CLAUDE_CODE_CHILD_SESSION", "1"),
            ("CLAUDE_CODE_SESSION_ID", "s"),
            ("CLAUDE_PROJECT_DIR", project_dir),
        ]),
    )
    .unwrap()
}

fn codex_environment(event: &EventId) -> hookkit_codex::CodexCommandEnvironment {
    hookkit_codex::CodexCommandEnvironment::from_variables(event, &EnvironmentVariables::new())
        .unwrap()
}

fn antigravity_environment(event: &EventId) -> hookkit_antigravity::AntigravityCommandEnvironment {
    hookkit_antigravity::AntigravityCommandEnvironment::from_variables(
        event,
        &EnvironmentVariables::new(),
    )
    .unwrap()
}

fn claude_pre_tool(cwd: &str) -> PreToolUseInput {
    PreToolUseInput::Claude(parse::<hookkit_claude::catalog::PreToolUse>(json!({
        "session_id": "s",
        "transcript_path": "/tmp/t.jsonl",
        "cwd": cwd,
        "hook_event_name": "PreToolUse",
        "permission_mode": "default",
        "tool_name": "Read",
        "tool_input": {"file_path": "/repo/.env"},
        "tool_use_id": "toolu_1"
    })))
}

fn codex_pre_tool() -> PreToolUseInput {
    PreToolUseInput::Codex(parse::<hookkit_codex::protocol::PreToolUse>(json!({
        "session_id": "s",
        "transcript_path": null,
        "cwd": "/repo",
        "hook_event_name": "PreToolUse",
        "model": "gpt-test",
        "turn_id": "t",
        "permission_mode": "default",
        "tool_name": "apply_patch",
        "tool_use_id": "call_1",
        "tool_input": {"command": "*** Begin Patch\n*** End Patch\n"}
    })))
}

fn antigravity_pre_tool() -> PreToolUseInput {
    PreToolUseInput::Antigravity(parse::<hookkit_antigravity::PreToolUse>(json!({
        "conversationId": "c",
        "workspacePaths": ["/repo", "/lib"],
        "transcriptPath": "/tmp/t.jsonl",
        "artifactDirectoryPath": "/tmp/a",
        "toolCall": {"name": "view_file", "args": {"AbsolutePath": "/repo/.env"}},
        "stepIdx": 3
    })))
}

#[test]
fn pass_through_defers_to_each_harness_normal_permission_flow() {
    let claude = emit_pre_tool(PreToolUseOutput::pass_through(&HarnessId::CLAUDE_CODE).unwrap());
    assert_eq!(stdout_json(&claude), json!({}));

    let codex = emit_pre_tool(PreToolUseOutput::pass_through(&HarnessId::CODEX).unwrap());
    assert!(codex.stdout().is_empty());

    // Antigravity requires a decision; "ask" respects Always-Allow grants.
    let antigravity =
        emit_pre_tool(PreToolUseOutput::pass_through(&HarnessId::ANTIGRAVITY).unwrap());
    assert_eq!(stdout_json(&antigravity), json!({"decision": "ask"}));

    for harness in HARNESSES {
        let emission = emit_pre_tool(PreToolUseOutput::pass_through(&harness).unwrap());
        assert_eq!(emission.exit_code(), 0);
        assert!(emission.stderr().is_empty());
        let stdout = String::from_utf8_lossy(emission.stdout());
        assert!(!stdout.contains("allow"), "{harness}: {stdout}");
    }
}

#[test]
fn allow_is_an_explicit_approval_that_codex_cannot_express() {
    let claude = emit_pre_tool(PreToolUseOutput::allow(&HarnessId::CLAUDE_CODE).unwrap());
    assert_eq!(
        stdout_json(&claude),
        json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "allow"}})
    );
    let antigravity = emit_pre_tool(PreToolUseOutput::allow(&HarnessId::ANTIGRAVITY).unwrap());
    assert_eq!(stdout_json(&antigravity), json!({"decision": "allow"}));
    let codex = emit_pre_tool(PreToolUseOutput::allow(&HarnessId::CODEX).unwrap());
    assert!(
        codex.stdout().is_empty(),
        "Codex lowers allow to no decision"
    );

    assert!(PreToolUseOutput::explicit_allow_supported(
        &HarnessId::CLAUDE_CODE
    ));
    assert!(PreToolUseOutput::explicit_allow_supported(
        &HarnessId::ANTIGRAVITY
    ));
    assert!(!PreToolUseOutput::explicit_allow_supported(
        &HarnessId::CODEX
    ));
    assert!(!PreToolUseOutput::explicit_allow_supported(
        &HarnessId::new("claude").unwrap()
    ));
}

#[test]
fn deny_uses_each_exact_native_shape() {
    let claude = emit_pre_tool(PreToolUseOutput::deny(&HarnessId::CLAUDE_CODE, "no").unwrap());
    assert_eq!(
        stdout_json(&claude),
        json!({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": "no"
        }})
    );
    let codex = emit_pre_tool(PreToolUseOutput::deny(&HarnessId::CODEX, "no").unwrap());
    assert_eq!(
        stdout_json(&codex),
        json!({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": "no"
        }})
    );
    let antigravity = emit_pre_tool(PreToolUseOutput::deny(&HarnessId::ANTIGRAVITY, "no").unwrap());
    assert_eq!(
        stdout_json(&antigravity),
        json!({"decision": "deny", "reason": "no"})
    );
}

#[test]
fn every_deny_and_block_helper_rejects_blank_reasons_on_every_harness() {
    for reason in ["", "  ", "\t\n"] {
        for harness in HARNESSES {
            assert!(
                matches!(
                    PreToolUseOutput::deny(&harness, reason),
                    Err(HookkitError::InvalidProcessEmission(_))
                ),
                "{harness} {reason:?}"
            );
        }
        for harness in [HarnessId::CLAUDE_CODE, HarnessId::CODEX] {
            assert!(UserPromptSubmitOutput::block(&harness, reason).is_err());
            assert!(SubagentStopOutput::block(&harness, reason).is_err());
            assert!(PermissionRequestOutput::deny(&harness, reason).is_err());
        }
    }
}

#[test]
fn rewrite_lets_the_caller_choose_the_claude_decision() {
    let updated = json!({"command": "timeout 60 cargo test"})
        .as_object()
        .cloned()
        .unwrap();

    let approved = emit_pre_tool(
        PreToolUseOutput::rewrite(
            &HarnessId::CLAUDE_CODE,
            updated.clone(),
            RewriteApproval::AutoApprove,
        )
        .unwrap(),
    );
    assert_eq!(
        stdout_json(&approved)["hookSpecificOutput"],
        json!({
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "updatedInput": {"command": "timeout 60 cargo test"}
        })
    );

    let asked = emit_pre_tool(
        PreToolUseOutput::rewrite(
            &HarnessId::CLAUDE_CODE,
            updated.clone(),
            RewriteApproval::Ask("added a timeout".into()),
        )
        .unwrap(),
    );
    assert_eq!(
        stdout_json(&asked)["hookSpecificOutput"],
        json!({
            "hookEventName": "PreToolUse",
            "permissionDecision": "ask",
            "permissionDecisionReason": "added a timeout",
            "updatedInput": {"command": "timeout 60 cargo test"}
        })
    );

    // Codex treats allow plus updatedInput as a replacement only, whichever
    // approval the caller selected for Claude Code.
    for approval in [
        RewriteApproval::AutoApprove,
        RewriteApproval::Ask("ignored".into()),
    ] {
        let codex = emit_pre_tool(
            PreToolUseOutput::rewrite(&HarnessId::CODEX, updated.clone(), approval).unwrap(),
        );
        assert_eq!(
            stdout_json(&codex)["hookSpecificOutput"],
            json!({
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow",
                "updatedInput": {"command": "timeout 60 cargo test"}
            })
        );
    }

    assert!(matches!(
        PreToolUseOutput::rewrite(
            &HarnessId::ANTIGRAVITY,
            updated,
            RewriteApproval::AutoApprove
        ),
        Err(HookkitError::UnsupportedHarness { .. })
    ));
}

#[test]
fn unknown_harnesses_and_aliases_are_unsupported_not_unrecognized_events() {
    let alias = HarnessId::new("claude").unwrap();
    for result in [
        PreToolUseOutput::pass_through(&alias).map(drop),
        PreToolUseOutput::allow(&alias).map(drop),
        PreToolUseOutput::deny(&alias, "no").map(drop),
        PostToolUseOutput::no_op(&alias).map(drop),
        TurnCompletionOutput::allow(&alias).map(drop),
        SessionStartOutput::no_op(&alias).map(drop),
        UserPromptSubmitOutput::with_context(&alias, "c").map(drop),
    ] {
        assert!(matches!(
            result,
            Err(HookkitError::UnsupportedHarness { .. })
        ));
    }

    for result in [
        PermissionRequestOutput::no_op(&HarnessId::ANTIGRAVITY).map(drop),
        PreCompactOutput::no_op(&HarnessId::ANTIGRAVITY).map(drop),
        PostCompactOutput::no_op(&HarnessId::ANTIGRAVITY).map(drop),
        SessionEndOutput::no_op(&HarnessId::ANTIGRAVITY).map(drop),
        SubagentStartOutput::no_op(&HarnessId::ANTIGRAVITY).map(drop),
        SubagentStopOutput::no_op(&HarnessId::ANTIGRAVITY).map(drop),
    ] {
        let Err(HookkitError::UnsupportedHarness { harness, message }) = result else {
            panic!("pair families must reject Antigravity as unsupported");
        };
        assert_eq!(harness, HarnessId::ANTIGRAVITY);
        assert!(message.contains("claude-code and codex only"), "{message}");
    }
}

#[test]
fn user_prompt_context_uses_the_structured_channel_on_both_harnesses() {
    // Codex parses exit-0 stdout that starts with `[` or `{` as JSON, so
    // plain-text context like this would be dropped as a malformed response.
    let context = "[policy] use pnpm, not npm";
    let codex = match UserPromptSubmitOutput::with_context(&HarnessId::CODEX, context).unwrap() {
        UserPromptSubmitOutput::Codex(output) => {
            hookkit_codex::catalog::UserPromptSubmit::emit(output).unwrap()
        }
        other => panic!("unexpected arm {other:?}"),
    };
    let claude =
        match UserPromptSubmitOutput::with_context(&HarnessId::CLAUDE_CODE, context).unwrap() {
            UserPromptSubmitOutput::Claude(output) => {
                hookkit_claude::catalog::UserPromptSubmit::emit(output).unwrap()
            }
            other => panic!("unexpected arm {other:?}"),
        };
    for emission in [codex, claude] {
        assert_eq!(emission.exit_code(), 0);
        assert_eq!(
            stdout_json(&emission),
            json!({"hookSpecificOutput": {
                "hookEventName": "UserPromptSubmit",
                "additionalContext": context
            }})
        );
    }
}

#[test]
fn post_compact_notice_is_not_claimed_where_claude_discards_it() {
    let claude =
        match PostCompactOutput::with_system_notice(&HarnessId::CLAUDE_CODE, "done").unwrap() {
            PostCompactOutput::Claude(output) => {
                hookkit_claude::catalog::PostCompact::emit(output).unwrap()
            }
            other => panic!("unexpected arm {other:?}"),
        };
    assert_eq!(stdout_json(&claude), json!({}));
    assert!(!PostCompactOutput::system_notice_delivered(
        &HarnessId::CLAUDE_CODE
    ));

    let codex = match PostCompactOutput::with_system_notice(&HarnessId::CODEX, "done").unwrap() {
        PostCompactOutput::Codex(output) => {
            hookkit_codex::catalog::PostCompact::emit(output).unwrap()
        }
        other => panic!("unexpected arm {other:?}"),
    };
    assert_eq!(stdout_json(&codex), json!({"systemMessage": "done"}));
    assert!(PostCompactOutput::system_notice_delivered(
        &HarnessId::CODEX
    ));
    assert!(!PostCompactOutput::system_notice_delivered(
        &HarnessId::ANTIGRAVITY
    ));
}

#[test]
fn claude_project_roots_come_from_the_environment_not_the_moving_cwd() {
    let event = hookkit_claude::catalog::PreToolUse::EVENT;
    // After `cd src`, Claude reports the agent's new directory as `cwd`.
    let input = claude_pre_tool("/repo/src");
    let environment = PreToolUseCommandEnvironment::Claude(claude_environment(&event, "/repo"));

    assert_eq!(input.cwd(), Some(Utf8Path::new("/repo/src")));
    assert_eq!(
        input.workspace_roots().as_ref(),
        [Utf8PathBuf::from("/repo/src")]
    );
    assert!(matches!(input.workspace_roots(), Cow::Borrowed(_)));
    assert_eq!(
        input.project_roots(&environment).as_ref(),
        [Utf8PathBuf::from("/repo")]
    );
    assert_eq!(environment.project_dir(), Some(Utf8Path::new("/repo")));

    let codex = codex_pre_tool();
    let codex_environment = PreToolUseCommandEnvironment::Codex(codex_environment(
        &hookkit_codex::protocol::PreToolUse::EVENT,
    ));
    assert_eq!(codex_environment.project_dir(), None);
    assert_eq!(
        codex.project_roots(&codex_environment).as_ref(),
        [Utf8PathBuf::from("/repo")]
    );

    let antigravity = antigravity_pre_tool();
    let antigravity_environment = PreToolUseCommandEnvironment::Antigravity(
        antigravity_environment(&hookkit_antigravity::PreToolUse::EVENT),
    );
    assert_eq!(antigravity.cwd(), None);
    assert_eq!(
        antigravity.project_roots(&antigravity_environment).as_ref(),
        [Utf8PathBuf::from("/repo"), Utf8PathBuf::from("/lib")]
    );
    // A mismatched environment arm falls back to the input's own roots.
    assert_eq!(
        input.project_roots(&codex_environment).as_ref(),
        [Utf8PathBuf::from("/repo/src")]
    );
}

#[test]
fn pre_tool_accessors_borrow_each_native_arm() {
    let claude = claude_pre_tool("/repo");
    assert_eq!(claude.tool_name(), Some("Read"));
    assert_eq!(claude.tool_call_id(), Some("toolu_1"));
    assert_eq!(
        claude.tool_input().and_then(|input| input.get("file_path")),
        Some(&json!("/repo/.env"))
    );

    let codex = codex_pre_tool();
    assert_eq!(codex.tool_name(), Some("apply_patch"));
    assert_eq!(codex.tool_call_id(), Some("call_1"));

    let antigravity = antigravity_pre_tool();
    assert_eq!(antigravity.tool_name(), Some("view_file"));
    assert_eq!(antigravity.tool_call_id(), None);
    assert!(matches!(
        antigravity.tool_input(),
        Some(ToolInputRef::Object(_))
    ));
}

#[test]
fn post_tool_accessors_cover_every_arm_including_antigravity_without_a_tool_call() {
    let claude = PostToolUseInput::Claude(parse::<hookkit_claude::protocol::PostToolUse>(json!({
        "session_id": "s",
        "transcript_path": "/tmp/t.jsonl",
        "cwd": "/repo/src",
        "hook_event_name": "PostToolUse",
        "tool_name": "Edit",
        "tool_input": {"file_path": "/repo/src/lib.rs"},
        "tool_use_id": "toolu_2",
        "tool_response": {}
    })));
    let environment = PostToolUseCommandEnvironment::Claude(claude_environment(
        &hookkit_claude::protocol::PostToolUse::EVENT,
        "/repo",
    ));
    assert_eq!(claude.tool_name(), Some("Edit"));
    assert_eq!(claude.tool_call_id(), Some("toolu_2"));
    assert_eq!(claude.cwd(), Some(Utf8Path::new("/repo/src")));
    assert_eq!(
        claude.project_roots(&environment).as_ref(),
        [Utf8PathBuf::from("/repo")]
    );
    assert_eq!(
        claude.tool_input().and_then(|input| input.get("file_path")),
        Some(&json!("/repo/src/lib.rs"))
    );

    let codex = PostToolUseInput::Codex(parse::<hookkit_codex::protocol::PostToolUse>(json!({
        "session_id": "s",
        "transcript_path": null,
        "cwd": "/repo",
        "hook_event_name": "PostToolUse",
        "model": "gpt-test",
        "turn_id": "t",
        "permission_mode": "default",
        "tool_name": "apply_patch",
        "tool_use_id": "call_2",
        "tool_input": {"command": "*** Begin Patch\n*** End Patch\n"},
        "tool_response": {}
    })));
    assert_eq!(codex.tool_name(), Some("apply_patch"));
    assert_eq!(codex.tool_call_id(), Some("call_2"));
    assert_eq!(codex.cwd(), Some(Utf8Path::new("/repo")));

    let antigravity =
        PostToolUseInput::Antigravity(parse::<hookkit_antigravity::PostToolUse>(json!({
            "conversationId": "c",
            "workspacePaths": ["/repo"],
            "transcriptPath": "/tmp/t.jsonl",
            "artifactDirectoryPath": "/tmp/a",
            "stepIdx": 4
        })));
    assert_eq!(antigravity.tool_name(), None);
    assert!(antigravity.tool_input().is_none());
    assert_eq!(antigravity.tool_call_id(), None);
    assert_eq!(antigravity.cwd(), None);
    assert_eq!(
        antigravity.workspace_roots().as_ref(),
        [Utf8PathBuf::from("/repo")]
    );
}

#[test]
fn turn_completion_exposes_each_harness_loop_and_termination_signals() {
    let claude = TurnCompletionInput::Claude(parse::<hookkit_claude::catalog::Stop>(json!({
        "session_id": "s",
        "transcript_path": "/tmp/t.jsonl",
        "cwd": "/repo",
        "hook_event_name": "Stop",
        "stop_hook_active": true,
        "last_assistant_message": "done"
    })));
    assert_eq!(claude.stop_hook_active(), Some(true));
    assert_eq!(claude.last_assistant_message(), Some("done"));
    assert_eq!(claude.termination_reason(), None);
    assert_eq!(claude.fully_idle(), None);

    let codex = TurnCompletionInput::Codex(parse::<hookkit_codex::catalog::Stop>(json!({
        "session_id": "s",
        "transcript_path": null,
        "cwd": "/repo",
        "hook_event_name": "Stop",
        "model": "gpt-test",
        "turn_id": "t",
        "permission_mode": "default",
        "stop_hook_active": false,
        "last_assistant_message": null
    })));
    assert_eq!(codex.stop_hook_active(), Some(false));
    assert_eq!(codex.last_assistant_message(), None);

    let antigravity = TurnCompletionInput::Antigravity(parse::<hookkit_antigravity::Stop>(json!({
        "conversationId": "c",
        "workspacePaths": ["/repo"],
        "transcriptPath": "/tmp/t.jsonl",
        "artifactDirectoryPath": "/tmp/a",
        "executionNum": 2,
        "terminationReason": "max_steps_exceeded",
        "fullyIdle": false
    })));
    assert_eq!(antigravity.stop_hook_active(), None);
    assert_eq!(antigravity.termination_reason(), Some("max_steps_exceeded"));
    assert_eq!(antigravity.fully_idle(), Some(false));
    assert_eq!(antigravity.cwd(), None);
}

#[test]
fn pair_accessors_read_native_fields() {
    let claude =
        SessionStartInput::Claude(parse::<hookkit_claude::protocol::SessionStart>(json!({
            "session_id": "s",
            "transcript_path": "/tmp/t.jsonl",
            "cwd": "/repo/src",
            "hook_event_name": "SessionStart",
            "source": "resume"
        })));
    assert_eq!(claude.source(), Some("resume"));
    let environment = SessionStartCommandEnvironment::Claude(claude_environment(
        &hookkit_claude::protocol::SessionStart::EVENT,
        "/repo",
    ));
    assert_eq!(
        claude.project_roots(&environment).as_ref(),
        [Utf8PathBuf::from("/repo")]
    );
    assert!(matches!(claude.workspace_roots(), Cow::Borrowed(_)));

    let codex = SessionStartInput::Codex(parse::<hookkit_codex::catalog::SessionStart>(json!({
        "session_id": "s",
        "transcript_path": null,
        "cwd": "/repo",
        "hook_event_name": "SessionStart",
        "model": "gpt-test",
        "permission_mode": "default",
        "source": "clear"
    })));
    assert_eq!(codex.source(), Some("clear"));

    let permission =
        PermissionRequestInput::Codex(parse::<hookkit_codex::catalog::PermissionRequest>(json!({
            "session_id": "s",
            "transcript_path": null,
            "cwd": "/repo",
            "hook_event_name": "PermissionRequest",
            "model": "gpt-test",
            "turn_id": "t",
            "permission_mode": "default",
            "tool_name": "Bash",
            "tool_input": {"command": "true"}
        })));
    assert_eq!(permission.tool_name(), Some("Bash"));
    assert_eq!(
        permission
            .tool_input()
            .and_then(|input| input.get("command")),
        Some(&json!("true"))
    );

    let subagent =
        SubagentStopInput::Claude(parse::<hookkit_claude::catalog::SubagentStop>(json!({
            "session_id": "s",
            "transcript_path": "/tmp/t.jsonl",
            "cwd": "/repo",
            "hook_event_name": "SubagentStop",
            "stop_hook_active": true,
            "agent_id": "a",
            "agent_type": "Explore",
            "agent_transcript_path": "/tmp/a.jsonl",
            "last_assistant_message": "done"
        })));
    assert_eq!(subagent.stop_hook_active(), Some(true));
    assert_eq!(subagent.agent_type(), Some("Explore"));
}
