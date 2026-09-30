//! Generates the draft Codex event contracts for one snapshot from the vendored
//! upstream schemas.
//!
//! `PreToolUse` is hand-authored in each snapshot and is intentionally not
//! seeded here. Frozen snapshots are immutable: point `SNAPSHOT`/`REVISION` at a
//! new draft before running this generator.

use base64::Engine as _;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

const SNAPSHOT: &str = "commit-ff6aec9-r1";
const REVISION: &str = "ff6aec96948b70d94983af2641a6b67c94faeff5";

/// Stderr written by the synthesized non-zero-exit failure fixtures.
const FAILURE_STDERR: &[u8] = b"hook failed\n";
/// Stderr that Codex trims to nothing, so exit 2 cannot block with it.
const BLANK_STDERR: &[u8] = b" \n";

const COMPATIBILITY_FIELDS: &str = "The official generated schemas include parsed compatibility fields; runtime support restrictions from the hooks reference and pinned source remain controlling.";
const EXIT_ZERO_STDERR: &str = "Codex never reads stderr from a command that exits 0, so exit-0 outcomes model stderr as ignored rather than as diagnostics.";
const ASYNC_HANDLERS: &str = "Command handlers with `async: true` (Codex >= 0.148.0) run in the background: Codex ignores every control effect (decision, permissionDecision, updatedInput, PermissionRequest decisions, continue:false, stopReason, and exit 2), treats exit 2 as a plain failure, and delivers only additionalContext and systemMessage at the next safe point. The stdin payload carries no execution-mode marker, so these outcomes describe synchronous handlers.";
const HANDLER_KINDS: &str = "Codex >= 0.148.0 also runs `mcp_tool` handlers, always synchronously, and parses a successful tool result like exit-0 stdout; `prompt` and `agent` handlers are parsed but skipped. Only the command binding is modeled.";
const PROCESS_LIFECYCLE: &str = "Since Codex 0.149.0 command hooks receive a replay of the environment Codex captured when the session hook registry was created, not the live environment; since 0.155.0 they start in a new session without a controlling terminal and their timeout also covers stdin delivery. On timeout, I/O failure, or turn abort Codex SIGKILLs the hook's whole process group.";
const CLOUD_ORCHESTRATION: &str = "Codex does not dispatch command hooks for Work threads that use cloud orchestration, even when tools execute locally; only admin-managed remote mcp_tool hooks run there.";
const TEXT_CONTEXT: &str = "Codex trims plain-text stdout before using it as context, and treats stdout whose first non-whitespace character is `{` or `[` as JSON: such text fails the run when it is not a valid output object, and a valid object such as `{}` is consumed as structured output without context.";
const BLOCKING_EXIT_2: &str = "Exit 2 selects the exit-2 outcome only when stderr is non-empty after trimming; exit 2 with empty or whitespace-only stderr is a failed run (failure outcome) and has no blocking effect.";
const BLOCK_REASON: &str = "`decision: block` requires a `reason` that is non-empty after trimming unless `continue: false` takes precedence; otherwise Codex fails the synchronous run without blocking. The output schema adds this runtime rule, which the generated schema does not express.";
const TOOL_HOOK_SCOPE: &str = "Since Codex 0.157.0 `cwd` and the process working directory are the step's local-environment cwd when one is selected (falling back to the turn cwd), so they can differ from the cwd reported by turn-scoped events; the hooks reference still says the session cwd. Since 0.156.0 `model` and `permission_mode` come from the step's captured settings (PermissionRequest: the review context) and can change within a turn.";
const SILENT_FAILURE: &str = "For non-zero exits that do not select a blocking outcome Codex records only `hook exited with code N`, discards stderr, and continues (fail-open); a process that dies without an exit status is also a failed run.";
const VISIBLE_FAILURE: &str = "For non-zero exits Codex marks the run failed and reports the trimmed stderr (or `hook exited with code N` when stderr is blank) as the failure message; exit 2 has no special meaning and the operation continues.";

struct Seed {
    file_key: &'static str,
    wire_name: &'static str,
    rust_key: &'static str,
    category: &'static str,
    /// Effect of a synchronous exit 2 with non-blank stderr, when Codex gives exit 2 one.
    block_effect: Option<&'static str>,
    /// Plain-text exit-0 stdout becomes model context.
    text_context: bool,
    /// Representative structured stdout; `None` when Codex ignores exit-0 output.
    structured: Option<Value>,
    /// Codex reports trimmed stderr as the failure message for non-zero exits.
    failure_stderr_visible: bool,
    /// Structured `decision: block` requires a reason that is non-empty after trimming.
    block_reason_required: bool,
    /// Pinned core source controls part of this event's input semantics.
    core_semantics: bool,
    uncertainties: &'static [&'static str],
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask parent")
        .to_path_buf();
    refuse_frozen_snapshot(&root);
    for seed in seeds() {
        generate(&root, seed);
    }
}

fn seeds() -> Vec<Seed> {
    vec![
        Seed {
            file_key: "session-start",
            wire_name: "SessionStart",
            rust_key: "session_start",
            category: "session",
            block_effect: None,
            text_context: true,
            structured: Some(
                json!({"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"Load repository conventions."}}),
            ),
            failure_stderr_visible: false,
            block_reason_required: false,
            core_semantics: true,
            uncertainties: &[
                "Codex >= 0.155.0 reports `fork` for threads forked from a parent thread (also the matcher input) and `resume` instead of `startup` for thread/resume with supplied history. The hooks reference still lists only startup, resume, clear, and compact; the pinned release schema and core source control.",
                TEXT_CONTEXT,
                "`continue: false` from a synchronous handler ends the turn without another model request.",
            ],
        },
        Seed {
            file_key: "session-end",
            wire_name: "SessionEnd",
            rust_key: "session_end",
            category: "session",
            block_effect: None,
            text_context: false,
            structured: None,
            failure_stderr_visible: true,
            block_reason_required: false,
            core_semantics: false,
            uncertainties: &[
                "SessionEnd is advisory: Codex ignores stdout at exit 0, never runs it for subagents, and `reason` is always `other`.",
                "SessionEnd handlers always run synchronously even when `async` is true, default to a one-second timeout clamped to one through three seconds, and reject `mcp_tool` handlers; `prompt` and `agent` handlers are parsed but skipped.",
            ],
        },
        Seed {
            file_key: "subagent-start",
            wire_name: "SubagentStart",
            rust_key: "subagent_start",
            category: "subagent",
            block_effect: None,
            text_context: true,
            structured: Some(
                json!({"hookSpecificOutput":{"hookEventName":"SubagentStart","additionalContext":"Review test conventions."}}),
            ),
            failure_stderr_visible: false,
            block_reason_required: false,
            core_semantics: false,
            uncertainties: &[
                TEXT_CONTEXT,
                "`continue: false` is parsed for compatibility but does not stop the subagent from starting.",
            ],
        },
        Seed {
            file_key: "permission-request",
            wire_name: "PermissionRequest",
            rust_key: "permission_request",
            category: "tool",
            block_effect: Some("deny"),
            text_context: false,
            structured: Some(
                json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Blocked by policy."}}}),
            ),
            failure_stderr_visible: false,
            block_reason_required: false,
            core_semantics: true,
            uncertainties: &[
                TOOL_HOOK_SCOPE,
                BLOCKING_EXIT_2,
                "A structured deny whose message is missing, empty, or whitespace-only is still a denial; Codex substitutes `PermissionRequest hook denied approval`. `updatedInput`, `updatedPermissions`, and `interrupt: true` fail closed, and `continue: false`, `stopReason`, and `suppressOutput` fail the run.",
            ],
        },
        Seed {
            file_key: "post-tool-use",
            wire_name: "PostToolUse",
            rust_key: "post_tool_use",
            category: "tool",
            block_effect: Some("replace-result"),
            text_context: false,
            structured: Some(
                json!({"decision":"block","reason":"Review the output.","hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"Generated files changed."}}),
            ),
            failure_stderr_visible: false,
            block_reason_required: true,
            core_semantics: true,
            uncertainties: &[
                TOOL_HOOK_SCOPE,
                BLOCKING_EXIT_2,
                BLOCK_REASON,
                "`reason` without `decision: block` fails the run unless `continue` is false; `suppressOutput` and `updatedMCPToolOutput` are parsed but fail the run.",
            ],
        },
        Seed {
            file_key: "pre-compact",
            wire_name: "PreCompact",
            rust_key: "pre_compact",
            category: "compaction",
            block_effect: None,
            text_context: false,
            structured: Some(
                json!({"continue":false,"stopReason":"Save state before compacting."}),
            ),
            failure_stderr_visible: true,
            block_reason_required: false,
            core_semantics: false,
            uncertainties: &[
                "Plain text on stdout is ignored at exit 0. A synchronous `continue: false` stops before compacting; on a manual compact this aborts the turn as interrupted, which also dispatches Interrupt.",
            ],
        },
        Seed {
            file_key: "post-compact",
            wire_name: "PostCompact",
            rust_key: "post_compact",
            category: "compaction",
            block_effect: None,
            text_context: false,
            structured: Some(json!({"systemMessage":"Compaction completed."})),
            failure_stderr_visible: true,
            block_reason_required: false,
            core_semantics: false,
            uncertainties: &["Plain text on stdout is ignored at exit 0."],
        },
        Seed {
            file_key: "user-prompt-submit",
            wire_name: "UserPromptSubmit",
            rust_key: "user_prompt_submit",
            category: "prompt",
            block_effect: Some("deny"),
            text_context: true,
            structured: Some(
                json!({"decision":"block","reason":"Ask for confirmation.","hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"Clarify the reproduction."}}),
            ),
            failure_stderr_visible: false,
            block_reason_required: true,
            core_semantics: false,
            uncertainties: &[TEXT_CONTEXT, BLOCKING_EXIT_2, BLOCK_REASON],
        },
        Seed {
            file_key: "subagent-stop",
            wire_name: "SubagentStop",
            rust_key: "subagent_stop",
            category: "subagent",
            block_effect: Some("continue"),
            text_context: false,
            structured: Some(json!({"decision":"block","reason":"Run another focused pass."})),
            failure_stderr_visible: false,
            block_reason_required: true,
            core_semantics: false,
            uncertainties: &[
                "Synchronous handlers must write JSON or nothing at exit 0; plain text fails the run.",
                BLOCKING_EXIT_2,
                BLOCK_REASON,
            ],
        },
        Seed {
            file_key: "stop",
            wire_name: "Stop",
            rust_key: "stop",
            category: "turn",
            block_effect: Some("continue"),
            text_context: false,
            structured: Some(json!({"decision":"block","reason":"Run the failing tests again."})),
            failure_stderr_visible: false,
            block_reason_required: true,
            core_semantics: true,
            uncertainties: &[
                "Synchronous handlers must write JSON or nothing at exit 0; plain text fails the run.",
                BLOCKING_EXIT_2,
                BLOCK_REASON,
                "Since Codex 0.150.0 managed-source Stop hooks (system, MDM, cloud, and legacy managed configuration) also fire for internal memory-consolidation turns with an ordinary Stop payload; user, project, session-flag, and plugin Stop hooks do not, and a block there ends the memory worker with an error instead of continuing.",
                "The pinned core source dispatches Interrupt, not Stop, from the path that aborts an interrupted turn; this has not been live-observed.",
            ],
        },
        Seed {
            file_key: "interrupt",
            wire_name: "Interrupt",
            rust_key: "interrupt",
            category: "turn",
            block_effect: None,
            text_context: false,
            structured: Some(
                json!({"systemMessage":"Saved the interrupted turn to the local audit log."}),
            ),
            failure_stderr_visible: false,
            block_reason_required: false,
            core_semantics: true,
            uncertainties: &[
                "Interrupt (Codex >= 0.150.0) runs for an active main-thread turn aborted as interrupted, after Codex flushes the transcript and before it reports the aborted turn. It never runs for subagents or idle threads, ignores any configured matcher, and carries no agent_id or agent_type.",
                "The hooks reference describes user interrupts only; the pinned core source and tests also dispatch Interrupt for self-aborts reported as interrupted, such as a manual compact stopped by a PreCompact `continue: false`.",
                "Output cannot prevent the interruption or restart the turn. Codex accepts empty stdout or a JSON object whose only member is `systemMessage`, which surfaces as a warning; plain text, any other member (including continue, stopReason, suppressOutput, and decision), and every non-zero exit (including 2) fail the run.",
                "The runtime deserializes `systemMessage` as an optional string and therefore also accepts null, but the generated schema declares a string; emit a string or omit the member.",
                "Command handlers default to a one-second timeout clamped to one through three seconds, including when `async` is true.",
            ],
        },
    ]
}

/// Frozen evidence is immutable, so regenerating into a frozen snapshot is an
/// error rather than a silent manifest mismatch.
fn refuse_frozen_snapshot(root: &Path) {
    let index = root
        .join("contracts/harnesses/codex/snapshots")
        .join(SNAPSHOT)
        .join("snapshot.yaml");
    let Ok(text) = fs::read_to_string(&index) else {
        return;
    };
    let snapshot: Value = serde_yaml_ng::from_str(&text)
        .unwrap_or_else(|error| panic!("{}: {error}", index.display()));
    assert!(
        snapshot["state"] != "frozen",
        "{SNAPSHOT} is frozen; create a successor snapshot instead of regenerating it"
    );
}

fn generate(root: &Path, seed: Seed) {
    let vendor = root
        .join("contracts/vendor/codex")
        .join(REVISION)
        .join("generated");
    let event_dir = root
        .join("contracts/harnesses/codex/snapshots")
        .join(SNAPSHOT)
        .join("events")
        .join(seed.file_key);
    fs::create_dir_all(&event_dir).expect("create Codex event directory");

    let mut input = read_json(&vendor.join(format!("{}.command.input.schema.json", seed.file_key)));
    normalize_schema(&mut input, &seed, "input");
    write_json(&event_dir.join("input.schema.json"), &input);

    if seed.structured.is_some() {
        let mut output =
            read_json(&vendor.join(format!("{}.command.output.schema.json", seed.file_key)));
        normalize_schema(&mut output, &seed, "command-output");
        if seed.block_reason_required {
            require_block_reason(&mut output);
        }
        write_json(&event_dir.join("output.command.schema.json"), &output);
    }

    let contract = contract(&seed);
    write_yaml(&event_dir.join("contract.yaml"), &contract);
    let fixtures = fixtures(&seed, &input);
    write_yaml(&event_dir.join("fixtures.yaml"), &fixtures);
}

fn normalize_schema(schema: &mut Value, seed: &Seed, suffix: &str) {
    let object = schema.as_object_mut().expect("official schema object");
    object.insert(
        "$schema".to_string(),
        Value::String("https://json-schema.org/draft/2020-12/schema".to_string()),
    );
    object.insert(
        "$id".to_string(),
        Value::String(format!(
            "urn:agent-hook-kit:contracts:codex:{SNAPSHOT}:{}:{suffix}",
            seed.file_key
        )),
    );
}

/// Narrows a generated output schema to the synchronous runtime rule that a
/// structured block needs a reason that is non-empty after trimming.
fn require_block_reason(schema: &mut Value) {
    let object = schema.as_object_mut().expect("official schema object");
    assert!(
        object["properties"].get("decision").is_some() && !object.contains_key("allOf"),
        "block-reason narrowing needs a root decision property and no existing allOf"
    );
    object.insert(
        "allOf".to_string(),
        // `continue: false` takes precedence over the block, so only a
        // continuing block needs a reason.
        json!([{
            "if": {"properties": {"decision": {"const": "block"}, "continue": {"const": true}}, "required": ["decision"]},
            "then": {"properties": {"reason": {"type": "string", "pattern": "\\S"}}, "required": ["reason"]}
        }]),
    );
}

fn is_session_end(seed: &Seed) -> bool {
    seed.wire_name == "SessionEnd"
}

fn ignored_stdout() -> Value {
    json!({"presence":"optional","role":"ignored","content_kind":"opaque"})
}

fn ignored_stderr() -> Value {
    json!({"presence":"optional","role":"ignored","content_kind":"opaque"})
}

fn contract(seed: &Seed) -> Value {
    let mut outcomes = Vec::new();
    if seed.structured.is_some() {
        outcomes.push(json!({
            "id":"structured",
            "effect":"event-specific-control",
            "exit":{"exact":0},
            "stdout":{"presence":"required","role":"protocol-value","content_kind":"json"},
            "stderr":ignored_stderr(),
            "output_schema":"command-response",
            "sources":["codex-hooks-reference","codex-generated-schemas","codex-hooks-source"],
            "assurance":{"confidence":"high","verification":"source-reviewed"}
        }));
    }
    outcomes.push(json!({
        "id":"no-op",
        "effect":"none",
        "exit":{"exact":0},
        "stdout": if seed.structured.is_some() {
            json!({"presence":"optional","role":"none","content_kind":"empty"})
        } else {
            ignored_stdout()
        },
        "stderr":ignored_stderr(),
        "sources":["codex-hooks-reference","codex-hooks-source"],
        "assurance":{"confidence":"high","verification":"source-reviewed"}
    }));
    if seed.text_context {
        outcomes.push(json!({
            "id":"text-context",
            "effect":"provide-context",
            "exit":{"exact":0},
            "stdout":{"presence":"required","role":"agent-context","content_kind":"text","encoding":"utf-8","trailing_newline":"allowed"},
            "stderr":ignored_stderr(),
            "sources":["codex-hooks-reference","codex-hooks-source"],
            "assurance":{"confidence":"high","verification":"source-reviewed"}
        }));
    }
    if let Some(effect) = seed.block_effect {
        outcomes.push(json!({
            "id":"exit-2",
            "effect":effect,
            "exit":{"exact":2},
            "stdout":ignored_stdout(),
            "stderr":{"presence":"required","role":"agent-context","content_kind":"text","encoding":"utf-8"},
            "sources":["codex-hooks-reference","codex-hooks-source"],
            "assurance":{"confidence":"high","verification":"source-reviewed"}
        }));
    }
    outcomes.push(json!({
        "id":"failure",
        "effect":"nonblocking-error",
        "exit":{"range":{"min":1,"max":255}},
        "stdout":ignored_stdout(),
        "stderr": if seed.failure_stderr_visible {
            json!({"presence":"optional","role":"diagnostics","content_kind":"text","encoding":"utf-8"})
        } else {
            ignored_stderr()
        },
        "sources":["codex-hooks-reference","codex-hooks-source"],
        "assurance":{"confidence":"high","verification":"source-reviewed"}
    }));

    let mut input_sources = vec!["codex-hooks-reference", "codex-generated-schemas"];
    if seed.core_semantics {
        input_sources.push("codex-core-source");
    }
    let mut output_sources = vec!["codex-hooks-reference", "codex-generated-schemas"];
    if seed.block_reason_required {
        output_sources.push("codex-hooks-source");
    }

    let mut uncertainties: Vec<&str> = seed.uncertainties.to_vec();
    uncertainties.push(COMPATIBILITY_FIELDS);
    uncertainties.push(EXIT_ZERO_STDERR);
    uncertainties.push(if seed.failure_stderr_visible {
        VISIBLE_FAILURE
    } else {
        SILENT_FAILURE
    });
    if !is_session_end(seed) {
        uncertainties.push(ASYNC_HANDLERS);
        uncertainties.push(HANDLER_KINDS);
    }
    uncertainties.push(PROCESS_LIFECYCLE);
    uncertainties.push(CLOUD_ORCHESTRATION);

    json!({
        "format_version":1,
        "id":format!("codex/{SNAPSHOT}/{}",seed.wire_name),
        "harness":"codex",
        "snapshot":SNAPSHOT,
        "event":{
            "wire_name":seed.wire_name,
            "rust_key":seed.rust_key,
            "category":seed.category,
            "identification":{
                "inferability":"definitive",
                "discriminator":{"json_pointer":"/hook_event_name","const":seed.wire_name}
            }
        },
        "schemas":{
            "input":{"file":"input.schema.json","origin":"derived","sources":input_sources,"assurance":{"confidence":"high","verification":"source-reviewed"}},
            "outputs": if seed.structured.is_some() {
                vec![json!({"id":"command-response","file":"output.command.schema.json","origin":"derived","sources":output_sources,"assurance":{"confidence":"high","verification":"source-reviewed"}})]
            } else {
                Vec::<Value>::new()
            }
        },
        "bindings":{
            "command":{
                "kind":"process",
                "request":{"channel":"stdin","framing":"single-document-at-eof","content_kind":"json"},
                "outcomes":outcomes
            }
        },
        "handler_kinds": if is_session_end(seed) { json!(["command"]) } else { json!(["command","mcp_tool"]) },
        "fixtures":"fixtures.yaml",
        "uncertainties":uncertainties
    })
}

fn fixtures(seed: &Seed, input_schema: &Value) -> Value {
    let minimal = example_input(input_schema);
    let mut representative = minimal.clone();
    let representative_object = representative.as_object_mut().expect("input object");
    let properties = input_schema["properties"].as_object().expect("properties");
    if properties.contains_key("agent_id") {
        representative_object.insert("agent_id".to_string(), json!("agent-1"));
    }
    if properties.contains_key("agent_type") {
        representative_object.insert("agent_type".to_string(), json!("reviewer"));
    }
    if representative_object.get("transcript_path") == Some(&Value::Null) {
        representative_object.insert(
            "transcript_path".to_string(),
            json!("/tmp/transcript.jsonl"),
        );
    }

    let mut positive = vec![
        json!({"id":"minimal","origin":"synthesized","sources":["codex-generated-schemas"],"value":minimal}),
        json!({"id":"representative","origin":"synthesized","sources":["codex-hooks-reference","codex-generated-schemas"],"value":representative}),
    ];
    if seed.wire_name == "SessionStart" {
        let mut fork = representative.clone();
        fork["source"] = json!("fork");
        positive.push(json!({
            "id":"fork-source",
            "origin":"regression",
            "sources":["codex-generated-schemas","codex-core-source"],
            "value":fork
        }));
    }

    let mut wrong = minimal.clone();
    wrong["hook_event_name"] = json!("WrongEvent");
    let required = input_schema["required"].as_array().expect("required array");
    let common = [
        "cwd",
        "hook_event_name",
        "model",
        "permission_mode",
        "session_id",
        "transcript_path",
        "turn_id",
    ];
    let missing = if seed.wire_name == "Interrupt" {
        // Every Interrupt field is shared; the interrupted turn id is the
        // event's reason to exist.
        "turn_id"
    } else {
        required
            .iter()
            .filter_map(Value::as_str)
            .find(|field| !common.contains(field))
            .unwrap_or_else(|| required[0].as_str().expect("required name"))
    };
    let mut missing_value = minimal.clone();
    missing_value
        .as_object_mut()
        .expect("input object")
        .remove(missing);
    let mut negative = vec![
        json!({"id":"wrong-discriminator","origin":"regression","sources":["codex-generated-schemas"],"value":wrong,"expected_pointer":"/hook_event_name","expected_keyword":"const"}),
        json!({"id":format!("missing-{missing}"),"origin":"synthesized","sources":["codex-generated-schemas"],"value":missing_value,"expected_pointer":"","expected_keyword":"required"}),
    ];
    if seed.wire_name == "SessionEnd" {
        let mut invalid_reason = minimal.clone();
        invalid_reason["reason"] = json!("exit");
        negative.push(json!({
            "id":"invalid-reason",
            "origin":"regression",
            "sources":["codex-hooks-reference","codex-generated-schemas"],
            "value":invalid_reason,
            "expected_pointer":"/reason",
            "expected_keyword":"const"
        }));
    }
    if seed.wire_name == "SessionStart" {
        let mut unknown_source = minimal.clone();
        unknown_source["source"] = json!("restore");
        negative.push(json!({
            "id":"unknown-source",
            "origin":"regression",
            "sources":["codex-generated-schemas"],
            "value":unknown_source,
            "expected_pointer":"/source",
            "expected_keyword":"enum"
        }));
    }

    let mut output = Vec::new();
    let mut output_negative = Vec::new();
    let mut process = Vec::new();
    if let Some(structured) = &seed.structured {
        let structured_bytes = serde_json::to_vec(structured).expect("serialize output");
        let (origin, sources) = if seed.wire_name == "Interrupt" {
            // Verbatim example from the hooks reference.
            ("official", json!(["codex-hooks-reference"]))
        } else {
            (
                "synthesized",
                json!(["codex-hooks-reference", "codex-generated-schemas"]),
            )
        };
        output.push(json!({
            "id":"structured",
            "schema":"command-response",
            "origin":origin,
            "sources":sources,
            "value":structured
        }));
        process.push(process_case(
            "structured",
            "structured",
            0,
            &structured_bytes,
            b"",
        ));
        for (id, value) in documented_outputs(seed) {
            let bytes = serde_json::to_vec(&value).expect("serialize output");
            output.push(json!({
                "id":id,
                "schema":"command-response",
                "origin":"official",
                "sources":["codex-hooks-reference"],
                "value":value
            }));
            process.push(process_case(
                &format!("structured-{id}"),
                "structured",
                0,
                &bytes,
                b"",
            ));
        }
    }
    process.push(process_case("no-op", "no-op", 0, b"", b""));
    if seed.text_context {
        process.push(process_case(
            "text-context",
            "text-context",
            0,
            b"Hook-provided developer context.",
            b"",
        ));
    }
    if seed.block_effect.is_some() {
        process.push(process_case("exit-2", "exit-2", 2, b"", b"blocked by hook"));
        process.push(process_case(
            "exit-2-blank-stderr",
            "failure",
            2,
            b"",
            BLANK_STDERR,
        ));
    } else if seed.structured.is_some() {
        process.push(process_case(
            "exit-2-failure",
            "failure",
            2,
            b"",
            FAILURE_STDERR,
        ));
    }
    process.push(process_case("failure", "failure", 1, b"", FAILURE_STDERR));

    if seed.block_reason_required {
        output_negative.push(json!({
            "id":"block-without-reason",
            "schema":"command-response",
            "origin":"regression",
            "sources":["codex-hooks-source"],
            "value":{"decision":"block"},
            "expected_pointer":"",
            "expected_keyword":"required"
        }));
        output_negative.push(json!({
            "id":"block-blank-reason",
            "schema":"command-response",
            "origin":"regression",
            "sources":["codex-hooks-source"],
            "value":{"decision":"block","reason":" \t"},
            "expected_pointer":"/reason",
            "expected_keyword":"pattern"
        }));
    }
    if seed.wire_name == "Interrupt" {
        output_negative.push(json!({
            "id":"universal-continue",
            "schema":"command-response",
            "origin":"regression",
            "sources":["codex-generated-schemas","codex-hooks-source"],
            "value":{"continue":true},
            "expected_pointer":"",
            "expected_keyword":"additionalProperties"
        }));
        output_negative.push(json!({
            "id":"decision-block",
            "schema":"command-response",
            "origin":"regression",
            "sources":["codex-generated-schemas","codex-hooks-source"],
            "value":{"decision":"block","reason":"Keep going."},
            "expected_pointer":"",
            "expected_keyword":"additionalProperties"
        }));
    }

    let mut fixtures = json!({
        "format_version":1,
        "input":{
            "positive":positive,
            "negative":negative
        },
        "output":output,
        "process":process
    });
    if !output_negative.is_empty() {
        fixtures["output_negative"] = Value::Array(output_negative);
    }
    fixtures
}

/// Verbatim hooks-reference examples of structured shapes that the
/// representative `structured` fixture does not already cover.
fn documented_outputs(seed: &Seed) -> Vec<(&'static str, Value)> {
    match seed.wire_name {
        "PermissionRequest" => vec![(
            "allow",
            json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}),
        )],
        "UserPromptSubmit" => vec![
            (
                "context",
                json!({"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"Ask for a clearer reproduction before editing files."}}),
            ),
            (
                "block",
                json!({"decision":"block","reason":"Ask for confirmation before doing that."}),
            ),
        ],
        _ => Vec::new(),
    }
}

fn process_case(id: &str, outcome: &str, exit_code: i32, stdout: &[u8], stderr: &[u8]) -> Value {
    json!({
        "id":id,
        "binding":"command",
        "outcome":outcome,
        "exit_code":exit_code,
        "stdout_base64":base64::engine::general_purpose::STANDARD.encode(stdout),
        "stdout_sha256":sha256(stdout),
        "stderr_base64":base64::engine::general_purpose::STANDARD.encode(stderr),
        "stderr_sha256":sha256(stderr)
    })
}

fn example_input(schema: &Value) -> Value {
    let properties = schema["properties"].as_object().expect("properties");
    let definitions = schema
        .get("definitions")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut value = Map::new();
    for name in schema["required"].as_array().expect("required") {
        let name = name.as_str().expect("required string");
        value.insert(
            name.to_string(),
            example_value(name, &properties[name], &definitions),
        );
    }
    Value::Object(value)
}

fn example_value(name: &str, schema: &Value, definitions: &Map<String, Value>) -> Value {
    if let Some(value) = schema.get("const") {
        return value.clone();
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return values.first().cloned().unwrap_or(Value::Null);
    }
    if let Some(key) = schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| reference.strip_prefix("#/definitions/"))
    {
        return example_value(name, &definitions[key], definitions);
    }
    if let Some(first) = schema
        .get("allOf")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
    {
        return example_value(name, first, definitions);
    }
    match schema.get("type") {
        Some(Value::String(kind)) if kind == "string" => json!(match name {
            "cwd" => "/repo",
            "model" => "gpt-test",
            "session_id" => "session-1",
            "turn_id" => "turn-1",
            "tool_name" => "Bash",
            "tool_use_id" => "call-1",
            "prompt" => "Run the tests",
            "source" => "startup",
            "trigger" => "manual",
            "agent_id" => "agent-1",
            "agent_type" => "reviewer",
            _ if name.ends_with("path") => "/tmp/transcript.jsonl",
            _ => "value",
        }),
        Some(Value::Array(types)) if types.iter().any(|kind| kind == "null") => Value::Null,
        Some(Value::String(kind)) if kind == "boolean" => Value::Bool(false),
        Some(Value::String(kind)) if kind == "integer" => json!(0),
        Some(Value::String(kind)) if kind == "array" => json!([]),
        Some(Value::String(kind)) if kind == "object" => json!({}),
        Some(Value::Bool(true)) | None => json!({}),
        _ => Value::Null,
    }
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(
        &fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display())),
    )
    .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn write_json(path: &Path, value: &Value) {
    let mut bytes = serde_json::to_vec_pretty(value).expect("serialize JSON");
    bytes.push(b'\n');
    fs::write(path, bytes).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
}

fn write_yaml(path: &Path, value: &Value) {
    let yaml = serde_yaml_ng::to_string(value).expect("serialize YAML");
    fs::write(path, yaml).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
