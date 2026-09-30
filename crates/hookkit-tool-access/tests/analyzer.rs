use hookkit_common::{PostToolUseInput, PreToolUseInput};
use hookkit_core::{EventId, HarnessId, Utf8Path, Utf8PathBuf};
use hookkit_tool_access::{
    AccessCertainty, AccessIntent, AccessProvenance, AccessSource, AccessTarget, JsonRef,
    PatchOperation, PathBase, StructuredFieldAnalyzer, StructuredFieldMatch,
    TargetResolutionOptions, ToolAccessAnalyzer, ToolAccessGapReason, ToolCallRef, ToolPhase,
    resolve_targets,
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

fn claude_shell(command: &str, cwd: &str) -> PreToolUseInput {
    PreToolUseInput::Claude(
        serde_json::from_value(serde_json::json!({
            "session_id": "session",
            "transcript_path": "/tmp/transcript.jsonl",
            "cwd": cwd,
            "hook_event_name": "PreToolUse",
            "permission_mode": "default",
            "tool_name": "Bash",
            "tool_input": {"command": command},
            "tool_use_id": "toolu_1"
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
    // Codex sends apply_patch hook input as `{"command": "<patch>"}`.
    let input = serde_json::json!({
        "command": "*** Begin Patch\n*** Add File: src/new.rs\n+new\n*** Update File: src/lib.rs\n@@\n-old\n+new\n*** Delete File: src/old.rs\n*** Update File: src/from.rs\n*** Move to: src/to.rs\n*** End Patch"
    });
    let report = ToolAccessAnalyzer::default().analyze_call(&call("apply_patch", &input));

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
            } if pointer == "/command" && line > 0
        )
    }));
    assert!(report.is_complete());

    // Custom namespaced patch tools may still use a `patch` field.
    let custom = serde_json::json!({
        "patch": "*** Begin Patch\n*** Add File: src/new.rs\n+new\n*** End Patch"
    });
    let report = ToolAccessAnalyzer::default().analyze_call(&call("tools.apply_patch", &custom));
    assert_eq!(report.candidates.len(), 1);
    assert!(report.is_complete());
}

#[test]
fn malformed_patch_retains_partial_evidence_and_a_typed_gap() {
    let input = serde_json::json!({
        "command": "*** Add File: recovered.txt\n*** Move to: orphaned.txt"
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
    // A malformed shell heredoc patch is attributed to shell analysis, like
    // the candidates it would have produced.
    assert!(malformed.gaps.iter().any(|gap| {
        gap.source == AccessSource::Shell
            && matches!(gap.reason, ToolAccessGapReason::MalformedPatch { .. })
    }));

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
fn antigravity_post_tool_analyzes_the_originating_call() {
    let input = PostToolUseInput::Antigravity(
        serde_json::from_value(serde_json::json!({
            "conversationId": "conversation",
            "workspacePaths": ["/repo"],
            "transcriptPath": "/tmp/transcript",
            "artifactDirectoryPath": "/tmp/artifacts",
            "toolCall": {
                "name": "run_command",
                "args": {
                    "CommandLine": "printf generated > src/generated.txt",
                    "Cwd": "/repo"
                }
            },
            "stepIdx": 2
        }))
        .unwrap(),
    );
    let report = ToolAccessAnalyzer::default().analyze_post_tool(&input);
    assert!(report.may_modify().any(|candidate| {
        candidate.intent == AccessIntent::Modify
            && matches!(
                &candidate.target,
                AccessTarget::Path { expression, .. }
                    if expression.resolved.as_deref() == Some(Utf8Path::new("/repo/src/generated.txt"))
            )
    }));
}

/// Resolved path, base, and certainty of every shell-patch candidate.
fn shell_patch_targets(
    report: &hookkit_tool_access::ToolAccessReport,
) -> Vec<(Option<&str>, PathBase, AccessCertainty)> {
    report
        .candidates
        .iter()
        .filter(|candidate| {
            matches!(
                candidate.provenance,
                AccessProvenance::ShellPatch { .. } | AccessProvenance::ShellPatchArgument { .. }
            )
        })
        .map(|candidate| match &candidate.target {
            AccessTarget::Path { expression, .. } => (
                expression.resolved.as_deref().map(Utf8Path::as_str),
                expression.base,
                candidate.certainty,
            ),
            other => panic!("unexpected target {other:?}"),
        })
        .collect()
}

fn has_gap(
    report: &hookkit_tool_access::ToolAccessReport,
    predicate: impl Fn(&ToolAccessGapReason) -> bool,
) -> bool {
    report.gaps.iter().any(|gap| predicate(&gap.reason))
}

fn directory_change_gap(reason: &ToolAccessGapReason) -> bool {
    matches!(
        reason,
        ToolAccessGapReason::ShellPatchWorkingDirectoryMayHaveChanged { .. }
    )
}

fn missing_heredoc_gap(reason: &ToolAccessGapReason) -> bool {
    matches!(
        reason,
        ToolAccessGapReason::MissingShellPatchHereDocument { .. }
    )
}

#[test]
fn codex_cd_and_apply_patch_resolves_against_the_literal_directory() {
    const PATCH: &str = "<<'PATCH'\n*** Begin Patch\n*** Add File: src/new.rs\n+fn main() {}\n*** End Patch\nPATCH\n";
    let cases = [
        // Codex intercepts this form and applies the patch in cwd/<dir>.
        (
            format!("cd crates/core && apply_patch {PATCH}"),
            "/repo/crates/core/src/new.rs",
            PathBase::SessionCwd,
        ),
        (
            format!("cd a && cd ../b && apply_patch {PATCH}"),
            "/repo/b/src/new.rs",
            PathBase::SessionCwd,
        ),
        (
            format!("cd -- a && \\\n  apply_patch {PATCH}"),
            "/repo/a/src/new.rs",
            PathBase::SessionCwd,
        ),
        // An absolute operand is exact however the hook cwd is labeled.
        (
            format!("cd /work/other && apply_patch {PATCH}"),
            "/work/other/src/new.rs",
            PathBase::InvocationCwd,
        ),
        // Statement-level here-documents reach the enclosed command.
        (
            format!("(cd crates/core && apply_patch) {PATCH}"),
            "/repo/crates/core/src/new.rs",
            PathBase::SessionCwd,
        ),
        (
            format!("{{ apply_patch; }} {PATCH}"),
            "/repo/src/new.rs",
            PathBase::SessionCwd,
        ),
    ];
    for (command, resolved, base) in cases {
        let report =
            ToolAccessAnalyzer::default().analyze_pre_tool(&codex_shell(&command, "/repo"));
        assert_eq!(
            shell_patch_targets(&report),
            vec![(Some(resolved), base, AccessCertainty::Direct)],
            "{command}: {report:#?}"
        );
        assert!(
            !has_gap(&report, directory_change_gap),
            "{command}: {report:#?}"
        );
        assert!(
            !has_gap(&report, missing_heredoc_gap),
            "{command}: {report:#?}"
        );
    }
}

#[test]
fn uncertain_shell_patch_directories_and_inputs_stay_uncertain() {
    const PATCH: &str =
        "<<'PATCH'\n*** Begin Patch\n*** Add File: new.rs\n+x\n*** End Patch\nPATCH\n";
    // A non-literal `cd` operand cannot be resolved: heuristic and a gap.
    for command in [
        format!("cd \"$TARGET\" && apply_patch {PATCH}"),
        format!("cd ~/project && apply_patch {PATCH}"),
        format!("cd && apply_patch {PATCH}"),
        format!("cd - && apply_patch {PATCH}"),
    ] {
        let report =
            ToolAccessAnalyzer::default().analyze_pre_tool(&codex_shell(&command, "/repo"));
        assert_eq!(
            shell_patch_targets(&report),
            vec![(
                None,
                PathBase::UnknownAfterDirectoryChange,
                AccessCertainty::Heuristic
            )],
            "{command}: {report:#?}"
        );
        assert!(
            has_gap(&report, directory_change_gap),
            "{command}: {report:#?}"
        );
    }

    // Other directory changes keep direct but unresolved evidence.
    for command in [
        format!("cd a; apply_patch {PATCH}"),
        format!("pushd a && apply_patch {PATCH}"),
        format!("(cd a) && apply_patch {PATCH}"),
        format!("cd \"$B\"; cd c && apply_patch {PATCH}"),
    ] {
        let report =
            ToolAccessAnalyzer::default().analyze_pre_tool(&codex_shell(&command, "/repo"));
        assert_eq!(
            shell_patch_targets(&report),
            vec![(
                None,
                PathBase::UnknownAfterDirectoryChange,
                AccessCertainty::Direct
            )],
            "{command}: {report:#?}"
        );
        assert!(
            has_gap(&report, directory_change_gap),
            "{command}: {report:#?}"
        );
    }

    // Another command of the group may consume the here-document first.
    let shared = ToolAccessAnalyzer::default().analyze_pre_tool(&codex_shell(
        &format!("{{ cat; apply_patch; }} {PATCH}"),
        "/repo",
    ));
    assert_eq!(
        shell_patch_targets(&shared),
        vec![(
            Some("/repo/new.rs"),
            PathBase::SessionCwd,
            AccessCertainty::Heuristic
        )],
        "{shared:#?}"
    );

    // A command's own input redirection or a pipe takes precedence over the
    // statement's here-document.
    for command in [
        format!("{{ apply_patch < patch.txt; }} {PATCH}"),
        format!("{{ printf x | apply_patch; }} {PATCH}"),
    ] {
        let report =
            ToolAccessAnalyzer::default().analyze_pre_tool(&codex_shell(&command, "/repo"));
        assert!(
            shell_patch_targets(&report).is_empty(),
            "{command}: {report:#?}"
        );
        assert!(
            has_gap(&report, missing_heredoc_gap),
            "{command}: {report:#?}"
        );
    }
}

fn dynamic_heredoc_gap(reason: &ToolAccessGapReason) -> bool {
    matches!(
        reason,
        ToolAccessGapReason::DynamicShellPatchHereDocument { .. }
    )
}

fn analyze(input: &PreToolUseInput) -> hookkit_tool_access::ToolAccessReport {
    ToolAccessAnalyzer::default().analyze_pre_tool(input)
}

/// Resolved path and intent of every shell-patch candidate.
fn shell_patch_paths(
    report: &hookkit_tool_access::ToolAccessReport,
) -> Vec<(Option<&str>, AccessIntent)> {
    report
        .candidates
        .iter()
        .filter(|candidate| {
            matches!(
                candidate.provenance,
                AccessProvenance::ShellPatch { .. } | AccessProvenance::ShellPatchArgument { .. }
            )
        })
        .map(|candidate| match &candidate.target {
            AccessTarget::Path { expression, .. } => (
                expression.resolved.as_deref().map(Utf8Path::as_str),
                candidate.intent,
            ),
            other => panic!("unexpected target {other:?}"),
        })
        .collect()
}

#[test]
fn wrapped_shell_patches_are_analyzed_like_bare_ones() {
    const PATCH: &str =
        "<<'EOF'\n*** Begin Patch\n*** Update File: .env\n@@\n-A=1\n+A=2\n*** End Patch\nEOF\n";
    // Bash runs the `apply_patch` executable behind each wrapper, which reads
    // the here-document; Codex intercepts only the bare forms.
    for prefix in [
        "command ",
        "exec ",
        "env ",
        "env FOO=1 ",
        "nohup ",
        "nice -n 5 ",
        "time ",
        "sudo ",
        "sudo -u root ",
        "builtin ",
        "timeout 5 ",
        "stdbuf -o0 ",
        "/usr/bin/env ",
        "sudo env PATH=/opt/apply_patch nice ",
    ] {
        let command = format!("{prefix}apply_patch {PATCH}");
        for input in [
            claude_shell(&command, "/repo"),
            codex_shell(&command, "/repo"),
        ] {
            let report = analyze(&input);
            assert_eq!(
                shell_patch_paths(&report),
                vec![(Some("/repo/.env"), AccessIntent::ReadModify)],
                "{command}: {report:#?}"
            );
            assert!(
                !has_gap(&report, missing_heredoc_gap),
                "{command}: {report:#?}"
            );
        }
    }

    // `command -v` only looks the command up.
    let lookup = analyze(&claude_shell("command -v apply_patch", "/repo"));
    assert!(
        lookup.candidates.is_empty() && lookup.is_complete(),
        "{lookup:#?}"
    );

    // `env -C` runs the wrapped patch in another directory.
    let elsewhere = analyze(&claude_shell(
        &format!("env -C secrets apply_patch {PATCH}"),
        "/repo",
    ));
    assert_eq!(
        shell_patch_targets(&elsewhere),
        vec![(
            None,
            PathBase::UnknownAfterDirectoryChange,
            AccessCertainty::Direct
        )],
        "{elsewhere:#?}"
    );
    assert!(has_gap(&elsewhere, directory_change_gap), "{elsewhere:#?}");
}

#[test]
fn dynamic_shell_patch_bodies_are_exact_only_when_codex_intercepts_them() {
    const BODY: &str =
        "*** Begin Patch\n*** Update File: .env\n@@\n-TOKEN=$OLD\n+TOKEN=new\n*** End Patch";
    let bare = format!("apply_patch <<EOF\n{BODY}\nEOF\n");

    // Codex applies these whole-script forms itself, without expansion.
    for (command, resolved) in [
        (bare.clone(), "/repo/.env"),
        (
            format!("cd sub && apply_patch <<EOF\n{BODY}\nEOF\n"),
            "/repo/sub/.env",
        ),
    ] {
        let report = analyze(&codex_shell(&command, "/repo"));
        assert_eq!(
            shell_patch_paths(&report),
            vec![(Some(resolved), AccessIntent::ReadModify)],
            "{command}: {report:#?}"
        );
        assert!(report.is_complete(), "{command}: {report:#?}");
    }

    // Any other script runs in Bash, which expands the body first: literal
    // headers are recovered, but the expansion stays a gap.
    for input in [
        claude_shell(&bare, "/repo"),
        codex_shell(&format!("{bare}echo done\n"), "/repo"),
        codex_shell(&format!("# edit\n{bare}"), "/repo"),
        codex_shell(
            &format!("apply_patch <<EOF && echo done\n{BODY}\nEOF\n"),
            "/repo",
        ),
        codex_shell(&format!("(apply_patch) <<EOF\n{BODY}\nEOF\n"), "/repo"),
    ] {
        let report = analyze(&input);
        assert_eq!(
            shell_patch_paths(&report),
            vec![(Some("/repo/.env"), AccessIntent::ReadModify)],
            "{input:?}: {report:#?}"
        );
        assert!(
            has_gap(&report, dynamic_heredoc_gap),
            "{input:?}: {report:#?}"
        );
    }

    // An expansion can inject headers that no literal line shows.
    let injected = analyze(&codex_shell(
        "apply_patch <<EOF\n*** Begin Patch\n*** Add File: a.txt\n+$(printf '\\n*** Delete File: important.rs')\n*** End Patch\nEOF\necho done\n",
        "/repo",
    ));
    assert_eq!(
        shell_patch_paths(&injected),
        vec![(Some("/repo/a.txt"), AccessIntent::Modify)],
        "{injected:#?}"
    );
    assert!(has_gap(&injected, dynamic_heredoc_gap), "{injected:#?}");

    // `<<-` strips leading tabs before `apply_patch` reads the body, so a
    // tab-indented header inside an update hunk is a header.
    let stripped = analyze(&claude_shell(
        "apply_patch <<-EOF\n\t*** Begin Patch\n\t*** Update File: a.rs\n\t@@\n\t-$OLD\n\t+new\n\t*** Delete File: important.rs\n\t*** End Patch\n\tEOF\necho done\n",
        "/repo",
    ));
    assert_eq!(
        shell_patch_paths(&stripped),
        vec![
            (Some("/repo/a.rs"), AccessIntent::ReadModify),
            (Some("/repo/important.rs"), AccessIntent::Delete),
        ],
        "{stripped:#?}"
    );
    assert!(has_gap(&stripped, dynamic_heredoc_gap), "{stripped:#?}");
}

#[test]
fn only_the_final_standard_input_supplies_a_shell_patch() {
    const DECOY: &str = "*** Begin Patch\n*** Add File: decoy.txt\n+x\n*** End Patch\nEOF\n";
    // The patch comes from a file or pipe, or the here-document is on
    // another descriptor.
    for command in [
        format!("apply_patch <<'EOF' < evil.patch\n{DECOY}"),
        format!("cat evil.patch | apply_patch 3<<'EOF'\n{DECOY}"),
        format!("apply_patch 3<<'EOF' < evil.patch\n{DECOY}"),
        format!("apply_patch 3<<'EOF'\n{DECOY}"),
    ] {
        for input in [
            claude_shell(&command, "/repo"),
            codex_shell(&command, "/repo"),
        ] {
            let report = analyze(&input);
            assert!(
                shell_patch_targets(&report).is_empty(),
                "{command}: {report:#?}"
            );
            assert!(
                has_gap(&report, missing_heredoc_gap),
                "{command}: {report:#?}"
            );
        }
    }

    // A read-write `<>` (whose default descriptor is 0), a closed standard
    // input, or an inner statement's input redirection also replaces the
    // here-document. hookkit-shell reports `<>` as `ReadWrite`; after a
    // here-document the grammar reduces `0<>` to an output `>`.
    for command in [
        format!("apply_patch <<'EOF' <> evil.patch\n{DECOY}"),
        format!("apply_patch <<'EOF' 0<> evil.patch\n{DECOY}"),
        format!("apply_patch <<'EOF' 0< evil.patch\n{DECOY}"),
        format!("apply_patch <<'EOF' <&-\n{DECOY}"),
        format!("{{ (apply_patch) < evil.patch; }} <<'EOF'\n{DECOY}"),
        format!("{{ apply_patch; }} <<'EOF' <> evil.patch\n{DECOY}"),
    ] {
        let report = analyze(&claude_shell(&command, "/repo"));
        assert!(
            shell_patch_targets(&report).is_empty(),
            "{command}: {report:#?}"
        );
        assert!(
            has_gap(&report, missing_heredoc_gap),
            "{command}: {report:#?}"
        );
    }

    // The last standard-input redirection wins, and a here-document
    // replaces piped input.
    for command in [
        format!("apply_patch < evil.patch <<'EOF'\n{DECOY}"),
        format!("apply_patch <> evil.patch <<'EOF'\n{DECOY}"),
        format!("apply_patch 0<> evil.patch <<'EOF'\n{DECOY}"),
        format!("apply_patch <<'EOF' 3<> evil.patch\n{DECOY}"),
        format!("cat evil.patch | apply_patch <<'EOF'\n{DECOY}"),
    ] {
        let report = analyze(&claude_shell(&command, "/repo"));
        assert_eq!(
            shell_patch_paths(&report),
            vec![(Some("/repo/decoy.txt"), AccessIntent::Modify)],
            "{command}: {report:#?}"
        );
        assert!(
            !has_gap(&report, missing_heredoc_gap),
            "{command}: {report:#?}"
        );
    }
}

#[test]
fn a_patch_argument_takes_precedence_over_standard_input() {
    const PATCH: &str = "'*** Begin Patch\n*** Delete File: important.rs\n*** End Patch'";
    const DECOY: &str =
        "<<'EOF'\n*** Begin Patch\n*** Add File: decoy.txt\n+x\n*** End Patch\nEOF\n";
    // The standalone `apply_patch` executable reads its argument and never
    // reads standard input.
    for command in [
        format!("apply_patch {PATCH} {DECOY}"),
        format!("{{ apply_patch {PATCH}; }} {DECOY}"),
        format!("command apply_patch {PATCH} {DECOY}"),
    ] {
        for input in [
            claude_shell(&command, "/repo"),
            codex_shell(&command, "/repo"),
        ] {
            let report = analyze(&input);
            assert_eq!(
                shell_patch_paths(&report),
                vec![(Some("/repo/important.rs"), AccessIntent::Delete)],
                "{command}: {report:#?}"
            );
            assert!(
                matches!(
                    report.candidates[0].provenance,
                    AccessProvenance::ShellPatchArgument { line: 2, .. }
                ),
                "{command}: {report:#?}"
            );
            assert!(report.is_complete(), "{command}: {report:#?}");
        }
    }

    // A dynamic or extra argument hides the patch.
    for command in [
        format!("apply_patch \"$(cat evil.patch)\" {DECOY}"),
        format!("apply_patch a b {DECOY}"),
    ] {
        let report = analyze(&claude_shell(&command, "/repo"));
        assert!(
            shell_patch_targets(&report).is_empty(),
            "{command}: {report:#?}"
        );
        assert!(
            has_gap(&report, missing_heredoc_gap),
            "{command}: {report:#?}"
        );
    }

    // Codex applies the here-document of its intercepted `cd` form even when
    // `apply_patch` has arguments; Bash runs the executable, which reads them.
    let command = format!("cd sub && apply_patch ignored {DECOY}");
    let intercepted = analyze(&codex_shell(&command, "/repo"));
    assert_eq!(
        shell_patch_paths(&intercepted),
        vec![(Some("/repo/sub/decoy.txt"), AccessIntent::Modify)],
        "{intercepted:#?}"
    );
    let executed = analyze(&claude_shell(&command, "/repo"));
    assert!(shell_patch_paths(&executed).is_empty(), "{executed:#?}");
    assert!(
        has_gap(&executed, |reason| matches!(
            reason,
            ToolAccessGapReason::MalformedPatch { .. }
        )),
        "{executed:#?}"
    );
}

#[test]
fn shell_patch_directory_changes_bash_may_skip_or_redirect_stay_uncertain() {
    const PATCH: &str =
        "<<'EOF'\n*** Begin Patch\n*** Update File: key.pem\n@@\n-a\n+b\n*** End Patch\nEOF\n";
    // Wrapped, negated, skipped, piped, repeated, or deferred directory
    // changes leave the directory unknown.
    for command in [
        format!("builtin cd secrets && apply_patch {PATCH}"),
        format!("command cd secrets && apply_patch {PATCH}"),
        format!("! cd nonexistent && apply_patch {PATCH}"),
        format!("true || cd secrets && apply_patch {PATCH}"),
        format!("printf x | cd secrets && apply_patch {PATCH}"),
        format!("for i in 1 2; do cd secrets && apply_patch {PATCH}done\n"),
        format!("for i in 1 2; do apply_patch {PATCH}cd secrets; done\n"),
        format!("f() {{ apply_patch {PATCH}}}; cd secrets; f\n"),
    ] {
        for input in [
            claude_shell(&command, "/repo"),
            codex_shell(&command, "/repo"),
        ] {
            let report = analyze(&input);
            assert_eq!(
                shell_patch_targets(&report),
                vec![(
                    None,
                    PathBase::UnknownAfterDirectoryChange,
                    AccessCertainty::Direct
                )],
                "{command}: {report:#?}"
            );
            assert!(
                has_gap(&report, directory_change_gap),
                "{command}: {report:#?}"
            );
        }
    }

    // `CDPATH` or `cdable_vars` set in the script can send `cd` elsewhere.
    for command in [
        format!("export CDPATH=/repo/private; cd config && apply_patch {PATCH}"),
        format!("CDPATH=/repo/private cd config && apply_patch {PATCH}"),
        format!("export CD\"PATH\"=/repo/private; cd config && apply_patch {PATCH}"),
        format!("shopt -s cdable_vars; config=/repo/private; cd config && apply_patch {PATCH}"),
    ] {
        let report = analyze(&claude_shell(&command, "/repo"));
        assert_eq!(
            shell_patch_targets(&report),
            vec![(
                None,
                PathBase::UnknownAfterDirectoryChange,
                AccessCertainty::Heuristic
            )],
            "{command}: {report:#?}"
        );
        assert!(
            has_gap(&report, directory_change_gap),
            "{command}: {report:#?}"
        );
    }

    // Codex joins the operand of its intercepted form itself, so a body
    // that merely mentions `CDPATH` does not matter there; Bash may still
    // consult it.
    let mention = "cd sub && apply_patch <<'EOF'\n*** Begin Patch\n*** Add File: notes.md\n+Set CDPATH with care.\n*** End Patch\nEOF\n";
    let intercepted = analyze(&codex_shell(mention, "/repo"));
    assert_eq!(
        shell_patch_targets(&intercepted),
        vec![(
            Some("/repo/sub/notes.md"),
            PathBase::SessionCwd,
            AccessCertainty::Direct
        )],
        "{intercepted:#?}"
    );
    let executed = analyze(&claude_shell(mention, "/repo"));
    assert_eq!(
        shell_patch_targets(&executed),
        vec![(
            None,
            PathBase::UnknownAfterDirectoryChange,
            AccessCertainty::Heuristic
        )],
        "{executed:#?}"
    );

    // A chain its list always runs, or a later directory change, still
    // resolves.
    for (command, resolved) in [
        (
            format!("true && cd a && apply_patch {PATCH}"),
            "/repo/a/key.pem",
        ),
        (
            format!("if cd a && apply_patch {PATCH}then :; fi\n"),
            "/repo/a/key.pem",
        ),
        (format!("apply_patch {PATCH}cd secrets\n"), "/repo/key.pem"),
    ] {
        let report = analyze(&claude_shell(&command, "/repo"));
        assert_eq!(
            shell_patch_targets(&report),
            vec![(
                Some(resolved),
                PathBase::InvocationCwd,
                AccessCertainty::Direct
            )],
            "{command}: {report:#?}"
        );
        assert!(
            !has_gap(&report, directory_change_gap),
            "{command}: {report:#?}"
        );
    }
}
