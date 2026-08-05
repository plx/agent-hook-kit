use hookkit_common::{PostToolUseInput, PreToolUseInput};
use hookkit_core::{EventId, HarnessId, Utf8Path, Utf8PathBuf};
use hookkit_tool_access::{
    AccessCertainty, AccessIntent, AccessProvenance, AccessSource, AccessTarget, JsonRef,
    PatchOperation, StructuredFieldAnalyzer, StructuredFieldMatch, TargetResolutionOptions,
    ToolAccessAnalyzer, ToolAccessGapReason, ToolCallRef, ToolPhase, resolve_targets,
};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

fn call<'a>(tool_name: &'a str, input: &'a serde_json::Value) -> ToolCallRef<'a> {
    ToolCallRef::new(
        EventId::builtin(HarnessId::CODEX, "PreToolUse"),
        ToolPhase::Pre,
        tool_name,
        JsonRef::Value(input),
        Some(Utf8Path::new("/repo/native-cwd")),
        vec![Utf8PathBuf::from("/repo")],
    )
}

fn codex_shell(command: &str, cwd: &str) -> PreToolUseInput {
    PreToolUseInput::Codex(
        serde_json::from_value(serde_json::json!({
            "session_id": "session",
            "transcript_path": null,
            "cwd": cwd,
            "hook_event_name": "PreToolUse",
            "model": "gpt-test",
            "turn_id": "turn",
            "permission_mode": "default",
            "tool_name": "Bash",
            "tool_use_id": "call",
            "tool_input": {"command": command}
        }))
        .unwrap(),
    )
}

struct TempDirectory(Utf8PathBuf);

impl TempDirectory {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = Utf8PathBuf::from_path_buf(std::env::temp_dir())
            .unwrap()
            .join(format!("hookkit-access-analyzer-{label}-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn structured_fields_are_recursive_lossless_and_not_double_visited() {
    let input = serde_json::json!({
        "path": "src/lib.rs",
        "wrapper": {
            "filePath": "generated/out.rs",
            "nested": [{"target_file": "/absolute/target.rs"}],
            "paths": ["one.txt", ["two.txt"]]
        },
        "absolutePath": "/absolute/config.json",
        "message": "not/a/candidate.txt"
    });
    let structured = StructuredFieldAnalyzer::default()
        .with_pointer("/wrapper/filePath", AccessIntent::Modify)
        .unwrap();
    let analyzer = ToolAccessAnalyzer::new(
        hookkit_shell::BashAnalyzer::default(),
        hookkit_shell::FileAccessAnalyzer::default(),
        structured,
    );
    let report = analyzer.analyze_call(&call("write_files", &input));

    assert_eq!(report.candidates.len(), 6);
    assert!(report.is_complete());
    let direct = report
        .candidates
        .iter()
        .filter(|candidate| {
            matches!(
                &candidate.provenance,
                AccessProvenance::StructuredField {
                    pointer,
                    matched_by: StructuredFieldMatch::ExactPointer,
                } if pointer == "/wrapper/filePath"
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        direct.len(),
        1,
        "exact pointer must suppress key re-visiting"
    );
    assert_eq!(direct[0].certainty, AccessCertainty::Direct);

    assert!(report.candidates.iter().any(|candidate| {
        matches!(
            &candidate.target,
            AccessTarget::Path { expression, .. }
                if expression.raw == "src/lib.rs"
                    && expression.resolved.as_deref() == Some(Utf8Path::new("/repo/native-cwd/src/lib.rs"))
        )
    }));
    assert!(report.candidates.iter().any(|candidate| {
        matches!(
            &candidate.target,
            AccessTarget::Path { expression, .. }
                if expression.raw == "/absolute/config.json"
                    && expression.resolved.as_deref() == Some(Utf8Path::new("/absolute/config.json"))
        )
    }));
    assert!(!report.candidates.iter().any(|candidate| {
        matches!(
            &candidate.target,
            AccessTarget::Path { expression, .. } if expression.raw == "not/a/candidate.txt"
        )
    }));
}

#[test]
fn unknown_structured_tools_keep_unclassified_references_and_gaps() {
    let input = serde_json::json!({"path": "possible.txt"});
    let report = ToolAccessAnalyzer::default().analyze_call(&call("mystery_tool", &input));

    assert_eq!(report.candidates.len(), 1);
    assert_eq!(report.candidates[0].intent, AccessIntent::Unclassified);
    assert!(!report.is_complete());
    assert!(matches!(
        report.gaps[0].reason,
        ToolAccessGapReason::UnclassifiedAccess { .. }
    ));
}

#[test]
fn patch_parser_preserves_add_update_delete_and_move_roles() {
    let input = serde_json::json!({
        "patch": "*** Begin Patch\n*** Add File: src/new.rs\n+new\n*** Update File: src/lib.rs\n@@\n-old\n+new\n*** Delete File: src/old.rs\n*** Update File: src/from.rs\n*** Move to: src/to.rs\n*** End Patch"
    });
    let report = ToolAccessAnalyzer::default().analyze_call(&call("tools.apply_patch", &input));

    let roles = report
        .candidates
        .iter()
        .map(|candidate| candidate.intent)
        .collect::<Vec<_>>();
    assert_eq!(
        roles,
        vec![
            AccessIntent::Modify,
            AccessIntent::ReadModify,
            AccessIntent::Delete,
            AccessIntent::MoveSource,
            AccessIntent::MoveDestination,
        ]
    );
    assert!(report.candidates.iter().all(|candidate| {
        matches!(
            candidate.provenance,
            AccessProvenance::Patch {
                payload_pointer: ref pointer,
                line,
                ..
            } if pointer == "/patch" && line > 0
        )
    }));
    assert!(report.is_complete());
}

#[test]
fn malformed_patch_retains_partial_evidence_and_a_typed_gap() {
    let input = serde_json::json!({
        "patch": "*** Add File: recovered.txt\n*** Move to: orphaned.txt"
    });
    let report = ToolAccessAnalyzer::default().analyze_call(&call("apply_patch", &input));

    assert_eq!(report.candidates.len(), 2);
    assert!(report.candidates.iter().any(|candidate| {
        matches!(
            candidate.provenance,
            AccessProvenance::Patch {
                operation: PatchOperation::Add,
                ..
            }
        )
    }));
    assert!(report.gaps.iter().any(|gap| matches!(
        gap.reason,
        ToolAccessGapReason::MalformedPatch { line: Some(2), .. }
    )));
}

#[test]
fn unified_patch_headers_preserve_create_delete_update_and_move_roles() {
    let cases = [
        ("--- /dev/null\n+++ b/new.txt", AccessIntent::Modify),
        ("--- a/old.txt\n+++ /dev/null", AccessIntent::Delete),
        ("--- a/same.txt\n+++ b/same.txt", AccessIntent::ReadModify),
    ];
    for (patch, expected) in cases {
        let input = serde_json::json!({"input": patch});
        let report = ToolAccessAnalyzer::default().analyze_call(&call("apply_patch", &input));
        assert_eq!(report.candidates.len(), 1);
        assert_eq!(report.candidates[0].intent, expected);
        assert!(report.gaps.is_empty());
    }

    let input = serde_json::json!({"input": "--- a/from.txt\n+++ b/to.txt"});
    let report = ToolAccessAnalyzer::default().analyze_call(&call("apply_patch", &input));
    assert_eq!(
        report
            .candidates
            .iter()
            .map(|candidate| candidate.intent)
            .collect::<Vec<_>>(),
        vec![AccessIntent::MoveSource, AccessIntent::MoveDestination]
    );
}

#[test]
fn aligned_shell_reads_modifications_and_unknowns_map_without_role_loss() {
    let input = PreToolUseInput::Codex(
        serde_json::from_value(serde_json::json!({
            "session_id": "session",
            "transcript_path": null,
            "cwd": "/repo",
            "hook_event_name": "PreToolUse",
            "model": "gpt-test",
            "turn_id": "turn",
            "permission_mode": "default",
            "tool_name": "Bash",
            "tool_use_id": "call",
            "tool_input": {"command": "cat input.txt > output.txt; mystery dynamic.txt"}
        }))
        .unwrap(),
    );
    let report = ToolAccessAnalyzer::default().analyze_pre_tool(&input);

    assert!(report.may_read().any(|candidate| {
        candidate.intent == AccessIntent::Read
            && matches!(candidate.provenance, AccessProvenance::Shell { .. })
    }));
    assert!(report.may_modify().any(|candidate| {
        candidate.intent == AccessIntent::Modify
            && matches!(candidate.provenance, AccessProvenance::Shell { .. })
    }));
    assert!(report.gaps.iter().any(|gap| {
        gap.source == AccessSource::Shell
            && matches!(
                &gap.reason,
                ToolAccessGapReason::ShellUnresolved(unresolved)
                    if matches!(unresolved.reason, hookkit_shell::UnresolvedFileAccessReason::UnknownCommandSemantics { .. })
            )
    }));
}

#[test]
fn recursive_remove_scope_materializes_descendants_for_policy_matching() {
    let temporary = TempDirectory::new("recursive-remove");
    fs::create_dir_all(temporary.0.join("secrets/nested")).unwrap();
    fs::write(temporary.0.join("secrets/nested/token.txt"), "token").unwrap();
    let input = codex_shell("rm -rf secrets", temporary.0.as_str());
    let report = ToolAccessAnalyzer::default().analyze_pre_tool(&input);
    let options = TargetResolutionOptions::new(vec![temporary.0.clone()]);
    let materialized = resolve_targets(
        report.may_modify().map(|candidate| &candidate.target),
        &options,
    )
    .unwrap();

    assert!(
        materialized
            .paths
            .contains(&temporary.0.join("secrets/nested/token.txt"))
    );
}

#[test]
fn bundled_profiles_are_public_and_custom_shell_aliases_are_explicit() {
    assert_eq!(
        hookkit_shell::CLAUDE_BASH_PROFILE.command_pointer(),
        "/command"
    );
    assert_eq!(hookkit_shell::CODEX_BASH_PROFILE.tool_name(), "Bash");
    assert_eq!(
        hookkit_shell::ANTIGRAVITY_RUN_COMMAND_PROFILE.cwd_pointer(),
        Some("/Cwd")
    );

    let input = serde_json::json!({"request": {"command": "cat input.txt"}});
    let custom_call = call("project_shell", &input);
    let conservative = ToolAccessAnalyzer::default().analyze_call(&custom_call);
    assert!(
        !conservative
            .candidates
            .iter()
            .any(|candidate| matches!(candidate.provenance, AccessProvenance::Shell { .. }))
    );

    let profile =
        hookkit_shell::ShellToolProfile::new("project_shell", "/request/command").unwrap();
    let explicit = ToolAccessAnalyzer::default()
        .with_shell_profile(profile)
        .analyze_call(&custom_call);
    assert!(
        explicit
            .candidates
            .iter()
            .any(|candidate| matches!(candidate.provenance, AccessProvenance::Shell { .. }))
    );
}

#[test]
fn literal_shell_patch_heredoc_preserves_add_delete_and_move_provenance() {
    let command = "apply_patch <<'PATCH'\n*** Begin Patch\n*** Add File: added.txt\n+new\n*** Delete File: deleted.txt\n*** Update File: old.txt\n*** Move to: moved.txt\n*** End Patch\nPATCH\n";
    let report = ToolAccessAnalyzer::default().analyze_pre_tool(&codex_shell(command, "/repo"));

    assert_eq!(
        report
            .candidates
            .iter()
            .map(|candidate| candidate.intent)
            .collect::<Vec<_>>(),
        vec![
            AccessIntent::Modify,
            AccessIntent::Delete,
            AccessIntent::MoveSource,
            AccessIntent::MoveDestination,
        ]
    );
    assert!(report.candidates.iter().all(|candidate| matches!(
        candidate.provenance,
        AccessProvenance::ShellPatch {
            ref delimiter,
            line,
            ..
        } if delimiter == "PATCH" && line > 0
    )));
    assert!(report.is_complete());
}

#[test]
fn shell_patch_reports_malformed_dynamic_and_uncertain_cwd_cases() {
    let malformed = ToolAccessAnalyzer::default().analyze_pre_tool(&codex_shell(
        "apply_patch <<'PATCH'\nnot a patch\nPATCH\n",
        "/repo",
    ));
    assert!(
        malformed
            .gaps
            .iter()
            .any(|gap| matches!(gap.reason, ToolAccessGapReason::MalformedPatch { .. }))
    );

    let dynamic = ToolAccessAnalyzer::default().analyze_pre_tool(&codex_shell(
        "apply_patch <<PATCH\n*** Add File: $TARGET\nPATCH\n",
        "/repo",
    ));
    assert!(dynamic.gaps.iter().any(|gap| matches!(
        gap.reason,
        ToolAccessGapReason::DynamicShellPatchHereDocument { .. }
    )));

    let dynamic_delimiter = ToolAccessAnalyzer::default().analyze_pre_tool(&codex_shell(
        "apply_patch <<$DELIMITER\n*** Add File: recovered.txt\n$DELIMITER\n",
        "/repo",
    ));
    assert!(dynamic_delimiter.gaps.iter().any(|gap| matches!(
        gap.reason,
        ToolAccessGapReason::DynamicShellPatchHereDocument { .. }
    )));

    let after_cd = ToolAccessAnalyzer::default().analyze_pre_tool(&codex_shell(
        "cd subdir; apply_patch <<'PATCH'\n*** Add File: new.txt\n+new\nPATCH\n",
        "/repo",
    ));
    assert!(after_cd.gaps.iter().any(|gap| matches!(
        gap.reason,
        ToolAccessGapReason::ShellPatchWorkingDirectoryMayHaveChanged { .. }
    )));
    assert!(after_cd.candidates.iter().any(|candidate| matches!(
        &candidate.target,
        AccessTarget::Path { expression, .. }
            if expression.raw == "new.txt" && expression.resolved.is_none()
    )));
}

#[test]
fn malformed_native_shell_call_is_a_typed_gap() {
    let input = PreToolUseInput::Codex(
        serde_json::from_value(serde_json::json!({
            "session_id": "session",
            "transcript_path": null,
            "cwd": "/repo",
            "hook_event_name": "PreToolUse",
            "model": "gpt-test",
            "turn_id": "turn",
            "permission_mode": "default",
            "tool_name": "Bash",
            "tool_use_id": "call",
            "tool_input": {}
        }))
        .unwrap(),
    );
    let report = ToolAccessAnalyzer::default().analyze_pre_tool(&input);
    assert!(report.candidates.is_empty());
    assert!(matches!(
        report.gaps[0].reason,
        ToolAccessGapReason::MalformedShellCall(_)
    ));
}

#[test]
fn antigravity_post_tool_reports_the_missing_originating_call() {
    let input = PostToolUseInput::Antigravity(
        serde_json::from_value(serde_json::json!({
            "conversationId": "conversation",
            "workspacePaths": ["/repo"],
            "transcriptPath": "/tmp/transcript",
            "artifactDirectoryPath": "/tmp/artifacts",
            "stepIdx": 2
        }))
        .unwrap(),
    );
    let report = ToolAccessAnalyzer::default().analyze_post_tool(&input);
    assert!(report.candidates.is_empty());
    assert!(matches!(
        report.gaps[0].reason,
        ToolAccessGapReason::MissingToolCall
    ));
}
