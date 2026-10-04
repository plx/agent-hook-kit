use super::*;
use hookkit_common::RewriteApproval;
use hookkit_core::{Diagnostic, DiagnosticLevel};
use serde_json::{Value, json};
use std::sync::Mutex;

fn claude_variables() -> EnvironmentVariables {
    EnvironmentVariables::from_pairs([
        ("CLAUDECODE", "1"),
        ("CLAUDE_CODE_CHILD_SESSION", "1"),
        ("CLAUDE_CODE_SESSION_ID", "s"),
        ("CLAUDE_PROJECT_DIR", "/repo"),
        ("CLAUDE_ENV_FILE", "/tmp/claude-env"),
    ])
}

fn pre_tool_cases() -> Vec<(HarnessId, &'static [u8], EnvironmentVariables)> {
    vec![
        (
            HarnessId::CLAUDE_CODE,
            br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"PreToolUse","permission_mode":"default","tool_name":"Read","tool_input":{"file_path":"/repo/.env"},"tool_use_id":"u","claude_only":{"retained":true}}"#,
            claude_variables(),
        ),
        (
            HarnessId::CODEX,
            br#"{"session_id":"s","transcript_path":null,"cwd":"/repo","hook_event_name":"PreToolUse","model":"gpt-5","turn_id":"t","permission_mode":"default","tool_name":"mcp__fs__read","tool_input":{"path":".env"},"tool_use_id":"u","codex_only":"retained"}"#,
            EnvironmentVariables::new(),
        ),
        (
            HarnessId::ANTIGRAVITY,
            br#"{"conversationId":"s","workspacePaths":["/repo","/lib"],"transcriptPath":"/tmp/t","artifactDirectoryPath":"/tmp/a","toolCall":{"name":"view_file","args":{"AbsolutePath":"/repo/.env"},"nativeFlag":true},"stepIdx":7,"antigravityOnly":"retained"}"#,
            EnvironmentVariables::from_pairs([("AMBIENT_ONLY", "ignored")]),
        ),
    ]
}

fn pair_inputs(
    event: &'static str,
    claude_fields: Value,
    codex_fields: Value,
) -> Vec<(HarnessId, Vec<u8>, EnvironmentVariables)> {
    fn merged(mut base: Value, fields: Value) -> Value {
        base.as_object_mut()
            .expect("base is an object")
            .extend(fields.as_object().expect("fields are an object").clone());
        base
    }

    let claude = merged(
        json!({
            "session_id": "s",
            "transcript_path": "/tmp/claude.jsonl",
            "cwd": "/repo",
            "hook_event_name": event,
            "native_only": {"claude": true}
        }),
        claude_fields,
    );
    let mut codex = json!({
        "session_id": "s",
        "transcript_path": null,
        "cwd": "/repo",
        "hook_event_name": event,
        "native_only": {"codex": true}
    });
    if event != "SessionEnd" {
        codex
            .as_object_mut()
            .expect("base is an object")
            .insert("model".into(), "gpt-test".into());
    }
    let codex = merged(codex, codex_fields);

    vec![
        (
            HarnessId::CLAUDE_CODE,
            serde_json::to_vec(&claude).unwrap(),
            claude_variables(),
        ),
        (
            HarnessId::CODEX,
            serde_json::to_vec(&codex).unwrap(),
            EnvironmentVariables::new(),
        ),
    ]
}

/// The exact stdout a native emission carries: `None` for empty stdout,
/// otherwise the parsed JSON.
fn stdout(emission: &ProcessEmission) -> Option<Value> {
    (!emission.stdout().is_empty()).then(|| serde_json::from_slice(emission.stdout()).unwrap())
}

macro_rules! assert_pair_family {
    (
        marker: $marker:ty,
        input: $input:ident,
        event: $event:literal,
        claude: $claude_fields:expr,
        codex: $codex_fields:expr,
        output: $output:expr,
        claude_stdout: $claude_stdout:expr,
        codex_stdout: $codex_stdout:expr
    ) => {{
        let build_output = $output;
        for (harness, bytes, variables) in pair_inputs($event, $claude_fields, $codex_fields) {
            let expected_harness = harness.clone();
            let handler_harness = harness.clone();
            let emission = execute_aligned_event::<$marker, _>(
                harness,
                bytes,
                &variables,
                move |input, environment, context| {
                    assert_eq!(input.harness(), handler_harness);
                    assert_eq!(input.event_id().name(), $event);
                    assert_eq!(input.event_id().harness(), &handler_harness);
                    assert_eq!(input.session_id(), "s");
                    assert_eq!(environment.harness(), handler_harness);
                    assert_eq!(context.harness(), &handler_harness);
                    assert_eq!(context.event(), &input.event_id());
                    assert_eq!(input.workspace_roots(), context.workspace_roots());
                    assert_eq!(input.cwd().as_str(), "/repo");
                    assert_eq!(
                        input.project_roots(environment).as_ref(),
                        [hookkit_core::Utf8PathBuf::from("/repo")]
                    );
                    match input {
                        $input::Claude(_) => {
                            assert_eq!(handler_harness, HarnessId::CLAUDE_CODE)
                        }
                        $input::Codex(_) => assert_eq!(handler_harness, HarnessId::CODEX),
                        _ => unreachable!("only Claude and Codex are aligned here"),
                    }
                    build_output(&handler_harness)
                },
            )
            .unwrap();
            assert_eq!(emission.exit_code(), 0, "{} {}", expected_harness, $event);
            assert!(emission.stderr().is_empty());
            let expected: Option<Value> = if expected_harness == HarnessId::CLAUDE_CODE {
                $claude_stdout
            } else {
                $codex_stdout
            };
            assert_eq!(
                stdout(&emission),
                expected,
                "{} {}",
                expected_harness,
                $event
            );
        }
    }};
}

#[test]
fn claude_codex_pair_families_parse_and_emit_their_exact_portable_floors() {
    let permission_allow = json!({"hookSpecificOutput": {
        "hookEventName": "PermissionRequest",
        "decision": {"behavior": "allow"}
    }});
    assert_pair_family!(
        marker: PermissionRequest,
        input: PermissionRequestInput,
        event: "PermissionRequest",
        claude: json!({"tool_name": "Bash", "tool_input": {"command": "true"}}),
        codex: json!({
            "turn_id": "t",
            "permission_mode": "default",
            "tool_name": "Bash",
            "tool_input": {"command": "true"}
        }),
        output: |harness: &HarnessId| PermissionRequestOutput::allow(harness),
        claude_stdout: Some(permission_allow.clone()),
        codex_stdout: Some(permission_allow.clone())
    );
    assert_pair_family!(
        marker: PermissionRequest,
        input: PermissionRequestInput,
        event: "PermissionRequest",
        claude: json!({"tool_name": "Bash", "tool_input": {"command": "true"}}),
        codex: json!({
            "turn_id": "t",
            "permission_mode": "default",
            "tool_name": "Bash",
            "tool_input": {"command": "true"}
        }),
        output: |harness: &HarnessId| PermissionRequestOutput::no_op(harness),
        claude_stdout: Some(json!({})),
        codex_stdout: None
    );
    assert_pair_family!(
        marker: PreCompact,
        input: PreCompactInput,
        event: "PreCompact",
        // Claude Code sends `custom_instructions: null` for automatic
        // compaction; only a manual `/compact` carries instructions.
        claude: json!({"trigger": "auto", "custom_instructions": null}),
        codex: json!({"turn_id": "t", "trigger": "auto"}),
        output: |harness: &HarnessId| PreCompactOutput::no_op(harness),
        claude_stdout: Some(json!({})),
        codex_stdout: None
    );
    assert_pair_family!(
        marker: PostCompact,
        input: PostCompactInput,
        event: "PostCompact",
        claude: json!({"trigger": "auto", "compact_summary": "summary"}),
        codex: json!({"turn_id": "t", "trigger": "auto"}),
        output: |harness: &HarnessId| PostCompactOutput::with_system_notice(harness, "done"),
        // Claude Code discards a PostCompact `systemMessage`, so none is sent.
        claude_stdout: Some(json!({})),
        codex_stdout: Some(json!({"systemMessage": "done"}))
    );
    let session_context = json!({"hookSpecificOutput": {
        "hookEventName": "SessionStart",
        "additionalContext": "context"
    }});
    assert_pair_family!(
        marker: SessionStart,
        input: SessionStartInput,
        event: "SessionStart",
        claude: json!({"source": "startup"}),
        codex: json!({"permission_mode": "default", "source": "startup"}),
        output: |harness: &HarnessId| SessionStartOutput::with_context(harness, "context"),
        claude_stdout: Some(session_context.clone()),
        codex_stdout: Some(session_context.clone())
    );
    assert_pair_family!(
        marker: SessionEnd,
        input: SessionEndInput,
        event: "SessionEnd",
        claude: json!({"reason": "prompt_input_exit"}),
        codex: json!({"reason": "other"}),
        output: |harness: &HarnessId| SessionEndOutput::no_op(harness),
        claude_stdout: Some(json!({})),
        codex_stdout: None
    );
    let subagent_context = json!({"hookSpecificOutput": {
        "hookEventName": "SubagentStart",
        "additionalContext": "context"
    }});
    assert_pair_family!(
        marker: SubagentStart,
        input: SubagentStartInput,
        event: "SubagentStart",
        claude: json!({"agent_id": "a", "agent_type": "Explore"}),
        codex: json!({
            "turn_id": "t",
            "permission_mode": "default",
            "agent_id": "a",
            "agent_type": "Explore"
        }),
        output: |harness: &HarnessId| SubagentStartOutput::with_context(harness, "context"),
        claude_stdout: Some(subagent_context.clone()),
        codex_stdout: Some(subagent_context.clone())
    );
    let block = json!({"decision": "block", "reason": "continue"});
    assert_pair_family!(
        marker: SubagentStop,
        input: SubagentStopInput,
        event: "SubagentStop",
        claude: json!({
            "stop_hook_active": false,
            "agent_id": "a",
            "agent_type": "Explore",
            "agent_transcript_path": "/tmp/a.jsonl",
            "last_assistant_message": "done"
        }),
        codex: json!({
            "turn_id": "t",
            "permission_mode": "default",
            "stop_hook_active": false,
            "agent_id": "a",
            "agent_type": "Explore",
            "agent_transcript_path": "/tmp/a.jsonl",
            "last_assistant_message": "done"
        }),
        output: |harness: &HarnessId| SubagentStopOutput::block(harness, "continue"),
        claude_stdout: Some(block.clone()),
        codex_stdout: Some(block.clone())
    );
    // Context that starts with `[` would be parsed (and dropped) as malformed
    // JSON if Codex received it as plain stdout.
    let prompt_context = json!({"hookSpecificOutput": {
        "hookEventName": "UserPromptSubmit",
        "additionalContext": "[policy] use pnpm, not npm"
    }});
    assert_pair_family!(
        marker: UserPromptSubmit,
        input: UserPromptSubmitInput,
        event: "UserPromptSubmit",
        claude: json!({"prompt": "ship it"}),
        codex: json!({
            "turn_id": "t",
            "permission_mode": "default",
            "prompt": "ship it"
        }),
        output: |harness: &HarnessId| {
            UserPromptSubmitOutput::with_context(harness, "[policy] use pnpm, not npm")
        },
        claude_stdout: Some(prompt_context.clone()),
        codex_stdout: Some(prompt_context.clone())
    );
}

#[test]
fn pair_only_portable_block_and_deny_helpers_use_exact_native_shapes() {
    for (harness, bytes, variables) in pair_inputs(
        "PermissionRequest",
        json!({"tool_name": "Bash", "tool_input": {"command": "true"}}),
        json!({
            "turn_id": "t",
            "permission_mode": "default",
            "tool_name": "Bash",
            "tool_input": {"command": "true"}
        }),
    ) {
        let selected = harness.clone();
        let emission = execute_permission_request(harness, bytes, &variables, move |_, _, _| {
            PermissionRequestOutput::deny(&selected, "denied")
        })
        .unwrap();
        assert_eq!(
            stdout(&emission),
            Some(json!({"hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": {"behavior": "deny", "message": "denied"}
            }}))
        );
    }

    for (harness, bytes, variables) in pair_inputs(
        "UserPromptSubmit",
        json!({"prompt": "ship it"}),
        json!({
            "turn_id": "t",
            "permission_mode": "default",
            "prompt": "ship it"
        }),
    ) {
        let selected = harness.clone();
        let emission = execute_user_prompt_submit(harness, bytes, &variables, move |_, _, _| {
            UserPromptSubmitOutput::block(&selected, "blocked")
        })
        .unwrap();
        assert_eq!(emission.exit_code(), 2);
        assert!(emission.stdout().is_empty());
        assert_eq!(emission.stderr(), b"blocked");
    }
}

#[test]
fn blank_block_reasons_fail_before_emission_on_every_harness() {
    for (harness, bytes, variables) in pair_inputs(
        "UserPromptSubmit",
        json!({"prompt": "ship it"}),
        json!({
            "turn_id": "t",
            "permission_mode": "default",
            "prompt": "ship it"
        }),
    ) {
        let selected = harness.clone();
        let result = execute_user_prompt_submit(harness, bytes, &variables, move |_, _, _| {
            UserPromptSubmitOutput::block(&selected, "  ")
        });
        assert!(matches!(
            result,
            Err(HookkitError::InvalidProcessEmission(_))
        ));
    }
    for (harness, bytes, variables) in pre_tool_cases() {
        let selected = harness.clone();
        let result = execute_pre_tool_use(harness, bytes, &variables, move |_, _, _| {
            PreToolUseOutput::deny(&selected, "")
        });
        assert!(matches!(
            result,
            Err(HookkitError::InvalidProcessEmission(_))
        ));
    }
}

#[test]
fn unsupported_harnesses_fail_before_parsing_without_invoking_the_handler() {
    let unsupported = execute_aligned_event::<SessionStart, _>(
        HarnessId::ANTIGRAVITY,
        b"not json".to_vec(),
        &EnvironmentVariables::new(),
        |_, _, _| panic!("unsupported harness must not invoke the handler"),
    );
    assert!(matches!(
        unsupported,
        Err(HookkitError::UnsupportedHarness { .. })
    ));

    // The CLI alias is not a harness identity.
    let alias = execute_pre_tool_use(
        HarnessId::new("claude").unwrap(),
        pre_tool_cases().remove(0).1,
        &claude_variables(),
        |_, _, _| panic!("an alias must not invoke the handler"),
    );
    let Err(HookkitError::UnsupportedHarness { harness, message }) = alias else {
        panic!("expected an unsupported-harness error, got {alias:?}");
    };
    assert_eq!(harness.as_str(), "claude");
    assert!(message.contains("PreToolUse"), "{message}");
}

#[test]
fn pair_only_markers_reject_mismatched_output_arms() {
    let (_, bytes, variables) = pair_inputs(
        "SessionStart",
        json!({"source": "startup"}),
        json!({"permission_mode": "default", "source": "startup"}),
    )
    .remove(1);
    let mismatched = execute_session_start(HarnessId::CODEX, bytes, &variables, |_, _, _| {
        Ok(SessionStartOutput::Claude(
            hookkit_claude::protocol::SessionStartOutput::no_op(),
        ))
    });
    assert!(matches!(
        mismatched,
        Err(HookkitError::EventHarnessMismatch { .. })
    ));
}

#[test]
fn aligned_pre_tool_parses_every_native_contract_and_passes_through() {
    for (harness, bytes, variables) in pre_tool_cases() {
        let expected_harness = harness.clone();
        let handler_harness = harness.clone();
        let emission = execute_pre_tool_use(
            harness,
            bytes,
            &variables,
            move |input, environment, context| {
                assert_eq!(input.harness(), handler_harness);
                assert_eq!(environment.harness(), handler_harness);
                assert_eq!(context.harness(), &handler_harness);
                assert_eq!(input.event_id().harness(), &handler_harness);
                assert_eq!(input.workspace_roots(), context.workspace_roots());
                assert_eq!(
                    input.cwd().is_none(),
                    handler_harness == HarnessId::ANTIGRAVITY
                );
                // Each payload uses a real tool of its harness, so the
                // path argument keeps that tool's native name.
                let (path_key, path) = match handler_harness.as_str() {
                    "claude-code" => ("file_path", "/repo/.env"),
                    "codex" => ("path", ".env"),
                    "antigravity" => ("AbsolutePath", "/repo/.env"),
                    _ => unreachable!(),
                };
                assert_eq!(
                    input.tool_input().and_then(|input| input.get(path_key)),
                    Some(&json!(path))
                );
                assert!(input.tool_name().is_some());

                match (&input, environment) {
                    (
                        PreToolUseInput::Claude(input),
                        PreToolUseCommandEnvironment::Claude(environment),
                    ) => {
                        assert_eq!(input.field("claude_only"), Some(&json!({"retained": true})));
                        assert_eq!(environment.project_dir, "/repo");
                        assert_eq!(input.cwd, "/repo");
                    }
                    (
                        PreToolUseInput::Codex(input),
                        PreToolUseCommandEnvironment::Codex(environment),
                    ) => {
                        assert_eq!(input.extra.get("codex_only"), Some(&json!("retained")));
                        assert!(environment.plugin.is_none());
                        assert_eq!(input.cwd, "/repo");
                    }
                    (
                        PreToolUseInput::Antigravity(input),
                        PreToolUseCommandEnvironment::Antigravity(_),
                    ) => {
                        assert_eq!(input.step_idx, 7);
                        assert_eq!(input.tool_call.extra.get("nativeFlag"), Some(&json!(true)));
                        assert_eq!(input.extra.get("antigravityOnly"), Some(&json!("retained")));
                        assert!(input.workspace_paths.len() == 2);
                    }
                    _ => panic!("input and command-environment arms must match"),
                }

                let output = PreToolUseOutput::pass_through(&handler_harness)?;
                assert_eq!(output.event_id(), input.event_id());
                Ok(output)
            },
        )
        .unwrap();

        assert_eq!(emission.exit_code(), 0);
        assert!(emission.stderr().is_empty());
        // No arm answers "allow": each defers to the harness's normal
        // permission flow, and Antigravity's least-privilege neutral is "ask".
        let expected = match expected_harness.as_str() {
            "claude-code" => Some(json!({})),
            "codex" => None,
            "antigravity" => Some(json!({"decision": "ask"})),
            _ => unreachable!(),
        };
        assert_eq!(stdout(&emission), expected, "{expected_harness}");
    }
}

#[test]
fn aligned_pre_tool_allow_is_explicit_approval_where_the_harness_supports_it() {
    for (harness, bytes, variables) in pre_tool_cases() {
        let expected_harness = harness.clone();
        let emission = execute_pre_tool_use(harness, bytes, &variables, move |_, _, context| {
            PreToolUseOutput::allow(context.harness())
        })
        .unwrap();
        let expected = match expected_harness.as_str() {
            "claude-code" => Some(json!({"hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow"
            }})),
            // Codex cannot express an explicit approval.
            "codex" => None,
            "antigravity" => Some(json!({"decision": "allow"})),
            _ => unreachable!(),
        };
        assert_eq!(stdout(&emission), expected, "{expected_harness}");
        assert_eq!(
            PreToolUseOutput::explicit_allow_supported(&expected_harness),
            expected_harness != HarnessId::CODEX
        );
    }
}

#[test]
fn aligned_pre_tool_denies_with_each_exact_native_shape() {
    for (harness, bytes, variables) in pre_tool_cases() {
        let expected_harness = harness.clone();
        let selected = harness.clone();
        let emission =
            execute_aligned_event::<PreToolUse, _>(harness, bytes, &variables, move |_, _, _| {
                PreToolUseOutput::deny(&selected, "blocked by policy")
            })
            .unwrap();

        assert_eq!(emission.exit_code(), 0);
        assert!(emission.stderr().is_empty());
        let hook_specific = json!({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": "blocked by policy"
        }});
        let expected = match expected_harness.as_str() {
            "claude-code" | "codex" => hook_specific,
            "antigravity" => json!({"decision": "deny", "reason": "blocked by policy"}),
            _ => unreachable!(),
        };
        assert_eq!(stdout(&emission), Some(expected), "{expected_harness}");
    }
}

#[test]
fn aligned_pre_tool_rewrite_keeps_the_claude_decision_explicit() {
    let (_, claude_bytes, claude_env) = pre_tool_cases().remove(0);
    let emission = execute_pre_tool_use(
        HarnessId::CLAUDE_CODE,
        claude_bytes,
        &claude_env,
        |input, _, context| {
            let updated = input
                .tool_input()
                .and_then(|input| input.as_object().cloned());
            PreToolUseOutput::rewrite(
                context.harness(),
                updated.unwrap(),
                RewriteApproval::Ask("normalized the path".into()),
            )
        },
    )
    .unwrap();
    assert_eq!(
        stdout(&emission),
        Some(json!({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "ask",
            "permissionDecisionReason": "normalized the path",
            "updatedInput": {"file_path": "/repo/.env"}
        }}))
    );

    let (_, codex_bytes, codex_env) = pre_tool_cases().remove(1);
    let emission = execute_pre_tool_use(
        HarnessId::CODEX,
        codex_bytes,
        &codex_env,
        |input, _, context| {
            let updated = input
                .tool_input()
                .and_then(|input| input.as_object().cloned());
            PreToolUseOutput::rewrite(
                context.harness(),
                updated.unwrap(),
                RewriteApproval::Ask("ignored by Codex".into()),
            )
        },
    )
    .unwrap();
    assert_eq!(
        stdout(&emission),
        Some(json!({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "updatedInput": {"path": ".env"}
        }}))
    );
}

#[test]
fn aligned_pre_tool_rejects_wrong_output_arm_before_emission() {
    let (_, bytes, variables) = pre_tool_cases().remove(1);
    let result = execute_pre_tool_use(HarnessId::CODEX, bytes, &variables, |_, environment, _| {
        assert!(matches!(
            environment,
            PreToolUseCommandEnvironment::Codex(_)
        ));
        Ok(PreToolUseOutput::Claude(
            hookkit_claude::catalog::PreToolUseOutput::no_op(),
        ))
    });
    assert!(matches!(
        result,
        Err(HookkitError::EventHarnessMismatch { .. })
    ));
}

#[test]
fn aligned_pre_tool_validates_required_native_environments() {
    let (claude, claude_bytes, _) = pre_tool_cases().remove(0);
    let claude_result = execute_pre_tool_use(
        claude,
        claude_bytes,
        &EnvironmentVariables::new(),
        |_, _, _| panic!("handler must not run for an invalid environment"),
    );
    assert!(matches!(
        claude_result,
        Err(HookkitError::InvalidHookEnvironment { .. })
    ));
}

#[derive(Default)]
struct Recording(Mutex<Vec<Diagnostic>>);

impl DiagnosticsSink for Recording {
    fn record(&self, diagnostic: Diagnostic) {
        self.0.lock().unwrap().push(diagnostic);
    }
}

#[test]
fn handlers_record_diagnostics_through_the_configured_sink() {
    let sink = Recording::default();
    let (harness, bytes, variables) = pre_tool_cases().remove(2);
    execute_aligned_event_with_diagnostics::<PreToolUse, _>(
        harness,
        bytes,
        &variables,
        &sink,
        |_, _, context| {
            context.diagnostics().record(Diagnostic::new(
                DiagnosticLevel::Info,
                "inspected view_file",
            ));
            PreToolUseOutput::pass_through(context.harness())
        },
    )
    .unwrap();
    {
        let recorded = sink.0.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].message, "inspected view_file");
    }

    // The plain entry point keeps diagnostics disabled.
    let (harness, bytes, variables) = pre_tool_cases().remove(2);
    execute_aligned_event::<PreToolUse, _>(harness, bytes, &variables, |_, _, context| {
        context
            .diagnostics()
            .record(Diagnostic::new(DiagnosticLevel::Info, "discarded"));
        PreToolUseOutput::pass_through(context.harness())
    })
    .unwrap();
    assert_eq!(sink.0.lock().unwrap().len(), 1);
}

#[test]
fn mismatched_harness_arms_fail_before_emission() {
    let bytes = br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"PostToolUse","tool_name":"Bash","tool_input":{},"tool_use_id":"u","tool_response":{}}"#.to_vec();
    let result = execute_post_tool_use(
        HarnessId::CLAUDE_CODE,
        bytes,
        &claude_variables(),
        |_, environment, _| {
            assert!(matches!(
                environment,
                PostToolUseCommandEnvironment::Claude(_)
            ));
            Ok(PostToolUseOutput::Codex(
                hookkit_codex::protocol::PostToolUseOutput::no_op(),
            ))
        },
    );
    assert!(matches!(
        result,
        Err(HookkitError::EventHarnessMismatch { .. })
    ));
}

#[test]
fn antigravity_post_tool_preserves_typed_tool_data() {
    let bytes = br#"{"conversationId":"c","workspacePaths":["/repo","/lib"],"transcriptPath":"/tmp/t","artifactDirectoryPath":"/tmp/a","toolCall":{"name":"run_command","args":{"CommandLine":"cargo test","Cwd":"/repo"}},"stepIdx":2}"#.to_vec();
    let emission = execute_post_tool_use(
        HarnessId::ANTIGRAVITY,
        bytes,
        &EnvironmentVariables::new(),
        |input, environment, context| {
            assert_eq!(input.workspace_roots().len(), 2);
            assert_eq!(context.workspace_roots().len(), 2);
            assert_eq!(input.tool_name(), Some("run_command"));
            assert_eq!(
                input.tool_input().and_then(|args| args.get("CommandLine")),
                Some(&json!("cargo test"))
            );
            assert!(matches!(
                environment,
                PostToolUseCommandEnvironment::Antigravity(_)
            ));
            PostToolUseOutput::no_op(context.harness())
        },
    )
    .unwrap();
    assert_eq!(emission.stdout(), b"{}");
}

#[test]
fn turn_completion_preserves_each_native_contract() {
    let cases = [
        (
            HarnessId::CLAUDE_CODE,
            br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"Stop","stop_hook_active":false,"last_assistant_message":"done"}"#.as_slice(),
            claude_variables(),
        ),
        (
            HarnessId::CODEX,
            br#"{"session_id":"s","transcript_path":"/tmp/t","cwd":"/repo","hook_event_name":"Stop","model":"gpt-5","turn_id":"t","permission_mode":"default","stop_hook_active":false,"last_assistant_message":"done"}"#.as_slice(),
            EnvironmentVariables::new(),
        ),
        (
            HarnessId::ANTIGRAVITY,
            br#"{"conversationId":"s","workspacePaths":["/repo"],"transcriptPath":"/tmp/t","artifactDirectoryPath":"/tmp/a","executionNum":1,"terminationReason":"model_stop","fullyIdle":true}"#.as_slice(),
            EnvironmentVariables::new(),
        ),
    ];

    for (harness, bytes, variables) in cases {
        let expected = harness.clone();
        let handler_harness = expected.clone();
        let emission = execute_turn_completion(
            harness,
            bytes,
            &variables,
            move |input, environment, context| {
                assert_eq!(input.harness(), handler_harness);
                assert_eq!(environment.harness(), handler_harness);
                assert_eq!(context.workspace_roots()[0].as_str(), "/repo");
                assert_eq!(
                    input.stop_hook_active(),
                    (handler_harness != HarnessId::ANTIGRAVITY).then_some(false)
                );
                assert_eq!(
                    input.fully_idle(),
                    (handler_harness == HarnessId::ANTIGRAVITY).then_some(true)
                );
                TurnCompletionOutput::allow(context.harness())
            },
        )
        .unwrap();
        assert_eq!(emission.exit_code(), 0);
        assert!(emission.stderr().is_empty());
        let expected_stdout = match expected.as_str() {
            "claude-code" => Some(json!({})),
            // Empty stdout is Codex's documented `no-op` outcome.
            "codex" => None,
            "antigravity" => Some(json!({"decision": "stop"})),
            _ => unreachable!(),
        };
        assert_eq!(stdout(&emission), expected_stdout, "{expected}");
    }
}

#[test]
fn every_marker_names_its_native_event_for_failure_reporting() {
    fn native<K: AlignedEventSpec>() -> (&'static str, &'static str) {
        (K::FAMILY, <K as sealed::Sealed>::NATIVE_EVENT)
    }
    assert_eq!(native::<PreToolUse>(), ("PreToolUse", "PreToolUse"));
    assert_eq!(native::<PostToolUse>(), ("PostToolUse", "PostToolUse"));
    assert_eq!(native::<TurnCompletion>(), ("TurnCompletion", "Stop"));
    assert_eq!(
        native::<UserPromptSubmit>(),
        ("UserPromptSubmit", "UserPromptSubmit")
    );
    assert_eq!(
        native::<PermissionRequest>(),
        ("PermissionRequest", "PermissionRequest")
    );
}
