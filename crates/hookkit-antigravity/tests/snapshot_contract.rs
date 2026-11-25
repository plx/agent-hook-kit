//! Checks the native crate against every fixture and output schema of the
//! snapshot it implements.
//!
//! The conformance crate exercises only the registry-selected snapshot and
//! only the `representative` input. This suite follows [`SNAPSHOT`] instead
//! and covers every positive and negative input fixture, every output
//! fixture, every process case, and every typed output constructor.

use base64::Engine as _;
use hookkit_antigravity::{
    InjectStep, PostInvocation, PostInvocationOutput, PostToolUse, PostToolUseOutput,
    PreInvocation, PreInvocationOutput, PreInvocationTermination, PreToolUse, PreToolUseOutput,
    SNAPSHOT, SessionStart, SessionStartOutput, Stop, StopDecision, StopOutput,
    TerminationBehavior, ToolDecision,
};
use hookkit_core::{EventSpec, ProcessEmission, RawInvocation};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;

const EVENT_DIRS: [&str; 6] = [
    "session-start",
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
    check_inputs::<SessionStart>("session-start");
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
    // A tool that takes no arguments may send `args: null` or leave it out.
    for event_dir in ["pre-tool-use", "post-tool-use"] {
        let fixtures = read_yaml(event_dir);
        let calls: Vec<&Value> = fixture_list(&fixtures, "/input/positive")
            .iter()
            .filter_map(|fixture| fixture["value"].get("toolCall"))
            .collect();
        assert!(
            calls
                .iter()
                .any(|call| call.get("args") == Some(&Value::Null)),
            "{event_dir}: expected a null-args positive"
        );
        assert!(
            calls.iter().any(|call| call.get("args").is_none()),
            "{event_dir}: expected a positive without args"
        );
    }
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
        // The common fields documented or announced after
        // docs-2026-09-30-r1 must be exercised by every event.
        for field in ["lastUserInput", "hookEventName", "cwd"] {
            assert!(
                positives
                    .iter()
                    .any(|fixture| fixture["value"].get(field).is_some()),
                "{event_dir}: expected a {field} positive"
            );
        }
    }
    // CLI 1.5.4 tool-call identifiers may be strings or numbers.
    for event_dir in ["pre-tool-use", "post-tool-use"] {
        let fixtures = read_yaml(event_dir);
        let ids: Vec<&Value> = fixture_list(&fixtures, "/input/positive")
            .iter()
            .filter_map(|fixture| fixture["value"].pointer("/toolCall/id"))
            .collect();
        assert!(
            ids.iter().any(|id| id.is_number()),
            "{event_dir}: expected a numeric toolCall.id positive"
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

fn metadata(source: &str) -> serde_json::Map<String, Value> {
    serde_json::json!({ "source": source })
        .as_object()
        .unwrap()
        .clone()
}

fn args(value: Value) -> serde_json::Map<String, Value> {
    value.as_object().unwrap().clone()
}

/// Typed outputs reproducing each documented output fixture, by fixture id.
fn fixture_output(event_dir: &str, id: &str) -> ProcessEmission {
    let emission = match (event_dir, id) {
        ("session-start", "no-op") => SessionStart::emit(SessionStartOutput::no_op()),
        ("session-start", "inject-system-message") => {
            SessionStart::emit(SessionStartOutput::inject(InjectStep::system_message(
                "Environment initialized for workspace /workspace/project",
            )))
        }
        ("session-start", "inject-user-message") => SessionStart::emit(SessionStartOutput::inject(
            InjectStep::user_message("Startup context."),
        )),
        ("session-start", "inject-system-message-with-metadata") => {
            SessionStart::emit(SessionStartOutput::inject(
                InjectStep::system_message_with_metadata("Remember to lint", metadata("policy")),
            ))
        }
        ("session-start", "inject-error-message") => {
            SessionStart::emit(SessionStartOutput::inject(InjectStep::error_message(
                "Workspace policy could not be loaded.",
            )))
        }
        ("pre-invocation", "no-op") => PreInvocation::emit(PreInvocationOutput::no_op()),
        ("pre-invocation", "inject-reminder") => PreInvocation::emit(PreInvocationOutput::inject(
            InjectStep::system_message("Remember to lint"),
        )),
        ("pre-invocation", "block-prompt") => {
            PreInvocation::emit(PreInvocationOutput::block("Prompt blocked by policy."))
        }
        ("pre-invocation", "inject-user-message") => {
            PreInvocation::emit(PreInvocationOutput::inject(InjectStep::user_message(
                "Run the linter before continuing.",
            )))
        }
        ("pre-invocation", "inject-system-message-with-metadata") => {
            PreInvocation::emit(PreInvocationOutput::inject(
                InjectStep::system_message_with_metadata("Remember to lint", metadata("policy")),
            ))
        }
        ("pre-invocation", "inject-deprecated-ephemeral") => PreInvocation::emit(
            PreInvocationOutput::inject(InjectStep::ephemeral_message("Remember to lint")),
        ),
        ("pre-invocation", "explicit-default") => PreInvocation::emit(
            PreInvocationOutput::no_op()
                .with_termination_behavior(PreInvocationTermination::Default),
        ),
        ("pre-invocation", "terminate-without-message") => PreInvocation::emit(
            PreInvocationOutput::no_op()
                .with_termination_behavior(PreInvocationTermination::Terminate),
        ),
        ("post-invocation", "default") => PostInvocation::emit(
            PostInvocationOutput::no_op().with_termination_behavior(TerminationBehavior::Default),
        ),
        ("post-invocation", "force-continue") => PostInvocation::emit(
            PostInvocationOutput::no_op()
                .with_termination_behavior(TerminationBehavior::ForceContinue),
        ),
        ("post-invocation", "terminate-after-reminder") => PostInvocation::emit(
            PostInvocationOutput::inject(InjectStep::system_message("Summarize the changes."))
                .with_termination_behavior(TerminationBehavior::Terminate),
        ),
        ("post-invocation", "inject-deprecated-ephemeral") => PostInvocation::emit(
            PostInvocationOutput::inject(InjectStep::ephemeral_message("Summarize the changes.")),
        ),
        ("post-invocation", "inject-error-message") => PostInvocation::emit(
            PostInvocationOutput::inject(InjectStep::error_message("Post-turn check failed."))
                .with_termination_behavior(TerminationBehavior::Terminate),
        ),
        ("pre-tool-use", "allow-pass-through") => PreToolUse::emit(PreToolUseOutput::allow()),
        ("pre-tool-use", "auto-approve") => {
            PreToolUse::emit(PreToolUseOutput::auto_approve().with_reason("Lint-only command."))
        }
        ("pre-tool-use", "deny") => PreToolUse::emit(
            PreToolUseOutput::deny().with_reason("Destructive command blocked by policy."),
        ),
        ("pre-tool-use", "ask") => PreToolUse::emit(
            PreToolUseOutput::ask()
                .with_reason("Requires confirmation for test execution.")
                .with_permission_override("command(npm test)"),
        ),
        ("pre-tool-use", "force-ask") => PreToolUse::emit(
            PreToolUseOutput::force_ask().with_reason("Always confirm deployments."),
        ),
        ("pre-tool-use", "deny-unless-prior-grant") => PreToolUse::emit(
            PreToolUseOutput::deny_unless_prior_grant()
                .with_permission_override("read_file(/workspace/project/.env)"),
        ),
        ("pre-tool-use", "overwrite-cwd") => PreToolUse::emit(
            PreToolUseOutput::allow()
                .with_overwrite(args(serde_json::json!({"Cwd": "/workspace/project/sub"}))),
        ),
        ("pre-tool-use", "auto-approve-with-overwrite") => PreToolUse::emit(
            PreToolUseOutput::auto_approve()
                .with_overwrite(args(serde_json::json!({"CommandLine": "npm test -- --ci"}))),
        ),
        ("pre-tool-use", "ask-with-extended-overrides") => {
            PreToolUse::emit(PreToolUseOutput::ask().with_permission_overrides([
                "download_url(example.com)",
                "unsandboxed(git push)",
                "mcp(linter/*)",
            ]))
        }
        ("post-tool-use", "no-op") => PostToolUse::emit(PostToolUseOutput::no_op()),
        ("stop", "continue") => Stop::emit(StopOutput::continue_with("Not done yet")),
        ("stop", "stop") => Stop::emit(StopOutput::allow_stop()),
        ("stop", "continue-without-reason") => Stop::emit(StopOutput::new(StopDecision::Continue)),
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
            "session-start" => "SessionStart",
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
        // Typed outputs always exit 0. A non-zero `failure` case is what a
        // failing process produces (Antigravity logs and ignores it), so it
        // is checked for shape here but not declared or reproduced.
        let (exit_zero, failures): (Vec<&Value>, Vec<&Value>) = cases
            .iter()
            .partition(|case| case["exit_code"].as_u64() == Some(0));
        let declared: BTreeSet<&str> = descriptor.conformance_cases().iter().copied().collect();
        let documented: BTreeSet<&str> = exit_zero.iter().copied().map(fixture_id).collect();
        assert_eq!(declared, documented, "{event_dir}: declared process cases");
        assert_eq!(
            failures.iter().copied().map(fixture_id).collect::<Vec<_>>(),
            ["failure"],
            "{event_dir}: expected exactly one failure process case"
        );
        for failure in failures {
            assert_eq!(failure["outcome"], "failure", "{event_dir}");
            assert_eq!(failure["stdout_base64"], "", "{event_dir}");
            assert_ne!(failure["exit_code"].as_u64(), Some(0), "{event_dir}");
        }

        for case in exit_zero {
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
            InjectStep::system_message("instruction"),
            InjectStep::system_message_with_metadata("instruction", metadata("policy")),
            InjectStep::user_message("persistent"),
            InjectStep::ephemeral_message("deprecated transient"),
            InjectStep::error_message("shown to the user"),
        ]
    };

    let session_start = [
        SessionStartOutput::no_op(),
        SessionStartOutput::inject(InjectStep::system_message("one")),
        SessionStartOutput::inject_all(steps()),
        SessionStartOutput::no_op().with_step(InjectStep::user_message("appended")),
    ];
    for output in session_start {
        emitted_json("session-start", &SessionStart::emit(output).unwrap());
    }

    let mut pre_invocation = vec![
        PreInvocationOutput::no_op(),
        PreInvocationOutput::inject(InjectStep::user_message("one")),
        PreInvocationOutput::inject_all(steps()),
        PreInvocationOutput::no_op().with_step(InjectStep::system_message("appended")),
        PreInvocationOutput::block("blocked"),
        PreInvocationOutput::block("blocked").with_step(InjectStep::system_message("why")),
    ];
    for behavior in [
        PreInvocationTermination::Default,
        PreInvocationTermination::Terminate,
    ] {
        pre_invocation.push(PreInvocationOutput::no_op().with_termination_behavior(behavior));
        pre_invocation
            .push(PreInvocationOutput::inject_all(steps()).with_termination_behavior(behavior));
    }
    for output in pre_invocation {
        emitted_json("pre-invocation", &PreInvocation::emit(output).unwrap());
    }

    let mut post_invocation = vec![
        PostInvocationOutput::no_op(),
        PostInvocationOutput::inject(InjectStep::system_message("one")),
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
        PreToolUseOutput::auto_approve(),
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
                    .clone()
                    .with_reason("because")
                    .with_permission_overrides(["command(npm test)", "read_file(/a)"]),
            )
            .unwrap(),
        );
        assert_eq!(value["permissionOverrides"].as_array().unwrap().len(), 2);
        for overwrite in [
            serde_json::json!({}),
            serde_json::json!({"Cwd": "/w", "Nested": {"a": [1, null]}, "Null": null}),
        ] {
            emitted_json(
                "pre-tool-use",
                &PreToolUse::emit(output.clone().with_overwrite(args(overwrite))).unwrap(),
            );
        }
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
                (
                    "pre-invocation" | "post-invocation",
                    "inject-step-mutually-exclusive" | "two-step-kinds",
                ) => {}
                // Only the deprecated `InjectStep::tool_call` emits the
                // withdrawn step, and the schemas reject what it emits.
                (
                    "session-start" | "pre-invocation" | "post-invocation",
                    "inject-tool-call-undocumented",
                ) => {
                    #[allow(deprecated)]
                    let step =
                        InjectStep::tool_call(args(serde_json::json!({"name": "run_command"})));
                    let value = serde_json::json!({ "injectSteps": [step] });
                    assert_eq!(value, fixture["value"], "{event_dir}/{id}");
                    assert!(!validator.is_valid(&value));
                }
                // `SystemMessageStep` and `ErrorMessageStep` are objects with
                // a required text member.
                (
                    "session-start" | "pre-invocation" | "post-invocation",
                    "system-message-not-object",
                )
                | (
                    "pre-invocation",
                    "system-message-missing-text" | "error-message-missing-user-message",
                ) => {}
                // `SessionStartOutput` has no `terminationBehavior`.
                ("session-start", "termination-behavior-undocumented") => {}
                // `PreInvocationTermination` has no `force_continue`, and both
                // termination enums are closed to the documented values.
                ("pre-invocation", "force-continue-undocumented" | "unknown-termination")
                | ("post-invocation", "unknown-termination-behavior") => {}
                // `ToolDecision` is closed and has no empty value, and
                // `decision` is a required member.
                (
                    "pre-tool-use",
                    "empty-decision"
                    | "unknown-decision"
                    | "auto-approve-wrong-case"
                    | "overwrite-without-decision",
                ) => {}
                // `overwrite` is typed as a JSON object.
                ("pre-tool-use", "overwrite-not-object" | "overwrite-array") => {}
                // `PostToolUseOutput` always emits exactly `{}`.
                ("post-tool-use", "non-empty-response") => {}
                ("pre-tool-use", "duplicate-permission-override") => {
                    let output =
                        PreToolUseOutput::ask().with_permission_override("command(npm test)");
                    let mut duplicated = output.clone();
                    duplicated
                        .permission_overrides
                        .push("command(npm test)".into());
                    let value = emitted_json(event_dir, &PreToolUse::emit(duplicated).unwrap());
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
                // `StopOutput` always serializes a decision.
                ("stop", "empty-object") => {}
                _ => panic!("{event_dir}/{id}: classify this output negative"),
            }
        }
    }
}
