use base64::Engine as _;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

const SNAPSHOT: &str = "commit-f354eeb-r1";
const SOURCE: &str = "gemini-hooks-reference";
const SOURCE_CODE: &str = "gemini-hooks-source";
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

struct Seed {
    wire_name: &'static str,
    rust_key: &'static str,
    category: &'static str,
    fields: Vec<Field>,
    structured: Value,
    block_effect: Option<&'static str>,
}

struct Field {
    name: &'static str,
    schema: Value,
    required: bool,
    example: Value,
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask parent")
        .to_path_buf();
    for seed in seeds() {
        generate(&root, &seed);
    }
}

fn field(name: &'static str, schema: Value, example: Value) -> Field {
    Field {
        name,
        schema,
        required: true,
        example,
    }
}

fn optional(name: &'static str, schema: Value, example: Value) -> Field {
    Field {
        name,
        schema,
        required: false,
        example,
    }
}

fn seeds() -> Vec<Seed> {
    vec![
        Seed {
            wire_name: "BeforeTool",
            rust_key: "before_tool",
            category: "tool",
            fields: vec![
                field(
                    "tool_name",
                    json!({"type":"string"}),
                    json!("run_shell_command"),
                ),
                field(
                    "tool_input",
                    json!({"type":"object"}),
                    json!({"command":"cargo test"}),
                ),
                optional(
                    "mcp_context",
                    json!({"type":"object"}),
                    json!({"server":"fs"}),
                ),
                optional(
                    "original_request_name",
                    json!({"type":"string"}),
                    json!("run_shell_command"),
                ),
            ],
            structured: json!({"decision":"deny","reason":"Blocked by policy.","hookSpecificOutput":{"hookEventName":"BeforeTool","tool_input":{"command":"echo rewritten"}}}),
            block_effect: Some("deny"),
        },
        Seed {
            wire_name: "AfterTool",
            rust_key: "after_tool",
            category: "tool",
            fields: vec![
                field(
                    "tool_name",
                    json!({"type":"string"}),
                    json!("run_shell_command"),
                ),
                field(
                    "tool_input",
                    json!({"type":"object"}),
                    json!({"command":"cargo test"}),
                ),
                field(
                    "tool_response",
                    json!({"type":"object"}),
                    json!({"llmContent":"ok","returnDisplay":"ok"}),
                ),
                optional(
                    "mcp_context",
                    json!({"type":"object"}),
                    json!({"server":"fs"}),
                ),
                optional(
                    "original_request_name",
                    json!({"type":"string"}),
                    json!("run_shell_command"),
                ),
            ],
            structured: json!({"hookSpecificOutput":{"hookEventName":"AfterTool","additionalContext":"Review generated files.","tailToolCallRequest":{"name":"read_file","args":{"path":"README.md"}}}}),
            block_effect: Some("replace-result"),
        },
        Seed {
            wire_name: "BeforeAgent",
            rust_key: "before_agent",
            category: "agent",
            fields: vec![field(
                "prompt",
                json!({"type":"string"}),
                json!("Run the tests"),
            )],
            structured: json!({"hookSpecificOutput":{"hookEventName":"BeforeAgent","additionalContext":"Use repository conventions."}}),
            block_effect: Some("deny"),
        },
        Seed {
            wire_name: "AfterAgent",
            rust_key: "after_agent",
            category: "agent",
            fields: vec![
                field("prompt", json!({"type":"string"}), json!("Run the tests")),
                field(
                    "prompt_response",
                    json!({"type":"string"}),
                    json!("All tests pass."),
                ),
                field("stop_hook_active", json!({"type":"boolean"}), json!(false)),
            ],
            structured: json!({"decision":"deny","reason":"Verify the result again.","hookSpecificOutput":{"hookEventName":"AfterAgent","clearContext":false}}),
            block_effect: Some("retry"),
        },
        Seed {
            wire_name: "BeforeModel",
            rust_key: "before_model",
            category: "model",
            fields: vec![field(
                "llm_request",
                llm_request_schema(),
                llm_request_example(),
            )],
            structured: json!({"hookSpecificOutput":{"hookEventName":"BeforeModel","llm_request":{"model":"gemini-test","messages":[],"config":{"temperature":0.0}}}}),
            block_effect: Some("deny"),
        },
        Seed {
            wire_name: "AfterModel",
            rust_key: "after_model",
            category: "model",
            fields: vec![
                field("llm_request", llm_request_schema(), llm_request_example()),
                field(
                    "llm_response",
                    json!({"type":"object"}),
                    json!({"candidates":[]}),
                ),
            ],
            structured: json!({"hookSpecificOutput":{"hookEventName":"AfterModel","llm_response":{"candidates":[]}}}),
            block_effect: Some("deny"),
        },
        Seed {
            wire_name: "SessionStart",
            rust_key: "session_start",
            category: "session",
            fields: vec![field(
                "source",
                json!({"enum":["startup","resume","clear"]}),
                json!("startup"),
            )],
            structured: json!({"systemMessage":"Loading session context.","hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"Read repository conventions."}}),
            block_effect: None,
        },
        Seed {
            wire_name: "SessionEnd",
            rust_key: "session_end",
            category: "session",
            fields: vec![field(
                "reason",
                json!({"enum":["exit","clear","logout","prompt_input_exit","other"]}),
                json!("exit"),
            )],
            structured: json!({"systemMessage":"Session cleanup complete."}),
            block_effect: None,
        },
        Seed {
            wire_name: "Notification",
            rust_key: "notification",
            category: "notification",
            fields: vec![
                field(
                    "notification_type",
                    json!({"type":"string"}),
                    json!("ToolPermission"),
                ),
                field(
                    "message",
                    json!({"type":"string"}),
                    json!("Permission required"),
                ),
                field(
                    "details",
                    json!({"type":"object"}),
                    json!({"tool":"run_shell_command"}),
                ),
            ],
            structured: json!({"systemMessage":"A permission notification was emitted."}),
            block_effect: None,
        },
        Seed {
            wire_name: "PreCompress",
            rust_key: "pre_compress",
            category: "compaction",
            fields: vec![field(
                "trigger",
                json!({"enum":["auto","manual"]}),
                json!("auto"),
            )],
            structured: json!({"systemMessage":"Saving state before compression."}),
            block_effect: None,
        },
    ]
}

fn llm_request_schema() -> Value {
    json!({
        "type":"object",
        "required":["model","messages","config"],
        "properties":{
            "model":{"type":"string"},
            "messages":{"type":"array","items":{"type":"object","required":["role","content"],"properties":{"role":{"enum":["user","model","system"]},"content":{"type":"string"}},"additionalProperties":false}},
            "config":{"type":"object"},
            "toolConfig":{"type":"object"}
        },
        "additionalProperties":true
    })
}

fn llm_request_example() -> Value {
    json!({"model":"gemini-test","messages":[{"role":"user","content":"hello"}],"config":{"temperature":0.0}})
}

fn generate(root: &Path, seed: &Seed) {
    let key = kebab(seed.wire_name);
    let directory = root
        .join("contracts/harnesses/gemini-cli/snapshots")
        .join(SNAPSHOT)
        .join("events")
        .join(&key);
    fs::create_dir_all(&directory).expect("create event directory");
    write_json(
        &directory.join("input.schema.json"),
        &input_schema(seed, &key),
    );
    write_json(
        &directory.join("output.command.schema.json"),
        &output_schema(seed, &key),
    );
    write_yaml(&directory.join("contract.yaml"), &contract(seed));
    write_yaml(&directory.join("fixtures.yaml"), &fixtures(seed));
}

fn input_schema(seed: &Seed, key: &str) -> Value {
    let mut properties = Map::from_iter([
        ("session_id".to_string(), json!({"type":"string"})),
        ("transcript_path".to_string(), json!({"type":"string"})),
        ("cwd".to_string(), json!({"type":"string"})),
        (
            "hook_event_name".to_string(),
            json!({"const":seed.wire_name}),
        ),
        ("timestamp".to_string(), json!({"type":"string"})),
    ]);
    let mut required = vec![
        "session_id",
        "transcript_path",
        "cwd",
        "hook_event_name",
        "timestamp",
    ];
    for field in &seed.fields {
        properties.insert(field.name.to_string(), field.schema.clone());
        if field.required {
            required.push(field.name);
        }
    }
    json!({
        "$schema":"https://json-schema.org/draft/2020-12/schema",
        "$id":format!("urn:agent-hook-kit:contracts:gemini-cli:{SNAPSHOT}:{key}:input"),
        "type":"object",
        "required":required,
        "properties":properties,
        "additionalProperties":true
    })
}

fn output_schema(seed: &Seed, key: &str) -> Value {
    let mut properties = Map::new();
    match seed.wire_name {
        "SessionEnd" | "Notification" | "PreCompress" => {
            properties.insert("systemMessage".to_string(), json!({"type":"string"}));
        }
        "SessionStart" => {
            properties.insert("systemMessage".to_string(), json!({"type":"string"}));
            properties.insert(
                "hookSpecificOutput".to_string(),
                hook_specific(seed, json!({"additionalContext":{"type":"string"}}), vec![]),
            );
        }
        "BeforeTool" => {
            add_common_control(&mut properties);
            properties.insert(
                "hookSpecificOutput".to_string(),
                hook_specific(seed, json!({"tool_input":{"type":"object"}}), vec![]),
            );
        }
        "AfterTool" => {
            add_common_control(&mut properties);
            properties.insert("hookSpecificOutput".to_string(), hook_specific(seed, json!({
                "additionalContext":{"type":"string"},
                "tailToolCallRequest":{"type":"object","required":["name","args"],"properties":{"name":{"type":"string"},"args":{"type":"object"}},"additionalProperties":false}
            }), vec![]));
        }
        "BeforeAgent" => {
            add_common_control(&mut properties);
            properties.insert(
                "hookSpecificOutput".to_string(),
                hook_specific(seed, json!({"additionalContext":{"type":"string"}}), vec![]),
            );
        }
        "AfterAgent" => {
            add_common_control(&mut properties);
            properties.insert(
                "hookSpecificOutput".to_string(),
                hook_specific(seed, json!({"clearContext":{"type":"boolean"}}), vec![]),
            );
        }
        "BeforeModel" => {
            add_common_control(&mut properties);
            properties.insert(
                "hookSpecificOutput".to_string(),
                hook_specific(
                    seed,
                    json!({"llm_request":{"type":"object"},"llm_response":{"type":"object"}}),
                    vec![],
                ),
            );
        }
        "AfterModel" => {
            add_common_control(&mut properties);
            properties.insert(
                "hookSpecificOutput".to_string(),
                hook_specific(
                    seed,
                    json!({"llm_response":{"type":"object"}}),
                    vec!["llm_response"],
                ),
            );
        }
        _ => unreachable!(),
    }
    json!({
        "$schema":"https://json-schema.org/draft/2020-12/schema",
        "$id":format!("urn:agent-hook-kit:contracts:gemini-cli:{SNAPSHOT}:{key}:command-output"),
        "type":"object",
        "properties":properties,
        "additionalProperties":false
    })
}

fn add_common_control(properties: &mut Map<String, Value>) {
    properties.insert("systemMessage".to_string(), json!({"type":"string"}));
    properties.insert("suppressOutput".to_string(), json!({"type":"boolean"}));
    properties.insert("continue".to_string(), json!({"type":"boolean"}));
    properties.insert("stopReason".to_string(), json!({"type":"string"}));
    properties.insert(
        "decision".to_string(),
        json!({"enum":["allow","deny","block"]}),
    );
    properties.insert("reason".to_string(), json!({"type":"string"}));
}

fn hook_specific(seed: &Seed, fields: Value, mut required: Vec<&str>) -> Value {
    let mut properties = fields.as_object().expect("field map").clone();
    properties.insert("hookEventName".to_string(), json!({"const":seed.wire_name}));
    required.insert(0, "hookEventName");
    json!({"type":"object","required":required,"properties":properties,"additionalProperties":false})
}

fn contract(seed: &Seed) -> Value {
    let mut outcomes = vec![json!({
        "id":"structured",
        "effect":"event-specific-control",
        "exit":{"exact":0},
        "stdout":{"presence":"required","role":"protocol-value","content_kind":"json"},
        "stderr":{"presence":"optional","role":"diagnostics","content_kind":"text","encoding":"utf-8"},
        "output_schema":"command-response",
        "sources":[SOURCE,SOURCE_CODE],
        "assurance":{"confidence":"high","verification":"source-reviewed"}
    })];
    if let Some(effect) = seed.block_effect {
        outcomes.push(json!({
            "id":"exit-2",
            "effect":effect,
            "exit":{"exact":2},
            "stdout":{"presence":"forbidden","role":"none","content_kind":"empty"},
            "stderr":{"presence":"required","role":"agent-context","content_kind":"text","encoding":"utf-8"},
            "sources":[SOURCE],
            "assurance":{"confidence":"high","verification":"source-reviewed"}
        }));
    }
    json!({
        "format_version":1,
        "id":format!("gemini-cli/{SNAPSHOT}/{}",seed.wire_name),
        "harness":"gemini-cli",
        "snapshot":SNAPSHOT,
        "event":{"wire_name":seed.wire_name,"rust_key":seed.rust_key,"category":seed.category,"identification":{"inferability":"definitive","discriminator":{"json_pointer":"/hook_event_name","const":seed.wire_name}}},
        "schemas":{
            "input":{"file":"input.schema.json","origin":"derived","sources":[SOURCE,SOURCE_CODE],"assurance":{"confidence":"high","verification":"source-reviewed"}},
            "outputs":[{"id":"command-response","file":"output.command.schema.json","origin":"derived","sources":[SOURCE,SOURCE_CODE],"assurance":{"confidence":"high","verification":"source-reviewed"}}]
        },
        "bindings":{"command":{"kind":"process","request":{"channel":"stdin","framing":"single-document-at-eof","content_kind":"json"},"outcomes":outcomes}},
        "fixtures":"fixtures.yaml"
    })
}

fn fixtures(seed: &Seed) -> Value {
    let mut minimal = Map::from_iter([
        ("session_id".to_string(), json!("session-1")),
        ("transcript_path".to_string(), json!("/tmp/transcript.json")),
        ("cwd".to_string(), json!("/repo")),
        ("hook_event_name".to_string(), json!(seed.wire_name)),
        ("timestamp".to_string(), json!("2026-07-12T00:00:00Z")),
    ]);
    for field in &seed.fields {
        if field.required {
            minimal.insert(field.name.to_string(), field.example.clone());
        }
    }
    let mut representative = minimal.clone();
    for field in &seed.fields {
        if !field.required {
            representative.insert(field.name.to_string(), field.example.clone());
        }
    }
    let mut wrong = minimal.clone();
    wrong.insert("hook_event_name".to_string(), json!("WrongEvent"));
    let missing_field = seed
        .fields
        .iter()
        .find(|field| field.required)
        .expect("event field")
        .name;
    let mut missing = minimal.clone();
    missing.remove(missing_field);

    let output_bytes = serde_json::to_vec(&seed.structured).expect("output JSON");
    let mut process = vec![json!({
        "id":"structured","binding":"command","outcome":"structured","exit_code":0,
        "stdout_base64":base64::engine::general_purpose::STANDARD.encode(&output_bytes),"stdout_sha256":sha256(&output_bytes),
        "stderr_base64":"","stderr_sha256":EMPTY_SHA256
    })];
    if seed.block_effect.is_some() {
        let reason = b"blocked by hook";
        process.push(json!({
            "id":"exit-2","binding":"command","outcome":"exit-2","exit_code":2,
            "stdout_base64":"","stdout_sha256":EMPTY_SHA256,
            "stderr_base64":base64::engine::general_purpose::STANDARD.encode(reason),"stderr_sha256":sha256(reason)
        }));
    }
    json!({
        "format_version":1,
        "input":{
            "positive":[
                {"id":"minimal","origin":"synthesized","sources":[SOURCE],"value":minimal},
                {"id":"representative","origin":"synthesized","sources":[SOURCE,SOURCE_CODE],"value":representative}
            ],
            "negative":[
                {"id":"wrong-discriminator","origin":"regression","sources":[SOURCE],"value":wrong,"expected_pointer":"/hook_event_name"},
                {"id":format!("missing-{missing_field}"),"origin":"synthesized","sources":[SOURCE],"value":missing,"expected_pointer":""}
            ]
        },
        "output":[
            {"id":"no-op","schema":"command-response","origin":"synthesized","sources":[SOURCE],"value":{}},
            {"id":"structured","schema":"command-response","origin":"synthesized","sources":[SOURCE,SOURCE_CODE],"value":seed.structured}
        ],
        "process":process
    })
}

fn kebab(value: &str) -> String {
    let mut output = String::new();
    for (index, character) in value.chars().enumerate() {
        if character.is_uppercase() && index > 0 {
            output.push('-');
        }
        output.extend(character.to_lowercase());
    }
    output
}

fn write_json(path: &Path, value: &Value) {
    let mut bytes = serde_json::to_vec_pretty(value).expect("serialize JSON");
    bytes.push(b'\n');
    fs::write(path, bytes).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
}

fn write_yaml(path: &Path, value: &Value) {
    fs::write(
        path,
        serde_yaml_ng::to_string(value).expect("serialize YAML"),
    )
    .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
