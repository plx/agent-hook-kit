//! Checks the native crate against every fixture and output schema of the
//! frozen snapshot it implements.
//!
//! The conformance crate exercises only the registry-selected snapshot and
//! only the `representative` input. This suite follows [`SNAPSHOT`] instead
//! and covers every positive and negative input fixture, every output
//! fixture, every process case, and every typed output constructor.

use base64::Engine as _;
use hookkit_antigravity::{
    InjectStep, PostInvocation, PostInvocationOutput, PostToolUse, PostToolUseOutput,
    PreInvocation, PreInvocationOutput, PreToolUse, PreToolUseOutput, SNAPSHOT, Stop, StopDecision,
    StopOutput, TerminationBehavior, ToolDecision,
};
use hookkit_core::{EventSpec, ProcessEmission, RawInvocation};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;

const EVENT_DIRS: [&str; 5] = [
    "pre-invocation",
    "post-invocation",
    "pre-tool-use",
    "post-tool-use",
    "stop",
];

fn snapshot_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/harnesses/antigravity/snapshots")
        .join(SNAPSHOT.as_str())
}

fn read_yaml(event_dir: &str) -> Value {
    let path = snapshot_root()
        .join("events")
        .join(event_dir)
        .join("fixtures.yaml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_yaml_ng::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn output_validator(event_dir: &str) -> jsonschema::Validator {
    let path = snapshot_root()
        .join("events")
        .join(event_dir)
        .join("output.command.schema.json");
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .build(&schema)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn fixture_list<'a>(fixtures: &'a Value, pointer: &str) -> &'a [Value] {
    fixtures
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn fixture_id(fixture: &Value) -> &str {
    fixture["id"].as_str().expect("fixture id")
}

fn invocation(value: &Value) -> RawInvocation {
    RawInvocation::parse(serde_json::to_vec(value).unwrap()).unwrap()
}

/// Every positive fixture parses and serializes back to the exact same JSON
/// value; every negative fixture is rejected by the typed layer.
fn check_inputs<E>(event_dir: &str)
where
    E: EventSpec,
    E::Input: Serialize,
{
    let fixtures = read_yaml(event_dir);
    let positives = fixture_list(&fixtures, "/input/positive");
    assert!(
        positives.len() >= 2,
        "{event_dir}: expected minimal and representative positives"
    );
    for fixture in positives {
        let id = fixture_id(fixture);
        let raw = invocation(&fixture["value"]);
        let input =
            E::parse(&raw).unwrap_or_else(|error| panic!("{event_dir}/{id} must parse: {error}"));
        assert_eq!(
            serde_json::to_value(&input).unwrap(),
            fixture["value"],
            "{event_dir}/{id} must round-trip losslessly"
        );
        // Context extraction must not assume a non-empty workspace list.
        let context = E::context(&input);
        assert_eq!(
            context.workspace_roots.len(),
            fixture["value"]["workspacePaths"].as_array().unwrap().len(),
            "{event_dir}/{id}"
        );
    }

    let negatives = fixture_list(&fixtures, "/input/negative");
    assert!(!negatives.is_empty(), "{event_dir}: expected negatives");
    for fixture in negatives {
        let id = fixture_id(fixture);
        assert!(
            E::parse(&invocation(&fixture["value"])).is_err(),
            "{event_dir}/{id} must be rejected by the native parser"
        );
    }
}

#[test]
fn every_positive_input_fixture_parses_and_every_negative_is_rejected() {
    check_inputs::<PreInvocation>("pre-invocation");
    check_inputs::<PostInvocation>("post-invocation");
    check_inputs::<PreToolUse>("pre-tool-use");
    check_inputs::<PostToolUse>("post-tool-use");
    check_inputs::<Stop>("stop");
}

#[test]
fn snapshot_fixtures_exercise_the_relaxed_input_constraints() {
    // Guard against a fixture refresh silently dropping the cases that
    // motivated the relaxations implemented here.
    let post_tool = read_yaml("post-tool-use");
    let positives = fixture_list(&post_tool, "/input/positive");
    assert!(
        positives
            .iter()
            .any(|fixture| fixture["value"].get("toolCall").is_none())
    );
    assert!(
        positives
            .iter()
            .any(|fixture| fixture["value"]["error"].as_str() == Some(""))
    );
    for event_dir in EVENT_DIRS {
        let fixtures = read_yaml(event_dir);
        let positives = fixture_list(&fixtures, "/input/positive");
        assert!(
            positives
                .iter()
                .any(|fixture| fixture["value"]["workspacePaths"] == serde_json::json!([])),
            "{event_dir}: expected an empty-workspace positive"
        );
        assert!(
            positives
                .iter()
                .any(|fixture| fixture["value"].get("modelName").is_some()),
            "{event_dir}: expected a modelName positive"
        );
    }
}

fn emitted_json(event_dir: &str, emission: &ProcessEmission) -> Value {
    assert_eq!(emission.exit_code(), 0, "{event_dir}");
    let value: Value = serde_json::from_slice(emission.stdout()).unwrap();
    let validator = output_validator(event_dir);
    let errors: Vec<String> = validator
        .iter_errors(&value)
        .map(|error| format!("{}: {error}", error.instance_path().as_str()))
        .collect();
    assert!(
        errors.is_empty(),
        "{event_dir}: emitted {value} violates the output schema: {errors:?}"
    );
    value
}

/// Typed outputs reproducing each documented output fixture, by fixture id.
fn fixture_output(event_dir: &str, id: &str) -> ProcessEmission {
    let emission = match (event_dir, id) {
        ("pre-invocation", "no-op") => PreInvocation::emit(PreInvocationOutput::no_op()),
        ("pre-invocation", "inject-reminder") => PreInvocation::emit(PreInvocationOutput::inject(
            InjectStep::ephemeral_message("Remember to lint"),
        )),
        ("pre-invocation", "inject-user-message") => {
            PreInvocation::emit(PreInvocationOutput::inject(InjectStep::user_message(
                "Run the linter before continuing.",
            )))
        }
        ("post-invocation", "default") => PostInvocation::emit(
            PostInvocationOutput::no_op().with_termination_behavior(TerminationBehavior::Default),
        ),
        ("post-invocation", "force-continue") => PostInvocation::emit(
            PostInvocationOutput::no_op()
                .with_termination_behavior(TerminationBehavior::ForceContinue),
        ),
        ("post-invocation", "terminate-after-reminder") => PostInvocation::emit(
            PostInvocationOutput::inject(InjectStep::ephemeral_message("Summarize the changes."))
                .with_termination_behavior(TerminationBehavior::Terminate),
        ),
        ("pre-tool-use", "allow") => PreToolUse::emit(PreToolUseOutput::allow()),
        ("pre-tool-use", "ask") => PreToolUse::emit(
            PreToolUseOutput::ask()
                .with_reason("Requires confirmation for test execution.")
                .with_permission_override("command(npm test)"),
        ),
        ("pre-tool-use", "deny-unless-prior-grant") => PreToolUse::emit(
            PreToolUseOutput::deny_unless_prior_grant()
                .with_permission_override("read_file(/workspace/project/.env)"),
        ),
        ("post-tool-use", "no-op") => PostToolUse::emit(PostToolUseOutput::no_op()),
        ("stop", "continue") => Stop::emit(StopOutput::continue_with("Not done yet")),
        ("stop", "stop") => Stop::emit(StopOutput::allow_stop()),
        _ => panic!("{event_dir}/{id}: add a typed constructor for this output fixture"),
    };
    emission.unwrap_or_else(|error| panic!("{event_dir}/{id}: {error}"))
}

#[test]
fn every_output_fixture_is_reproduced_by_typed_constructors() {
    for event_dir in EVENT_DIRS {
        let fixtures = read_yaml(event_dir);
        let outputs = fixture_list(&fixtures, "/output");
        assert!(!outputs.is_empty(), "{event_dir}: expected output fixtures");
        for fixture in outputs {
            let id = fixture_id(fixture);
            let emission = fixture_output(event_dir, id);
            assert_eq!(
                emitted_json(event_dir, &emission),
                fixture["value"],
                "{event_dir}/{id}"
            );
        }
    }
}

#[test]
fn every_process_case_matches_exact_bytes_and_is_declared() {
    let engine = base64::engine::general_purpose::STANDARD;
    let descriptors = hookkit_antigravity::events();
    for event_dir in EVENT_DIRS {
        let fixtures = read_yaml(event_dir);
        let cases = fixture_list(&fixtures, "/process");
        assert!(!cases.is_empty(), "{event_dir}: expected process cases");
        let wire_name = match event_dir {
            "pre-invocation" => "PreInvocation",
            "post-invocation" => "PostInvocation",
            "pre-tool-use" => "PreToolUse",
            "post-tool-use" => "PostToolUse",
            "stop" => "Stop",
            _ => unreachable!(),
        };
        let descriptor = descriptors
            .iter()
            .find(|descriptor| descriptor.event().name() == wire_name)
            .unwrap();
        assert_eq!(
            descriptor.contract().as_str(),
            format!("antigravity/{}/{wire_name}", SNAPSHOT.as_str())
        );
        let declared: BTreeSet<&str> = descriptor.conformance_cases().iter().copied().collect();
        let documented: BTreeSet<&str> = cases.iter().map(fixture_id).collect();
        assert_eq!(declared, documented, "{event_dir}: declared process cases");

        for case in cases {
            let id = fixture_id(case);
            let emission = fixture_output(event_dir, id);
            let stdout = engine
                .decode(case["stdout_base64"].as_str().unwrap())
                .unwrap();
            let stderr = engine
                .decode(case["stderr_base64"].as_str().unwrap())
                .unwrap();
            assert_eq!(
                emission.stdout(),
                stdout.as_slice(),
                "{event_dir}/{id} stdout: {}",
                String::from_utf8_lossy(emission.stdout())
            );
            assert_eq!(emission.stderr(), stderr.as_slice(), "{event_dir}/{id}");
            assert_eq!(
                u64::from(emission.exit_code()),
                case["exit_code"].as_u64().unwrap(),
                "{event_dir}/{id}"
            );
        }
    }
}

#[test]
fn every_typed_output_constructor_satisfies_the_output_schema() {
    let steps = || {
        vec![
            InjectStep::ephemeral_message("transient"),
            InjectStep::user_message("persistent"),
            InjectStep::tool_call(
                serde_json::json!({"name": "run_command", "args": {"CommandLine": "ls"}})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        ]
    };

    let pre_invocation = [
        PreInvocationOutput::no_op(),
        PreInvocationOutput::inject(InjectStep::user_message("one")),
        PreInvocationOutput::inject_all(steps()),
        PreInvocationOutput::no_op().with_step(InjectStep::ephemeral_message("appended")),
    ];
    for output in pre_invocation {
        emitted_json("pre-invocation", &PreInvocation::emit(output).unwrap());
    }

    let mut post_invocation = vec![
        PostInvocationOutput::no_op(),
        PostInvocationOutput::inject(InjectStep::ephemeral_message("one")),
        PostInvocationOutput::inject_all(steps()),
        PostInvocationOutput::no_op().with_step(InjectStep::user_message("appended")),
    ];
    for behavior in [
        TerminationBehavior::Default,
        TerminationBehavior::ForceContinue,
        TerminationBehavior::Terminate,
    ] {
        post_invocation.push(PostInvocationOutput::no_op().with_termination_behavior(behavior));
        post_invocation
            .push(PostInvocationOutput::inject_all(steps()).with_termination_behavior(behavior));
    }
    for output in post_invocation {
        emitted_json("post-invocation", &PostInvocation::emit(output).unwrap());
    }

    let decisions = [
        PreToolUseOutput::allow(),
        PreToolUseOutput::deny(),
        PreToolUseOutput::ask(),
        PreToolUseOutput::force_ask(),
        PreToolUseOutput::deny_unless_prior_grant(),
        PreToolUseOutput::new(ToolDecision::Ask),
    ];
    for output in decisions {
        emitted_json("pre-tool-use", &PreToolUse::emit(output.clone()).unwrap());
        emitted_json(
            "pre-tool-use",
            &PreToolUse::emit(output.clone().with_reason("because")).unwrap(),
        );
        let value = emitted_json(
            "pre-tool-use",
            &PreToolUse::emit(
                output
                    .with_reason("because")
                    .with_permission_overrides(["command(npm test)", "read_file(/a)"]),
            )
            .unwrap(),
        );
        assert_eq!(value["permissionOverrides"].as_array().unwrap().len(), 2);
    }

    emitted_json(
        "post-tool-use",
        &PostToolUse::emit(PostToolUseOutput::no_op()).unwrap(),
    );
    emitted_json(
        "post-tool-use",
        &PostToolUse::emit(
            PostToolUseOutput::no_op()
                .with_protocol_stderr("diagnostics")
                .unwrap(),
        )
        .unwrap(),
    );

    for output in [
        StopOutput::allow_stop(),
        StopOutput::continue_with("keep going"),
        StopOutput::new(StopDecision::Continue),
        StopOutput::new(StopDecision::Stop),
        StopOutput::new(StopDecision::Other("finished".into())).with_reason("done"),
        StopOutput::new("stop"),
    ] {
        emitted_json("stop", &Stop::emit(output).unwrap());
    }
}

#[test]
fn documented_output_negatives_are_unrepresentable_or_repaired() {
    for event_dir in EVENT_DIRS {
        let fixtures = read_yaml(event_dir);
        let validator = output_validator(event_dir);
        for fixture in fixture_list(&fixtures, "/output_negative") {
            let id = fixture_id(fixture);
            assert!(
                !validator.is_valid(&fixture["value"]),
                "{event_dir}/{id} must violate the schema"
            );
            match (event_dir, id) {
                // `InjectStep` is a one-of enum and cannot carry two kinds.
                ("pre-invocation" | "post-invocation", "inject-step-mutually-exclusive") => {}
                // `TerminationBehavior` is closed to the documented values.
                ("post-invocation", "unknown-termination-behavior") => {}
                // `ToolDecision` has no empty value.
                ("pre-tool-use", "empty-decision") => {}
                // `PostToolUseOutput` always emits exactly `{}`.
                ("post-tool-use", "non-empty-response") => {}
                ("pre-tool-use", "duplicate-permission-override") => {
                    let output = PreToolUseOutput {
                        decision: ToolDecision::Ask,
                        reason: None,
                        permission_overrides: vec![
                            "command(npm test)".into(),
                            "command(npm test)".into(),
                        ],
                    };
                    let value = emitted_json(event_dir, &PreToolUse::emit(output).unwrap());
                    assert_eq!(
                        value,
                        serde_json::json!({
                            "decision": "ask",
                            "permissionOverrides": ["command(npm test)"]
                        })
                    );
                }
                ("stop", "empty-decision") => {
                    assert!(Stop::emit(StopOutput::new("")).is_err());
                }
                _ => panic!("{event_dir}/{id}: classify this output negative"),
            }
        }
    }
}
