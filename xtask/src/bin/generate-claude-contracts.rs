//! Generates the seeded event directories of the Claude Code snapshot named by
//! [`SNAPSHOT`].
//!
//! `events/worktree-create`, `snapshot.yaml`, and `sources.yaml` are
//! hand-authored beside the generated files. Once that snapshot is frozen,
//! regenerating must reproduce it byte for byte; any seed change needs a new
//! [`SNAPSHOT`] id.

use base64::Engine as _;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

const SNAPSHOT: &str = "docs-2026-09-30-r1";
const SOURCE: &str = "claude-hooks-reference";
const CHANGELOG: &str = "claude-code-changelog";
const SDK: &str = "claude-agent-sdk-types";
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

const ALL_FIVE: &[&str] = &["command", "http", "mcp_tool", "prompt", "agent"];
const NO_AGENT: &[&str] = &["command", "http", "mcp_tool", "prompt"];
const THREE: &[&str] = &["command", "http", "mcp_tool"];
const COMMAND_MCP: &[&str] = &["command", "mcp_tool"];
const COMMAND_ONLY: &[&str] = &["command"];

const HTTP_FAILURE_CONTRACT: &str = "An HTTP hook applies this event's failure contract from the per-event exit-code table: a non-2xx status, or a 2xx body that is neither empty nor a JSON object, has the effect of a failed command hook rather than the generic non-blocking error.";
const EXIT_PRECEDENCE: &str = "Exit status 2 is described only by the exact exit-2 outcomes; the 1-255 nonzero outcomes model the reference's other exit codes and apply to status 2 only where no exit-2 outcome exists.";
const TEXT_JSON_RULE: &str = "Stdout whose trimmed text starts with { and ends with } is parsed as JSON (Claude Code v2.1.248 or later); a parse or schema-validation failure is a non-blocking error and the text is not added as context. Output of two or more lines that each parse as JSON, none setting an output field, remains plain text.";
const MCP_SERVER_NOTE: &str = "mcp_server (Claude Code v2.1.274 or later) is present only for MCP tools; its source vocabulary is open, the schema lists the documented values as examples, and an unrecognized source must be treated as an unrecognized configured source rather than sdk.";
const TEAM_NAME_NOTE: &str = "team_name is deprecated upstream (a session-derived name that will be removed in a future release).";
const EXIT_2_NOTICE_JSON: &str = "Exit status 2 cannot block this event. A schema-valid JSON object printed with exit 2 is read as on exit 0, because Claude Code reads JSON on every exit code; the reference does not say whether the exit-2 notice is still shown then, so the exit-2-structured outcome keeps stderr as a user message.";
const STOP_CONTEXT: &str = "hookSpecificOutput.additionalContext is non-error feedback that keeps Claude working: like decision block it continues the conversation, subject to the stop_hook_active input and the 8-consecutive-continuation cap (CLAUDE_CODE_STOP_HOOK_BLOCK_CAP), but the transcript labels it hook feedback rather than a hook error.";
const BLOCK_REASON: &str = "The reference requires reason when decision is block, so the output schema requires it; it does not require the reason to be non-empty.";

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
    ModelSwitch,
    NoControl,
}

/// How Claude Code treats a JSON object printed on the event's stdout.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Json {
    /// Standard decision model: fields take effect on exit 0 and on other exit codes except 2.
    Control,
    /// Output fields are discarded on every exit code; side-effect fields such as
    /// `terminalSequence` still fire.
    SideEffects,
    /// Every output field is discarded on every exit code.
    Discarded,
    /// Stdout is never read; the exit code alone decides the outcome.
    Absent {
        success: &'static str,
        failure: &'static str,
    },
}

/// Nonzero exit codes that are not modeled by a dedicated exit-2 outcome.
#[derive(Clone, Copy)]
enum Nonzero {
    /// Separate `nonzero-structured` (schema-valid JSON) and `nonzero-unstructured` outcomes.
    Split {
        effect: &'static str,
        stderr_role: &'static str,
    },
    /// One `nonzero` outcome regardless of stdout.
    Merged {
        effect: &'static str,
        stderr_role: &'static str,
    },
}

/// Exit-2 behavior for events whose status 2 differs from other nonzero codes.
struct Exit2 {
    effect: &'static str,
    stderr_role: &'static str,
    json: Exit2Json,
    /// Exit 2 blocks the action, so stderr is the blocking message; otherwise
    /// exit 2 only changes where a failure is reported.
    blocks: bool,
}

/// What Claude Code does with a schema-valid JSON object printed with exit 2.
enum Exit2Json {
    /// The JSON is ignored.
    Ignored,
    /// The JSON is read with `effect`; `fixture` is a dedicated output example.
    Read {
        effect: &'static str,
        fixture: Value,
    },
    /// The JSON is read exactly as on exit 0, so the structured fixture applies.
    AsStructured,
}

struct Field {
    name: &'static str,
    schema: Value,
    example: Value,
    required: bool,
    /// Whether the representative input carries this optional field; conditional
    /// fields are exercised by a dedicated positive input instead.
    representative: bool,
}

struct Example {
    id: &'static str,
    value: Value,
    sources: Vec<&'static str>,
}

struct NegativeOutput {
    id: &'static str,
    value: Value,
    pointer: &'static str,
    keyword: &'static str,
}

struct ProcessCase {
    id: &'static str,
    outcome: &'static str,
    exit: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

struct Seed {
    wire: &'static str,
    key: &'static str,
    category: &'static str,
    fields: Vec<Field>,
    profile: Profile,
    json: Json,
    structured: Value,
    exit2: Option<Exit2>,
    nonzero: Nonzero,
    handlers: &'static [&'static str],
    text_context: bool,
    top_level_block: bool,
    /// `decision: block` must carry a `reason`.
    block_reason_required: bool,
    reason_description: Option<&'static str>,
    /// Effect of unparseable or schema-invalid exit-0 JSON when it differs
    /// from the standard non-blocking error.
    invalid_json_effect: Option<&'static str>,
    permission_mode: Option<&'static str>,
    effort: bool,
    representative: Vec<(&'static str, Value)>,
    positives: Vec<(&'static str, Value)>,
    outputs: Vec<Example>,
    output_negatives: Vec<NegativeOutput>,
    process: Vec<ProcessCase>,
    input_sources: Vec<&'static str>,
    output_sources: Vec<&'static str>,
    uncertainties: Vec<&'static str>,
}

impl Seed {
    fn new(
        wire: &'static str,
        key: &'static str,
        category: &'static str,
        profile: Profile,
    ) -> Self {
        Self {
            wire,
            key,
            category,
            fields: Vec::new(),
            profile,
            json: Json::Control,
            structured: json!({}),
            exit2: None,
            nonzero: Nonzero::Split {
                effect: "nonblocking-error",
                stderr_role: "user-message",
            },
            handlers: THREE,
            text_context: false,
            top_level_block: false,
            block_reason_required: false,
            reason_description: None,
            invalid_json_effect: None,
            permission_mode: None,
            effort: false,
            representative: Vec::new(),
            positives: Vec::new(),
            outputs: Vec::new(),
            output_negatives: Vec::new(),
            process: Vec::new(),
            input_sources: vec![SOURCE],
            output_sources: vec![SOURCE],
            uncertainties: Vec::new(),
        }
    }

    fn fields(mut self, fields: Vec<Field>) -> Self {
        self.fields = fields;
        self
    }

    fn structured(mut self, value: Value) -> Self {
        self.structured = value;
        self
    }

    fn json(mut self, json: Json) -> Self {
        self.json = json;
        self
    }

    /// Exit 2 blocks with `effect` and ignores any JSON on stdout.
    fn exit2(mut self, effect: &'static str, stderr_role: &'static str) -> Self {
        self.exit2 = Some(Exit2 {
            effect,
            stderr_role,
            json: Exit2Json::Ignored,
            blocks: true,
        });
        self
    }

    /// Exit 2 blocks with `effect`, and a schema-valid JSON object printed
    /// with it is read with `json_effect`.
    fn exit2_json(
        mut self,
        effect: &'static str,
        stderr_role: &'static str,
        json_effect: &'static str,
        fixture: Value,
    ) -> Self {
        self.exit2 = Some(Exit2 {
            effect,
            stderr_role,
            json: Exit2Json::Read {
                effect: json_effect,
                fixture,
            },
            blocks: true,
        });
        self
    }

    /// Exit 2 cannot block but shows stderr to the user as a hook error
    /// notice, and a schema-valid JSON object printed with it is read as on
    /// exit 0.
    fn exit2_notice(mut self) -> Self {
        self.exit2 = Some(Exit2 {
            effect: "nonblocking-error",
            stderr_role: "user-message",
            json: Exit2Json::AsStructured,
            blocks: false,
        });
        self
    }

    /// Exit 2 is a failure with `effect` whatever stdout holds.
    fn exit2_failure(mut self, effect: &'static str, stderr_role: &'static str) -> Self {
        self.exit2 = Some(Exit2 {
            effect,
            stderr_role,
            json: Exit2Json::Ignored,
            blocks: false,
        });
        self
    }

    fn nonzero(mut self, nonzero: Nonzero) -> Self {
        self.nonzero = nonzero;
        self
    }

    fn handlers(mut self, handlers: &'static [&'static str]) -> Self {
        self.handlers = handlers;
        self
    }

    fn text_context(mut self) -> Self {
        self.text_context = true;
        self
    }

    fn top_level_block(mut self) -> Self {
        self.top_level_block = true;
        self
    }

    fn block_reason_required(mut self) -> Self {
        self.block_reason_required = true;
        self
    }

    fn reason_description(mut self, description: &'static str) -> Self {
        self.reason_description = Some(description);
        self
    }

    fn invalid_json_effect(mut self, effect: &'static str) -> Self {
        self.invalid_json_effect = Some(effect);
        self
    }

    fn permission_mode(mut self, mode: &'static str) -> Self {
        self.permission_mode = Some(mode);
        self
    }

    fn effort(mut self) -> Self {
        self.effort = true;
        self
    }

    /// Overrides one value of the representative positive input.
    fn representative(mut self, name: &'static str, value: Value) -> Self {
        self.representative.push((name, value));
        self
    }

    /// Adds a positive input that extends the minimal input with `overrides`.
    fn positive(mut self, id: &'static str, overrides: Value) -> Self {
        self.positives.push((id, overrides));
        self
    }

    fn output(mut self, id: &'static str, value: Value, sources: &[&'static str]) -> Self {
        self.outputs.push(Example {
            id,
            value,
            sources: sources.to_vec(),
        });
        self
    }

    fn output_negative(
        mut self,
        id: &'static str,
        value: Value,
        pointer: &'static str,
        keyword: &'static str,
    ) -> Self {
        self.output_negatives.push(NegativeOutput {
            id,
            value,
            pointer,
            keyword,
        });
        self
    }

    fn process(
        mut self,
        id: &'static str,
        outcome: &'static str,
        exit: i32,
        stdout: &[u8],
        stderr: &[u8],
    ) -> Self {
        self.process.push(ProcessCase {
            id,
            outcome,
            exit,
            stdout: stdout.to_vec(),
            stderr: stderr.to_vec(),
        });
        self
    }

    fn input_source(mut self, source: &'static str) -> Self {
        self.input_sources.push(source);
        self
    }

    fn output_source(mut self, source: &'static str) -> Self {
        self.output_sources.push(source);
        self
    }

    fn uncertainty(mut self, text: &'static str) -> Self {
        self.uncertainties.push(text);
        self
    }

    fn structured_effect(&self) -> &'static str {
        match self.json {
            Json::Control => "event-specific-control",
            Json::SideEffects => "terminal-sequence-only",
            Json::Discarded => "ignored",
            Json::Absent { success, .. } => success,
        }
    }

    /// Effect of a failed hook, which the reference also applies to a failed
    /// HTTP hook: a non-2xx status or a 2xx body that is not a JSON object.
    fn failure_effect(&self) -> &'static str {
        match (self.json, self.nonzero) {
            (Json::Absent { failure, .. }, _) => failure,
            (_, Nonzero::Split { effect, .. } | Nonzero::Merged { effect, .. }) => effect,
        }
    }
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
        representative: true,
    }
}

fn optional(name: &'static str, schema: Value, example: Value) -> Field {
    Field {
        name,
        schema,
        example,
        required: false,
        representative: true,
    }
}

/// An optional field that upstream sends only under a condition the
/// representative input does not satisfy.
fn conditional(field: Field) -> Field {
    Field {
        representative: false,
        ..field
    }
}

fn string(name: &'static str, example: &'static str) -> Field {
    required(name, json!({"type":"string"}), json!(example))
}

fn optional_string(name: &'static str, example: &'static str) -> Field {
    optional(name, json!({"type":"string"}), json!(example))
}

fn seeds() -> Vec<Seed> {
    use Profile::{
        Context, Display, Elicitation, ModelSwitch, NoControl, PermissionRequest as Permission,
        PostTool, PreTool, Prompt, Retry, SessionStart as Session, Stop, WatchPaths,
    };
    let ignored = Nonzero::Split {
        effect: "ignored",
        stderr_role: "ignored",
    };
    vec![
        Seed::new("SessionStart", "session_start", "session", Session)
            .fields(vec![
                required(
                    "source",
                    json!({"enum":["startup","resume","clear","compact","fork"]}),
                    json!("startup"),
                ),
                optional_string("model", "claude-opus-5"),
                optional_string("agent_type", "reviewer"),
                optional_string("session_title", "Review"),
                optional(
                    "seconds_since_last_response",
                    json!({"type":"number","minimum":0,"description":"Sent only when source is resume or fork and the transcript holds a response (Claude Code v2.1.251 or later)."}),
                    json!(5400),
                ),
                optional(
                    "context_tokens",
                    json!({"type":"integer","minimum":0,"description":"Sent only when source is resume or fork and the transcript holds a response (Claude Code v2.1.251 or later)."}),
                    json!(182340),
                ),
                optional(
                    "prompt_cache_likely_expired",
                    json!({"type":"boolean","description":"Sent only when source is resume or fork and the transcript holds a response (Claude Code v2.1.251 or later)."}),
                    json!(true),
                ),
                optional(
                    "estimated_cache_write_usd",
                    json!({"type":"number","minimum":0,"description":"Sent only when source is resume or fork and the transcript holds a response (Claude Code v2.1.251 or later)."}),
                    json!(1.1396),
                ),
            ])
            .structured(json!({"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"Read conventions.","sessionTitle":"Review","watchPaths":["/repo/.env"],"reloadSkills":true}}))
            .exit2_notice()
            .handlers(COMMAND_MCP)
            .text_context()
            .representative("source", json!("resume"))
            .input_source(SDK)
            .uncertainty("seconds_since_last_response, context_tokens, prompt_cache_likely_expired, and estimated_cache_write_usd are sent only when source is resume or fork and the transcript holds at least one response; the schema keeps them optional and does not encode that condition.")
            .uncertainty("sessionTitle applies when source is startup, resume, or fork and is ignored on clear and compact.")
            .uncertainty("mcp_tool handlers are skipped for SessionStart at launch, including --continue and --resume, and run only for later SessionStart events after /clear or compaction.")
            .uncertainty("Exit status 2 renders stderr as a hook error notice for the user, the same way as any other non-blocking error; Claude does not see it.")
            .uncertainty(EXIT_2_NOTICE_JSON)
            .uncertainty(TEXT_JSON_RULE),
        Seed::new("Setup", "setup", "session", NoControl)
            .fields(vec![required(
                "trigger",
                json!({"enum":["init","maintenance"]}),
                json!("init"),
            )])
            .json(Json::Discarded)
            .nonzero(Nonzero::Merged {
                effect: "ignored",
                stderr_role: "diagnostics",
            })
            .handlers(COMMAND_ONLY)
            .output(
                "discarded-universal-fields",
                json!({"systemMessage":"Dependencies installed.","continue":true}),
                &[SOURCE],
            )
            .output_negative(
                "hook-specific-output-rejected",
                json!({"hookSpecificOutput":{"hookEventName":"Setup","additionalContext":"Dependencies installed."}}),
                "",
                "additionalProperties",
            )
            .output_source(SDK)
            .uncertainty("Claude Code discards every Setup JSON output field, including hookSpecificOutput.additionalContext, on every exit code; the Agent SDK output union still types Setup additionalContext, so the schema omits hookSpecificOutput as documented.")
            .uncertainty("Only command handlers run on Setup. The handler-support list still names mcp_tool, but a configured mcp_tool Setup hook is always skipped, so it is omitted from handler_kinds.")
            .uncertainty("terminalSequence is emitted only by an interactive session with its interface on screen; Setup fires only for --init-only or non-interactive -p runs, so every field is modeled as ignored.")
            .uncertainty("Exit code and stderr are ignored; with -p and --output-format stream-json --verbose they appear only as hook_response events."),
        Seed::new("InstructionsLoaded", "instructions_loaded", "context", NoControl)
            .fields(vec![
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
            ])
            .json(Json::SideEffects)
            .nonzero(Nonzero::Split {
                effect: "ignored",
                stderr_role: "diagnostics",
            })
            .output(
                "discarded-universal-fields",
                json!({"systemMessage":"Instructions audited.","continue":false,"stopReason":"Ignored."}),
                &[SOURCE],
            )
            .uncertainty("Claude Code discards InstructionsLoaded JSON output fields such as systemMessage and continue; terminalSequence is documented to keep working on events that discard those fields and is the only field modeled with an effect.")
            .uncertainty("The event does not fire when Claude reads AGENTS.md directly through the Project instructions setting."),
        Seed::new("UserPromptSubmit", "user_prompt_submit", "prompt", Prompt)
            .fields(vec![string("prompt", "Run the tests")])
            .structured(json!({"decision":"block","reason":"Confirmation required.","hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"Clarify scope.","sessionTitle":"Clarify","suppressOriginalPrompt":true}}))
            .exit2_json(
                "deny",
                "user-message",
                "deny",
                json!({"decision":"block","reason":"Blocked by JSON reason."}),
            )
            .handlers(ALL_FIVE)
            .text_context()
            .top_level_block()
            .reason_description("Shown to the user when decision is block; not added to Claude's context.")
            .permission_mode("default")
            .output(
                "session-title-only",
                json!({"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","sessionTitle":"Test run"}}),
                &[SOURCE],
            )
            .process(
                "command-exit-2-suppress-original-prompt",
                "exit-2-structured",
                2,
                br#"{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","suppressOriginalPrompt":true}}"#,
                b"blocked by hook",
            )
            .input_source(SDK)
            .uncertainty("reason and exit-2 stderr are shown to the user and are not added to Claude's context.")
            .uncertainty("A blocked prompt never reaches Claude, but by default the block message shown to the user ends with Original prompt: and the submitted text, and Claude Code writes that message to the session transcript on disk. suppressOriginalPrompt leaves the text out of the message whether the hook blocks with decision block or by exiting 2 while printing it; an exit-2 hook that prints no JSON always keeps the text. It changes only the block message: the text can still reach local files such as the transcript and prompt history.")
            .uncertainty("The Agent SDK types two optional inputs the reference does not document: source (user, sdk, system, loop_wakeup, schedule_wakeup, or poll_event, which payloads may omit while the field rolls out) and session_title. The open input schema accepts them without encoding them.")
            .uncertainty("prompt carries pasted text expanded in place; when Claude Code marks pasted text, it sits between pasted_content id marker lines.")
            .uncertainty(TEXT_JSON_RULE),
        Seed::new("UserPromptExpansion", "user_prompt_expansion", "prompt", Prompt)
            .fields(vec![
                required(
                    "expansion_type",
                    json!({"enum":["slash_command","mcp_prompt"]}),
                    json!("slash_command"),
                ),
                string("command_name", "review"),
                string("command_args", "--strict"),
                optional_string("command_source", "plugin"),
                string("prompt", "/review --strict"),
            ])
            .structured(json!({"decision":"block","reason":"Unavailable.","hookSpecificOutput":{"hookEventName":"UserPromptExpansion","additionalContext":"Use the team checklist."}}))
            .exit2_json(
                "deny",
                "user-message",
                "deny",
                json!({"decision":"block","reason":"Blocked by JSON reason."}),
            )
            .handlers(ALL_FIVE)
            .text_context()
            .top_level_block()
            .reason_description("Shown to the user when decision is block.")
            .permission_mode("default")
            .positive(
                "mcp-prompt",
                json!({"expansion_type":"mcp_prompt","command_name":"mcp__github__triage","command_args":"","prompt":"/mcp__github__triage"}),
            )
            .output(
                "suppress-original-prompt",
                json!({"decision":"block","reason":"Unavailable.","hookSpecificOutput":{"hookEventName":"UserPromptExpansion","suppressOriginalPrompt":true}}),
                &[SDK],
            )
            .input_source(SDK)
            .output_source(SDK)
            .uncertainty("reason and exit-2 stderr are shown to the user.")
            .uncertainty("The reference lists command_source among the inputs, but the Agent SDK has typed it optional since 0.3.223, so the schema keeps it optional.")
            .uncertainty("suppressOriginalPrompt is typed by Agent SDK 0.3.285 (omit the original prompt from the block message when decision is block) but not documented for this event; the reference documents it only for UserPromptSubmit. It is accepted as optional, and its effect on an exit-2 block is unknown.")
            .uncertainty(TEXT_JSON_RULE),
        Seed::new("MessageDisplay", "message_display", "display", Display)
            .fields(vec![
                string("turn_id", "turn-1"),
                string("message_id", "message-1"),
                required("index", json!({"type":"integer","minimum":0}), json!(0)),
                required("final", json!({"type":"boolean"}), json!(false)),
                string("delta", "Here is the plan:\n"),
            ])
            .structured(json!({"hookSpecificOutput":{"hookEventName":"MessageDisplay","displayContent":"Here is the plan:"}}))
            .exit2_failure("display-original", "diagnostics")
            .nonzero(Nonzero::Split {
                effect: "display-original",
                stderr_role: "diagnostics",
            })
            .invalid_json_effect("display-original")
            .process(
                "command-exit-2-json-ignored",
                "exit-2",
                2,
                br#"{"hookSpecificOutput":{"hookEventName":"MessageDisplay","displayContent":"Here is the plan:"}}"#,
                b"hook failed",
            )
            .uncertainty("Claude Code acts only on displayContent; systemMessage and continue are discarded.")
            .uncertainty("A failed or timed-out hook displays the original text, and Claude Code notes the failure only in debug output, not in the session. The per-event exit-code table gives exit 2 its own row (the original text is displayed), so exit 2 ignores displayContent; invalid JSON is modeled as the same quiet failure, although the general rule reports a hook error notice for it.")
            .uncertainty("The reference does not say whether displayContent printed with a nonzero exit other than 2 is honored, so the standard other-exit-code rule is assumed for those codes."),
        Seed::new("PreToolUse", "pre_tool_use", "tool", PreTool)
            .fields(tool_fields(false))
            .structured(json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"ask","permissionDecisionReason":"Review command.","updatedInput":{"command":"cargo test"},"additionalContext":"Production environment."}}))
            .exit2_json(
                "deny",
                "agent-context",
                "deny",
                json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow"}}),
            )
            .handlers(ALL_FIVE)
            .permission_mode("default")
            .effort()
            .positive("mcp-tool", mcp_tool_call(false))
            .positive(
                "unrecognized-mode-and-effort",
                json!({"permission_mode":"reviewOnly","effort":{"level":"ultra","source":"setting"}}),
            )
            .input_source(SDK)
            .uncertainty(MCP_SERVER_NOTE)
            .uncertainty("permission_mode and effort.level are open strings: the reference lists their current values, which the schema keeps as examples, but the Agent SDK types both as string, and effort may gain members.")
            .uncertainty("updatedInput is documented only together with a permissionDecision: allow auto-approves the rewritten input, ask shows it to the user, and defer ignores it. Whether Claude Code applies updatedInput without a permissionDecision is undocumented.")
            .uncertainty("permissionDecisionReason is shown to Claude for deny, to the user for ask, and written only to the debug log for allow and defer.")
            .uncertainty("A timed-out command, http, or mcp_tool hook does not block the tool call.")
            .uncertainty("The deprecated top-level decision and reason (approve or block) are still mapped by Claude Code but are intentionally omitted in favor of hookSpecificOutput."),
        Seed::new("PermissionRequest", "permission_request", "tool", Permission)
            .fields(vec![
                string("tool_name", "Bash"),
                required("tool_input", json!({}), json!({"command":"cargo test"})),
                optional("permission_suggestions", json!({"type":"array"}), json!([])),
                mcp_server_field(),
            ])
            .structured(json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Blocked by policy.","interrupt":false}}}))
            .exit2_json(
                "ignored",
                "ignored",
                "event-specific-control",
                json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Blocked by policy."}}}),
            )
            .handlers(NO_AGENT)
            .permission_mode("default")
            .positive(
                "mcp-tool",
                json!({"tool_name":"mcp__github__create_issue","tool_input":{"title":"Flaky test"},"mcp_server":{"name":"github","source":"user"}}),
            )
            .input_source(SDK)
            .uncertainty("Exit status 2 is not honored: without a decision object the permission flow proceeds unchanged and stderr is discarded; a decision object printed with any exit code still applies.")
            .uncertainty("agent handlers are skipped on PermissionRequest since Claude Code v2.1.280.")
            .uncertainty("updatedInput and updatedPermissions apply only to allow; message and interrupt apply only to deny.")
            .uncertainty(MCP_SERVER_NOTE),
        Seed::new("PostToolUse", "post_tool_use", "tool", PostTool)
            .fields(tool_fields(true))
            .structured(json!({"decision":"block","reason":"Review result.","hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"Generated files changed.","updatedToolOutput":{"status":"redacted"}}}))
            .exit2_json(
                "provide-context",
                "agent-context",
                "provide-context",
                json!({"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"Generated files changed."}}),
            )
            .handlers(ALL_FIVE)
            .top_level_block()
            .permission_mode("default")
            .effort()
            .positive("mcp-tool", mcp_tool_call(true))
            .output(
                "classifier-context",
                json!({"hookSpecificOutput":{"hookEventName":"PostToolUse","classifierContext":"This query ran against the staging database, not production."}}),
                &[SOURCE],
            )
            .process(
                "command-classifier-context",
                "structured",
                0,
                br#"{"hookSpecificOutput":{"hookEventName":"PostToolUse","classifierContext":"This query ran against the staging database, not production."}}"#,
                b"",
            )
            .input_source(SDK)
            .output_source(SDK)
            .uncertainty("classifierContext (Claude Code v2.1.236 or later) is capped at 2,000 UTF-16 code units shared by every hook for one call, is ignored for async hooks, and is unused for calls the classifier transcript omits; the cap is not encoded as a schema limit.")
            .uncertainty("updatedToolOutput for a built-in tool must match that tool's output shape; otherwise Claude Code ignores it.")
            .uncertainty(MCP_SERVER_NOTE),
        Seed::new("PostToolUseFailure", "post_tool_use_failure", "tool", Context)
            .fields(vec![
                string("tool_name", "Bash"),
                required("tool_input", json!({}), json!({"command":"cargo test"})),
                string("tool_use_id", "toolu_1"),
                string("error", "Exit code 1\nerror: test failed"),
                optional(
                    "is_interrupt",
                    json!({"type":"boolean","description":"True when the failure reached Claude Code as an abort rather than as an error the tool reported."}),
                    json!(false),
                ),
                optional(
                    "duration_ms",
                    json!({"type":"number","minimum":0}),
                    json!(12),
                ),
                mcp_server_field(),
            ])
            .structured(context("PostToolUseFailure"))
            .exit2_json(
                "provide-context",
                "agent-context",
                "provide-context",
                json!({"hookSpecificOutput":{"hookEventName":"PostToolUseFailure","additionalContext":"Retry with --offline."}}),
            )
            .handlers(ALL_FIVE)
            .top_level_block()
            .permission_mode("default")
            .positive(
                "mcp-tool",
                json!({"tool_name":"mcp__github__create_issue","tool_input":{"title":"Flaky test"},"tool_use_id":"toolu_2","error":"MCP error -32603: rate limited","mcp_server":{"name":"github","source":"plugin"}}),
            )
            .input_source(SDK)
            .uncertainty("The decision-control summary lists top-level decision and reason for PostToolUseFailure while the event section documents only additionalContext; both remain accepted.")
            .uncertainty("is_interrupt is true when the failure reached Claude Code as an abort; cancelling a running tool does not fire this event. The error text starts with an Exit code N line for shell tools and may be middle-truncated.")
            .uncertainty(MCP_SERVER_NOTE),
        Seed::new("PostToolBatch", "post_tool_batch", "tool", Context)
            .fields(vec![required(
                "tool_calls",
                json!({"type":"array","items":{"type":"object","required":["tool_name","tool_input","tool_use_id"],"properties":{"tool_name":{"type":"string"},"tool_use_id":{"type":"string"}},"additionalProperties":true}}),
                json!([{"tool_name":"Read","tool_input":{"file_path":"/repo/a"},"tool_use_id":"toolu_1","tool_response":"1\tcontent"}]),
            )])
            .structured(context("PostToolBatch"))
            .exit2_json(
                "stop",
                "user-message-and-agent-context",
                "stop",
                json!({"decision":"block","reason":"Stop before the next model call."}),
            )
            .handlers(ALL_FIVE)
            .top_level_block()
            .permission_mode("default")
            .input_source(SDK)
            .uncertainty("The reference example always carries tool_response in each tool_calls item, but the Agent SDK types it optional in both 0.3.223 and 0.3.285, so items keep it optional.")
            .uncertainty("decision block or continue false stops the loop; the blocking message (JSON reason or stopReason, or exit-2 stderr) is shown as a transcript warning and stays in the conversation, so Claude sees it when the conversation continues."),
        Seed::new("PermissionDenied", "permission_denied", "tool", Retry)
            .fields(vec![
                string("tool_name", "Bash"),
                required(
                    "tool_input",
                    json!({}),
                    json!({"command":"rm -rf /tmp/build"}),
                ),
                string("tool_use_id", "toolu_1"),
                string("reason", "[Irreversible Local Destruction]"),
                mcp_server_field(),
            ])
            .structured(json!({"hookSpecificOutput":{"hookEventName":"PermissionDenied","retry":true}}))
            .nonzero(ignored)
            .handlers(ALL_FIVE)
            .permission_mode("auto")
            .positive(
                "mcp-tool",
                json!({"tool_name":"mcp__github__delete_repo","tool_input":{"name":"scratch"},"tool_use_id":"toolu_2","reason":"Classifier unavailable","mcp_server":{"name":"github","source":"managed"}}),
            )
            .input_source(SDK)
            .uncertainty("retry true only tells the model it may retry; Claude Code ignores it for no-verdict denials.")
            .uncertainty("The exit-code table says exit code and stderr are ignored because the denial already occurred; other nonzero codes are modeled the same way.")
            .uncertainty(MCP_SERVER_NOTE),
        Seed::new("Notification", "notification", "notification", NoControl)
            .fields(vec![
                string("message", "Permission required"),
                optional_string("title", "Claude Code"),
                required(
                    "notification_type",
                    json!({"type":"string","examples":["permission_prompt","idle_prompt","auth_success","elicitation_dialog","elicitation_url_dialog","elicitation_complete","elicitation_response","agent_needs_input","agent_completed","quota_auto_resume_fired","quota_auto_resume_stale","quota_auto_resume_disabled"]}),
                    json!("permission_prompt"),
                ),
            ])
            .structured(json!({"terminalSequence":"\u{7}"}))
            .json(Json::SideEffects)
            .nonzero(ignored)
            .positive(
                "quota-auto-resume",
                json!({"message":"Continuing after the usage limit reset","notification_type":"quota_auto_resume_fired"}),
            )
            .output(
                "discarded-universal-fields",
                json!({"systemMessage":"Permission notification emitted.","continue":true}),
                &[SOURCE],
            )
            .input_source(SDK)
            .output_source(SDK)
            .uncertainty("notification_type is modeled as an open string: the Agent SDK types it as string, and the reference added elicitation_url_dialog and quota_auto_resume_fired, quota_auto_resume_stale, and quota_auto_resume_disabled (Claude Code v2.1.234 or later) after the previous snapshot. The schema lists the documented values as examples.")
            .uncertainty("systemMessage and continue are discarded; terminalSequence is still emitted on every exit code.")
            .uncertainty("The Agent SDK output union types hookSpecificOutput.additionalContext for Notification, which the reference does not document; it is omitted."),
        Seed::new("SubagentStart", "subagent_start", "subagent", Context)
            .fields(vec![
                string("agent_id", "agent-1"),
                string("agent_type", "Explore"),
            ])
            .structured(context("SubagentStart"))
            .exit2_notice()
            .uncertainty("SubagentStart fires on spawn, on subagent resume, and for each message an in-process teammate handles; repeated context is injected only when the subagent context no longer holds the earlier copy.")
            .uncertainty("Exit status 2 renders stderr as a hook error notice in the subagent's own transcript; Claude does not see it.")
            .uncertainty(EXIT_2_NOTICE_JSON),
        Seed::new("SubagentStop", "subagent_stop", "subagent", Stop)
            .fields(vec![
                required("stop_hook_active", json!({"type":"boolean"}), json!(false)),
                string("agent_id", "agent-1"),
                required(
                    "agent_type",
                    json!({"type":"string","description":"May be empty for internal agents such as prompt suggestions and /btw side questions."}),
                    json!("Explore"),
                ),
                string("agent_transcript_path", "/tmp/agent.jsonl"),
                optional_string("last_assistant_message", "Done"),
                background_tasks(),
                session_crons(),
            ])
            .structured(json!({"decision":"block","reason":"Run another pass.","hookSpecificOutput":{"hookEventName":"SubagentStop","additionalContext":"Check edge cases."}}))
            .exit2_json(
                "continue",
                "agent-context",
                "continue",
                json!({"decision":"block","reason":"Run another pass."}),
            )
            .handlers(ALL_FIVE)
            .top_level_block()
            .block_reason_required()
            .permission_mode("default")
            .effort()
            .positive("internal-agent", json!({"agent_type":""}))
            .output_negative(
                "block-without-reason",
                json!({"decision":"block"}),
                "",
                "required",
            )
            .input_source(SDK)
            .uncertainty("Internal agents such as prompt suggestions and /btw side questions also fire SubagentStop with agent_type set to the session's agent name or an empty string; the reference does not say whether agent_id and agent_transcript_path are always present for them, so both remain required.")
            .uncertainty("last_assistant_message is optional: since Claude Code v2.1.271 a subagent using SubagentHandback delivers its report through that tool and the field holds only closing text, if any, and the Agent SDK types the field optional.")
            .uncertainty("SubagentStop uses the Stop decision control: decision block with a reason, or exit-2 stderr, keeps the subagent running and becomes its next instruction.")
            .uncertainty(STOP_CONTEXT)
            .uncertainty(BLOCK_REASON),
        Seed::new("TaskCreated", "task_created", "task", NoControl)
            .fields(task_fields())
            .structured(json!({"decision":"block","reason":"Task needs an owner."}))
            .exit2_json(
                "rollback",
                "agent-context",
                "rollback",
                json!({"decision":"block","reason":"Task needs an owner."}),
            )
            .handlers(ALL_FIVE)
            .top_level_block()
            .reason_description("Returned to Claude as the TaskCreate tool error when decision is block.")
            .output(
                "continue-ignored",
                json!({"continue":false,"stopReason":"Ignored for TaskCreated."}),
                &[SOURCE],
            )
            .uncertainty("decision block or exit 2 deletes the task and returns the message to Claude as the tool error; continue false is ignored and Claude keeps working.")
            .uncertainty(TEAM_NAME_NOTE)
            .uncertainty("The event does not fire in sessions without the Task tools."),
        Seed::new("TaskCompleted", "task_completed", "task", NoControl)
            .fields(task_fields())
            .structured(json!({"continue":false,"stopReason":"Verification is incomplete."}))
            .exit2_json(
                "deny",
                "agent-context",
                "deny",
                json!({"systemMessage":"Tests are still failing."}),
            )
            .handlers(ALL_FIVE)
            .permission_mode("default")
            .uncertainty("continue false stops the teammate only when a teammate finishing its turn triggered the event; it is ignored when the TaskUpdate tool triggered it, while exit 2 still blocks the completion.")
            .uncertainty(TEAM_NAME_NOTE),
        Seed::new("Stop", "stop", "turn", Stop)
            .fields(vec![
                required("stop_hook_active", json!({"type":"boolean"}), json!(false)),
                optional_string("last_assistant_message", "All work complete"),
                background_tasks(),
                session_crons(),
            ])
            .structured(json!({"decision":"block","reason":"Run tests again.","hookSpecificOutput":{"hookEventName":"Stop","additionalContext":"Focus on failures."}}))
            .exit2_json(
                "continue",
                "agent-context",
                "continue",
                json!({"decision":"block","reason":"Run tests again."}),
            )
            .handlers(ALL_FIVE)
            .top_level_block()
            .block_reason_required()
            .permission_mode("default")
            .effort()
            .output_negative(
                "block-without-reason",
                json!({"decision":"block"}),
                "",
                "required",
            )
            .input_source(SDK)
            .uncertainty("The reference lists last_assistant_message among Stop inputs, but the Agent SDK has typed it optional since 0.3.223, so the schema keeps it optional.")
            .uncertainty("background_tasks and session_crons are present when the task registry is reachable.")
            .uncertainty(STOP_CONTEXT)
            .uncertainty(BLOCK_REASON),
        Seed::new("StopFailure", "stop_failure", "turn", NoControl)
            .fields(vec![
                required(
                    "error",
                    json!({"enum":["rate_limit","overloaded","authentication_failed","oauth_org_not_allowed","account_on_hold","billing_error","invalid_request","model_not_found","server_error","max_output_tokens","cloud_credential_error","verification_required","unknown"]}),
                    json!("rate_limit"),
                ),
                optional_string("error_details", "429 Too Many Requests"),
                optional_string("last_assistant_message", "API Error: Rate limit reached"),
            ])
            .json(Json::SideEffects)
            .nonzero(ignored)
            .positive(
                "cloud-credential-error",
                json!({"error":"cloud_credential_error"}),
            )
            .output(
                "terminal-sequence",
                json!({"terminalSequence":"\u{7}"}),
                &[SOURCE],
            )
            .process(
                "command-terminal-sequence",
                "structured",
                0,
                b"{\"terminalSequence\":\"\\u0007\"}",
                b"",
            )
            .input_source(SDK)
            .uncertainty("error includes verification_required, which the Agent SDK SDKAssistantMessageError type lists but the hooks reference does not document.")
            .uncertainty("Matching cloud_credential_error requires Claude Code v2.1.267 or later; earlier releases reported credential-load failures as server_error or unknown.")
            .uncertainty("Claude Code ignores the hook's output and exit code apart from terminalSequence."),
        Seed::new("TeammateIdle", "teammate_idle", "team", NoControl)
            .fields(vec![
                string("teammate_name", "reviewer"),
                required(
                    "team_name",
                    json!({"type":"string","description":"Deprecated upstream; a session-derived name that will be removed in a future release."}),
                    json!("session-a1b2c3d4"),
                ),
            ])
            .structured(json!({"continue":false,"stopReason":"Continue reviewing."}))
            .exit2_json(
                "continue",
                "agent-context",
                "continue",
                json!({"systemMessage":"Teammate kept working."}),
            )
            .handlers(ALL_FIVE)
            .permission_mode("default")
            .input_source(SDK)
            .uncertainty("team_name is deprecated upstream and will be removed in a future release, but the reference and Agent SDK still describe it as always sent, so it remains required."),
        Seed::new("ConfigChange", "config_change", "configuration", NoControl)
            .fields(vec![
                required(
                    "source",
                    json!({"enum":["user_settings","project_settings","local_settings","policy_settings","skills"]}),
                    json!("project_settings"),
                ),
                optional_string("file_path", "/repo/.claude/settings.json"),
            ])
            .structured(json!({"decision":"block","reason":"Configuration change rejected."}))
            .exit2_json(
                "deny",
                "diagnostics",
                "deny",
                json!({"decision":"block","reason":"Configuration change rejected."}),
            )
            .top_level_block()
            .reason_description("Accepted but never shown.")
            .uncertainty("A blocked change surfaces no message to the user or to Claude, whether it is blocked by reason or by exit-2 stderr; Claude Code writes only a debug-log line.")
            .uncertainty("policy_settings changes cannot be blocked; blocking decisions for them are ignored.")
            .uncertainty("systemMessage and continue are discarded."),
        Seed::new("CwdChanged", "cwd_changed", "workspace", WatchPaths)
            .fields(vec![string("old_cwd", "/repo"), string("new_cwd", "/repo/crate")])
            .structured(json!({"systemMessage":"Working directory changed.","hookSpecificOutput":{"hookEventName":"CwdChanged","watchPaths":["/repo/crate/.env"]}}))
            .exit2_notice()
            .uncertainty("continue is discarded; systemMessage is shown as a brief terminal notification in interactive sessions and does not reach the SDK message stream.")
            .uncertainty("Exit status 2 shows stderr to the user only.")
            .uncertainty(EXIT_2_NOTICE_JSON),
        Seed::new("DirectoryAdded", "directory_added", "workspace", NoControl)
            .fields(vec![
                string("directory", "/repo/related"),
                required(
                    "source",
                    json!({"enum":["slash_command","register_repo_root"]}),
                    json!("slash_command"),
                ),
            ])
            .structured(json!({"systemMessage":"Working directory added."}))
            .nonzero(Nonzero::Split {
                effect: "nonblocking-error",
                stderr_role: "diagnostics",
            })
            .uncertainty("continue is discarded; systemMessage is delivered to Claude as context for slash_command additions and written only to the debug log for register_repo_root.")
            .uncertainty("Claude Code does not wait for the hook: the add completes immediately and the hook runs in the background with the 600-second default timeout.")
            .uncertainty("Failure stderr goes to the debug log; for slash_command a count of failed hooks appears in the transcript."),
        Seed::new("FileChanged", "file_changed", "workspace", WatchPaths)
            .fields(vec![
                string("file_path", "/repo/.env"),
                required(
                    "event",
                    json!({"enum":["change","add","unlink"]}),
                    json!("change"),
                ),
            ])
            .structured(json!({"systemMessage":"Watched file changed.","hookSpecificOutput":{"hookEventName":"FileChanged","watchPaths":["/repo/.env","/repo/.env.local"]}}))
            .exit2_notice()
            .uncertainty("continue is discarded; systemMessage is shown as a brief terminal notification in interactive sessions and does not reach the SDK message stream.")
            .uncertainty("Exit status 2 shows stderr to the user only.")
            .uncertainty(EXIT_2_NOTICE_JSON),
        Seed::new("WorktreeRemove", "worktree_remove", "workspace", NoControl)
            .fields(vec![string("worktree_path", "/tmp/worktree")])
            .json(Json::Absent {
                success: "removed",
                failure: "fail-if-directory-remains",
            })
            .uncertainty("Claude Code reads only the exit code: exit 0 counts as removed, and a nonzero exit fails the removal when worktree_path still exists afterward, leaving the worktree on disk with no git fallback.")
            .uncertainty("The reference says Claude Code reads nothing else from the hook and discards its JSON output fields, so stdout is modeled as ignored; the general statement that terminalSequence still fires on discarding events is not restated for WorktreeRemove.")
            .uncertainty("A failed removal writes the hook command and stderr to the debug log; agent view quotes the start of stderr when a background-session delete is refused.")
            .uncertainty("An HTTP hook applies the same failure contract: a non-2xx status, or a 2xx body that is neither empty nor a JSON object, fails the removal when the directory remains. The reference does not say whether a 2xx JSON object that fails the universal output schema also counts as a failure.")
            .uncertainty("Without any WorktreeRemove hook, Claude Code falls back to git worktree remove --force on the WorktreeCreate path."),
        Seed::new("PreCompact", "pre_compact", "compaction", NoControl)
            .fields(vec![
                required(
                    "trigger",
                    json!({"enum":["manual","auto"]}),
                    json!("auto"),
                ),
                required(
                    "custom_instructions",
                    json!({"type":["string","null"],"description":"Null for auto compaction and for a manual /compact without instructions."}),
                    Value::Null,
                ),
            ])
            .structured(json!({"decision":"block","reason":"Save state first."}))
            .exit2_json(
                "deny",
                "user-message",
                "deny",
                json!({"decision":"block","reason":"Save state first."}),
            )
            .top_level_block()
            .representative("trigger", json!("manual"))
            .representative("custom_instructions", json!("Preserve test results"))
            .positive("manual-without-instructions", json!({"trigger":"manual"}))
            .input_source(SDK)
            .uncertainty("custom_instructions is null for auto compaction and for a manual /compact without instructions; the Agent SDK typed it string or null already at 0.3.223.")
            .uncertainty("Exit-2 stderr is shown to the user for a manual /compact; the reference does not say where it goes for auto compaction or where a JSON reason is surfaced.")
            .uncertainty("systemMessage and continue are discarded."),
        Seed::new("PostCompact", "post_compact", "compaction", NoControl)
            .fields(vec![
                required("trigger", json!({"enum":["manual","auto"]}), json!("auto")),
                string("compact_summary", "Summary"),
            ])
            .json(Json::SideEffects)
            .nonzero(Nonzero::Merged {
                effect: "nonblocking-error",
                stderr_role: "user-message",
            })
            .output(
                "discarded-universal-fields",
                json!({"systemMessage":"Compaction complete.","continue":true}),
                &[SOURCE],
            )
            .uncertainty("systemMessage and continue are discarded; terminalSequence is documented to keep working on events that discard those fields.")
            .uncertainty("Exit status 2 shows stderr to the user only; the reference does not say whether terminalSequence fires on a nonzero exit, so nonzero exits are modeled as one outcome."),
        Seed::new("PreModelSwitch", "pre_model_switch", "model", ModelSwitch)
            .fields(model_switch_fields(&["command", "picker", "sdk"]))
            .structured(json!({"hookSpecificOutput":{"hookEventName":"PreModelSwitch","permissionDecision":"ask","permissionDecisionReason":"Switching now re-sends about 180k tokens to the new model. Continue?"}}))
            .exit2_json(
                "deny",
                "user-message",
                "deny",
                json!({"hookSpecificOutput":{"hookEventName":"PreModelSwitch","permissionDecision":"allow"}}),
            )
            .top_level_block()
            .representative("requested_model", json!("opus"))
            .representative("source", json!("command"))
            .representative("context_tokens", json!(182340))
            .representative("prompt_cache_warm", json!(true))
            .representative("estimated_cache_write_usd", json!(1.1396))
            .output(
                "block",
                json!({"decision":"block","reason":"Opus 4.6 is retired for this project."}),
                &[SOURCE],
            )
            .output(
                "cost-notice",
                json!({"systemMessage":"Switching re-caches about 182k tokens (about $1.14)."}),
                &[SOURCE],
            )
            .output_negative(
                "defer-rejected",
                json!({"hookSpecificOutput":{"hookEventName":"PreModelSwitch","permissionDecision":"defer"}}),
                "/hookSpecificOutput/permissionDecision",
                "enum",
            )
            .output_negative(
                "additional-context-rejected",
                json!({"hookSpecificOutput":{"hookEventName":"PreModelSwitch","additionalContext":"Not accepted."}}),
                "/hookSpecificOutput",
                "additionalProperties",
            )
            .process(
                "command-block",
                "structured",
                0,
                br#"{"decision":"block","reason":"Opus 4.6 is retired for this project."}"#,
                b"",
            )
            .input_source(SDK)
            .output_source(SDK)
            .uncertainty("PreModelSwitch requires Claude Code v2.1.251 or later and runs only for switches a user or client requested, not for automatic fallback or restore on resume.")
            .uncertainty("A hook canceled at its timeout blocks the switch.")
            .uncertainty("ask is treated as a refusal everywhere except interactive /model; precedence across hooks is deny over ask over allow.")
            .uncertainty("systemMessage is shown to the user regardless of the decision; permissionDecisionReason is shown for deny and ask and ignored for allow. The reference does not say where a top-level reason is shown when decision is block.")
            .uncertainty("The matcher compares the canonical name derived from to_model, ignoring any [1m] suffix; when no canonical name can be derived every hook runs."),
        Seed::new("PostModelSwitch", "post_model_switch", "model", Context)
            .fields(model_switch_fields(&["command", "picker", "sdk", "auto", "resume"]))
            .structured(json!({"hookSpecificOutput":{"hookEventName":"PostModelSwitch","additionalContext":"On Opus, delegate implementation work to subagents."}}))
            .text_context()
            .representative("requested_model", json!("opus"))
            .representative("source", json!("command"))
            .representative("context_tokens", json!(182340))
            .representative("prompt_cache_warm", json!(true))
            .representative("estimated_cache_write_usd", json!(1.1396))
            .positive(
                "automatic-fallback",
                json!({"source":"auto","requested_model":null,"to_model":"claude-sonnet-5","from_model":"claude-opus-5"}),
            )
            .exit2_notice()
            .input_source(SDK)
            .output_source(SDK)
            .uncertainty("PostModelSwitch requires Claude Code v2.1.251 or later and cannot block; requested_model is null when source is auto and is the restored setting when source is resume.")
            .uncertainty("Plain-text stdout or additionalContext is delivered with the next request; output from a hook still running five seconds after the next prompt rolls over to the following request, and only the last switch's output is delivered.")
            .uncertainty("Exit status 2 renders stderr as a hook error notice for the user; Claude does not see it.")
            .uncertainty(EXIT_2_NOTICE_JSON)
            .uncertainty(TEXT_JSON_RULE),
        Seed::new("SessionEnd", "session_end", "session", NoControl)
            .fields(vec![required(
                "reason",
                json!({"enum":["clear","resume","logout","prompt_input_exit","other","bypass_permissions_disabled"],"description":"bypass_permissions_disabled was removed in Claude Code v2.1.234 and is sent only by older releases."}),
                json!("other"),
            )])
            .json(Json::SideEffects)
            .nonzero(Nonzero::Merged {
                effect: "nonblocking-error",
                stderr_role: "user-message",
            })
            .positive(
                "legacy-bypass-permissions-disabled",
                json!({"reason":"bypass_permissions_disabled"}),
            )
            .output(
                "discarded-universal-fields",
                json!({"systemMessage":"Session ended."}),
                &[SOURCE],
            )
            .input_source(SDK)
            .uncertainty("bypass_permissions_disabled was removed in Claude Code v2.1.234 and is no longer sent; it stays in the enum because older releases may still send it.")
            .uncertainty("Claude Code discards SessionEnd JSON output fields such as systemMessage; terminalSequence is documented to keep working on events that discard those fields.")
            .uncertainty("Exit status 2 shows stderr to the user only; the reference does not say whether terminalSequence fires on a nonzero exit, so nonzero exits are modeled as one outcome."),
        Seed::new("Elicitation", "elicitation", "mcp", Elicitation)
            .fields(vec![
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
            ])
            .structured(json!({"hookSpecificOutput":{"hookEventName":"Elicitation","action":"accept","content":{"name":"Ada"}}}))
            .exit2("deny", "ignored")
            .top_level_block()
            .output(
                "decision-block",
                json!({"decision":"block","reason":"Declined by policy."}),
                &[CHANGELOG],
            )
            .process(
                "command-decision-block",
                "structured",
                0,
                br#"{"decision":"block","reason":"Declined by policy."}"#,
                b"",
            )
            .output_source(CHANGELOG)
            .uncertainty("Top-level decision block declining the elicitation is sourced only from the Claude Code 2.1.284 changelog; the hooks reference does not yet document decision or reason for this event, and where a reason is surfaced is unknown.")
            .uncertainty("Exit status 2 denies the elicitation; stderr is shown nowhere and an exit-2 hookSpecificOutput is ignored.")
            .uncertainty("systemMessage and continue are discarded."),
        Seed::new("ElicitationResult", "elicitation_result", "mcp", Elicitation)
            .fields(vec![
                string("mcp_server_name", "forms"),
                required(
                    "action",
                    json!({"enum":["accept","decline","cancel"]}),
                    json!("accept"),
                ),
                optional("mode", json!({"enum":["form","url"]}), json!("form")),
                optional_string("elicitation_id", "e1"),
                optional("content", json!({"type":"object"}), json!({"name":"Ada"})),
            ])
            .structured(json!({"hookSpecificOutput":{"hookEventName":"ElicitationResult","action":"decline"}}))
            .exit2("decline", "ignored")
            .top_level_block()
            .output(
                "decision-block",
                json!({"decision":"block","reason":"Declined by policy."}),
                &[CHANGELOG],
            )
            .process(
                "command-decision-block",
                "structured",
                0,
                br#"{"decision":"block","reason":"Declined by policy."}"#,
                b"",
            )
            .output_source(CHANGELOG)
            .uncertainty("Top-level decision block declining the response is sourced only from the Claude Code 2.1.284 changelog; the hooks reference does not yet document decision or reason for this event, and where a reason is surfaced is unknown.")
            .uncertainty("Exit status 2 changes the effective action to decline; stderr is shown nowhere and an exit-2 hookSpecificOutput is ignored.")
            .uncertainty("systemMessage and continue are discarded."),
    ]
}

fn context(event: &str) -> Value {
    json!({"hookSpecificOutput":{"hookEventName":event,"additionalContext":"Hook-provided context."}})
}

fn mcp_server_field() -> Field {
    conditional(optional(
        "mcp_server",
        json!({
            "type":"object",
            "description":"Present only for MCP tools (Claude Code v2.1.274 or later). The source vocabulary is open.",
            "required":["name","source"],
            "properties":{
                "name":{"type":"string"},
                "source":{
                    "type":"string",
                    "examples":["sdk","plugin","user","project","local","dynamic","managed","enterprise","claudeai","agent"]
                }
            },
            "additionalProperties":true
        }),
        json!({"name":"github","source":"user"}),
    ))
}

/// Minimal-input overrides that turn a tool event fixture into an MCP tool call.
fn mcp_tool_call(response: bool) -> Value {
    let mut value = json!({
        "tool_name":"mcp__github__search_issues",
        "tool_input":{"query":"is:open label:bug"},
        "tool_use_id":"toolu_2",
        "mcp_server":{"name":"github","source":"plugin"}
    });
    if response {
        value["tool_response"] = json!([{"type":"text","text":"3 issues"}]);
    }
    value
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
    fields.push(mcp_server_field());
    fields
}

fn task_fields() -> Vec<Field> {
    vec![
        string("task_id", "task-1"),
        string("task_subject", "Run tests"),
        optional_string("task_description", "Run all checks"),
        optional_string("teammate_name", "reviewer"),
        optional(
            "team_name",
            json!({"type":"string","description":"Deprecated upstream; a session-derived name that will be removed in a future release."}),
            json!("session-a1b2c3d4"),
        ),
    ]
}

/// Input fields shared by PreModelSwitch and PostModelSwitch; examples are the minimal input.
fn model_switch_fields(sources: &[&str]) -> Vec<Field> {
    vec![
        string("from_model", "claude-sonnet-5"),
        string("to_model", "claude-opus-5"),
        required(
            "requested_model",
            json!({"type":["string","null"]}),
            Value::Null,
        ),
        required("source", json!({"enum":sources}), json!("picker")),
        required(
            "context_tokens",
            json!({"type":"integer","minimum":0}),
            json!(0),
        ),
        required("prompt_cache_warm", json!({"type":"boolean"}), json!(false)),
        required("cache_ttl", json!({"enum":["5m","1h"]}), json!("5m")),
        required(
            "estimated_cache_write_usd",
            json!({"type":"number","minimum":0}),
            json!(0),
        ),
        required(
            "pricing",
            json!({"enum":["configured","catalog","default"]}),
            json!("catalog"),
        ),
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
    if !matches!(seed.json, Json::Absent { .. }) {
        write_json(
            &directory.join("output.command.schema.json"),
            &output_schema(seed),
        );
    }
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
            "scratchpad_dir".into(),
            json!({"type":"string","description":"Session scratchpad directory; absent when the session has none or the temp directory is unavailable (Claude Code v2.1.257 or later)."}),
        ),
        (
            "permission_mode".into(),
            json!({"type":"string","examples":["default","plan","acceptEdits","auto","dontAsk","bypassPermissions"],"description":"Open string: the Agent SDK types it as string; the examples are the documented modes."}),
        ),
        (
            "effort".into(),
            json!({"type":"object","required":["level"],"properties":{"level":{"type":"string","examples":["low","medium","high","xhigh","max"],"description":"Open string: the Agent SDK types it as string; the examples are the documented levels."}},"additionalProperties":true}),
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
    if seed.top_level_block {
        top.insert("decision".into(), json!({"const":"block"}));
        let mut reason = json!({"type":"string"});
        if let Some(description) = seed.reason_description {
            reason["description"] = json!(description);
        }
        top.insert("reason".into(), reason);
    }
    let fields = match seed.profile {
        Profile::Context | Profile::Stop => json!({"additionalContext":{"type":"string"}}),
        Profile::SessionStart => {
            json!({"additionalContext":{"type":"string"},"initialUserMessage":{"type":"string"},"sessionTitle":{"type":"string"},"watchPaths":{"type":"array","items":{"type":"string"}},"reloadSkills":{"type":"boolean"}})
        }
        Profile::Prompt if seed.wire == "UserPromptSubmit" => {
            json!({"additionalContext":{"type":"string"},"sessionTitle":{"type":"string"},"suppressOriginalPrompt":{"type":"boolean","description":"Leaves the prompt text out of the block message when the hook blocks with decision block or with exit 2."}})
        }
        Profile::Prompt => {
            json!({"additionalContext":{"type":"string"},"suppressOriginalPrompt":{"type":"boolean","description":"Typed by the Agent SDK (0.3.285) but not documented for this event by the reference."}})
        }
        Profile::Display => json!({"displayContent":{"type":"string"}}),
        Profile::PreTool => {
            json!({"permissionDecision":{"enum":["allow","deny","ask","defer"]},"permissionDecisionReason":{"type":"string"},"updatedInput":{},"additionalContext":{"type":"string"}})
        }
        Profile::PermissionRequest => {
            json!({"decision":{"type":"object","required":["behavior"],"properties":{"behavior":{"enum":["allow","deny"]},"updatedInput":{},"updatedPermissions":{"type":"array"},"message":{"type":"string"},"interrupt":{"type":"boolean"}},"additionalProperties":false}})
        }
        Profile::PostTool => {
            json!({"additionalContext":{"type":"string"},"classifierContext":{"type":"string","description":"Note for the auto-mode classifier (Claude Code v2.1.236 or later), capped at 2,000 UTF-16 code units shared by every hook for one call."},"updatedToolOutput":{},"updatedMCPToolOutput":{}})
        }
        Profile::Retry => json!({"retry":{"type":"boolean"}}),
        Profile::WatchPaths => {
            json!({"watchPaths":{"type":"array","items":{"type":"string"}}})
        }
        Profile::Elicitation => {
            json!({"action":{"enum":["accept","decline","cancel"]},"content":{"type":"object"}})
        }
        Profile::ModelSwitch => {
            json!({"permissionDecision":{"enum":["allow","deny","ask"]},"permissionDecisionReason":{"type":"string"}})
        }
        Profile::NoControl => json!({}),
    };
    if !fields.as_object().expect("fields").is_empty() {
        let mut props = fields.as_object().unwrap().clone();
        props.insert("hookEventName".into(), json!({"const":seed.wire}));
        top.insert("hookSpecificOutput".into(),json!({"type":"object","required":["hookEventName"],"properties":props,"additionalProperties":false}));
    }
    let mut schema = json!({"$schema":"https://json-schema.org/draft/2020-12/schema","$id":format!("urn:agent-hook-kit:contracts:claude-code:{SNAPSHOT}:{}:command-output",kebab(seed.wire)),"type":"object","properties":top,"additionalProperties":false});
    if seed.block_reason_required {
        assert!(
            seed.top_level_block,
            "{}: a block reason needs decision",
            seed.wire
        );
        schema["allOf"] = json!([{
            "if":{"properties":{"decision":{"const":"block"}},"required":["decision"]},
            "then":{"required":["reason"]}
        }]);
    }
    schema
}

fn add_universal(top: &mut Map<String, Value>) {
    top.insert("continue".into(), json!({"type":"boolean"}));
    top.insert(
        "stopReason".into(),
        json!({"type":"string","description":"Shown to the user when continue is false; it stays in the conversation, so Claude sees it if the conversation continues."}),
    );
    top.insert(
        "suppressOutput".into(),
        json!({"type":"boolean","description":"Accepted but has no effect."}),
    );
    top.insert("systemMessage".into(), json!({"type":"string"}));
    top.insert("terminalSequence".into(), json!({"type":"string"}));
}

fn contract(seed: &Seed) -> Value {
    let mut bindings = Map::new();
    bindings.insert("command".into(),json!({"kind":"process","request":{"channel":"stdin","framing":"single-document-at-eof","content_kind":"json"},"outcomes":command_outcomes(seed)}));
    if seed.handlers.contains(&"http") {
        bindings.insert("http".into(),json!({"kind":"http","method":"POST","request_content_type":"application/json","response_content_type":"application/json","request":{"channel":"body","framing":"one-http-message","content_kind":"json"},"outcomes":http_outcomes(seed)}));
    }
    let outputs = if matches!(seed.json, Json::Absent { .. }) {
        json!([])
    } else {
        json!([{"id":"command-response","file":"output.command.schema.json","origin":"derived","sources":seed.output_sources,"assurance":low()}])
    };
    let mut value = json!({"format_version":1,"id":format!("claude-code/{SNAPSHOT}/{}",seed.wire),"harness":"claude-code","snapshot":SNAPSHOT,"event":{"wire_name":seed.wire,"rust_key":seed.key,"category":seed.category,"identification":{"inferability":"definitive","discriminator":{"json_pointer":"/hook_event_name","const":seed.wire}}},"schemas":{"input":{"file":"input.schema.json","origin":"derived","sources":seed.input_sources,"assurance":low()},"outputs":outputs},"bindings":bindings,"handler_kinds":seed.handlers,"fixtures":"fixtures.yaml"});
    let mut uncertainties = seed.uncertainties.clone();
    if seed.exit2.is_some() {
        uncertainties.push(EXIT_PRECEDENCE);
    }
    if seed.handlers.contains(&"http")
        && !matches!(seed.json, Json::Absent { .. })
        && seed.failure_effect() != "nonblocking-error"
    {
        uncertainties.push(HTTP_FAILURE_CONTRACT);
    }
    if !uncertainties.is_empty() {
        value["uncertainties"] = json!(uncertainties);
    }
    value
}

fn low() -> Value {
    json!({"confidence":"low","verification":"source-reviewed"})
}

fn channel(presence: &str, role: &str, kind: &str) -> Value {
    let mut value = json!({"presence":presence,"role":role,"content_kind":kind});
    if kind == "text" {
        value["encoding"] = json!("utf-8");
    }
    value
}

fn outcome(id: &str, effect: &str, exit: Value, stdout: Value, stderr: Value) -> Value {
    let mut value = json!({"id":id,"effect":effect,"exit":exit,"stdout":stdout,"stderr":stderr,"sources":[SOURCE],"assurance":low()});
    if value["stdout"]["content_kind"] == "json" {
        value["output_schema"] = json!("command-response");
    }
    value
}

fn command_outcomes(seed: &Seed) -> Vec<Value> {
    let zero = json!({"exact":0});
    let two = json!({"exact":2});
    let nonzero = json!({"range":{"min":1,"max":255}});
    let json_out = channel("required", "protocol-value", "json");
    let diagnostics = channel("optional", "diagnostics", "text");
    let ignored_stdout = channel("optional", "ignored", "opaque");
    if let Json::Absent { success, failure } = seed.json {
        return vec![
            outcome(
                "removed",
                success,
                zero,
                ignored_stdout.clone(),
                diagnostics.clone(),
            ),
            outcome("failed", failure, nonzero, ignored_stdout, diagnostics),
        ];
    }
    let mut outcomes = vec![
        outcome(
            "structured",
            seed.structured_effect(),
            zero.clone(),
            json_out.clone(),
            diagnostics.clone(),
        ),
        // Exit 0 with no output reports no decision.
        outcome(
            "no-op",
            "no-op",
            zero.clone(),
            channel("forbidden", "none", "empty"),
            diagnostics.clone(),
        ),
    ];
    let mut plain_text = channel("required", "protocol-value", "text");
    plain_text["semantic_format"] = json!("plain-text-unless-brace-delimited");
    plain_text["trailing_newline"] = json!("allowed");
    if seed.text_context {
        outcomes.push(outcome(
            "text-context",
            "provide-context",
            zero.clone(),
            plain_text,
            diagnostics.clone(),
        ));
    } else {
        // Every other event writes plain-text stdout to the debug log.
        plain_text["role"] = json!("diagnostics");
        outcomes.push(outcome(
            "plain-text",
            "no-op",
            zero.clone(),
            plain_text,
            diagnostics.clone(),
        ));
    }
    if seed.json == Json::Control {
        let effect = match (seed.invalid_json_effect, seed.text_context) {
            (Some(effect), _) => effect,
            (None, true) => "nonblocking-error-context-dropped",
            (None, false) => "nonblocking-error",
        };
        let mut stdout = channel("required", "ignored", "opaque");
        stdout["semantic_format"] = json!("brace-delimited");
        let mut invalid = outcome(
            "invalid-json",
            effect,
            zero.clone(),
            stdout,
            diagnostics.clone(),
        );
        invalid["sources"] = json!([SOURCE, CHANGELOG]);
        outcomes.push(invalid);
    }
    if let Some(exit2) = &seed.exit2 {
        // Stderr is the blocking message only where someone reads it; a message
        // routed to the debug log or discarded does not have to be written,
        // and a failure notice may be empty.
        let presence = if exit2.blocks && !matches!(exit2.stderr_role, "ignored" | "diagnostics") {
            "required"
        } else {
            "optional"
        };
        outcomes.push(outcome(
            "exit-2",
            exit2.effect,
            two.clone(),
            ignored_stdout.clone(),
            channel(presence, exit2.stderr_role, "text"),
        ));
        let json_effect = match &exit2.json {
            Exit2Json::Ignored => None,
            Exit2Json::Read { effect, .. } => Some(*effect),
            Exit2Json::AsStructured => Some(seed.structured_effect()),
        };
        if let Some(effect) = json_effect {
            outcomes.push(outcome(
                "exit-2-structured",
                effect,
                two,
                json_out.clone(),
                channel("optional", exit2.stderr_role, "text"),
            ));
        }
    }
    match seed.nonzero {
        Nonzero::Split {
            effect,
            stderr_role,
        } => {
            let structured_stderr = if seed.json == Json::Control {
                diagnostics
            } else {
                channel("optional", stderr_role, "text")
            };
            outcomes.push(outcome(
                "nonzero-structured",
                seed.structured_effect(),
                nonzero.clone(),
                json_out,
                structured_stderr,
            ));
            outcomes.push(outcome(
                "nonzero-unstructured",
                effect,
                nonzero,
                ignored_stdout,
                channel("optional", stderr_role, "text"),
            ));
        }
        Nonzero::Merged {
            effect,
            stderr_role,
        } => outcomes.push(outcome(
            "nonzero",
            effect,
            nonzero,
            ignored_stdout,
            channel("optional", stderr_role, "text"),
        )),
    }
    outcomes
}

fn http_outcomes(seed: &Seed) -> Vec<Value> {
    let success = json!({"range":{"min":200,"max":299}});
    let failure = json!({"range":{"min":400,"max":599}});
    let no_stderr = channel("forbidden", "none", "empty");
    // A 2xx body that is neither empty nor a JSON object is a failed hook.
    let mut non_json_body = channel("required", "ignored", "opaque");
    non_json_body["semantic_format"] = json!("not-a-json-object");
    let failed = seed.failure_effect();
    if let Json::Absent {
        success: removed, ..
    } = seed.json
    {
        let mut ignored_body = channel("optional", "ignored", "opaque");
        ignored_body["semantic_format"] = json!("empty-or-json-object");
        return vec![
            outcome(
                "removed",
                removed,
                success.clone(),
                ignored_body,
                no_stderr.clone(),
            ),
            outcome(
                "non-json-body",
                failed,
                success,
                non_json_body,
                no_stderr.clone(),
            ),
            outcome(
                "failed",
                failed,
                failure,
                channel("optional", "ignored", "opaque"),
                no_stderr,
            ),
        ];
    }
    vec![
        outcome(
            "structured",
            seed.structured_effect(),
            success.clone(),
            channel("required", "http-body", "json"),
            no_stderr.clone(),
        ),
        outcome(
            "empty-body",
            "no-op",
            success.clone(),
            channel("forbidden", "none", "empty"),
            no_stderr.clone(),
        ),
        outcome(
            "non-json-body",
            failed,
            success,
            non_json_body,
            no_stderr.clone(),
        ),
        outcome(
            "non-success",
            failed,
            failure,
            channel("optional", "ignored", "opaque"),
            no_stderr,
        ),
    ]
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
    representative.insert(
        "scratchpad_dir".into(),
        json!("/tmp/claude-501/-repo/s1/scratchpad"),
    );
    if let Some(mode) = seed.permission_mode {
        representative.insert("permission_mode".into(), json!(mode));
    }
    if seed.effort {
        representative.insert("effort".into(), json!({"level":"high"}));
    }
    for field in &seed.fields {
        if !field.required && field.representative {
            representative.insert(field.name.into(), field.example.clone());
        }
    }
    for (name, value) in &seed.representative {
        representative.insert((*name).into(), value.clone());
    }
    let mut positive = vec![
        json!({"id":"minimal","origin":"synthesized","sources":[SOURCE],"value":minimal}),
        json!({"id":"representative","origin":"synthesized","sources":seed.input_sources,"value":representative}),
    ];
    for (id, overrides) in &seed.positives {
        let mut value = minimal.clone();
        for (name, field) in overrides.as_object().expect("positive overrides") {
            value.insert(name.clone(), field.clone());
        }
        positive.push(
            json!({"id":id,"origin":"synthesized","sources":seed.input_sources,"value":value}),
        );
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
    let negative = json!([{"id":"wrong-discriminator","origin":"regression","sources":[SOURCE],"value":wrong,"expected_pointer":"/hook_event_name","expected_keyword":"const"},{"id":format!("missing-{missing_field}"),"origin":"synthesized","sources":[SOURCE],"value":missing,"expected_pointer":"","expected_keyword":"required"}]);
    let mut document = json!({"format_version":1,"input":{"positive":positive,"negative":negative},"process":process_cases(seed)});
    if !matches!(seed.json, Json::Absent { .. }) {
        let mut outputs = vec![
            json!({"id":"structured","schema":"command-response","origin":"synthesized","sources":[SOURCE],"value":seed.structured}),
        ];
        if let Some(Exit2 {
            json: Exit2Json::Read { fixture, .. },
            ..
        }) = &seed.exit2
        {
            outputs.push(json!({"id":"exit-2-structured","schema":"command-response","origin":"synthesized","sources":[SOURCE],"value":fixture}));
        }
        for example in &seed.outputs {
            outputs.push(json!({"id":example.id,"schema":"command-response","origin":"synthesized","sources":example.sources,"value":example.value}));
        }
        document["output"] = json!(outputs);
    }
    if !seed.output_negatives.is_empty() {
        document["output_negative"] = seed
            .output_negatives
            .iter()
            .map(|negative| json!({"id":negative.id,"schema":"command-response","origin":"regression","sources":[SOURCE],"value":negative.value,"expected_pointer":negative.pointer,"expected_keyword":negative.keyword}))
            .collect();
    }
    document
}

fn process_cases(seed: &Seed) -> Vec<Value> {
    let structured = serde_json::to_vec(&seed.structured).unwrap();
    let mut process = Vec::new();
    if matches!(seed.json, Json::Absent { .. }) {
        process.push(case("command-removed", "command", "removed", 0, b"", b""));
        process.push(case(
            "command-removed-json-ignored",
            "command",
            "removed",
            0,
            br#"{"systemMessage":"Worktree removed."}"#,
            b"",
        ));
        process.push(case(
            "command-failed",
            "command",
            "failed",
            1,
            b"",
            b"worktree is still in use",
        ));
        process.push(case(
            "command-exit-2",
            "command",
            "failed",
            2,
            b"",
            b"worktree is still in use",
        ));
    } else {
        process.push(case(
            "command-structured",
            "command",
            "structured",
            0,
            &structured,
            b"",
        ));
        process.push(case("command-no-op", "command", "no-op", 0, b"", b""));
        if !seed.text_context {
            process.push(case(
                "command-plain-text",
                "command",
                "plain-text",
                0,
                b"Hook finished.\n",
                b"",
            ));
        }
        if seed.text_context {
            process.push(case(
                "command-text",
                "command",
                "text-context",
                0,
                b"Hook-provided context.",
                b"",
            ));
            process.push(case(
                "command-text-json-lines",
                "command",
                "text-context",
                0,
                b"{\"note\":1}\n{\"note\":2}\n",
                b"",
            ));
            process.push(case(
                "command-text-open-brace",
                "command",
                "text-context",
                0,
                b"{ context without a closing brace",
                b"",
            ));
        }
        if seed.json == Json::Control {
            process.push(case(
                "command-invalid-json",
                "command",
                "invalid-json",
                0,
                b"{not json}",
                b"",
            ));
            process.push(case(
                "command-schema-invalid-json",
                "command",
                "invalid-json",
                0,
                b"{\"continue\":\"yes\"}",
                b"",
            ));
        }
        if let Some(exit2) = &seed.exit2 {
            let stderr: &[u8] = if exit2.blocks {
                b"blocked by hook"
            } else {
                b"hook failed"
            };
            if exit2.effect == "ignored" {
                process.push(case(
                    "command-exit-2-ignored",
                    "command",
                    "exit-2",
                    2,
                    b"",
                    stderr,
                ));
            } else {
                process.push(case("command-exit-2", "command", "exit-2", 2, b"", stderr));
                // Brace-delimited stdout that fails to parse, and a parsed
                // object that fails schema validation, leave exit 2 in charge.
                process.push(case(
                    "command-exit-2-invalid-stdout",
                    "command",
                    "exit-2",
                    2,
                    b"{not json}",
                    stderr,
                ));
                process.push(case(
                    "command-exit-2-schema-invalid-json",
                    "command",
                    "exit-2",
                    2,
                    b"{\"continue\":\"yes\"}",
                    stderr,
                ));
            }
            let json_stdout = match &exit2.json {
                Exit2Json::Ignored => None,
                Exit2Json::Read { fixture, .. } => Some(serde_json::to_vec(fixture).unwrap()),
                Exit2Json::AsStructured => Some(structured.clone()),
            };
            if let Some(stdout) = json_stdout {
                process.push(case(
                    "command-exit-2-structured",
                    "command",
                    "exit-2-structured",
                    2,
                    &stdout,
                    stderr,
                ));
            }
        }
        match seed.nonzero {
            Nonzero::Split { .. } => {
                process.push(case(
                    "command-nonzero-structured",
                    "command",
                    "nonzero-structured",
                    1,
                    &structured,
                    b"",
                ));
                process.push(case(
                    "command-nonzero-unstructured",
                    "command",
                    "nonzero-unstructured",
                    1,
                    b"",
                    b"hook failed",
                ));
                if seed.exit2.is_none() {
                    process.push(case(
                        "command-exit-2",
                        "command",
                        "nonzero-unstructured",
                        2,
                        b"",
                        b"hook failed",
                    ));
                }
            }
            Nonzero::Merged { .. } => {
                process.push(case(
                    "command-nonzero",
                    "command",
                    "nonzero",
                    1,
                    b"",
                    b"hook failed",
                ));
                if seed.exit2.is_none() {
                    process.push(case(
                        "command-exit-2",
                        "command",
                        "nonzero",
                        2,
                        b"",
                        b"hook failed",
                    ));
                }
            }
        }
    }
    for extra in &seed.process {
        process.push(case(
            extra.id,
            "command",
            extra.outcome,
            extra.exit,
            &extra.stdout,
            &extra.stderr,
        ));
    }
    if seed.handlers.contains(&"http") {
        if matches!(seed.json, Json::Absent { .. }) {
            process.push(case("http-removed", "http", "removed", 204, b"", b""));
            process.push(case(
                "http-removed-json-ignored",
                "http",
                "removed",
                200,
                br#"{"systemMessage":"Worktree removed."}"#,
                b"",
            ));
            process.push(case(
                "http-non-json",
                "http",
                "non-json-body",
                200,
                b"ok",
                b"",
            ));
            process.push(case("http-error", "http", "failed", 500, b"", b""));
        } else {
            process.push(case(
                "http-structured",
                "http",
                "structured",
                200,
                &structured,
                b"",
            ));
            process.push(case("http-empty", "http", "empty-body", 204, b"", b""));
            process.push(case(
                "http-non-json",
                "http",
                "non-json-body",
                200,
                b"plain text body",
                b"",
            ));
            process.push(case("http-error", "http", "non-success", 500, b"", b""));
        }
    }
    process
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
