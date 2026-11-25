//! Runtime behavior of the Codex `commit-3d86e43-r1` additions: the
//! observation-only model-request events and the tool-event `cwd`, which
//! since Codex 0.166.0 is the tool command's working directory.

use hookkit_codex::catalog::{ModelRequestKind, PostModelRequestOutput, PreModelRequestOutput};
use hookkit_codex::protocol::{AnyCommandOutput, AnyInput, Codex, PreToolUseOutput};
use hookkit_core::{
    EnvironmentVariables, EventId, HarnessId, ResolutionProvenance, SnapshotId, Utf8PathBuf,
};
use hookkit_runtime::FailurePolicy;
use hookkit_runtime::failure::failure_response;
use hookkit_runtime::selected::execute_harness;
use hookkit_session_state::{SessionState, StateRoot};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

fn temporary_directory(label: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "hookkit-codex-refresh-{label}-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn model_request(event: &str) -> serde_json::Value {
    let mut value = serde_json::json!({
        "session_id": "session-1", "thread_id": "thread-1", "turn_id": "turn-1",
        "cwd": "/repo", "transcript_path": null, "model": "gpt-test",
        "kind": "foreground_compaction", "request_id": "request-1",
        "attempt_number": 1, "hook_event_name": event
    });
    if event == "PostModelRequest" {
        for field in ["error", "token_usage", "response_id"] {
            value[field] = serde_json::Value::Null;
        }
        value["status"] = "cancelled".into();
    }
    value
}

fn codex_event(name: &'static str) -> EventId {
    EventId::builtin(HarnessId::CODEX, name)
}

#[test]
fn fail_closed_model_request_failures_never_gate() {
    // Codex treats every non-zero exit, 2 included, as a plain failure on
    // these events, so a fail-closed runner lowers to the non-blocking exit 1.
    for name in ["PreModelRequest", "PostModelRequest"] {
        for event in [Some(codex_event(name)), None] {
            let response = failure_response(
                FailurePolicy::FailClosed,
                &HarnessId::CODEX,
                event.as_ref(),
                "hookkit: test failed",
            );
            if event.is_some() {
                assert_eq!(response.exit_code(), 1, "{name}");
                assert!(!response.fails_closed(), "{name}");
                assert!(response.stdout().is_empty());
                assert_eq!(response.stderr(), b"hookkit: test failed\n");
            } else {
                // Only an unidentifiable invocation still denies.
                assert_eq!(response.exit_code(), 2);
            }
        }
    }
}

#[test]
fn model_request_events_resolve_by_discriminator_and_emit_observer_outputs() {
    for (name, output) in [
        (
            "PreModelRequest",
            AnyCommandOutput::from(PreModelRequestOutput::diagnostic("slow log sink\n")),
        ),
        (
            "PostModelRequest",
            AnyCommandOutput::from(PostModelRequestOutput::failure("hook failed\n")),
        ),
    ] {
        let bytes = serde_json::to_vec(&model_request(name)).unwrap();
        let emission = execute_harness::<Codex, _>(
            bytes,
            None,
            &EnvironmentVariables::new(),
            |input, environment, context| {
                assert!(environment.plugin.is_none());
                assert_eq!(context.event(), &codex_event(name));
                assert_eq!(context.snapshot(), SnapshotId::builtin("commit-3d86e43-r1"));
                assert_eq!(
                    context.provenance(),
                    ResolutionProvenance::DefinitiveDiscriminator
                );
                assert_eq!(context.workspace_roots(), [Utf8PathBuf::from("/repo")]);
                assert_eq!(context.turn_id().unwrap().as_str(), "turn-1");
                assert_eq!(context.tool_call_id(), None);
                let AnyInput::Catalog(input) = &input else {
                    panic!("model-request inputs are catalog inputs")
                };
                assert_eq!(input.kind(), Some(ModelRequestKind::ForegroundCompaction));
                assert_eq!(input.permission_mode, None);
                Ok(output)
            },
        )
        .unwrap();
        assert!(emission.stdout().is_empty());
        assert_eq!(
            emission.contract().as_str(),
            format!("codex/commit-3d86e43-r1/{name}")
        );
    }
}

#[test]
fn ambient_codex_tool_call_id_never_becomes_hook_identity() {
    // A hook launched from a Codex shell tool inherits the outer call's
    // CODEX_TOOL_CALL_ID; the hook's own call is the JSON `tool_use_id`.
    let variables = EnvironmentVariables::from_pairs([
        ("CODEX_TOOL_CALL_ID", "outer-call"),
        ("CODEX_SESSION_ID", "outer-session"),
        ("CODEX_THREAD_ID", "outer-thread"),
        ("OPENAI_SPIFFE_ID", "spiffe://outer"),
    ]);
    let bytes = serde_json::to_vec(&serde_json::json!({
        "session_id": "session-1", "transcript_path": null, "cwd": "/repo/nested",
        "hook_event_name": "PreToolUse", "model": "gpt-test", "turn_id": "turn-1",
        "permission_mode": "default", "tool_name": "Bash", "tool_use_id": "call-1",
        "tool_input": {"command": "ls"}
    }))
    .unwrap();
    execute_harness::<Codex, _>(bytes, None, &variables, |_, environment, context| {
        assert!(environment.plugin.is_none());
        assert_eq!(context.tool_call_id().unwrap().as_str(), "call-1");
        assert_eq!(context.session_id().unwrap().as_str(), "session-1");
        assert!(context.workspace_roots().is_empty());
        Ok(AnyCommandOutput::from(PreToolUseOutput::no_op()))
    })
    .unwrap();
}

#[test]
fn tool_event_workdirs_do_not_become_session_project_roots() {
    let state = temporary_directory("project-roots");
    let root = || StateRoot::new(&state);
    let observe = |payload: serde_json::Value| {
        let bytes = serde_json::to_vec(&payload).unwrap();
        execute_harness::<Codex, _>(
            bytes,
            None,
            &EnvironmentVariables::new(),
            |input, _, context| {
                SessionState::ensure(context, root()).unwrap();
                Ok(match input {
                    AnyInput::PreToolUse(_) => PreToolUseOutput::no_op().into(),
                    AnyInput::PostToolUse(_) => {
                        hookkit_codex::protocol::PostToolUseOutput::no_op().into()
                    }
                    _ => hookkit_codex::catalog::SessionStartOutput::no_op().into(),
                })
            },
        )
        .unwrap();
    };
    observe(serde_json::json!({
        "session_id": "session-1", "transcript_path": "/tmp/t.jsonl", "cwd": "/repo",
        "hook_event_name": "SessionStart", "model": "gpt-test",
        "permission_mode": "default", "source": "startup"
    }));
    // A Bash command run with `workdir` reports that directory as `cwd`.
    for cwd in ["/tmp/scratch", "/repo/nested"] {
        observe(serde_json::json!({
            "session_id": "session-1", "transcript_path": "/tmp/t.jsonl", "cwd": cwd,
            "hook_event_name": "PreToolUse", "model": "gpt-test", "turn_id": "turn-1",
            "permission_mode": "default", "tool_name": "Bash", "tool_use_id": "call-1",
            "tool_input": {"command": "cat cwd-marker.txt"}
        }));
        observe(serde_json::json!({
            "session_id": "session-1", "transcript_path": "/tmp/t.jsonl", "cwd": cwd,
            "hook_event_name": "PostToolUse", "model": "gpt-test", "turn_id": "turn-1",
            "permission_mode": "default", "tool_name": "Bash", "tool_use_id": "call-1",
            "tool_input": {"command": "cat cwd-marker.txt"}, "tool_response": "ok"
        }));
    }
    let metadata = SessionState::open(
        HarnessId::CODEX,
        hookkit_session_state::SessionIdentity::Session("session-1".into()),
        root(),
    )
    .unwrap()
    .metadata()
    .unwrap();
    assert_eq!(
        metadata.project.workspace_roots,
        [Utf8PathBuf::from("/repo")]
    );
    assert_eq!(
        metadata.project.transcript_path,
        Some(Utf8PathBuf::from("/tmp/t.jsonl"))
    );
    std::fs::remove_dir_all(state).unwrap();
}
