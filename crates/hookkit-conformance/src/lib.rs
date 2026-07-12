use base64::Engine as _;
use hookkit_core::{EventSpec, NativeEventDescriptor, ProcessEmission, RawInvocation};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExecutedCase {
    pub contract: &'static str,
    pub binding: &'static str,
    pub case: &'static str,
}

/// Execute every case advertised by the implementation registry and reject any
/// declared/executed mismatch before the registry can be generated.
pub fn verified_descriptors() -> Result<Vec<NativeEventDescriptor>, String> {
    let descriptors = implementation_descriptors();
    verify_all_negative_inputs()?;
    let executed = execute_all_cases()?;
    let executed_by_contract = executed.iter().fold(
        BTreeMap::<&str, BTreeSet<&str>>::new(),
        |mut grouped, case| {
            grouped.entry(case.contract).or_default().insert(case.case);
            grouped
        },
    );

    for descriptor in &descriptors {
        let declared: BTreeSet<_> = descriptor.conformance_cases().iter().copied().collect();
        let actual = executed_by_contract
            .get(descriptor.contract().as_str())
            .cloned()
            .unwrap_or_default();
        if declared != actual {
            return Err(format!(
                "declared/executed conformance mismatch for {}: declared={declared:?}, executed={actual:?}",
                descriptor.contract()
            ));
        }
    }
    let known: BTreeSet<_> = descriptors
        .iter()
        .map(|descriptor| descriptor.contract().as_str())
        .collect();
    if let Some(case) = executed.iter().find(|case| !known.contains(case.contract)) {
        return Err(format!(
            "executed conformance case references unregistered contract {}",
            case.contract
        ));
    }
    Ok(descriptors)
}

fn verify_all_negative_inputs() -> Result<(), String> {
    verify_negative_inputs::<hookkit_claude::protocol::SessionStart>(
        "claude-code",
        "docs-2026-07-12-r2",
        "session-start",
    )?;
    verify_negative_inputs::<hookkit_claude::protocol::PostToolUse>(
        "claude-code",
        "docs-2026-07-12-r2",
        "post-tool-use",
    )?;
    verify_negative_inputs::<hookkit_claude::protocol::WorktreeCreate>(
        "claude-code",
        "docs-2026-07-12-r2",
        "worktree-create",
    )?;
    verify_negative_inputs::<hookkit_codex::protocol::PreToolUse>(
        "codex",
        "commit-9e552e9-r2",
        "pre-tool-use",
    )?;
    verify_negative_inputs::<hookkit_codex::protocol::PostToolUse>(
        "codex",
        "commit-9e552e9-r2",
        "post-tool-use",
    )?;
    verify_negative_inputs::<hookkit_gemini::protocol::BeforeTool>(
        "gemini-cli",
        "commit-f354eeb-r2",
        "before-tool",
    )?;
    verify_negative_inputs::<hookkit_gemini::protocol::AfterTool>(
        "gemini-cli",
        "commit-f354eeb-r2",
        "after-tool",
    )?;
    verify_negative_inputs::<hookkit_gemini::protocol::BeforeToolSelection>(
        "gemini-cli",
        "commit-f354eeb-r2",
        "before-tool-selection",
    )?;
    verify_negative_inputs::<hookkit_antigravity::PreInvocation>(
        "antigravity",
        "docs-2026-07-12-r2",
        "pre-invocation",
    )?;
    verify_negative_inputs::<hookkit_antigravity::PostInvocation>(
        "antigravity",
        "docs-2026-07-12-r2",
        "post-invocation",
    )?;
    verify_negative_inputs::<hookkit_antigravity::PreToolUse>(
        "antigravity",
        "docs-2026-07-12-r2",
        "pre-tool-use",
    )?;
    verify_negative_inputs::<hookkit_antigravity::PostToolUse>(
        "antigravity",
        "docs-2026-07-12-r2",
        "post-tool-use",
    )?;
    verify_negative_inputs::<hookkit_antigravity::Stop>(
        "antigravity",
        "docs-2026-07-12-r2",
        "stop",
    )?;
    Ok(())
}

pub fn execute_all_cases() -> Result<Vec<ExecutedCase>, String> {
    let mut executed = Vec::new();

    executed.push(verify_case::<hookkit_claude::protocol::SessionStart>(
        "claude-code",
        "docs-2026-07-12-r2",
        "session-start",
        "command-structured",
        hookkit_claude::protocol::SessionStartOutput::structured(
            Some("Read conventions.".into()),
            Some(true),
            Some("Review".into()),
            vec!["/repo/.env".into()],
        ),
    )?);
    executed.push(verify_case::<hookkit_claude::protocol::SessionStart>(
        "claude-code",
        "docs-2026-07-12-r2",
        "session-start",
        "command-text",
        hookkit_claude::protocol::SessionStartOutput::text_context("Hook-provided context."),
    )?);
    executed.push(verify_case::<hookkit_claude::protocol::PostToolUse>(
        "claude-code",
        "docs-2026-07-12-r2",
        "post-tool-use",
        "command-structured",
        hookkit_claude::protocol::PostToolUseOutput::with_context("Generated files changed.")
            .with_block("Review result.")
            .map_err(|error| error.to_string())?
            .with_updated_tool_output(serde_json::json!({"status":"redacted"}))
            .map_err(|error| error.to_string())?,
    )?);
    executed.push(verify_case::<hookkit_claude::protocol::WorktreeCreate>(
        "claude-code",
        "docs-2026-07-12-r2",
        "worktree-create",
        "command-created",
        hookkit_claude::protocol::WorktreeCreateOutput::path_with_newline(
            "/tmp/hookkit-worktree".into(),
        )
        .map_err(|error| error.to_string())?,
    )?);
    executed.push(verify_case::<hookkit_claude::protocol::WorktreeCreate>(
        "claude-code",
        "docs-2026-07-12-r2",
        "worktree-create",
        "command-failed",
        hookkit_claude::protocol::WorktreeCreateOutput::failed("", 1)
            .map_err(|error| error.to_string())?,
    )?);

    executed.push(verify_case::<hookkit_codex::protocol::PreToolUse>(
        "codex",
        "commit-9e552e9-r2",
        "pre-tool-use",
        "no-op",
        hookkit_codex::protocol::PreToolUseOutput::no_op(),
    )?);
    executed.push(verify_case::<hookkit_codex::protocol::PreToolUse>(
        "codex",
        "commit-9e552e9-r2",
        "pre-tool-use",
        "deny-json",
        hookkit_codex::protocol::PreToolUseOutput::deny(Some("blocked".into())),
    )?);
    executed.push(verify_case::<hookkit_codex::protocol::PreToolUse>(
        "codex",
        "commit-9e552e9-r2",
        "pre-tool-use",
        "deny-stderr",
        hookkit_codex::protocol::PreToolUseOutput::deny_stderr("blocked"),
    )?);
    executed.push(verify_case::<hookkit_codex::protocol::PostToolUse>(
        "codex",
        "commit-9e552e9-r2",
        "post-tool-use",
        "structured",
        hookkit_codex::protocol::PostToolUseOutput::with_context("Generated files changed.")
            .with_block("Review the output.")
            .map_err(|error| error.to_string())?,
    )?);
    executed.push(verify_case::<hookkit_codex::protocol::PostToolUse>(
        "codex",
        "commit-9e552e9-r2",
        "post-tool-use",
        "exit-2",
        hookkit_codex::protocol::PostToolUseOutput::blocking_error("blocked by hook"),
    )?);

    let rewritten = serde_json::Map::from_iter([(
        "command".into(),
        serde_json::Value::String("echo rewritten".into()),
    )]);
    executed.push(verify_case::<hookkit_gemini::protocol::BeforeTool>(
        "gemini-cli",
        "commit-f354eeb-r2",
        "before-tool",
        "structured",
        hookkit_gemini::protocol::BeforeToolOutput::deny_and_rewrite(
            "Blocked by policy.",
            rewritten,
        ),
    )?);
    executed.push(verify_case::<hookkit_gemini::protocol::BeforeTool>(
        "gemini-cli",
        "commit-f354eeb-r2",
        "before-tool",
        "exit-2",
        hookkit_gemini::protocol::BeforeToolOutput::blocking_error("blocked by hook"),
    )?);
    let tail_args = serde_json::Map::from_iter([(
        "path".into(),
        serde_json::Value::String("README.md".into()),
    )]);
    executed.push(verify_case::<hookkit_gemini::protocol::AfterTool>(
        "gemini-cli",
        "commit-f354eeb-r2",
        "after-tool",
        "structured",
        hookkit_gemini::protocol::AfterToolOutput::with_context("Review generated files.")
            .and_tail_tool_call("read_file", tail_args)
            .map_err(|error| error.to_string())?,
    )?);
    executed.push(verify_case::<hookkit_gemini::protocol::AfterTool>(
        "gemini-cli",
        "commit-f354eeb-r2",
        "after-tool",
        "exit-2",
        hookkit_gemini::protocol::AfterToolOutput::blocking_error("blocked by hook"),
    )?);
    executed.push(
        verify_case::<hookkit_gemini::protocol::BeforeToolSelection>(
            "gemini-cli",
            "commit-f354eeb-r2",
            "before-tool-selection",
            "no-op",
            hookkit_gemini::protocol::BeforeToolSelectionOutput::no_op(),
        )?,
    );
    executed.push(
        verify_case::<hookkit_gemini::protocol::BeforeToolSelection>(
            "gemini-cli",
            "commit-f354eeb-r2",
            "before-tool-selection",
            "disable-tools",
            hookkit_gemini::protocol::BeforeToolSelectionOutput::configure(
                Some(hookkit_gemini::protocol::ToolMode::None),
                Vec::new(),
            )
            .map_err(|error| error.to_string())?,
        )?,
    );

    executed.push(verify_case::<hookkit_antigravity::PreInvocation>(
        "antigravity",
        "docs-2026-07-12-r2",
        "pre-invocation",
        "inject-reminder",
        hookkit_antigravity::PreInvocationOutput::inject(
            hookkit_antigravity::InjectStep::EphemeralMessage {
                ephemeral_message: "Remember to lint".into(),
            },
        ),
    )?);
    executed.push(verify_case::<hookkit_antigravity::PostInvocation>(
        "antigravity",
        "docs-2026-07-12-r2",
        "post-invocation",
        "force-continue",
        hookkit_antigravity::PostInvocationOutput {
            inject_steps: Vec::new(),
            termination_behavior: Some(hookkit_antigravity::TerminationBehavior::ForceContinue),
        },
    )?);
    executed.push(verify_case::<hookkit_antigravity::PreToolUse>(
        "antigravity",
        "docs-2026-07-12-r2",
        "pre-tool-use",
        "ask",
        hookkit_antigravity::PreToolUseOutput {
            decision: hookkit_antigravity::ToolDecision::Ask,
            reason: Some("Requires confirmation.".into()),
            permission_overrides: vec!["command(npm test)".into()],
        },
    )?);
    executed.push(verify_case::<hookkit_antigravity::PostToolUse>(
        "antigravity",
        "docs-2026-07-12-r2",
        "post-tool-use",
        "no-op",
        hookkit_antigravity::PostToolUseOutput::default(),
    )?);
    executed.push(verify_case::<hookkit_antigravity::Stop>(
        "antigravity",
        "docs-2026-07-12-r2",
        "stop",
        "continue",
        hookkit_antigravity::StopOutput {
            decision: "continue".into(),
            reason: Some("Not done yet".into()),
        },
    )?);

    Ok(executed)
}

fn implementation_descriptors() -> Vec<NativeEventDescriptor> {
    let mut descriptors = Vec::new();
    descriptors.extend(hookkit_claude::protocol::events());
    descriptors.extend(hookkit_codex::protocol::events());
    descriptors.extend(hookkit_gemini::protocol::events());
    descriptors.extend(hookkit_antigravity::events());
    descriptors
}

fn verify_case<E: EventSpec>(
    harness: &str,
    snapshot: &str,
    event: &str,
    case: &'static str,
    output: E::CommandOutput,
) -> Result<ExecutedCase, String> {
    let fixture = fixture(harness, snapshot, event)?;
    let representative = positive_value(&fixture, "representative")?;
    let raw = RawInvocation::parse(
        serde_json::to_vec(&representative).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    E::parse(&raw).map_err(|error| {
        format!(
            "{} representative input failed native parse: {error}",
            E::CONTRACT
        )
    })?;
    let emission = E::emit(output).map_err(|error| error.to_string())?;
    assert_emission::<E>(&emission, process_case(&fixture, case)?)?;
    Ok(ExecutedCase {
        contract: E::CONTRACT.as_str(),
        binding: "command",
        case,
    })
}

fn verify_negative_inputs<E: EventSpec>(
    harness: &str,
    snapshot: &str,
    event: &str,
) -> Result<(), String> {
    let fixture = fixture(harness, snapshot, event)?;
    let negatives = fixture["input"]["negative"]
        .as_sequence()
        .ok_or_else(|| format!("{} has no negative input fixtures", E::CONTRACT))?;
    for negative in negatives {
        let id = negative["id"]
            .as_str()
            .ok_or_else(|| format!("{} has an unnamed negative fixture", E::CONTRACT))?;
        let value =
            serde_json::to_value(negative["value"].clone()).map_err(|error| error.to_string())?;
        let raw =
            RawInvocation::parse(serde_json::to_vec(&value).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
        if E::parse(&raw).is_ok() {
            return Err(format!(
                "{} negative input fixture {id} was accepted by the native parser",
                E::CONTRACT
            ));
        }
    }
    Ok(())
}

fn fixture(harness: &str, snapshot: &str, event: &str) -> Result<serde_yaml_ng::Value, String> {
    let path = workspace_root()
        .join("contracts/harnesses")
        .join(harness)
        .join("snapshots")
        .join(snapshot)
        .join("events")
        .join(event)
        .join("fixtures.yaml");
    serde_yaml_ng::from_slice(&std::fs::read(&path).map_err(|error| error.to_string())?)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn positive_value(document: &serde_yaml_ng::Value, id: &str) -> Result<serde_json::Value, String> {
    let fixtures = document["input"]["positive"]
        .as_sequence()
        .ok_or_else(|| "missing positive input fixtures".to_string())?;
    let value = fixtures
        .iter()
        .find(|fixture| fixture["id"].as_str() == Some(id))
        .ok_or_else(|| format!("missing positive fixture {id}"))?["value"]
        .clone();
    serde_json::to_value(value).map_err(|error| error.to_string())
}

fn process_case(
    document: &serde_yaml_ng::Value,
    id: &str,
) -> Result<(Vec<u8>, Vec<u8>, u8), String> {
    let cases = document["process"]
        .as_sequence()
        .ok_or_else(|| "missing process fixtures".to_string())?;
    let case = cases
        .iter()
        .find(|case| case["id"].as_str() == Some(id))
        .ok_or_else(|| format!("missing process fixture {id}"))?;
    let decode = |field: &str| {
        base64::engine::general_purpose::STANDARD
            .decode(
                case[field]
                    .as_str()
                    .ok_or_else(|| format!("missing {field}"))?,
            )
            .map_err(|error| error.to_string())
    };
    Ok((
        decode("stdout_base64")?,
        decode("stderr_base64")?,
        case["exit_code"]
            .as_u64()
            .ok_or_else(|| "missing exit_code".to_string())? as u8,
    ))
}

fn assert_emission<E: EventSpec>(
    actual: &ProcessEmission,
    expected: (Vec<u8>, Vec<u8>, u8),
) -> Result<(), String> {
    // JSON object member order is not protocol-significant. Its value is
    // checked structurally, while empty/text/opaque bytes and JSON framing are
    // checked exactly as required by the catalog contract.
    let stdout_matches = match (
        serde_json::from_slice::<serde_json::Value>(actual.stdout()),
        serde_json::from_slice::<serde_json::Value>(&expected.0),
    ) {
        (Ok(actual_json), Ok(expected_json)) => {
            actual_json == expected_json
                && actual.stdout().ends_with(b"\n") == expected.0.ends_with(b"\n")
        }
        _ => actual.stdout() == expected.0,
    };
    if actual.contract() != E::CONTRACT
        || actual.binding() != hookkit_core::HandlerKind::Command
        || !stdout_matches
        || actual.stderr() != expected.1
        || actual.exit_code() != expected.2
    {
        return Err(format!(
            "{} emission mismatch: stdout={:?}/{:?}, stderr={:?}/{:?}, exit={}/{}",
            E::CONTRACT,
            actual.stdout(),
            expected.0,
            actual.stderr(),
            expected.1,
            actual.exit_code(),
            expected.2
        ));
    }
    Ok(())
}

fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate lives under workspace/crates")
        .to_path_buf()
}
