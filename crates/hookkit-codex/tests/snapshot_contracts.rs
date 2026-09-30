//! Checks the native adapter against every fixture and schema in the Codex
//! snapshot it implements, independently of which snapshot the contract
//! registry currently selects.

use base64::Engine as _;
use hookkit_codex::catalog::{
    InterruptOutput, PermissionRequestOutput, PostCompactOutput, PreCompactOutput,
    SessionEndOutput, SessionStartOutput, StopOutput, SubagentStartOutput, SubagentStopOutput,
    UserPromptSubmitOutput,
};
use hookkit_codex::protocol::{
    AnyCommandOutput, Codex, PostToolUseOutput, PreToolUseOutput, SNAPSHOT_ID,
};
use hookkit_core::{EventId, HarnessId, HarnessSpec, ProcessEmission, RawInvocation};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn snapshot_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/harnesses/codex/snapshots")
        .join(SNAPSHOT_ID.as_str())
}

fn read_yaml(path: &Path) -> Value {
    let yaml: serde_yaml_ng::Value =
        serde_yaml_ng::from_slice(&std::fs::read(path).unwrap_or_else(|error| {
            panic!("{}: {error}", path.display());
        }))
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::to_value(yaml).unwrap()
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

struct SnapshotEvent {
    name: String,
    dir: PathBuf,
    contract: Value,
    fixtures: Value,
}

fn snapshot_events() -> Vec<SnapshotEvent> {
    let root = snapshot_dir();
    let snapshot = read_yaml(&root.join("snapshot.yaml"));
    assert_eq!(snapshot["id"], SNAPSHOT_ID.as_str());
    assert_eq!(snapshot["state"], "frozen");
    snapshot["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| {
            let dir = root.join(event["path"].as_str().unwrap());
            SnapshotEvent {
                name: event["wire_name"].as_str().unwrap().to_owned(),
                contract: read_yaml(&dir.join("contract.yaml")),
                fixtures: read_yaml(&dir.join("fixtures.yaml")),
                dir,
            }
        })
        .collect()
}

fn event_id(name: &str) -> EventId {
    EventId::new(HarnessId::CODEX, name).unwrap()
}

fn raw(value: &Value) -> RawInvocation {
    RawInvocation::parse(serde_json::to_vec(value).unwrap()).unwrap()
}

fn validator(schema: &Value) -> jsonschema::Validator {
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .build(schema)
        .unwrap()
}

fn validation_errors(validator: &jsonschema::Validator, value: &Value) -> Vec<String> {
    validator
        .iter_errors(value)
        .map(|error| format!("{}: {error}", error.instance_path().as_str()))
        .collect()
}

/// Negative fixtures that only close a documented value set (an enum, or a
/// constant other than the event discriminator) are the schema's job: Codex
/// can add values, so the native parser keeps them as `Unknown`.
fn is_value_closure(negative: &Value) -> bool {
    match negative["expected_keyword"].as_str() {
        Some("enum") => true,
        Some("const") => negative["expected_pointer"] != "/hook_event_name",
        _ => false,
    }
}

#[test]
fn every_snapshot_event_has_native_descriptors_and_contract_ids() {
    let events = snapshot_events();
    let snapshot_names: BTreeSet<_> = events.iter().map(|event| event.name.clone()).collect();

    let descriptors = hookkit_codex::protocol::events();
    let implemented: BTreeSet<_> = descriptors
        .iter()
        .map(|descriptor| descriptor.event().name().to_owned())
        .collect();
    assert_eq!(implemented, snapshot_names);

    let identified: BTreeSet<_> = Codex::identification_descriptors()
        .iter()
        .map(|descriptor| {
            assert_eq!(descriptor.snapshot(), SNAPSHOT_ID);
            assert!(descriptor.has_native_parser());
            assert_eq!(
                descriptor
                    .discriminator()
                    .map(|(pointer, value)| (pointer, value.to_owned())),
                Some(("/hook_event_name", descriptor.event().name().to_owned()))
            );
            descriptor.event().name().to_owned()
        })
        .collect();
    assert_eq!(identified, snapshot_names);

    for descriptor in &descriptors {
        let event = events
            .iter()
            .find(|event| event.name == descriptor.event().name())
            .unwrap();
        assert_eq!(descriptor.snapshot(), SNAPSHOT_ID);
        assert_eq!(
            descriptor.contract().as_str(),
            event.contract["id"].as_str().unwrap(),
            "{}",
            event.name
        );
    }
}

#[test]
fn positive_input_fixtures_parse_and_satisfy_the_input_schema() {
    for event in snapshot_events() {
        let schema = validator(&read_json(&event.dir.join("input.schema.json")));
        let positives = event.fixtures["input"]["positive"].as_array().unwrap();
        assert!(positives.len() >= 2, "{} needs positives", event.name);
        for fixture in positives {
            let id = fixture["id"].as_str().unwrap();
            let value = &fixture["value"];
            assert!(
                validation_errors(&schema, value).is_empty(),
                "{} positive {id} fails its schema",
                event.name
            );
            let input =
                Codex::decode(&event_id(&event.name), &raw(value)).unwrap_or_else(|error| {
                    panic!("{} positive {id} failed native parse: {error}", event.name)
                });
            assert_eq!(Codex::input_event(&input), event_id(&event.name));
            let context = Codex::context(&input);
            assert_eq!(
                context.workspace_roots[0].as_str(),
                value["cwd"].as_str().unwrap()
            );
            assert_eq!(
                context.session_id.as_ref().map(|id| id.as_str()),
                value["session_id"].as_str()
            );
            assert_eq!(
                context.turn_id.as_ref().map(|id| id.as_str()),
                value["turn_id"].as_str()
            );
        }
    }
}

#[test]
fn negative_input_fixtures_follow_the_typed_layer_boundary() {
    for event in snapshot_events() {
        let schema = validator(&read_json(&event.dir.join("input.schema.json")));
        let negatives = event.fixtures["input"]["negative"].as_array().unwrap();
        assert!(!negatives.is_empty(), "{} needs negatives", event.name);
        for fixture in negatives {
            let id = fixture["id"].as_str().unwrap();
            let value = &fixture["value"];
            assert!(
                !validation_errors(&schema, value).is_empty(),
                "{} negative {id} passes its schema",
                event.name
            );
            let parsed = Codex::decode(&event_id(&event.name), &raw(value));
            if is_value_closure(fixture) {
                assert!(
                    parsed.is_ok(),
                    "{} negative {id} only closes a value set; the native parser must keep it as Unknown: {:?}",
                    event.name,
                    parsed.err()
                );
            } else {
                assert!(
                    parsed.is_err(),
                    "{} negative {id} was accepted by the native parser",
                    event.name
                );
            }
        }
    }
}

#[test]
fn value_closure_negatives_surface_as_unknown_values() {
    let events = snapshot_events();
    let session_start = events
        .iter()
        .find(|event| event.name == "SessionStart")
        .unwrap();
    let unknown_source = session_start.fixtures["input"]["negative"]
        .as_array()
        .unwrap()
        .iter()
        .find(|fixture| fixture["id"] == "unknown-source")
        .unwrap();
    let input =
        hookkit_codex::catalog::decode(&event_id("SessionStart"), &raw(&unknown_source["value"]))
            .unwrap()
            .unwrap();
    assert_eq!(
        input.source(),
        Some(hookkit_codex::catalog::SessionStartSource::Unknown(
            "restore".into()
        ))
    );
    assert!(
        Codex::context(&hookkit_codex::protocol::AnyInput::Catalog(input))
            .session_boundary
            .is_none()
    );
}

/// A representative of every constructor and builder combination, keyed by
/// event, with a label for diagnostics.
fn constructed_outputs() -> Vec<(&'static str, &'static str, AnyCommandOutput)> {
    let mut rewrite = serde_json::Map::new();
    rewrite.insert("command".into(), "echo rewritten".into());
    let catalog = |output: hookkit_codex::catalog::CatalogOutput| AnyCommandOutput::Catalog(output);
    vec![
        ("PreToolUse", "no-op", PreToolUseOutput::no_op().into()),
        (
            "PreToolUse",
            "deny",
            PreToolUseOutput::deny("blocked").into(),
        ),
        (
            "PreToolUse",
            "deny+context+message",
            PreToolUseOutput::deny("blocked")
                .with_additional_context("see policy")
                .unwrap()
                .with_system_message("blocked rm")
                .unwrap()
                .into(),
        ),
        (
            "PreToolUse",
            "rewrite",
            PreToolUseOutput::rewrite(rewrite.clone()).into(),
        ),
        (
            "PreToolUse",
            "rewrite+context+message",
            PreToolUseOutput::rewrite(rewrite)
                .with_additional_context("rewritten")
                .unwrap()
                .with_system_message("rewrote command")
                .unwrap()
                .into(),
        ),
        (
            "PreToolUse",
            "legacy-block+context+message",
            PreToolUseOutput::block("Destructive command blocked by hook.")
                .with_additional_context("why")
                .unwrap()
                .with_system_message("blocked")
                .unwrap()
                .into(),
        ),
        (
            "PreToolUse",
            "context+message",
            PreToolUseOutput::with_context("generated files")
                .with_system_message("checked")
                .unwrap()
                .into(),
        ),
        (
            "PreToolUse",
            "system-message",
            PreToolUseOutput::system_message("checked").into(),
        ),
        (
            "PreToolUse",
            "deny-stderr",
            PreToolUseOutput::deny_stderr("blocked").into(),
        ),
        ("PostToolUse", "no-op", PostToolUseOutput::no_op().into()),
        (
            "PostToolUse",
            "context+block",
            PostToolUseOutput::with_context("Generated files changed.")
                .with_block("Review the output.")
                .unwrap()
                .into(),
        ),
        (
            "PostToolUse",
            "block+context+message",
            PostToolUseOutput::block("fix lint")
                .with_additional_context("lint failed")
                .unwrap()
                .with_system_message("ruff: 2 issues")
                .unwrap()
                .into(),
        ),
        (
            "PostToolUse",
            "stop",
            PostToolUseOutput::block("")
                .with_continue(false)
                .unwrap()
                .with_stop_reason("stopped")
                .unwrap()
                .into(),
        ),
        (
            "PostToolUse",
            "exit-2",
            PostToolUseOutput::blocking_error("blocked by hook").into(),
        ),
        (
            "PostToolUse",
            "protocol-stderr",
            PostToolUseOutput::with_context("ctx")
                .with_protocol_stderr("ignored by Codex")
                .unwrap()
                .into(),
        ),
        ("Interrupt", "no-op", InterruptOutput::no_op().into()),
        (
            "Interrupt",
            "system-message",
            InterruptOutput::system_message("saved").into(),
        ),
        (
            "Interrupt",
            "built-system-message",
            InterruptOutput::no_op()
                .with_system_message("saved")
                .unwrap()
                .into(),
        ),
        (
            "PermissionRequest",
            "deny",
            PermissionRequestOutput::deny("Blocked by policy.").into(),
        ),
        (
            "PermissionRequest",
            "deny-blank",
            PermissionRequestOutput::deny("").into(),
        ),
        (
            "PermissionRequest",
            "allow+message",
            PermissionRequestOutput::allow()
                .with_system_message("approved")
                .unwrap()
                .into(),
        ),
        (
            "PermissionRequest",
            "no-op",
            PermissionRequestOutput::no_op().into(),
        ),
        (
            "PermissionRequest",
            "exit-2",
            PermissionRequestOutput::blocking_error("blocked by hook").into(),
        ),
        (
            "PostCompact",
            "message",
            PostCompactOutput::no_op()
                .with_system_message("Compaction completed.")
                .unwrap()
                .into(),
        ),
        (
            "PostCompact",
            "stop",
            PostCompactOutput::no_op()
                .with_continue(false)
                .unwrap()
                .with_stop_reason("halt")
                .unwrap()
                .into(),
        ),
        (
            "PostCompact",
            "failure",
            PostCompactOutput::failure("hook failed\n").into(),
        ),
        (
            "PreCompact",
            "stop",
            PreCompactOutput::stop("Save state before compacting.").into(),
        ),
        (
            "PreCompact",
            "failure",
            PreCompactOutput::failure("hook failed\n").into(),
        ),
        ("SessionEnd", "no-op", SessionEndOutput::no_op().into()),
        (
            "SessionEnd",
            "failure",
            SessionEndOutput::failure("hook failed\n").into(),
        ),
        (
            "SessionStart",
            "context+controls",
            SessionStartOutput::with_context("Load repository conventions.")
                .with_system_message("loaded")
                .unwrap()
                .with_continue(false)
                .unwrap()
                .with_stop_reason("done")
                .unwrap()
                .into(),
        ),
        (
            "SessionStart",
            "text",
            SessionStartOutput::text_context("Hook-provided developer context.").into(),
        ),
        (
            "SessionStart",
            "json-lookalike-text",
            SessionStartOutput::text_context("{\"continue\":false}").into(),
        ),
        (
            "SubagentStart",
            "context+message",
            SubagentStartOutput::with_context("Review test conventions.")
                .with_system_message("hi")
                .unwrap()
                .into(),
        ),
        (
            "SubagentStart",
            "json-lookalike-text",
            SubagentStartOutput::text_context("[conventions] use rstest").into(),
        ),
        (
            "Stop",
            "block+message",
            StopOutput::block("Run the failing tests again.")
                .with_system_message("continuing")
                .unwrap()
                .into(),
        ),
        (
            "Stop",
            "stop-overrides-blank-block",
            StopOutput::block("")
                .with_continue(false)
                .unwrap()
                .with_stop_reason("halt")
                .unwrap()
                .into(),
        ),
        (
            "Stop",
            "exit-2",
            StopOutput::blocking_error("blocked by hook").into(),
        ),
        (
            "SubagentStop",
            "block",
            SubagentStopOutput::block("Run another focused pass.").into(),
        ),
        (
            "SubagentStop",
            "exit-2",
            SubagentStopOutput::blocking_error("blocked by hook").into(),
        ),
        (
            "UserPromptSubmit",
            "context",
            UserPromptSubmitOutput::with_context("Ask for a clearer reproduction.").into(),
        ),
        (
            "UserPromptSubmit",
            "block",
            UserPromptSubmitOutput::block("Ask for confirmation.").into(),
        ),
        (
            "UserPromptSubmit",
            "block-with-context+controls",
            UserPromptSubmitOutput::block_with_context("Ask.", "Clarify.")
                .with_system_message("blocked")
                .unwrap()
                .into(),
        ),
        (
            "UserPromptSubmit",
            "json-lookalike-text",
            UserPromptSubmitOutput::text_context("[repo-policy] never push to main").into(),
        ),
        (
            "UserPromptSubmit",
            "text",
            UserPromptSubmitOutput::text_context("plain").into(),
        ),
        (
            "UserPromptSubmit",
            "exit-2",
            UserPromptSubmitOutput::blocking_error("blocked by hook").into(),
        ),
        (
            "UserPromptSubmit",
            "catalog-conversion",
            catalog(UserPromptSubmitOutput::no_op().into()),
        ),
    ]
}

/// Returns the id of the contract outcome that `emission` selects.
fn matching_outcome(contract: &Value, emission: &ProcessEmission) -> Option<String> {
    let exit = u64::from(emission.exit_code());
    let stdout = emission.stdout();
    let stdout_kind = if stdout.is_empty() {
        "empty"
    } else if serde_json::from_slice::<Value>(stdout).is_ok_and(|value| value.is_object()) {
        "json"
    } else {
        "text"
    };
    contract["bindings"]["command"]["outcomes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|outcome| {
            let exit_matches = match outcome["exit"]["exact"].as_u64() {
                Some(exact) => exact == exit,
                None => {
                    let range = &outcome["exit"]["range"];
                    range["min"].as_u64().unwrap() <= exit && exit <= range["max"].as_u64().unwrap()
                }
            };
            let stdout_spec = &outcome["stdout"];
            let stdout_matches = match stdout_spec["role"].as_str().unwrap() {
                "ignored" => true,
                "none" => stdout_kind == "empty",
                _ => stdout_spec["content_kind"] == stdout_kind,
            };
            exit_matches && stdout_matches
        })
        // Prefer the most specific outcome when an exact exit code overlaps
        // a failure range.
        .min_by_key(|outcome| outcome["exit"]["exact"].is_null())
        .map(|outcome| outcome["id"].as_str().unwrap().to_owned())
}

#[test]
fn every_constructed_output_matches_a_contract_outcome_and_output_schema() {
    let events: BTreeMap<_, _> = snapshot_events()
        .into_iter()
        .map(|event| (event.name.clone(), event))
        .collect();
    let mut covered = BTreeSet::new();
    for (name, label, output) in constructed_outputs() {
        let event = &events[name];
        assert_eq!(
            Codex::output_event(&output),
            event_id(name),
            "{name} {label}"
        );
        let emission = Codex::encode_command(&event_id(name), output)
            .unwrap_or_else(|error| panic!("{name} {label} failed to emit: {error}"));
        assert_eq!(
            emission.contract().as_str(),
            event.contract["id"].as_str().unwrap()
        );
        let outcome = matching_outcome(&event.contract, &emission).unwrap_or_else(|| {
            panic!(
                "{name} {label} emission matches no contract outcome: exit={} stdout={:?}",
                emission.exit_code(),
                String::from_utf8_lossy(emission.stdout())
            )
        });
        covered.insert(name);
        if emission.exit_code() == 0 && !emission.stdout().is_empty() {
            let text = std::str::from_utf8(emission.stdout()).unwrap();
            match serde_json::from_str::<Value>(text) {
                Ok(value) => {
                    let schema =
                        validator(&read_json(&event.dir.join("output.command.schema.json")));
                    let errors = validation_errors(&schema, &value);
                    assert!(
                        errors.is_empty(),
                        "{name} {label} emitted schema-invalid output {value}: {errors:?}"
                    );
                }
                Err(_) => {
                    assert_eq!(outcome, "text-context", "{name} {label}");
                    let trimmed = text.trim_start();
                    assert!(
                        !trimmed.starts_with('{') && !trimmed.starts_with('['),
                        "{name} {label} emitted JSON-looking plain text"
                    );
                }
            }
        }
        if emission.exit_code() == 2 {
            assert!(
                !String::from_utf8_lossy(emission.stderr()).trim().is_empty(),
                "{name} {label} exit-2 stderr is blank"
            );
        }
    }
    assert_eq!(
        covered,
        events.keys().map(String::as_str).collect::<BTreeSet<_>>()
    );
}

fn declared_case_output(event: &str, case: &str) -> Option<AnyCommandOutput> {
    Some(match (event, case) {
        ("PreToolUse", "no-op") => PreToolUseOutput::no_op().into(),
        ("PreToolUse", "deny-json") => PreToolUseOutput::deny("blocked").into(),
        ("PreToolUse", "deny-stderr") => PreToolUseOutput::deny_stderr("blocked").into(),
        ("PostToolUse", "structured") => {
            PostToolUseOutput::with_context("Generated files changed.")
                .with_block("Review the output.")
                .unwrap()
                .into()
        }
        ("PostToolUse", "no-op") => PostToolUseOutput::no_op().into(),
        ("PostToolUse", "exit-2") => PostToolUseOutput::blocking_error("blocked by hook").into(),
        ("Interrupt", "structured") => {
            InterruptOutput::system_message("Saved the interrupted turn to the local audit log.")
                .into()
        }
        ("Interrupt", "no-op") => InterruptOutput::no_op().into(),
        ("PermissionRequest", "structured") => {
            PermissionRequestOutput::deny("Blocked by policy.").into()
        }
        ("PermissionRequest", "structured-allow") => PermissionRequestOutput::allow().into(),
        ("PermissionRequest", "no-op") => PermissionRequestOutput::no_op().into(),
        ("PermissionRequest", "exit-2") => {
            PermissionRequestOutput::blocking_error("blocked by hook").into()
        }
        ("PostCompact", "structured") => PostCompactOutput::no_op()
            .with_system_message("Compaction completed.")
            .unwrap()
            .into(),
        ("PostCompact", "no-op") => PostCompactOutput::no_op().into(),
        ("PostCompact", "failure") => PostCompactOutput::failure("hook failed\n").into(),
        ("PreCompact", "structured") => {
            PreCompactOutput::stop("Save state before compacting.").into()
        }
        ("PreCompact", "no-op") => PreCompactOutput::no_op().into(),
        ("PreCompact", "failure") => PreCompactOutput::failure("hook failed\n").into(),
        ("SessionEnd", "no-op") => SessionEndOutput::no_op().into(),
        ("SessionEnd", "failure") => SessionEndOutput::failure("hook failed\n").into(),
        ("SessionStart", "structured") => {
            SessionStartOutput::with_context("Load repository conventions.").into()
        }
        ("SessionStart", "no-op") => SessionStartOutput::no_op().into(),
        ("SessionStart", "text-context") => {
            SessionStartOutput::text_context("Hook-provided developer context.").into()
        }
        ("Stop", "structured") => StopOutput::block("Run the failing tests again.").into(),
        ("Stop", "no-op") => StopOutput::no_op().into(),
        ("Stop", "exit-2") => StopOutput::blocking_error("blocked by hook").into(),
        ("SubagentStart", "structured") => {
            SubagentStartOutput::with_context("Review test conventions.").into()
        }
        ("SubagentStart", "no-op") => SubagentStartOutput::no_op().into(),
        ("SubagentStart", "text-context") => {
            SubagentStartOutput::text_context("Hook-provided developer context.").into()
        }
        ("SubagentStop", "structured") => {
            SubagentStopOutput::block("Run another focused pass.").into()
        }
        ("SubagentStop", "no-op") => SubagentStopOutput::no_op().into(),
        ("SubagentStop", "exit-2") => SubagentStopOutput::blocking_error("blocked by hook").into(),
        ("UserPromptSubmit", "structured") => UserPromptSubmitOutput::block_with_context(
            "Ask for confirmation.",
            "Clarify the reproduction.",
        )
        .into(),
        ("UserPromptSubmit", "structured-context") => UserPromptSubmitOutput::with_context(
            "Ask for a clearer reproduction before editing files.",
        )
        .into(),
        ("UserPromptSubmit", "structured-block") => {
            UserPromptSubmitOutput::block("Ask for confirmation before doing that.").into()
        }
        ("UserPromptSubmit", "no-op") => UserPromptSubmitOutput::no_op().into(),
        ("UserPromptSubmit", "text-context") => {
            UserPromptSubmitOutput::text_context("Hook-provided developer context.").into()
        }
        ("UserPromptSubmit", "exit-2") => {
            UserPromptSubmitOutput::blocking_error("blocked by hook").into()
        }
        _ => return None,
    })
}

#[test]
fn declared_conformance_cases_reproduce_the_snapshot_process_fixtures() {
    let events: BTreeMap<_, _> = snapshot_events()
        .into_iter()
        .map(|event| (event.name.clone(), event))
        .collect();
    let base64 = base64::engine::general_purpose::STANDARD;
    for descriptor in hookkit_codex::protocol::events() {
        let name = descriptor.event().name();
        let event = &events[name];
        for case in descriptor.conformance_cases() {
            let fixture = event.fixtures["process"]
                .as_array()
                .unwrap()
                .iter()
                .find(|fixture| fixture["id"] == *case)
                .unwrap_or_else(|| panic!("{name} declares unknown process case {case}"));
            let output = declared_case_output(name, case)
                .unwrap_or_else(|| panic!("{name} case {case} has no output in this test"));
            let emission = Codex::encode_command(&event_id(name), output).unwrap();
            let stdout = base64
                .decode(fixture["stdout_base64"].as_str().unwrap())
                .unwrap();
            let stderr = base64
                .decode(fixture["stderr_base64"].as_str().unwrap())
                .unwrap();
            let stdout_matches = match (
                serde_json::from_slice::<Value>(emission.stdout()),
                serde_json::from_slice::<Value>(&stdout),
            ) {
                (Ok(actual), Ok(expected)) => actual == expected,
                _ => emission.stdout() == stdout.as_slice(),
            };
            assert!(
                stdout_matches,
                "{name} {case} stdout {:?} != {:?}",
                String::from_utf8_lossy(emission.stdout()),
                String::from_utf8_lossy(&stdout)
            );
            assert_eq!(emission.stderr(), stderr.as_slice(), "{name} {case}");
            assert_eq!(
                u64::from(emission.exit_code()),
                fixture["exit_code"].as_u64().unwrap(),
                "{name} {case}"
            );
            assert_eq!(
                matching_outcome(&event.contract, &emission).as_deref(),
                fixture["outcome"].as_str(),
                "{name} {case}"
            );
        }
    }
}

#[test]
fn official_output_fixtures_are_reproducible_by_constructors() {
    // Each documented example output has a constructor producing the same
    // JSON value, except the PreToolUse shapes Codex parses but rejects.
    let rejected_by_codex = ["ask", "approve", "common-controls"];
    let mut rewrite = serde_json::Map::new();
    rewrite.insert("command".into(), "echo rewritten".into());
    let built: [((&str, &str), AnyCommandOutput); 17] = [
        (
            ("PreToolUse", "deny"),
            PreToolUseOutput::deny("blocked").into(),
        ),
        (
            ("PreToolUse", "rewrite"),
            PreToolUseOutput::rewrite(rewrite).into(),
        ),
        (
            ("PreToolUse", "context"),
            PreToolUseOutput::with_context("The pending command touches generated files.").into(),
        ),
        (
            ("PreToolUse", "legacy-block"),
            PreToolUseOutput::block("Destructive command blocked by hook.").into(),
        ),
        (
            ("PostToolUse", "structured"),
            declared_case_output("PostToolUse", "structured").unwrap(),
        ),
        (
            ("Interrupt", "structured"),
            declared_case_output("Interrupt", "structured").unwrap(),
        ),
        (
            ("PermissionRequest", "structured"),
            declared_case_output("PermissionRequest", "structured").unwrap(),
        ),
        (
            ("PermissionRequest", "allow"),
            PermissionRequestOutput::allow().into(),
        ),
        (
            ("PostCompact", "structured"),
            declared_case_output("PostCompact", "structured").unwrap(),
        ),
        (
            ("PreCompact", "structured"),
            declared_case_output("PreCompact", "structured").unwrap(),
        ),
        (
            ("SessionStart", "structured"),
            declared_case_output("SessionStart", "structured").unwrap(),
        ),
        (
            ("Stop", "structured"),
            declared_case_output("Stop", "structured").unwrap(),
        ),
        (
            ("SubagentStart", "structured"),
            declared_case_output("SubagentStart", "structured").unwrap(),
        ),
        (
            ("SubagentStop", "structured"),
            declared_case_output("SubagentStop", "structured").unwrap(),
        ),
        (
            ("UserPromptSubmit", "structured"),
            declared_case_output("UserPromptSubmit", "structured").unwrap(),
        ),
        (
            ("UserPromptSubmit", "context"),
            declared_case_output("UserPromptSubmit", "structured-context").unwrap(),
        ),
        (
            ("UserPromptSubmit", "block"),
            declared_case_output("UserPromptSubmit", "structured-block").unwrap(),
        ),
    ];
    let mut built: BTreeMap<(String, String), AnyCommandOutput> = built
        .into_iter()
        .map(|((event, id), output)| ((event.to_owned(), id.to_owned()), output))
        .collect();
    for event in snapshot_events() {
        for fixture in event.fixtures["output"].as_array().into_iter().flatten() {
            let id = fixture["id"].as_str().unwrap();
            if event.name == "PreToolUse" && rejected_by_codex.contains(&id) {
                continue;
            }
            let output = built
                .remove(&(event.name.clone(), id.to_owned()))
                .unwrap_or_else(|| panic!("{} output fixture {id} has no constructor", event.name));
            let emission = Codex::encode_command(&event_id(&event.name), output).unwrap();
            let actual: Value = serde_json::from_slice(emission.stdout()).unwrap();
            assert_eq!(actual, fixture["value"], "{} {id}", event.name);
        }
    }
    assert!(built.is_empty(), "unused constructors: {:?}", built.keys());
}

#[test]
fn output_negative_fixtures_are_rejected_by_their_schema() {
    for event in snapshot_events() {
        let Some(negatives) = event.fixtures["output_negative"].as_array() else {
            continue;
        };
        let schema = validator(&read_json(&event.dir.join("output.command.schema.json")));
        for fixture in negatives {
            assert!(
                !validation_errors(&schema, &fixture["value"]).is_empty(),
                "{} output negative {} passes its schema",
                event.name,
                fixture["id"]
            );
        }
    }
    // The constructors that could express those shapes refuse them.
    for output in [
        AnyCommandOutput::from(PreToolUseOutput::deny("  ")),
        PreToolUseOutput::block("\t").into(),
        PostToolUseOutput::block(" \t").into(),
    ] {
        let event = Codex::output_event(&output);
        assert!(Codex::encode_command(&event, output).is_err());
    }
    for (event, output) in [
        ("Stop", StopOutput::block(" \t").into()),
        ("SubagentStop", SubagentStopOutput::block("").into()),
        (
            "UserPromptSubmit",
            UserPromptSubmitOutput::block(" \t").into(),
        ),
    ] {
        let output: AnyCommandOutput = output;
        assert!(Codex::encode_command(&event_id(event), output).is_err());
    }
}
