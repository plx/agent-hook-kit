//! Antigravity's refreshed PostToolUse payload retains typed originating args.

#[test]
fn antigravity_post_tool_produces_direct_file_activity() {
    let input = br#"{
      "conversationId": "conversation-1",
      "workspacePaths": ["/repo"],
      "transcriptPath": "/tmp/transcript.jsonl",
      "artifactDirectoryPath": "/tmp/artifacts",
      "toolCall": {
        "name": "run_command",
        "args": {
          "CommandLine": "printf ok > src/generated.rs",
          "Cwd": "/repo"
        }
      },
      "stepIdx": 2
    }"#;

    let emission = hookkit_runtime::aligned::execute_aligned_event::<
        hookkit_runtime::aligned::PostToolUse,
        _,
    >(
        hookkit_core::HarnessId::ANTIGRAVITY,
        input,
        &hookkit_core::EnvironmentVariables::new(),
        |input, _environment, context| {
            let report = hookkit_file_activity::observe_post_tool(&input, context);
            assert!(report.evidence().any(|evidence| {
                evidence.source == hookkit_file_activity::FileActivitySource::ShellInference
                    && evidence.certainty == hookkit_file_activity::ActivityCertainty::Direct
                    && evidence.target
                        == hookkit_file_activity::FileActivityTarget::exact(
                            hookkit_core::Utf8PathBuf::from("/repo/src/generated.rs"),
                        )
            }));
            hookkit_common::PostToolUseOutput::no_op(context.harness())
        },
    )
    .expect("Antigravity PostToolUse should retain its originating shell call");

    assert_eq!(emission.exit_code(), 0);
    assert_eq!(emission.stdout(), b"{}");
}
