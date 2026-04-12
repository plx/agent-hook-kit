use crate::input::*;
use crate::message::*;
use crate::output::*;
use hookkit_claude::input as claude;
use hookkit_codex::input as codex;
use hookkit_gemini::input as gemini;

// ---- Shared accessor tests ----

#[test]
fn post_tool_use_shared_accessors() {
    let claude_ev = claude::PostToolUse {
        common: claude::CommonFields {
            session_id: "sess-1".to_string(),
            transcript_path: None,
            cwd: "/home/user".to_string(),
            hook_event_name: "PostToolUse".to_string(),
            extra: Default::default(),
        },
        tool_name: Some("Write".to_string()),
        tool_input: Some(serde_json::json!({"file_path": "/tmp/test.rs"})),
        tool_result: Some(serde_json::json!({"success": true})),
    };

    let common = CommonPostToolUseInput::Claude(claude_ev);
    assert_eq!(common.session_id(), "sess-1");
    assert_eq!(common.cwd(), "/home/user");
    assert_eq!(common.tool_name(), Some("Write"));
    assert!(common.raw_tool_input().is_some());
    assert!(common.as_claude().is_some());
    assert!(common.as_codex().is_none());
    assert!(common.as_gemini().is_none());
}

#[test]
fn pre_tool_use_shared_accessors() {
    let codex_ev = codex::PreToolUse {
        common: codex::CommonFields {
            session_id: "codex-1".to_string(),
            cwd: "/tmp".to_string(),
            hook_event_name: "PreToolUse".to_string(),
            extra: Default::default(),
        },
        tool_name: Some("Bash".to_string()),
        tool_input: Some(serde_json::json!({"command": "ls"})),
    };

    let common = CommonPreToolUseInput::Codex(codex_ev);
    assert_eq!(common.tool_name(), Some("Bash"));
    assert!(common.as_codex().is_some());
}

#[test]
fn stop_last_assistant_message() {
    let gemini_ev = gemini::AfterAgent {
        common: gemini::CommonFields {
            session_id: "gem-1".to_string(),
            cwd: "/tmp".to_string(),
            hook_event_name: "AfterAgent".to_string(),
            extra: Default::default(),
        },
        agent_response: Some("All done.".to_string()),
    };

    let common = CommonStopInput::Gemini(gemini_ev);
    assert_eq!(common.last_assistant_message(), Some("All done."));
}

#[test]
fn prompt_submit_shared_accessors() {
    let ev = claude::UserPromptSubmit {
        common: claude::CommonFields {
            session_id: "s1".to_string(),
            transcript_path: None,
            cwd: "/home".to_string(),
            hook_event_name: "UserPromptSubmit".to_string(),
            extra: Default::default(),
        },
        user_prompt: Some("fix the bug".to_string()),
    };

    let common = CommonPromptSubmitInput::Claude(ev);
    assert_eq!(common.user_prompt(), Some("fix the bug"));
}

// ---- From conversion tests ----

#[test]
fn from_claude_session_start() {
    let ev = claude::SessionStart {
        common: claude::CommonFields {
            session_id: "s1".to_string(),
            transcript_path: None,
            cwd: "/tmp".to_string(),
            hook_event_name: "SessionStart".to_string(),
            extra: Default::default(),
        },
    };
    let common: CommonHookInput = ev.into();
    assert!(matches!(common, CommonHookInput::SessionStart(_)));
}

#[test]
fn from_gemini_before_agent() {
    let ev = gemini::BeforeAgent {
        common: gemini::CommonFields {
            session_id: "g1".to_string(),
            cwd: "/tmp".to_string(),
            hook_event_name: "BeforeAgent".to_string(),
            extra: Default::default(),
        },
        user_prompt: Some("hello".to_string()),
    };
    let common: CommonHookInput = ev.into();
    assert!(matches!(common, CommonHookInput::PromptSubmit(_)));
}

#[test]
fn from_claude_notification() {
    let ev = claude::Notification {
        common: claude::CommonFields {
            session_id: "s1".to_string(),
            transcript_path: None,
            cwd: "/tmp".to_string(),
            hook_event_name: "Notification".to_string(),
            extra: Default::default(),
        },
        message: Some("build done".to_string()),
        level: Some("info".to_string()),
    };
    let common: CommonHookInput = ev.into();
    assert!(matches!(common, CommonHookInput::Notification(_)));
}

#[test]
fn from_gemini_session_end() {
    let ev = gemini::SessionEnd {
        common: gemini::CommonFields {
            session_id: "g1".to_string(),
            cwd: "/tmp".to_string(),
            hook_event_name: "SessionEnd".to_string(),
            extra: Default::default(),
        },
    };
    let common: CommonHookInput = ev.into();
    assert!(matches!(common, CommonHookInput::SessionEnd(_)));
}

#[test]
fn from_pre_compress_variants() {
    let c = claude::PreCompact {
        common: claude::CommonFields {
            session_id: "c1".to_string(),
            transcript_path: None,
            cwd: "/tmp".to_string(),
            hook_event_name: "PreCompact".to_string(),
            extra: Default::default(),
        },
    };
    let g = gemini::PreCompress {
        common: gemini::CommonFields {
            session_id: "g1".to_string(),
            cwd: "/tmp".to_string(),
            hook_event_name: "PreCompress".to_string(),
            extra: Default::default(),
        },
    };

    let cc: CommonHookInput = c.into();
    let gg: CommonHookInput = g.into();
    assert!(matches!(cc, CommonHookInput::PreCompress(_)));
    assert!(matches!(gg, CommonHookInput::PreCompress(_)));
}

// ---- Output conversion tests ----

#[test]
fn post_tool_output_to_claude_with_context() {
    let out = CommonPostToolUseOutput::new()
        .with_agent_context("formatted file.rs")
        .with_agent_context("no lint errors");
    let claude = out.to_claude().unwrap();
    assert!(matches!(claude, hookkit_claude::ClaudeHookOutput::Json(_)));
}

#[test]
fn post_tool_output_to_codex_with_context_fails() {
    let out = CommonPostToolUseOutput::new().with_agent_context("some context");
    let result = out.to_codex();
    assert!(result.is_err());
}

#[test]
fn post_tool_output_empty() {
    let out = CommonPostToolUseOutput::new();
    let claude = out.to_claude().unwrap();
    assert!(matches!(claude, hookkit_claude::ClaudeHookOutput::Empty));
    let codex = out.to_codex().unwrap();
    assert!(matches!(codex, hookkit_codex::CodexHookOutput::Empty));
    let gemini = out.to_gemini().unwrap();
    assert!(matches!(gemini, hookkit_gemini::GeminiHookOutput::Empty));
}

#[test]
fn pre_tool_deny_to_all_harnesses() {
    let out = CommonPreToolUseOutput::deny("blocked");

    // Claude: JSON with hookSpecificOutput.permissionDecision
    let claude = out.to_claude();
    assert!(matches!(claude, hookkit_claude::ClaudeHookOutput::Json(_)));

    // Codex: stderr block
    let codex = out.to_codex().unwrap();
    assert!(matches!(
        codex,
        hookkit_codex::CodexHookOutput::BlockingDeny { .. }
    ));

    // Gemini: JSON deny
    let gemini = out.to_gemini();
    assert!(matches!(gemini, hookkit_gemini::GeminiHookOutput::Json(_)));
}

#[test]
fn pre_tool_allow_codex_unsupported() {
    let out = CommonPreToolUseOutput::allow();
    let result = out.to_codex();
    assert!(result.is_err());
}

#[test]
fn stop_continue_to_all_harnesses() {
    let out = CommonStopOutput::continue_session("not done");
    let claude = out.to_claude();
    assert!(matches!(claude, hookkit_claude::ClaudeHookOutput::Json(_)));
    let codex = out.to_codex();
    assert!(matches!(codex, hookkit_codex::CodexHookOutput::Json(_)));
    let gemini = out.to_gemini();
    assert!(matches!(gemini, hookkit_gemini::GeminiHookOutput::Json(_)));
}

#[test]
fn prompt_block_to_all_harnesses() {
    let out = CommonPromptSubmitOutput::block("banned prompt");
    let claude = out.to_claude();
    assert!(matches!(claude, hookkit_claude::ClaudeHookOutput::Json(_)));
    let codex = out.to_codex();
    assert!(matches!(
        codex,
        hookkit_codex::CodexHookOutput::BlockingDeny { .. }
    ));
    let gemini = out.to_gemini();
    assert!(matches!(gemini, hookkit_gemini::GeminiHookOutput::Json(_)));
}

#[test]
fn post_tool_user_notice_is_not_silently_dropped() {
    let out = CommonPostToolUseOutput::new().with_notice(UserNotice::info("done"));
    assert!(out.to_claude().is_err());
    assert!(out.to_codex().is_err());
    assert!(out.to_gemini().is_err());
}

#[test]
fn session_start_output_supported_and_unsupported_paths() {
    let with_context = CommonSessionStartOutput::new().with_agent_context("repo has rustfmt");
    assert!(with_context.to_claude().is_ok());
    assert!(with_context.to_codex().is_err());
    assert!(with_context.to_gemini().is_err());
}

#[test]
fn notification_and_session_end_outputs_exist_and_error_explicitly() {
    let note = CommonNotificationOutput::new().with_notice(UserNotice::warning("heads up"));
    assert!(note.to_claude().is_err());
    assert!(note.to_gemini().is_err());

    let end = CommonSessionEndOutput::new().with_notice(UserNotice::info("bye"));
    assert!(end.to_claude().is_err());
    assert!(end.to_gemini().is_err());
}

// ---- Message helper tests ----

#[test]
fn user_notice_levels() {
    let info = UserNotice::info("all good");
    assert_eq!(info.level, NoticeLevel::Info);
    let warn = UserNotice::warning("watch out");
    assert_eq!(warn.level, NoticeLevel::Warning);
    let err = UserNotice::error("failed");
    assert_eq!(err.level, NoticeLevel::Error);
}

#[test]
fn agent_context_builder() {
    let ctx = crate::message::AgentContext::new()
        .push("line 1")
        .push("line 2");
    assert_eq!(ctx.to_string_block(), "line 1\nline 2");
}

#[test]
fn agent_feedback() {
    let fb = crate::message::AgentFeedback::new("please fix the imports");
    assert_eq!(fb.message, "please fix the imports");
}

#[test]
fn tail_tool_call() {
    let ttc = crate::message::TailToolCall::new("Bash", serde_json::json!({"command": "ls"}));
    assert_eq!(ttc.name, "Bash");
}
