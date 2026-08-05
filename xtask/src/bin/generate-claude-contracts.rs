use base64::Engine as _;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

const SNAPSHOT: &str = "docs-2026-08-05-r1";
const SOURCE: &str = "claude-hooks-reference";
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

#[derive(Clone, Copy)]
enum Profile {
    Context,
    SessionStart,
    Prompt,
    Display,
    PreTool,
    PermissionRequest,
    PostTool,
    Retry,
    Stop,
    WatchPaths,
    Elicitation,
    NoControl,
}

struct Seed {
    wire: &'static str,
    key: &'static str,
    category: &'static str,
    fields: Vec<Field>,
    profile: Profile,
    structured: Value,
    exit2_effect: Option<&'static str>,
    handler_group: HandlerGroup,
    text_context: bool,
}

struct Field {
    name: &'static str,
    schema: Value,
    example: Value,
    required: bool,
}

#[derive(Clone, Copy)]
enum HandlerGroup {
    AllFive,
    Three,
    Two,
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

fn required(name: &'static str, schema: Value, example: Value) -> Field {
    Field {
        name,
        schema,
        example,
        required: true,
    }
}

fn optional(name: &'static str, schema: Value, example: Value) -> Field {
    Field {
        name,
        schema,
        example,
        required: false,
    }
}

fn string(name: &'static str, example: &'static str) -> Field {
    required(name, json!({"type":"string"}), json!(example))
}

fn optional_string(name: &'static str, example: &'static str) -> Field {
    optional(name, json!({"type":"string"}), json!(example))
}

fn seeds() -> Vec<Seed> {
    use HandlerGroup::{AllFive, Three, Two};
    use Profile::{
        Context, Display, Elicitation, NoControl, PermissionRequest as Permission, PostTool,
        PreTool, Prompt, Retry, SessionStart as Session, Stop, WatchPaths,
    };
    vec![
        Seed {
            wire: "SessionStart",
            key: "session_start",
            category: "session",
            fields: vec![
                required(
                    "source",
                    json!({"enum":["startup","resume","clear","compact","fork"]}),
                    json!("startup"),
                ),
                optional_string("model", "claude-test"),
                optional_string("agent_type", "reviewer"),
                optional_string("session_title", "Review"),
            ],
            profile: Session,
            structured: json!({"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"Read conventions.","sessionTitle":"Review","watchPaths":["/repo/.env"],"reloadSkills":true}}),
            exit2_effect: None,
            handler_group: Two,
            text_context: true,
        },
        Seed {
            wire: "Setup",
            key: "setup",
            category: "session",
            fields: vec![required(
                "trigger",
                json!({"enum":["init","maintenance"]}),
                json!("init"),
            )],
            profile: Context,
            structured: context("Setup"),
            exit2_effect: None,
            handler_group: Two,
            text_context: false,
        },
        Seed {
            wire: "InstructionsLoaded",
            key: "instructions_loaded",
            category: "context",
            fields: vec![
                string("file_path", "/repo/CLAUDE.md"),
                required(
                    "memory_type",
                    json!({"enum":["User","Project","Local","Managed"]}),
                    json!("Project"),
                ),
                required(
                    "load_reason",
                    json!({"enum":["session_start","nested_traversal","path_glob_match","include","compact"]}),
                    json!("session_start"),
                ),
                optional(
                    "globs",
                    json!({"type":"array","items":{"type":"string"}}),
                    json!(["src/**"]),
                ),
                optional_string("trigger_file_path", "/repo/src/lib.rs"),
                optional_string("parent_file_path", "/repo/CLAUDE.md"),
            ],
            profile: NoControl,
            structured: json!({}),
            exit2_effect: None,
            handler_group: Three,
            text_context: false,
        },
        Seed {
            wire: "UserPromptSubmit",
            key: "user_prompt_submit",
            category: "prompt",
            fields: vec![string("prompt", "Run the tests")],
            profile: Prompt,
            structured: json!({"decision":"block","reason":"Confirmation required.","hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"Clarify scope.","sessionTitle":"Clarify","suppressOriginalPrompt":true}}),
            exit2_effect: Some("deny"),
            handler_group: AllFive,
            text_context: true,
        },
        Seed {
            wire: "UserPromptExpansion",
            key: "user_prompt_expansion",
            category: "prompt",
            fields: vec![
                required(
                    "expansion_type",
                    json!({"enum":["slash_command","mcp_prompt"]}),
                    json!("slash_command"),
                ),
                string("command_name", "review"),
                string("command_args", "--strict"),
                string("command_source", "plugin"),
                string("prompt", "/review --strict"),
            ],
            profile: Prompt,
            structured: json!({"decision":"block","reason":"Unavailable.","hookSpecificOutput":{"hookEventName":"UserPromptExpansion","additionalContext":"Use the team checklist."}}),
            exit2_effect: Some("deny"),
            handler_group: AllFive,
            text_context: true,
        },
        Seed {
            wire: "MessageDisplay",
            key: "message_display",
            category: "display",
            fields: vec![
                string("turn_id", "turn-1"),
                string("message_id", "message-1"),
                required("index", json!({"type":"integer","minimum":0}), json!(0)),
                required("final", json!({"type":"boolean"}), json!(false)),
                string("delta", "Here is the plan:\n"),
            ],
            profile: Display,
            structured: json!({"hookSpecificOutput":{"hookEventName":"MessageDisplay","displayContent":"Here is the plan:"}}),
            exit2_effect: None,
            handler_group: Three,
            text_context: false,
        },
        Seed {
            wire: "PreToolUse",
            key: "pre_tool_use",
            category: "tool",
            fields: tool_fields(false),
            profile: PreTool,
            structured: json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"ask","permissionDecisionReason":"Review command.","updatedInput":{"command":"cargo test"},"additionalContext":"Production environment."}}),
            exit2_effect: Some("deny"),
            handler_group: AllFive,
            text_context: false,
        },
        Seed {
            wire: "PermissionRequest",
            key: "permission_request",
            category: "tool",
            fields: vec![
                string("tool_name", "Bash"),
                required("tool_input", json!({}), json!({"command":"cargo test"})),
                optional("permission_suggestions", json!({"type":"array"}), json!([])),
            ],
            profile: Permission,
            structured: json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Blocked by policy.","interrupt":false}}}),
            exit2_effect: Some("deny"),
            handler_group: AllFive,
            text_context: false,
        },
        Seed {
            wire: "PostToolUse",
            key: "post_tool_use",
            category: "tool",
            fields: tool_fields(true),
            profile: PostTool,
            structured: json!({"decision":"block","reason":"Review result.","hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"Generated files changed.","updatedToolOutput":{"status":"redacted"}}}),
            exit2_effect: Some("provide-context"),
            handler_group: AllFive,
            text_context: false,
        },
        Seed {
            wire: "PostToolUseFailure",
            key: "post_tool_use_failure",
            category: "tool",
            fields: vec![
                string("tool_name", "Bash"),
                required("tool_input", json!({}), json!({"command":"cargo test"})),
                string("tool_use_id", "toolu_1"),
                string("error", "exit status 1"),
                optional("is_interrupt", json!({"type":"boolean"}), json!(false)),
                optional(
                    "duration_ms",
                    json!({"type":"number","minimum":0}),
                    json!(12),
                ),
            ],
            profile: Context,
            structured: context("PostToolUseFailure"),
            exit2_effect: Some("provide-context"),
            handler_group: AllFive,
            text_context: false,
        },
        Seed {
            wire: "PostToolBatch",
            key: "post_tool_batch",
            category: "tool",
            fields: vec![required(
                "tool_calls",
                json!({"type":"array","items":{"type":"object","required":["tool_name","tool_input","tool_use_id","tool_response"],"additionalProperties":true}}),
                json!([{"tool_name":"Read","tool_input":{"file_path":"/repo/a"},"tool_use_id":"toolu_1","tool_response":"content"}]),
            )],
            profile: Context,
            structured: context("PostToolBatch"),
            exit2_effect: Some("stop"),
            handler_group: AllFive,
            text_context: false,
        },
        Seed {
            wire: "PermissionDenied",
            key: "permission_denied",
            category: "tool",
            fields: vec![
                string("tool_name", "Bash"),
                required(
                    "tool_input",
                    json!({}),
                    json!({"command":"rm -rf /tmp/build"}),
                ),
                string("tool_use_id", "toolu_1"),
                string("reason", "Auto mode denied"),
            ],
            profile: Retry,
            structured: json!({"hookSpecificOutput":{"hookEventName":"PermissionDenied","retry":true}}),
            exit2_effect: None,
            handler_group: AllFive,
            text_context: false,
        },
        Seed {
            wire: "Notification",
            key: "notification",
            category: "notification",
            fields: vec![
                string("message", "Permission required"),
                optional_string("title", "Claude Code"),
                required(
                    "notification_type",
                    json!({"enum":["permission_prompt","idle_prompt","auth_success","elicitation_dialog","elicitation_complete","elicitation_response","agent_needs_input","agent_completed"]}),
                    json!("permission_prompt"),
                ),
            ],
            profile: NoControl,
            structured: json!({"continue":true,"suppressOutput":true,"systemMessage":"Permission notification emitted.","terminalSequence":"\u{7}"}),
            exit2_effect: None,
            handler_group: Three,
            text_context: false,
        },
        Seed {
            wire: "SubagentStart",
            key: "subagent_start",
            category: "subagent",
            fields: vec![
                string("agent_id", "agent-1"),
                string("agent_type", "Explore"),
            ],
            profile: Context,
            structured: context("SubagentStart"),
            exit2_effect: None,
            handler_group: Three,
            text_context: false,
        },
        Seed {
            wire: "SubagentStop",
            key: "subagent_stop",
            category: "subagent",
            fields: vec![
                required("stop_hook_active", json!({"type":"boolean"}), json!(false)),
                string("agent_id", "agent-1"),
                string("agent_type", "Explore"),
                string("agent_transcript_path", "/tmp/agent.jsonl"),
                string("last_assistant_message", "Done"),
                background_tasks(),
                session_crons(),
            ],
            profile: Stop,
            structured: json!({"decision":"block","reason":"Run another pass.","hookSpecificOutput":{"hookEventName":"SubagentStop","additionalContext":"Check edge cases."}}),
            exit2_effect: Some("continue"),
            handler_group: AllFive,
            text_context: false,
        },
        Seed {
            wire: "TaskCreated",
            key: "task_created",
            category: "task",
            fields: task_fields(),
            profile: NoControl,
            structured: json!({"continue":false,"stopReason":"Task needs an owner."}),
            exit2_effect: Some("rollback"),
            handler_group: AllFive,
            text_context: false,
        },
        Seed {
            wire: "TaskCompleted",
            key: "task_completed",
            category: "task",
            fields: task_fields(),
            profile: NoControl,
            structured: json!({"continue":false,"stopReason":"Verification is incomplete."}),
            exit2_effect: Some("deny"),
            handler_group: AllFive,
            text_context: false,
        },
        Seed {
            wire: "Stop",
            key: "stop",
            category: "turn",
            fields: vec![
                required("stop_hook_active", json!({"type":"boolean"}), json!(false)),
                string("last_assistant_message", "All work complete"),
                background_tasks(),
                session_crons(),
            ],
            profile: Stop,
            structured: json!({"decision":"block","reason":"Run tests again.","hookSpecificOutput":{"hookEventName":"Stop","additionalContext":"Focus on failures."}}),
            exit2_effect: Some("continue"),
            handler_group: AllFive,
            text_context: false,
        },
        Seed {
            wire: "StopFailure",
            key: "stop_failure",
            category: "turn",
            fields: vec![
                required(
                    "error",
                    json!({"enum":["rate_limit","overloaded","authentication_failed","oauth_org_not_allowed","billing_error","invalid_request","model_not_found","server_error","max_output_tokens","unknown"]}),
                    json!("rate_limit"),
                ),
                optional("error_details", json!({}), json!({"type":"rate_limit"})),
                optional_string("last_assistant_message", "Partial response"),
            ],
            profile: NoControl,
            structured: json!({}),
            exit2_effect: None,
            handler_group: Three,
            text_context: false,
        },
        Seed {
            wire: "TeammateIdle",
            key: "teammate_idle",
            category: "team",
            fields: vec![
                string("teammate_name", "reviewer"),
                string("team_name", "quality"),
            ],
            profile: NoControl,
            structured: json!({"continue":false,"stopReason":"Continue reviewing."}),
            exit2_effect: Some("continue"),
            handler_group: AllFive,
            text_context: false,
        },
        Seed {
            wire: "ConfigChange",
            key: "config_change",
            category: "configuration",
            fields: vec![
                required(
                    "source",
                    json!({"enum":["user_settings","project_settings","local_settings","policy_settings","skills"]}),
                    json!("project_settings"),
                ),
                optional_string("file_path", "/repo/.claude/settings.json"),
            ],
            profile: NoControl,
            structured: json!({"decision":"block","reason":"Configuration change rejected."}),
            exit2_effect: Some("deny"),
            handler_group: Three,
            text_context: false,
        },
        Seed {
            wire: "CwdChanged",
            key: "cwd_changed",
            category: "workspace",
            fields: vec![string("old_cwd", "/repo"), string("new_cwd", "/repo/crate")],
            profile: WatchPaths,
            structured: json!({"systemMessage":"Working directory changed.","hookSpecificOutput":{"hookEventName":"CwdChanged","watchPaths":["/repo/crate/.env"]}}),
            exit2_effect: None,
            handler_group: Three,
            text_context: false,
        },
        Seed {
            wire: "DirectoryAdded",
            key: "directory_added",
            category: "workspace",
            fields: vec![
                string("directory", "/repo/related"),
                required(
                    "source",
                    json!({"enum":["slash_command","register_repo_root"]}),
                    json!("slash_command"),
                ),
            ],
            profile: NoControl,
            structured: json!({"systemMessage":"Working directory added."}),
            exit2_effect: None,
            handler_group: Three,
            text_context: false,
        },
        Seed {
            wire: "FileChanged",
            key: "file_changed",
            category: "workspace",
            fields: vec![
                string("file_path", "/repo/.env"),
                required(
                    "event",
                    json!({"enum":["change","add","unlink"]}),
                    json!("change"),
                ),
            ],
            profile: WatchPaths,
            structured: json!({"systemMessage":"Watched file changed.","hookSpecificOutput":{"hookEventName":"FileChanged","watchPaths":["/repo/.env","/repo/.env.local"]}}),
            exit2_effect: None,
            handler_group: Three,
            text_context: false,
        },
        Seed {
            wire: "WorktreeRemove",
            key: "worktree_remove",
            category: "workspace",
            fields: vec![string("worktree_path", "/tmp/worktree")],
            profile: NoControl,
            structured: json!({}),
            exit2_effect: None,
            handler_group: Three,
            text_context: false,
        },
        Seed {
            wire: "PreCompact",
            key: "pre_compact",
            category: "compaction",
            fields: vec![
                required(
                    "trigger",
                    json!({"enum":["manual","auto"]}),
                    json!("manual"),
                ),
                string("custom_instructions", "Preserve test results"),
            ],
            profile: NoControl,
            structured: json!({"decision":"block","reason":"Save state first."}),
            exit2_effect: Some("deny"),
            handler_group: Three,
            text_context: false,
        },
        Seed {
            wire: "PostCompact",
            key: "post_compact",
            category: "compaction",
            fields: vec![
                required("trigger", json!({"enum":["manual","auto"]}), json!("auto")),
                string("compact_summary", "Summary"),
            ],
            profile: NoControl,
            structured: json!({"systemMessage":"Compaction complete."}),
            exit2_effect: None,
            handler_group: Three,
            text_context: false,
        },
        Seed {
            wire: "SessionEnd",
            key: "session_end",
            category: "session",
            fields: vec![required(
                "reason",
                json!({"enum":["clear","resume","logout","prompt_input_exit","bypass_permissions_disabled","other"]}),
                json!("other"),
            )],
            profile: NoControl,
            structured: json!({"systemMessage":"Session ended."}),
            exit2_effect: None,
            handler_group: Three,
            text_context: false,
        },
        Seed {
            wire: "Elicitation",
            key: "elicitation",
            category: "mcp",
            fields: vec![
                string("mcp_server_name", "forms"),
                string("message", "Provide values"),
                optional("mode", json!({"enum":["form","url"]}), json!("form")),
                optional_string("url", "https://example.test"),
                optional_string("elicitation_id", "e1"),
                optional(
                    "requested_schema",
                    json!({"type":"object"}),
                    json!({"type":"object"}),
                ),
            ],
            profile: Elicitation,
            structured: json!({"hookSpecificOutput":{"hookEventName":"Elicitation","action":"accept","content":{"name":"Ada"}}}),
            exit2_effect: Some("deny"),
            handler_group: Three,
            text_context: false,
        },
        Seed {
            wire: "ElicitationResult",
            key: "elicitation_result",
            category: "mcp",
            fields: vec![
                string("mcp_server_name", "forms"),
                required(
                    "action",
                    json!({"enum":["accept","decline","cancel"]}),
                    json!("accept"),
                ),
                optional("mode", json!({"enum":["form","url"]}), json!("form")),
                optional_string("elicitation_id", "e1"),
                optional("content", json!({"type":"object"}), json!({"name":"Ada"})),
            ],
            profile: Elicitation,
            structured: json!({"hookSpecificOutput":{"hookEventName":"ElicitationResult","action":"decline"}}),
            exit2_effect: Some("deny"),
            handler_group: Three,
            text_context: false,
        },
    ]
}

fn context(event: &str) -> Value {
    json!({"hookSpecificOutput":{"hookEventName":event,"additionalContext":"Hook-provided context."}})
}

fn tool_fields(response: bool) -> Vec<Field> {
    let mut fields = vec![
        string("tool_name", "Bash"),
        required("tool_input", json!({}), json!({"command":"cargo test"})),
        string("tool_use_id", "toolu_1"),
    ];
    if response {
        fields.push(required("tool_response", json!({}), json!({"stdout":"ok"})));
        fields.push(optional(
            "duration_ms",
            json!({"type":"number","minimum":0}),
            json!(12),
        ));
    }
    fields
}

fn task_fields() -> Vec<Field> {
    vec![
        string("task_id", "task-1"),
        string("task_subject", "Run tests"),
        optional_string("task_description", "Run all checks"),
        optional_string("teammate_name", "reviewer"),
        optional_string("team_name", "quality"),
    ]
}

fn background_tasks() -> Field {
    optional(
        "background_tasks",
        json!({
            "type":"array",
            "items":{
                "type":"object",
                "required":["id","type","status","description"],
                "properties":{
                    "id":{"type":"string"},
                    "type":{"type":"string"},
                    "status":{"type":"string"},
                    "description":{"type":"string"},
                    "command":{"type":"string"},
                    "agent_type":{"type":"string"},
                    "server":{"type":"string"},
                    "tool":{"type":"string"},
                    "name":{"type":"string"}
                },
                "additionalProperties":true
            }
        }),
        json!([{
            "id":"task-1",
            "type":"shell",
            "status":"running",
            "description":"Run tests",
            "command":"cargo test"
        }]),
    )
}

fn session_crons() -> Field {
    optional(
        "session_crons",
        json!({
            "type":"array",
            "items":{
                "type":"object",
                "required":["id","schedule","recurring","prompt"],
                "properties":{
                    "id":{"type":"string"},
                    "schedule":{"type":"string"},
                    "recurring":{"type":"boolean"},
                    "prompt":{"type":"string"}
                },
                "additionalProperties":true
            }
        }),
        json!([{
            "id":"cron-1",
            "schedule":"0 9 * * 1-5",
            "recurring":true,
            "prompt":"Check the build"
        }]),
    )
}

fn generate(root: &Path, seed: &Seed) {
    let directory = root
        .join("contracts/harnesses/claude-code/snapshots")
        .join(SNAPSHOT)
        .join("events")
        .join(kebab(seed.wire));
    fs::create_dir_all(&directory).expect("create Claude event directory");
    write_json(&directory.join("input.schema.json"), &input_schema(seed));
    write_json(
        &directory.join("output.command.schema.json"),
        &output_schema(seed),
    );
    write_yaml(&directory.join("contract.yaml"), &contract(seed));
    write_yaml(&directory.join("fixtures.yaml"), &fixtures(seed));
}

fn input_schema(seed: &Seed) -> Value {
    let mut properties = Map::from_iter([
        ("session_id".into(), json!({"type":"string"})),
        ("prompt_id".into(), json!({"type":"string"})),
        ("transcript_path".into(), json!({"type":"string"})),
        ("cwd".into(), json!({"type":"string"})),
        (
            "permission_mode".into(),
            json!({"enum":["default","plan","acceptEdits","auto","dontAsk","bypassPermissions"]}),
        ),
        (
            "effort".into(),
            json!({"type":"object","required":["level"],"properties":{"level":{"enum":["low","medium","high","xhigh","max"]}},"additionalProperties":false}),
        ),
        ("hook_event_name".into(), json!({"const":seed.wire})),
        ("agent_id".into(), json!({"type":"string"})),
        ("agent_type".into(), json!({"type":"string"})),
    ]);
    let mut required_fields = vec!["session_id", "transcript_path", "cwd", "hook_event_name"];
    for field in &seed.fields {
        properties.insert(field.name.into(), field.schema.clone());
        if field.required {
            required_fields.push(field.name);
        }
    }
    json!({"$schema":"https://json-schema.org/draft/2020-12/schema","$id":format!("urn:agent-hook-kit:contracts:claude-code:{SNAPSHOT}:{}:input",kebab(seed.wire)),"type":"object","required":required_fields,"properties":properties,"additionalProperties":true})
}

fn output_schema(seed: &Seed) -> Value {
    let mut top = Map::new();
    add_universal(&mut top);
    if supports_top_level_block(seed.wire) {
        add_block(&mut top);
    }
    let fields = match seed.profile {
        Profile::Context => json!({"additionalContext":{"type":"string"}}),
        Profile::SessionStart => {
            json!({"additionalContext":{"type":"string"},"initialUserMessage":{"type":"string"},"sessionTitle":{"type":"string"},"watchPaths":{"type":"array","items":{"type":"string"}},"reloadSkills":{"type":"boolean"}})
        }
        Profile::Prompt if seed.wire == "UserPromptSubmit" => {
            json!({"additionalContext":{"type":"string"},"sessionTitle":{"type":"string"},"suppressOriginalPrompt":{"type":"boolean"}})
        }
        Profile::Prompt => json!({"additionalContext":{"type":"string"}}),
        Profile::Display => json!({"displayContent":{"type":"string"}}),
        Profile::PreTool => {
            json!({"permissionDecision":{"enum":["allow","deny","ask","defer"]},"permissionDecisionReason":{"type":"string"},"updatedInput":{},"additionalContext":{"type":"string"}})
        }
        Profile::PermissionRequest => {
            json!({"decision":{"type":"object","required":["behavior"],"properties":{"behavior":{"enum":["allow","deny"]},"updatedInput":{},"updatedPermissions":{"type":"array"},"message":{"type":"string"},"interrupt":{"type":"boolean"}},"additionalProperties":false}})
        }
        Profile::PostTool => {
            json!({"additionalContext":{"type":"string"},"updatedToolOutput":{},"updatedMCPToolOutput":{}})
        }
        Profile::Retry => json!({"retry":{"type":"boolean"}}),
        Profile::Stop => json!({"additionalContext":{"type":"string"}}),
        Profile::WatchPaths => {
            json!({"watchPaths":{"type":"array","items":{"type":"string"}}})
        }
        Profile::Elicitation => {
            json!({"action":{"enum":["accept","decline","cancel"]},"content":{"type":"object"}})
        }
        Profile::NoControl => json!({}),
    };
    if !fields.as_object().expect("fields").is_empty() {
        let mut props = fields.as_object().unwrap().clone();
        props.insert("hookEventName".into(), json!({"const":seed.wire}));
        top.insert("hookSpecificOutput".into(),json!({"type":"object","required":["hookEventName"],"properties":props,"additionalProperties":false}));
    }
    json!({"$schema":"https://json-schema.org/draft/2020-12/schema","$id":format!("urn:agent-hook-kit:contracts:claude-code:{SNAPSHOT}:{}:command-output",kebab(seed.wire)),"type":"object","properties":top,"additionalProperties":false})
}

fn add_universal(top: &mut Map<String, Value>) {
    top.insert("continue".into(), json!({"type":"boolean"}));
    top.insert("stopReason".into(), json!({"type":"string"}));
    top.insert("suppressOutput".into(), json!({"type":"boolean"}));
    top.insert("systemMessage".into(), json!({"type":"string"}));
    top.insert("terminalSequence".into(), json!({"type":"string"}));
}

fn add_block(top: &mut Map<String, Value>) {
    top.insert("decision".into(), json!({"const":"block"}));
    top.insert("reason".into(), json!({"type":"string"}));
}

fn supports_top_level_block(event: &str) -> bool {
    matches!(
        event,
        "UserPromptSubmit"
            | "UserPromptExpansion"
            | "PostToolUse"
            | "PostToolUseFailure"
            | "PostToolBatch"
            | "Stop"
            | "SubagentStop"
            | "ConfigChange"
            | "PreCompact"
    )
}

fn contract(seed: &Seed) -> Value {
    let handlers: Vec<&str> = match seed.handler_group {
        HandlerGroup::AllFive => vec!["command", "http", "mcp_tool", "prompt", "agent"],
        HandlerGroup::Three => vec!["command", "http", "mcp_tool"],
        HandlerGroup::Two => vec!["command", "mcp_tool"],
    };
    let mut command_outcomes = vec![outcome(
        "structured",
        "event-specific-control",
        json!({"exact":0}),
        "json",
        Some("command-response"),
    )];
    if seed.text_context {
        command_outcomes.push(outcome(
            "text-context",
            "provide-context",
            json!({"exact":0}),
            "text",
            None,
        ));
    }
    if let Some(effect) = seed.exit2_effect {
        command_outcomes.push(json!({"id":"exit-2","effect":effect,"exit":{"exact":2},"stdout":{"presence":"optional","role":"ignored","content_kind":"opaque"},"stderr":{"presence":"required","role":"agent-context","content_kind":"text","encoding":"utf-8"},"sources":[SOURCE],"assurance":{"confidence":"low","verification":"source-reviewed"}}));
    }
    let mut bindings = Map::new();
    bindings.insert("command".into(),json!({"kind":"process","request":{"channel":"stdin","framing":"single-document-at-eof","content_kind":"json"},"outcomes":command_outcomes}));
    if handlers.contains(&"http") {
        let mut non_success = outcome(
            "non-success",
            "nonblocking-error",
            json!({"range":{"min":400,"max":599}}),
            "empty",
            None,
        );
        non_success["stdout"]["presence"] = json!("optional");
        bindings.insert("http".into(),json!({"kind":"http","method":"POST","request_content_type":"application/json","response_content_type":"application/json","request":{"channel":"body","framing":"one-http-message","content_kind":"json"},"outcomes":[outcome("structured","event-specific-control",json!({"range":{"min":200,"max":299}}),"json",Some("command-response")),non_success]}));
    }
    json!({"format_version":1,"id":format!("claude-code/{SNAPSHOT}/{}",seed.wire),"harness":"claude-code","snapshot":SNAPSHOT,"event":{"wire_name":seed.wire,"rust_key":seed.key,"category":seed.category,"identification":{"inferability":"definitive","discriminator":{"json_pointer":"/hook_event_name","const":seed.wire}}},"schemas":{"input":{"file":"input.schema.json","origin":"derived","sources":[SOURCE],"assurance":{"confidence":"low","verification":"source-reviewed"}},"outputs":[{"id":"command-response","file":"output.command.schema.json","origin":"derived","sources":[SOURCE],"assurance":{"confidence":"low","verification":"source-reviewed"}}]},"bindings":bindings,"handler_kinds":handlers,"fixtures":"fixtures.yaml"})
}

fn outcome(id: &str, effect: &str, exit: Value, kind: &str, schema: Option<&str>) -> Value {
    let mut value = json!({"id":id,"effect":effect,"exit":exit,"stdout":{"presence":"required","role":"protocol-value","content_kind":kind},"stderr":{"presence":"optional","role":"diagnostics","content_kind":"text","encoding":"utf-8"},"sources":[SOURCE],"assurance":{"confidence":"low","verification":"source-reviewed"}});
    if kind == "text" {
        value["stdout"]["encoding"] = json!("utf-8");
        value["stdout"]["trailing_newline"] = json!("allowed");
    }
    if let Some(schema) = schema {
        value["output_schema"] = json!(schema);
    }
    value
}

fn fixtures(seed: &Seed) -> Value {
    let mut minimal = Map::from_iter([
        ("session_id".into(), json!("s1")),
        ("transcript_path".into(), json!("/tmp/t.jsonl")),
        ("cwd".into(), json!("/repo")),
        ("hook_event_name".into(), json!(seed.wire)),
    ]);
    for field in &seed.fields {
        if field.required {
            minimal.insert(field.name.into(), field.example.clone());
        }
    }
    let mut representative = minimal.clone();
    representative.insert("prompt_id".into(), json!("prompt-1"));
    representative.insert("permission_mode".into(), json!("default"));
    for field in &seed.fields {
        if !field.required {
            representative.insert(field.name.into(), field.example.clone());
        }
    }
    let mut wrong = minimal.clone();
    wrong.insert("hook_event_name".into(), json!("WrongEvent"));
    let missing_field = seed
        .fields
        .iter()
        .find(|field| field.required)
        .expect("field")
        .name;
    let mut missing = minimal.clone();
    missing.remove(missing_field);
    let output_bytes = serde_json::to_vec(&seed.structured).unwrap();
    let mut process = vec![case(
        "command-structured",
        "command",
        "structured",
        0,
        &output_bytes,
        b"",
    )];
    if seed.text_context {
        process.push(case(
            "command-text",
            "command",
            "text-context",
            0,
            b"Hook-provided context.",
            b"",
        ));
    }
    if seed.exit2_effect.is_some() {
        process.push(case(
            "command-exit-2",
            "command",
            "exit-2",
            2,
            b"",
            b"blocked by hook",
        ));
        process.push(case(
            "command-exit-2-invalid-stdout",
            "command",
            "exit-2",
            2,
            b"{invalid-json",
            b"blocked by hook",
        ));
    }
    if !matches!(seed.handler_group, HandlerGroup::Two) {
        process.push(case(
            "http-structured",
            "http",
            "structured",
            200,
            &output_bytes,
            b"",
        ));
        process.push(case("http-error", "http", "non-success", 500, b"", b""));
    }
    json!({"format_version":1,"input":{"positive":[{"id":"minimal","origin":"synthesized","sources":[SOURCE],"value":minimal},{"id":"representative","origin":"synthesized","sources":[SOURCE],"value":representative}],"negative":[{"id":"wrong-discriminator","origin":"regression","sources":[SOURCE],"value":wrong,"expected_pointer":"/hook_event_name","expected_keyword":"const"},{"id":format!("missing-{missing_field}"),"origin":"synthesized","sources":[SOURCE],"value":missing,"expected_pointer":"","expected_keyword":"required"}]},"output":[{"id":"structured","schema":"command-response","origin":"synthesized","sources":[SOURCE],"value":seed.structured}],"process":process})
}

fn case(id: &str, binding: &str, outcome: &str, exit: i32, stdout: &[u8], stderr: &[u8]) -> Value {
    json!({"id":id,"binding":binding,"outcome":outcome,"exit_code":exit,"stdout_base64":base64::engine::general_purpose::STANDARD.encode(stdout),"stdout_sha256":sha(stdout),"stderr_base64":base64::engine::general_purpose::STANDARD.encode(stderr),"stderr_sha256":sha(stderr)})
}
fn sha(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return EMPTY_SHA256.into();
    }
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn kebab(value: &str) -> String {
    let mut out = String::new();
    for (i, c) in value.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('-')
        }
        out.extend(c.to_lowercase())
    }
    out
}
fn write_json(path: &Path, value: &Value) {
    let mut b = serde_json::to_vec_pretty(value).unwrap();
    b.push(b'\n');
    fs::write(path, b).unwrap()
}
fn write_yaml(path: &Path, value: &Value) {
    fs::write(path, serde_yaml_ng::to_string(value).unwrap()).unwrap()
}
