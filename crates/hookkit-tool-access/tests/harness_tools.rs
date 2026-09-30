//! Regression tests driven by documented native payload shapes and the
//! checked-in contract fixtures, so fixture drift and invented shapes are
//! caught here rather than in production.

use hookkit_common::{PostToolUseInput, PreToolUseInput};
use hookkit_core::{EventId, HarnessId, Utf8Path, Utf8PathBuf};
use hookkit_tool_access::{
    AccessCandidate, AccessCertainty, AccessIntent, AccessProvenance, AccessScope, AccessTarget,
    JsonRef, PathBase, StructuredFieldMatch, ToolAccessAnalyzer, ToolAccessGapReason,
    ToolAccessReport, ToolCallObservation, ToolCallRef, ToolPhase,
};
use std::path::PathBuf;

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Loads one positive input fixture from a checked-in contract snapshot.
fn contract_fixture(event: &str, id: &str) -> serde_json::Value {
    let path = repository_root()
        .join("contracts/harnesses")
        .join(event)
        .join("fixtures.yaml");
    let text = std::fs::read_to_string(&path).unwrap();
    let document: serde_json::Value = serde_yaml_ng::from_str(&text).unwrap();
    document["input"]["positive"]
        .as_array()
        .unwrap()
        .iter()
        .find(|fixture| fixture["id"] == id)
        .unwrap_or_else(|| panic!("{} has no fixture `{id}`", path.display()))["value"]
        .clone()
}

fn codex_pre(tool_name: &str, tool_input: serde_json::Value) -> PreToolUseInput {
    PreToolUseInput::Codex(
        serde_json::from_value(serde_json::json!({
            "session_id": "session",
            "transcript_path": null,
            "cwd": "/repo",
            "hook_event_name": "PreToolUse",
            "model": "gpt-test",
            "turn_id": "turn",
            "permission_mode": "default",
            "tool_name": tool_name,
            "tool_use_id": "call",
            "tool_input": tool_input
        }))
        .unwrap(),
    )
}

fn claude_pre(tool_name: &str, tool_input: serde_json::Value) -> PreToolUseInput {
    PreToolUseInput::Claude(
        serde_json::from_value(serde_json::json!({
            "session_id": "session",
            "transcript_path": "/tmp/transcript.jsonl",
            "cwd": "/repo",
            "hook_event_name": "PreToolUse",
            "permission_mode": "default",
            "tool_name": tool_name,
            "tool_input": tool_input,
            "tool_use_id": "toolu_1"
        }))
        .unwrap(),
    )
}

fn antigravity_pre(tool_name: &str, args: serde_json::Value) -> PreToolUseInput {
    PreToolUseInput::Antigravity(
        serde_json::from_value(serde_json::json!({
            "conversationId": "conversation",
            "workspacePaths": ["/repo"],
            "transcriptPath": "/tmp/transcript.jsonl",
            "artifactDirectoryPath": "/tmp/artifacts",
            "toolCall": {"name": tool_name, "args": args},
            "stepIdx": 1
        }))
        .unwrap(),
    )
}

fn analyze(input: &PreToolUseInput) -> ToolAccessReport {
    ToolAccessAnalyzer::default().analyze_pre_tool(input)
}

fn path_of(candidate: &AccessCandidate) -> (&str, Option<&Utf8Path>, AccessScope, PathBase) {
    match &candidate.target {
        AccessTarget::Path { expression, scope } => (
            expression.raw.as_str(),
            expression.resolved.as_deref(),
            *scope,
            expression.base,
        ),
        other => panic!("unexpected target {other:?}"),
    }
}

/// Asserts a complete report with one candidate per `(resolved, intent, scope)`.
fn assert_targets(report: &ToolAccessReport, expected: &[(&str, AccessIntent, AccessScope)]) {
    assert!(report.is_complete(), "incomplete report: {report:#?}");
    let actual = report
        .candidates
        .iter()
        .map(|candidate| {
            let (_, resolved, scope, _) = path_of(candidate);
            (resolved.unwrap().as_str(), candidate.intent, scope)
        })
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn codex_contract_apply_patch_fixture_reads_the_command_payload() {
    let value = contract_fixture(
        "codex/snapshots/commit-ff6aec9-r1/events/pre-tool-use",
        "representative",
    );
    assert_eq!(value["tool_name"], "apply_patch");
    let input = PreToolUseInput::Codex(serde_json::from_value(value).unwrap());
    let report = analyze(&input);
    // The fixture's patch is only an envelope: the payload is found at
    // `/command`, and the missing file headers are the only gap.
    assert!(report.candidates.is_empty());
    assert!(
        !report
            .gaps
            .iter()
            .any(|gap| matches!(gap.reason, ToolAccessGapReason::MissingPatchPayload { .. }))
    );
    assert!(
        report
            .gaps
            .iter()
            .all(|gap| matches!(gap.reason, ToolAccessGapReason::MalformedPatch { .. }))
    );
}

#[test]
fn codex_apply_patch_command_payload_yields_direct_patch_evidence() {
    let report = analyze(&codex_pre(
        "apply_patch",
        serde_json::json!({
            "command": "*** Begin Patch\n*** Update File: .env\n@@\n-TOKEN=$OLD\n+TOKEN=new\n*** End Patch\n"
        }),
    ));
    assert_targets(
        &report,
        &[("/repo/.env", AccessIntent::ReadModify, AccessScope::Exact)],
    );
    assert!(matches!(
        &report.candidates[0].provenance,
        AccessProvenance::Patch { payload_pointer, .. } if payload_pointer == "/command"
    ));
    assert_eq!(path_of(&report.candidates[0]).3, PathBase::InvocationCwd);

    // Codex's lenient mode unwraps a literal heredoc wrapper.
    let wrapped = analyze(&codex_pre(
        "apply_patch",
        serde_json::json!({
            "command": "<<'EOF'\n*** Begin Patch\n*** Add File: src/new.rs\n+new\n*** End Patch\nEOF\n"
        }),
    ));
    assert_targets(
        &wrapped,
        &[("/repo/src/new.rs", AccessIntent::Modify, AccessScope::Exact)],
    );
}

#[test]
fn codex_patch_grammar_treats_hunk_lines_as_content() {
    let report = analyze(&codex_pre(
        "apply_patch",
        serde_json::json!({
            "command": "*** Begin Patch\n*** Update File: db/schema.sql\n@@\n--- legacy\n+++ note\n-- drop legacy table\n context\n*** Add File: notes.md\n+++ heading\n+-- dash\n  *** Add File: .env\n+SECRET=1\n*** End Patch"
        }),
    ));
    // Indented headers outside an update hunk are accepted, as in Codex.
    assert_targets(
        &report,
        &[
            (
                "/repo/db/schema.sql",
                AccessIntent::ReadModify,
                AccessScope::Exact,
            ),
            ("/repo/notes.md", AccessIntent::Modify, AccessScope::Exact),
            ("/repo/.env", AccessIntent::Modify, AccessScope::Exact),
        ],
    );
}

#[test]
fn unified_diff_hunk_counts_protect_content_lines() {
    let input = serde_json::json!({
        "input": "--- a/db.sql\n+++ b/db.sql\n@@ -1,2 +1,2 @@\n--- drop legacy table\n+++ heading\n context\n"
    });
    let call = ToolCallRef::new(
        EventId::builtin(HarnessId::CLAUDE_CODE, "PreToolUse"),
        ToolPhase::Pre,
        "tools.apply_patch",
        JsonRef::Value(&input),
        Some(Utf8Path::new("/repo")),
        vec![Utf8PathBuf::from("/repo")],
    );
    let report = ToolAccessAnalyzer::default().analyze_call(&call);
    assert_targets(
        &report,
        &[("/repo/db.sql", AccessIntent::ReadModify, AccessScope::Exact)],
    );
}

#[test]
fn patch_environment_ids_leave_paths_unresolved() {
    let report = analyze(&codex_pre(
        "apply_patch",
        serde_json::json!({
            "command": "*** Begin Patch\n*** Environment ID: remote-1\n*** Add File: src/new.rs\n+new\n*** End Patch"
        }),
    ));
    assert_eq!(report.candidates.len(), 1);
    let (raw, resolved, _, base) = path_of(&report.candidates[0]);
    assert_eq!(
        (raw, resolved, base),
        ("src/new.rs", None, PathBase::UnknownEnvironment)
    );
    assert!(report.gaps.iter().any(|gap| matches!(
        &gap.reason,
        ToolAccessGapReason::UnknownExecutionEnvironment { environment_id } if environment_id == "remote-1"
    )));
}

#[test]
fn shell_patch_alias_and_dynamic_hunks_recover_literal_headers() {
    let alias = analyze(&codex_pre(
        "Bash",
        serde_json::json!({
            "command": "applypatch <<'EOF'\n*** Begin Patch\n*** Add File: added.txt\n+new\n*** End Patch\nEOF\n"
        }),
    ));
    assert!(alias.candidates.iter().any(|candidate| {
        candidate.intent == AccessIntent::Modify
            && path_of(candidate).1 == Some(Utf8Path::new("/repo/added.txt"))
    }));

    // Only the hunk is dynamic; Codex applies the body verbatim.
    let dynamic = analyze(&codex_pre(
        "Bash",
        serde_json::json!({
            "command": "apply_patch <<EOF\n*** Begin Patch\n*** Update File: .env\n@@\n-TOKEN=$OLD\n+TOKEN=new\n*** End Patch\nEOF\n"
        }),
    ));
    assert!(dynamic.candidates.iter().any(|candidate| {
        candidate.intent == AccessIntent::ReadModify
            && path_of(candidate).1 == Some(Utf8Path::new("/repo/.env"))
    }));
    assert!(!dynamic.gaps.iter().any(|gap| matches!(
        gap.reason,
        ToolAccessGapReason::DynamicShellPatchHereDocument { .. }
    )));
}

#[test]
fn codex_shell_paths_are_labeled_with_the_session_cwd() {
    let report = analyze(&codex_pre(
        "Bash",
        serde_json::json!({"command": "rm -rf generated"}),
    ));
    let (_, resolved, _, base) = path_of(&report.candidates[0]);
    assert_eq!(resolved, Some(Utf8Path::new("/repo/generated")));
    assert_eq!(base, PathBase::SessionCwd);

    let claude = analyze(&claude_pre(
        "Bash",
        serde_json::json!({"command": "rm -rf generated"}),
    ));
    assert_eq!(path_of(&claude.candidates[0]).3, PathBase::InvocationCwd);

    let explicit_cwd = analyze(&antigravity_pre(
        "run_command",
        serde_json::json!({"CommandLine": "rm -rf generated", "Cwd": "/repo"}),
    ));
    assert_eq!(
        path_of(&explicit_cwd.candidates[0]).3,
        PathBase::InvocationCwd
    );
    let fallback_cwd = analyze(&antigravity_pre(
        "run_command",
        serde_json::json!({"CommandLine": "rm -rf generated"}),
    ));
    assert_eq!(path_of(&fallback_cwd.candidates[0]).3, PathBase::SessionCwd);
}

#[test]
fn antigravity_documented_file_tools_use_exact_pascal_case_arguments() {
    let cases = [
        (
            "view_file",
            serde_json::json!({"AbsolutePath": "/repo/.env", "StartLine": 1}),
            vec![("/repo/.env", AccessIntent::Read, AccessScope::Exact)],
        ),
        (
            "write_to_file",
            serde_json::json!({"TargetFile": "/repo/.env", "CodeContent": "x", "Overwrite": true}),
            vec![("/repo/.env", AccessIntent::Modify, AccessScope::Exact)],
        ),
        (
            "replace_file_content",
            serde_json::json!({"TargetFile": "/repo/src/lib.rs", "TargetContent": "a"}),
            vec![(
                "/repo/src/lib.rs",
                AccessIntent::ReadModify,
                AccessScope::Exact,
            )],
        ),
        (
            "multi_replace_file_content",
            serde_json::json!({"TargetFile": "/repo/src/lib.rs", "ReplacementChunks": []}),
            vec![(
                "/repo/src/lib.rs",
                AccessIntent::ReadModify,
                AccessScope::Exact,
            )],
        ),
        (
            "list_dir",
            serde_json::json!({"DirectoryPath": "/repo/src"}),
            vec![("/repo/src", AccessIntent::Enumerate, AccessScope::Exact)],
        ),
        (
            "find_by_name",
            serde_json::json!({"SearchDirectory": "/repo", "Pattern": "*.env"}),
            vec![
                ("/repo", AccessIntent::Enumerate, AccessScope::Exact),
                ("/repo/**/*.env", AccessIntent::Enumerate, AccessScope::Glob),
            ],
        ),
        (
            "grep_search",
            serde_json::json!({"SearchPath": "/repo", "Query": "KEY"}),
            vec![("/repo", AccessIntent::Read, AccessScope::ExactOrDescendants)],
        ),
        (
            "grep_search",
            serde_json::json!({"SearchPath": "/repo", "Query": "KEY", "Includes": ["*.rs", "src/**"]}),
            vec![
                ("/repo", AccessIntent::Read, AccessScope::Exact),
                ("/repo/**/*.rs", AccessIntent::Read, AccessScope::Glob),
                ("/repo/src/**", AccessIntent::Read, AccessScope::Glob),
            ],
        ),
        (
            "generate_image",
            serde_json::json!({"Prompt": "p", "ImageName": "i", "ImagePaths": ["/repo/in.png"]}),
            vec![("/repo/in.png", AccessIntent::Read, AccessScope::Exact)],
        ),
    ];
    for (tool, args, expected) in cases {
        let report = analyze(&antigravity_pre(tool, args));
        assert_targets(&report, &expected);
        assert!(report.candidates.iter().all(|candidate| matches!(
            candidate.provenance,
            AccessProvenance::StructuredField {
                matched_by: StructuredFieldMatch::BuiltinTool,
                ..
            }
        )));
    }

    // A relative argument is resolved against the first workspace root,
    // which is a session directory rather than an invocation directory.
    let relative = analyze(&antigravity_pre(
        "view_file",
        serde_json::json!({"AbsolutePath": "src/lib.rs"}),
    ));
    assert_eq!(path_of(&relative.candidates[0]).3, PathBase::SessionCwd);

    // The same PascalCase key on an unrelated tool is not a path.
    let unrelated = analyze(&antigravity_pre(
        "mcp_notes_create",
        serde_json::json!({"TargetFile": "/repo/.env"}),
    ));
    assert!(unrelated.candidates.is_empty());
}

#[test]
fn claude_builtin_file_tools_have_exact_roles_and_search_scopes() {
    let cases = [
        (
            "Read",
            serde_json::json!({"file_path": "/repo/.env"}),
            vec![("/repo/.env", AccessIntent::Read, AccessScope::Exact)],
        ),
        (
            "Write",
            serde_json::json!({"file_path": "/repo/a.rs", "content": "x"}),
            vec![("/repo/a.rs", AccessIntent::Modify, AccessScope::Exact)],
        ),
        (
            "Edit",
            serde_json::json!({"file_path": "/repo/a.rs", "old_string": "a", "new_string": "b"}),
            vec![("/repo/a.rs", AccessIntent::ReadModify, AccessScope::Exact)],
        ),
        (
            "MultiEdit",
            serde_json::json!({"file_path": "/repo/a.rs", "edits": []}),
            vec![("/repo/a.rs", AccessIntent::ReadModify, AccessScope::Exact)],
        ),
        (
            "NotebookEdit",
            serde_json::json!({"notebook_path": "/repo/a.ipynb", "new_source": "x"}),
            vec![(
                "/repo/a.ipynb",
                AccessIntent::ReadModify,
                AccessScope::Exact,
            )],
        ),
        (
            "Grep",
            serde_json::json!({"pattern": "API_KEY", "output_mode": "content"}),
            vec![("/repo", AccessIntent::Read, AccessScope::ExactOrDescendants)],
        ),
        (
            "Grep",
            serde_json::json!({"pattern": "API_KEY", "path": "/repo/src", "glob": ".env*"}),
            vec![
                ("/repo/src", AccessIntent::Read, AccessScope::Exact),
                ("/repo/src/**/.env*", AccessIntent::Read, AccessScope::Glob),
            ],
        ),
        (
            "Grep",
            serde_json::json!({"pattern": "KEY", "glob": "!*.md"}),
            vec![("/repo", AccessIntent::Read, AccessScope::ExactOrDescendants)],
        ),
        (
            "Glob",
            serde_json::json!({"pattern": "**/.env"}),
            vec![("/repo/**/.env", AccessIntent::Enumerate, AccessScope::Glob)],
        ),
        (
            "Glob",
            serde_json::json!({"pattern": "*.ts", "path": "/repo/web"}),
            vec![("/repo/web/*.ts", AccessIntent::Enumerate, AccessScope::Glob)],
        ),
    ];
    for (tool, input, expected) in cases {
        let report = analyze(&claude_pre(tool, input));
        assert_targets(&report, &expected);
    }

    let defaulted = analyze(&claude_pre("Grep", serde_json::json!({"pattern": "x"})));
    assert!(matches!(
        &defaulted.candidates[0].provenance,
        AccessProvenance::StructuredField {
            pointer,
            matched_by: StructuredFieldMatch::BuiltinDefault,
        } if pointer == "/path"
    ));
}

#[test]
fn file_free_builtins_yield_empty_complete_reports() {
    for (tool, input) in [
        (
            "WebFetch",
            serde_json::json!({"url": "https://example.com", "prompt": "p"}),
        ),
        ("WebSearch", serde_json::json!({"query": "rust"})),
        (
            "TodoWrite",
            serde_json::json!({"todos": [{"content": "write docs"}]}),
        ),
        (
            "Agent",
            serde_json::json!({"prompt": "find files", "subagent_type": "Explore"}),
        ),
        ("AskUserQuestion", serde_json::json!({"questions": []})),
        (
            "ExitPlanMode",
            serde_json::json!({"plan": "p", "planFilePath": "/home/u/.claude/plans/p.md"}),
        ),
    ] {
        let report = analyze(&claude_pre(tool, input));
        assert!(
            report.candidates.is_empty() && report.is_complete(),
            "{tool}"
        );
    }
    for tool in [
        "update_plan",
        "spawn_agent",
        "web_search",
        "request_user_input",
    ] {
        let report = analyze(&codex_pre(tool, serde_json::json!({"plan": []})));
        assert!(
            report.candidates.is_empty() && report.is_complete(),
            "{tool}"
        );
    }
    for tool in ["search_web", "read_url_content", "manage_task", "schedule"] {
        let report = analyze(&antigravity_pre(
            tool,
            serde_json::json!({"Url": "https://x"}),
        ));
        assert!(
            report.candidates.is_empty() && report.is_complete(),
            "{tool}"
        );
    }

    // Disabling the built-in contracts restores heuristic-only analysis.
    let heuristic = ToolAccessAnalyzer::default()
        .with_builtin_tools(false)
        .analyze_pre_tool(&claude_pre("TodoWrite", serde_json::json!({"todos": []})));
    assert!(!heuristic.is_complete());
}

#[test]
fn codex_view_image_reads_its_path_unless_another_environment_is_named() {
    let report = analyze(&codex_pre(
        "view_image",
        serde_json::json!({"path": "shot.png"}),
    ));
    assert_targets(
        &report,
        &[("/repo/shot.png", AccessIntent::Read, AccessScope::Exact)],
    );

    let remote = analyze(&codex_pre(
        "view_image",
        serde_json::json!({"path": "shot.png", "environment_id": "remote"}),
    ));
    assert_eq!(path_of(&remote.candidates[0]).1, None);
    assert!(!remote.is_complete());
}

#[test]
fn mcp_heuristics_use_whole_words_and_move_keys_only_for_moves() {
    let calendar = analyze(&claude_pre(
        "mcp__cal__create_event",
        serde_json::json!({"from": "2026-01-01", "to": "2026-01-02"}),
    ));
    assert!(calendar.candidates.is_empty());

    let moved = analyze(&claude_pre(
        "mcp__fs__move_file",
        serde_json::json!({"source": "/repo/a", "destination": "/repo/b"}),
    ));
    assert_eq!(
        moved
            .candidates
            .iter()
            .map(|candidate| candidate.intent)
            .collect::<Vec<_>>(),
        [AccessIntent::MoveDestination, AccessIntent::MoveSource]
    );

    let copied = analyze(&claude_pre(
        "mcp__fs__copy_file",
        serde_json::json!({"source": "/repo/a", "destination": "/repo/b"}),
    ));
    assert!(
        copied
            .candidates
            .iter()
            .any(|c| c.intent == AccessIntent::Read)
    );
    assert!(
        copied
            .candidates
            .iter()
            .any(|c| c.intent == AccessIntent::Modify)
    );

    let removed = analyze(&claude_pre(
        "mcp__fs__remove_file",
        serde_json::json!({"path": "/repo/old.rs"}),
    ));
    assert_targets(
        &removed,
        &[("/repo/old.rs", AccessIntent::Delete, AccessScope::Exact)],
    );

    let downloaded = analyze(&claude_pre(
        "mcp__dl__download_file",
        serde_json::json!({"path": "/repo/data.bin"}),
    ));
    assert_eq!(downloaded.candidates[0].intent, AccessIntent::Modify);
    assert_eq!(
        downloaded.candidates[0].certainty,
        AccessCertainty::Heuristic
    );
}

#[test]
fn structured_home_relative_paths_stay_unresolved() {
    let report = analyze(&claude_pre(
        "mcp__fs__write_file",
        serde_json::json!({"path": "~/notes/todo.md"}),
    ));
    let (raw, resolved, _, base) = path_of(&report.candidates[0]);
    assert_eq!(
        (raw, resolved, base),
        ("~/notes/todo.md", None, PathBase::UnexpandedHome)
    );
    assert!(
        report
            .gaps
            .iter()
            .any(|gap| matches!(gap.reason, ToolAccessGapReason::UnexpandedHomePath { .. }))
    );
}

#[test]
fn claude_post_tool_use_failure_is_observable_from_the_contract_fixture() {
    let mut value = contract_fixture(
        "claude-code/snapshots/docs-2026-09-29-r1/events/post-tool-use-failure",
        "representative",
    );
    value["tool_input"]["command"] = serde_json::json!("sed -i 's/a/b/' src/x.py && pytest");
    let input: hookkit_claude::catalog::CatalogInput = serde_json::from_value(value).unwrap();

    let ToolCallObservation::Call(call) =
        hookkit_tool_access::ObservableToolCall::observe_tool_call(&input)
    else {
        panic!("PostToolUseFailure carries its originating tool call");
    };
    assert_eq!(call.event.name(), "PostToolUseFailure");
    assert_eq!(call.phase, ToolPhase::Post);

    let report = ToolAccessAnalyzer::default().analyze_native(&input);
    assert!(
        report
            .may_modify()
            .any(|candidate| { path_of(candidate).1 == Some(Utf8Path::new("/repo/src/x.py")) })
    );
}

#[test]
fn typed_native_inputs_are_analyzed_without_an_aligned_clone() {
    let PreToolUseInput::Codex(native) = codex_pre(
        "apply_patch",
        serde_json::json!({"command": "*** Begin Patch\n*** Delete File: old.rs\n*** End Patch"}),
    ) else {
        unreachable!()
    };
    let analyzer = ToolAccessAnalyzer::default();
    let direct = analyzer.analyze_native(&native);
    let aligned = analyzer.analyze_pre_tool(&PreToolUseInput::Codex(native));
    assert_eq!(direct, aligned);
    assert_eq!(direct.candidates[0].intent, AccessIntent::Delete);

    let post = PostToolUseInput::Antigravity(
        serde_json::from_value(serde_json::json!({
            "conversationId": "conversation",
            "workspacePaths": ["/repo"],
            "transcriptPath": "/tmp/transcript.jsonl",
            "artifactDirectoryPath": "/tmp/artifacts",
            "toolCall": {"name": "write_to_file", "args": {"TargetFile": "/repo/new.rs"}},
            "stepIdx": 2
        }))
        .unwrap(),
    );
    assert_eq!(
        analyzer.analyze_native(&post),
        analyzer.analyze_post_tool(&post)
    );
}
