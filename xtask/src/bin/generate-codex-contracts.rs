use base64::Engine as _;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

const SNAPSHOT: &str = "commit-9e552e9-r1";
const REVISION: &str = "9e552e9d15ba52bed7077d5357f3e18e330f8f38";
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

struct Seed {
    file_key: &'static str,
    wire_name: &'static str,
    rust_key: &'static str,
    category: &'static str,
    block_effect: Option<&'static str>,
    text_context: bool,
    structured: Value,
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask parent")
        .to_path_buf();
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
            structured: json!({"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"Load repository conventions."}}),
        },
        Seed {
            file_key: "subagent-start",
            wire_name: "SubagentStart",
            rust_key: "subagent_start",
            category: "subagent",
            block_effect: None,
            text_context: true,
            structured: json!({"hookSpecificOutput":{"hookEventName":"SubagentStart","additionalContext":"Review test conventions."}}),
        },
        Seed {
            file_key: "permission-request",
            wire_name: "PermissionRequest",
            rust_key: "permission_request",
            category: "tool",
            block_effect: Some("deny"),
            text_context: false,
            structured: json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Blocked by policy."}}}),
        },
        Seed {
            file_key: "post-tool-use",
            wire_name: "PostToolUse",
            rust_key: "post_tool_use",
            category: "tool",
            block_effect: Some("replace-result"),
            text_context: false,
            structured: json!({"decision":"block","reason":"Review the output.","hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"Generated files changed."}}),
        },
        Seed {
            file_key: "pre-compact",
            wire_name: "PreCompact",
            rust_key: "pre_compact",
            category: "compaction",
            block_effect: Some("deny"),
            text_context: false,
            structured: json!({"continue":false,"stopReason":"Save state before compacting."}),
        },
        Seed {
            file_key: "post-compact",
            wire_name: "PostCompact",
            rust_key: "post_compact",
            category: "compaction",
            block_effect: None,
            text_context: false,
            structured: json!({"systemMessage":"Compaction completed."}),
        },
        Seed {
            file_key: "user-prompt-submit",
            wire_name: "UserPromptSubmit",
            rust_key: "user_prompt_submit",
            category: "prompt",
            block_effect: Some("deny"),
            text_context: true,
            structured: json!({"decision":"block","reason":"Ask for confirmation.","hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"Clarify the reproduction."}}),
        },
        Seed {
            file_key: "subagent-stop",
            wire_name: "SubagentStop",
            rust_key: "subagent_stop",
            category: "subagent",
            block_effect: Some("continue"),
            text_context: false,
            structured: json!({"decision":"block","reason":"Run another focused pass."}),
        },
        Seed {
            file_key: "stop",
            wire_name: "Stop",
            rust_key: "stop",
            category: "turn",
            block_effect: Some("continue"),
            text_context: false,
            structured: json!({"decision":"block","reason":"Run the failing tests again."}),
        },
    ]
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

    let mut output =
        read_json(&vendor.join(format!("{}.command.output.schema.json", seed.file_key)));
    normalize_schema(&mut output, &seed, "command-output");
    write_json(&event_dir.join("output.command.schema.json"), &output);

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

fn contract(seed: &Seed) -> Value {
    let mut outcomes = vec![json!({
        "id":"structured",
        "effect":"event-specific-control",
        "exit":{"exact":0},
        "stdout":{"presence":"required","role":"protocol-value","content_kind":"json"},
        "stderr":{"presence":"optional","role":"diagnostics","content_kind":"text","encoding":"utf-8"},
        "output_schema":"command-response",
        "sources":["codex-hooks-reference","codex-generated-schemas"],
        "assurance":{"confidence":"high","verification":"source-reviewed"}
    })];
    if seed.text_context {
        outcomes.push(json!({
            "id":"text-context",
            "effect":"provide-context",
            "exit":{"exact":0},
            "stdout":{"presence":"required","role":"agent-context","content_kind":"text","encoding":"utf-8","trailing_newline":"allowed"},
            "stderr":{"presence":"optional","role":"diagnostics","content_kind":"text","encoding":"utf-8"},
            "sources":["codex-hooks-reference"],
            "assurance":{"confidence":"high","verification":"source-reviewed"}
        }));
    }
    if let Some(effect) = seed.block_effect {
        outcomes.push(json!({
            "id":"exit-2",
            "effect":effect,
            "exit":{"exact":2},
            "stdout":{"presence":"forbidden","role":"none","content_kind":"empty"},
            "stderr":{"presence":"required","role":"agent-context","content_kind":"text","encoding":"utf-8"},
            "sources":["codex-hooks-reference"],
            "assurance":{"confidence":"high","verification":"source-reviewed"}
        }));
    }
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
            "input":{"file":"input.schema.json","origin":"derived","sources":["codex-hooks-reference","codex-generated-schemas"],"assurance":{"confidence":"high","verification":"source-reviewed"}},
            "outputs":[{"id":"command-response","file":"output.command.schema.json","origin":"derived","sources":["codex-hooks-reference","codex-generated-schemas"],"assurance":{"confidence":"high","verification":"source-reviewed"}}]
        },
        "bindings":{
            "command":{
                "kind":"process",
                "request":{"channel":"stdin","framing":"single-document-at-eof","content_kind":"json"},
                "outcomes":outcomes
            }
        },
        "fixtures":"fixtures.yaml",
        "uncertainties":["The official generated schema includes parsed compatibility fields; runtime support restrictions from the hooks reference remain controlling."]
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
    let missing = required
        .iter()
        .filter_map(Value::as_str)
        .find(|field| !common.contains(field))
        .unwrap_or_else(|| required[0].as_str().expect("required name"));
    let mut missing_value = minimal.clone();
    missing_value
        .as_object_mut()
        .expect("input object")
        .remove(missing);

    let structured_bytes = serde_json::to_vec(&seed.structured).expect("serialize output");
    let mut process = vec![json!({
        "id":"structured",
        "binding":"command",
        "outcome":"structured",
        "exit_code":0,
        "stdout_base64":base64::engine::general_purpose::STANDARD.encode(&structured_bytes),
        "stdout_sha256":sha256(&structured_bytes),
        "stderr_base64":"",
        "stderr_sha256":EMPTY_SHA256
    })];
    if seed.text_context {
        let text = b"Hook-provided developer context.";
        process.push(json!({
            "id":"text-context",
            "binding":"command",
            "outcome":"text-context",
            "exit_code":0,
            "stdout_base64":base64::engine::general_purpose::STANDARD.encode(text),
            "stdout_sha256":sha256(text),
            "stderr_base64":"",
            "stderr_sha256":EMPTY_SHA256
        }));
    }
    if seed.block_effect.is_some() {
        let reason = b"blocked by hook";
        process.push(json!({
            "id":"exit-2",
            "binding":"command",
            "outcome":"exit-2",
            "exit_code":2,
            "stdout_base64":"",
            "stdout_sha256":EMPTY_SHA256,
            "stderr_base64":base64::engine::general_purpose::STANDARD.encode(reason),
            "stderr_sha256":sha256(reason)
        }));
    }

    json!({
        "format_version":1,
        "input":{
            "positive":[
                {"id":"minimal","origin":"synthesized","sources":["codex-generated-schemas"],"value":minimal},
                {"id":"representative","origin":"synthesized","sources":["codex-hooks-reference","codex-generated-schemas"],"value":representative}
            ],
            "negative":[
                {"id":"wrong-discriminator","origin":"regression","sources":["codex-generated-schemas"],"value":wrong,"expected_pointer":"/hook_event_name"},
                {"id":format!("missing-{missing}"),"origin":"synthesized","sources":["codex-generated-schemas"],"value":missing_value,"expected_pointer":""}
            ]
        },
        "output":[
            {"id":"structured","schema":"command-response","origin":"synthesized","sources":["codex-hooks-reference","codex-generated-schemas"],"value":seed.structured}
        ],
        "process":process
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
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str)
        && let Some(key) = reference.strip_prefix("#/definitions/")
    {
        return example_value(name, &definitions[key], definitions);
    }
    if let Some(all_of) = schema.get("allOf").and_then(Value::as_array)
        && let Some(first) = all_of.first()
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
