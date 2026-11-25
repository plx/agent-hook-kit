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

const SNAPSHOT: &str = "docs-2026-11-25-r1";
const SOURCE: &str = "claude-hooks-reference";
const CHANGELOG: &str = "claude-code-changelog";
const SDK: &str = "claude-agent-sdk-types";
const CODE_MODE: &str = "claude-code-mode";
const PERMISSIONS: &str = "claude-permissions";
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

const ALL_FIVE: &[&str] = &["command", "http", "mcp_tool", "prompt", "agent"];
const NO_AGENT: &[&str] = &["command", "http", "mcp_tool", "prompt"];
const THREE: &[&str] = &["command", "http", "mcp_tool"];
const COMMAND_MCP: &[&str] = &["command", "mcp_tool"];
const COMMAND_ONLY: &[&str] = &["command"];

const HTTP_FAILURE_CONTRACT: &str = "An HTTP hook applies this event's failure contract from the per-event exit-code table: a non-2xx status, or a 2xx body that is neither empty nor a JSON object, has the effect of a failed command hook rather than the generic non-blocking error.";
const EXIT_PRECEDENCE: &str = "Exit status 2 is described only by the exact exit-2 outcomes; the 1-255 nonzero outcomes model the reference's other exit codes and apply to status 2 only where no exit-2 outcome exists.";
const TEXT_JSON_RULE: &str = "Stdout whose trimmed text starts with { and ends with } is parsed as JSON (Claude Code v2.1.248 or later); a parse or schema-validation failure is a non-blocking error and the text is not added as context. Anything else is plain text, including a JSON array or a quoted JSON string. Output of two or more lines that each parse as JSON, the first starting with { and the last ending with }, remains plain text when no line is a JSON output object that sets a field, and is a parse failure when one is.";
const JSON_LINES_RULE: &str = "For a hook that isn't async, stdout counts as JSON output only when it is one JSON object with nothing around it but whitespace. Anything that does not start with { and end with } is plain text, including a JSON array or a quoted JSON string. Two or more lines that each parse as JSON, the first starting with { and the last ending with }, are plain text when no line is a JSON output object that sets a field, and a parse failure (invalid-json) when one is; the plain-text and invalid-json semantic formats name those two cases.";
const MCP_SERVER_NOTE: &str = "mcp_server (Claude Code v2.1.274 or later) is present only for MCP tools; its source vocabulary is open, the schema lists the documented values as examples, and an unrecognized source must be treated as an unrecognized configured source rather than sdk.";
const PERMISSION_MCP_SERVER_NOTE: &str = "mcp_server (Claude Code v2.1.274 or later) is sent for MCP tools and, since Claude Code 2.1.332, for the ask raised before a skill an MCP server provides loads; that ask also carries internal tool_kind (mcp_skill) and mcp_skill fields that the Agent SDK strips and the reference does not document, so the open schema accepts them without encoding them. The source vocabulary is open, the schema lists the documented values as examples, and an unrecognized source must be treated as an unrecognized configured source rather than sdk.";
const ON_FAILURE_BLOCKS: &str = "The command and http outcomes describe onFailure unset or continue. With onFailure: block (Claude Code v2.1.295 or later; mcp_tool hooks v2.1.328 or later; ignored for command hooks that set async or asyncRewake), a hook that cannot start, times out, exits with a code other than 0 or 2 even after printing schema-valid JSON that allows the action, or prints unparseable or schema-invalid JSON does what exit code 2 does on this event (on PermissionRequest it denies the request); the command-on-failure-block and http-on-failure-block bindings model that configuration.";
const ON_FAILURE_NO_CHANGE: &str = "The outcomes describe onFailure unset or continue. onFailure: block (Claude Code v2.1.295 or later; mcp_tool hooks v2.1.328 or later; ignored for async and asyncRewake command hooks) makes a failure do what exit code 2 does on this event, which is the same notice, log line, or failure as a non-blocking error, so no modeled outcome changes and no on-failure-block binding is cataloged. The reference does not say whether such a failure is reported differently.";
const ON_FAILURE_IGNORED: &str = "onFailure has no effect on Stop, SubagentStop, TaskCompleted, and TeammateIdle hooks, because exit code 2 there sends Claude back to keep working and Claude cannot repair a hook that won't run, so these outcomes hold whatever onFailure is set to.";
const ON_FAILURE_BINDING_NOTES: &str = "In the command-on-failure-block and http-on-failure-block bindings, a start failure, a connection failure, and a timeout, which no exit or status selector expresses, also take the exit-2 effect. The message reads failed; blocking because onFailure is \"block\" (timed out after a timeout) followed by the program's output, and its stderr role follows this event's exit-2 row. Whether a schema-valid JSON object printed with an exit code other than 0 or 2 still applies its fields alongside the blocking failure is undocumented, so failure-blocked treats stdout as ignored.";
const POWERSHELL_EXIT_2: &str = "A hook that runs in PowerShell (shell: powershell, or a shell-form hook on Windows without Git Bash) did not take the exit-2 outcomes before Claude Code v2.1.309, nor before v2.1.316 when its command begins with a type cast, param, using, or an attribute.";
const OUTPUT_CAP: &str = "Each additionalContext, systemMessage, and initialUserMessage string, and plain stdout measured whole, is capped at 10,000 characters per hook (since Claude Code v2.1.89): longer output is saved to a file in the session directory and replaced by the file path and a preview of up to 2,000 characters, Claude is not asked to read the file, and the cap has no setting. The schemas set no maxLength because longer text is accepted and spilled.";
const ESCAPING: &str = "Since Claude Code v2.1.292, a <system-reminder> or </system-reminder> tag in a hook string reaches Claude with that tag's < replaced by &lt;.";
const LONG_LOG: &str = "Claude Code 2.1.332 shortens text over about 50,000 characters when a hook blocks or fails during a turn with a very large log (changelog only); which text it cuts, and where, is not documented.";
const ASYNC_RULES: &str = "These bindings model synchronous command hooks. A command hook that sets async or asyncRewake is not modeled: decision fields such as decision, permissionDecision, and continue have no effect; after it exits, additionalContext and systemMessage reach Claude on the next turn and are not shown to the user; its JSON must be alone on stdout or on a line of its own (spanning several lines requires v2.1.295 or later), and a field with the wrong type is dropped on its own; Claude Code does not enforce timeout on async hooks but does on asyncRewake hooks; an asyncRewake exit 2 wakes Claude at once with stderr, or stdout when stderr is empty; and onFailure and terminalSequence do not apply.";
const SUSPECT_CAPTURE: &str = "A hook that isn't async and leaves a background process holding its stdout or stderr takes up to 10 seconds to finish after its own process exits, and output printed after Claude Code stops reading is discarded. The reference states the resulting block (stdio went quiet; refusing to honor the suspect capture) only for a PreToolUse allow; whether it applies on this event is unknown.";
const HTTP_UNSTATUSED: &str = "An HTTP connection failure has no status and is a non-blocking error, and a timeout cancels the hook, so neither is an outcome here; how Claude Code handles a 3xx status is undocumented, so non-success covers only 400-599.";
const AGENT_ID_TASK: &str = "agent_id and agent_type identify the subagent or in-process teammate the hook fires in; they may be absent and require Claude Code v2.1.290 or later.";
const TEAM_NAME_NOTE: &str = "team_name is deprecated upstream (a session-derived name that will be removed in a future release).";
const EXIT_2_NOTICE_JSON: &str = "Exit status 2 cannot block this event. A schema-valid JSON object printed with exit 2 is read as on exit 0, because Claude Code reads JSON on every exit code; the reference does not say whether the exit-2 notice is still shown then, so the exit-2-structured outcome keeps stderr as a user message.";
const STOP_CONTEXT: &str = "hookSpecificOutput.additionalContext is non-error feedback that keeps Claude working: like decision block it continues the conversation, subject to the stop_hook_active input and the 8-consecutive-continuation cap (CLAUDE_CODE_STOP_HOOK_BLOCK_CAP), but the transcript labels it hook feedback rather than a hook error.";
const STOP_EXIT_2_EXCEPTIONS: &str = "Two exit-2 rows are narrower than exit-2: exit 2 with nothing on stdout and stderr saying a file is missing, such as No such file or directory, is a non-blocking error (exit-2-missing-file, probably Claude Code 2.1.327 or later; how the text is matched is unspecified, and any stdout disables the rule), and a JSON object with continue false printed with exit 2 ends the turn instead of continuing it (exit-2-continue-false, listed before exit-2-structured; what happens to stderr then is undocumented).";
const BACKGROUND_TASKS_NOTE: &str = "background_tasks[].type is a friendly label such as shell, subagent, monitor, workflow, teammate, cloud session, or MCP task; since Claude Code 2.1.329 a Monitor's script is labeled monitor rather than shell, and command is present for shell tasks and monitor tasks that run a command. Agent SDK 0.3.332 still documents command as shell-only, so the SDK and the reference disagree.";
const ELICITATION_DECISION: &str = "Top-level decision block also declines, overriding an action in the same output, and a decline from one hook overrides another hook's accept or cancel. Neither the user nor the server sees why: Claude Code shows neither stderr nor reason. Claude Code ignored decision on this event from v2.1.105 until v2.1.284.";
const ELICITATION_NONZERO: &str = "Claude Code applies hookSpecificOutput only when the hook exits 0 and ignores it on any other exit code, so schema-valid JSON printed with a nonzero exit is nonzero-json-ignored and the dialog proceeds. Whether a hook error notice is shown then, and whether a top-level decision block still declines on exit 1 or 3-255, are undocumented.";
const BLOCK_REASON: &str = "The reference requires reason when decision is block, so the output schema requires it; it does not require the reason to be non-empty.";

#[derive(Clone, Copy)]
enum Profile {
    Context,
    SessionStart,
    Prompt,
    Display,
    PreTool,
    PreSkillShell,
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
    /// Like `Split`, but schema-valid JSON printed with a nonzero exit is
    /// ignored (`nonzero-json-ignored`, effect `no-op`) instead of read.
    JsonIgnored {
        effect: &'static str,
        stderr_role: &'static str,
    },
}

/// What `onFailure: "block"` changes on an event.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OnFailure {
    /// A blocked failure has the same effect as a non-blocking one.
    NoChange,
    /// Claude Code ignores the field on this event.
    Ignored,
    /// A failure takes `effect` (the event's exit-2 effect, or a deny on
    /// PermissionRequest) with stderr in `stderr_role`; the sibling
    /// `command-on-failure-block` and `http-on-failure-block` bindings
    /// model it.
    Blocks {
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
    origin: &'static str,
    value: Value,
    sources: Vec<&'static str>,
}

struct NegativeOutput {
    id: &'static str,
    value: Value,
    pointer: &'static str,
    keyword: &'static str,
}

/// A positive input: `overrides` extend the minimal input, unless the
/// fixture is an official example, whose value is used verbatim.
struct PositiveInput {
    id: &'static str,
    origin: &'static str,
    value: Value,
    sources: Option<Vec<&'static str>>,
}

/// A negative input derived from the minimal input.
struct NegativeInput {
    id: &'static str,
    overrides: Value,
    remove: &'static [&'static str],
    pointer: &'static str,
    keyword: &'static str,
}

struct ProcessCase {
    id: &'static str,
    binding: &'static str,
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
    positives: Vec<PositiveInput>,
    negatives: Vec<NegativeInput>,
    outputs: Vec<Example>,
    output_negatives: Vec<NegativeOutput>,
    process: Vec<ProcessCase>,
    input_sources: Vec<&'static str>,
    output_sources: Vec<&'static str>,
    /// Sources of the outcomes and of the generic fixtures; the hooks
    /// reference unless it does not document the event.
    base_sources: Vec<&'static str>,
    uncertainties: Vec<&'static str>,
    on_failure: OnFailure,
    /// Exit-0 plain text becomes compaction instructions instead of a no-op.
    compaction_instructions: bool,
    /// The output schema accepts the deprecated top-level `decision`
    /// (`approve` or `block`) and `reason`.
    deprecated_decision: Option<&'static str>,
    /// Description of the universal `continue` field for this event.
    continue_description: Option<&'static str>,
    /// Exit 2 with empty stdout and a missing-file stderr is a non-blocking error.
    exit2_missing_file: bool,
    /// `continue: false` printed with exit 2 takes precedence over the exit-2 effect.
    exit2_continue_false: bool,
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
            negatives: Vec::new(),
            outputs: Vec::new(),
            output_negatives: Vec::new(),
            process: Vec::new(),
            input_sources: vec![SOURCE],
            output_sources: vec![SOURCE],
            base_sources: vec![SOURCE],
            uncertainties: Vec::new(),
            on_failure: OnFailure::NoChange,
            compaction_instructions: false,
            deprecated_decision: None,
            continue_description: None,
            exit2_missing_file: false,
            exit2_continue_false: false,
        }
    }

    /// `onFailure: "block"` turns a failure into the exit-2 effect.
    fn on_failure_block(mut self) -> Self {
        let exit2 = self.exit2.as_ref().expect("a gating event has an exit-2 row");
        self.on_failure = OnFailure::Blocks {
            effect: exit2.effect,
            stderr_role: exit2.stderr_role,
        };
        self
    }

    /// `onFailure: "block"` turns a failure into `effect`, which differs
    /// from the exit-2 effect.
    fn on_failure_block_as(mut self, effect: &'static str, stderr_role: &'static str) -> Self {
        self.on_failure = OnFailure::Blocks {
            effect,
            stderr_role,
        };
        self
    }

    fn on_failure_ignored(mut self) -> Self {
        self.on_failure = OnFailure::Ignored;
        self
    }

    fn compaction_instructions(mut self) -> Self {
        self.compaction_instructions = true;
        self
    }

    fn deprecated_decision(mut self, description: &'static str) -> Self {
        self.deprecated_decision = Some(description);
        self
    }

    fn continue_description(mut self, description: &'static str) -> Self {
        self.continue_description = Some(description);
        self
    }

    fn exit2_missing_file(mut self) -> Self {
        self.exit2_missing_file = true;
        self
    }

    fn exit2_continue_false(mut self) -> Self {
        self.exit2_continue_false = true;
        self
    }

    fn base_sources(mut self, sources: &[&'static str]) -> Self {
        self.base_sources = sources.to_vec();
        self
    }

    fn input_sources(mut self, sources: &[&'static str]) -> Self {
        self.input_sources = sources.to_vec();
        self
    }

    fn output_sources(mut self, sources: &[&'static str]) -> Self {
        self.output_sources = sources.to_vec();
        self
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
        self.positives.push(PositiveInput {
            id,
            origin: "synthesized",
            value: overrides,
            sources: None,
        });
        self
    }

    /// Adds a positive input that extends the minimal input with
    /// `overrides`, citing `sources` instead of the seed's input sources.
    fn positive_from(
        mut self,
        id: &'static str,
        overrides: Value,
        sources: &[&'static str],
    ) -> Self {
        self.positives.push(PositiveInput {
            id,
            origin: "synthesized",
            value: overrides,
            sources: Some(sources.to_vec()),
        });
        self
    }

    /// Adds an example input copied verbatim from the hooks reference.
    fn official_positive(mut self, id: &'static str, verbatim: &'static str) -> Self {
        self.positives.push(PositiveInput {
            id,
            origin: "official",
            value: serde_json::from_str(verbatim).expect("official example is JSON"),
            sources: Some(vec![SOURCE]),
        });
        self
    }

    /// Adds a negative input: the minimal input with `overrides` applied
    /// and the `remove` fields deleted.
    fn negative(
        mut self,
        id: &'static str,
        overrides: Value,
        remove: &'static [&'static str],
        pointer: &'static str,
        keyword: &'static str,
    ) -> Self {
        self.negatives.push(NegativeInput {
            id,
            overrides,
            remove,
            pointer,
            keyword,
        });
        self
    }

    fn output(mut self, id: &'static str, value: Value, sources: &[&'static str]) -> Self {
        self.outputs.push(Example {
            id,
            origin: "synthesized",
            value,
            sources: sources.to_vec(),
        });
        self
    }

    /// Adds an output example copied verbatim from the hooks reference.
    fn official_output(mut self, id: &'static str, verbatim: &'static str) -> Self {
        self.outputs.push(Example {
            id,
            origin: "official",
            value: serde_json::from_str(verbatim).expect("official example is JSON"),
            sources: vec![SOURCE],
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
        self,
        id: &'static str,
        outcome: &'static str,
        exit: i32,
        stdout: &[u8],
        stderr: &[u8],
    ) -> Self {
        self.process_on("command", id, outcome, exit, stdout, stderr)
    }

    /// Adds a process case on `binding`.
    fn process_on(
        mut self,
        binding: &'static str,
        id: &'static str,
        outcome: &'static str,
        exit: i32,
        stdout: &[u8],
        stderr: &[u8],
    ) -> Self {
        self.process.push(ProcessCase {
            id,
            binding,
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
            (
                _,
                Nonzero::Split { effect, .. }
                | Nonzero::Merged { effect, .. }
                | Nonzero::JsonIgnored { effect, .. },
            ) => effect,
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
        PostTool, PreSkillShell, PreTool, Prompt, Retry, SessionStart as Session, Stop,
        WatchPaths,
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
            .positive(
                "subagent-compact",
                json!({"source":"compact","agent_id":"agent-1","agent_type":"Explore"}),
            )
            .input_source(SDK)
            .uncertainty("seconds_since_last_response, context_tokens, prompt_cache_likely_expired, and estimated_cache_write_usd are sent only when source is resume or fork and the transcript holds at least one response; the schema keeps them optional and does not encode that condition.")
            .uncertainty("sessionTitle applies when source is startup, resume, or fork and is ignored on clear and compact.")
            .uncertainty("mcp_tool handlers are skipped for SessionStart at launch, including --continue and --resume, and run only for later SessionStart events after /clear or compaction.")
            .uncertainty("At interactive launch, on a --continue or --resume launch, and after /clear, SessionStart hooks run in the background: a resumed conversation appears without waiting, but Claude's first response, and any prompt sent meanwhile, waits for them. A /resume switch inside a session waits for the hooks instead. Nothing a background hook returns applies after /clear or a switch to another conversation made while it was still running.")
            .uncertainty("fork covers --fork-session with --resume or --continue, the /fork background copy, /branch, and a conversation moved to the background.")
            .uncertainty("session_title holds only a custom title (set by --name, /rename, a hook's sessionTitle, or the Agent SDK's renameSession()); a generated title never appears in it.")
            .uncertainty("Claude Code ignores initialUserMessage and sessionTitle from a plugin that finishes installing after the SessionStart hooks have run (Claude Code v2.1.296 or later).")
            .uncertainty("agent_id and agent_type identify the subagent or teammate the hook fires in; since Claude Code v2.1.323 the SessionStart that follows a compaction inside a subagent carries them, and agent_type is also sent on the main thread with --agent. A subagent's own start fires SubagentStart, not SessionStart.")
            .uncertainty("Agent SDK 0.3.332 says onFailure on an mcp_tool hook is ignored on SessionStart and Setup; the reference does not say so.")
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
            .uncertainty("Only command handlers run on Setup: the reference's handler-support list now says Setup supports only command hooks, and a configured mcp_tool Setup hook is skipped.")
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
            .positive_from(
                "subagent-file-access",
                json!({"load_reason":"nested_traversal","trigger_file_path":"/repo/src/lib.rs","agent_id":"agent-1","agent_type":"Explore","effort":{"level":"high"}}),
                &[SOURCE, CHANGELOG],
            )
            .output(
                "discarded-universal-fields",
                json!({"systemMessage":"Instructions audited.","continue":false,"stopReason":"Ignored."}),
                &[SOURCE],
            )
            .uncertainty("Claude Code discards InstructionsLoaded JSON output fields such as systemMessage and continue; terminalSequence is documented to keep working on events that discard those fields and is the only field modeled with an effect.")
            .uncertainty("The event does not fire when Claude reads AGENTS.md directly through the Project instructions setting.")
            .uncertainty("Since Claude Code v2.1.288, loads caused by a subagent's file access carry that subagent's agent_id and agent_type, and lazy loads carry effort (changelog only)."),
        Seed::new("UserPromptSubmit", "user_prompt_submit", "prompt", Prompt)
            .fields(vec![
                string("prompt", "Run the tests"),
                optional(
                    "session_title",
                    json!({"type":"string","description":"The session's custom title, present when one is set; same meaning as the SessionStart field."}),
                    json!("Review"),
                ),
            ])
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
            .on_failure_block()
            .official_positive(
                "official-example",
                r#"{
  "session_id": "abc123",
  "transcript_path": "/Users/.../.claude/projects/.../00893aaf-19fa-41d2-8238-13269b9b3ca0.jsonl",
  "cwd": "/Users/...",
  "permission_mode": "default",
  "hook_event_name": "UserPromptSubmit",
  "prompt": "Write a function to calculate the factorial of a number"
}"#,
            )
            .positive_from(
                "slash-command-line",
                json!({"prompt":"/deploy staging"}),
                &[SOURCE],
            )
            .positive_from("sdk-source", json!({"source":"loop_wakeup"}), &[SDK])
            .positive_from(
                "poll-event-source",
                json!({"source":"poll_event","prompt":"Background job finished: 3 tests failed."}),
                &[SDK, CODE_MODE],
            )
            .negative(
                "session-title-not-string",
                json!({"session_title":7}),
                &[],
                "/session_title",
                "type",
            )
            .official_output(
                "official-decision-control",
                r#"{
  "decision": "block",
  "reason": "Explanation for decision",
  "hookSpecificOutput": {
    "hookEventName": "UserPromptSubmit",
    "additionalContext": "My additional context here",
    "sessionTitle": "My session title",
    "suppressOriginalPrompt": true
  }
}"#,
            )
            .process(
                "command-exit-2-suppress-original-prompt",
                "exit-2-structured",
                2,
                br#"{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","suppressOriginalPrompt":true}}"#,
                b"blocked by hook",
            )
            .process(
                "command-exit-2-missing-file",
                "exit-2",
                2,
                b"",
                b"/bin/sh: /repo/.claude/hooks/check.sh: No such file or directory\n",
            )
            .input_source(SDK)
            .uncertainty("reason and exit-2 stderr are shown to the user and are not added to Claude's context, except that before Claude Code v2.1.328 a block of Code Mode output added reason to Claude's context.")
            .uncertainty("A blocked prompt never reaches Claude, but by default the block message shown to the user ends with Original prompt: and the submitted text, and Claude Code writes that message to the session transcript on disk. suppressOriginalPrompt leaves the text out of the message whether the hook blocks with decision block or by exiting 2 while printing it; an exit-2 hook that prints no JSON always keeps the text. It changes only the block message: the text can still reach local files such as the transcript and prompt history.")
            .uncertainty("The Agent SDK types an optional source input the reference does not document (user, sdk, system, loop_wakeup, schedule_wakeup, or poll_event, which payloads may omit while the field rolls out); the open input schema accepts it without encoding it. session_title is documented and sent only when the session has a custom title.")
            .uncertainty("The hook fires not only on typed prompts but also when a session-scoped scheduled task or /loop iteration fires, when a background subagent reports back, when another session sends a message to the main conversation, and, from Claude Code v2.1.323 until v2.1.328, on the output of Code Mode code that can run in the background (source poll_event). It does not fire on permission answers replayed in remote sessions (Claude Code v2.1.321 or later).")
            .uncertainty("For a typed skill or custom command, prompt is the line as typed (for example /deploy staging), not the expanded skill content, and the hook runs before expansion (Claude Code v2.1.299 or later); a block stops the command and the shell commands the skill injects. That a block therefore also prevents UserPromptExpansion and PreSkillShellCommand for that command is inferred, not documented. A skill's own frontmatter UserPromptSubmit hook first runs on the next prompt (Claude Code 2.1.299, changelog only).")
            .uncertainty("Esc or an interrupt while hooks check a prompt stops the check and returns typed text to the input; a hook that blocks at the same moment keeps the prompt blocked (Claude Code v2.1.296 or later).")
            .uncertainty("A command, HTTP, or MCP tool hook that reaches its timeout (30 seconds by default) is canceled, its output including additionalContext is discarded with a transcript notice, and the prompt proceeds unless the hook sets onFailure: block. An Agent SDK callback hook that reaches its timeout blocks the prompt.")
            .uncertainty("For a plugin's UserPromptSubmit hook, exit 2 with empty stdout and stderr saying a file is missing (such as No such file or directory) is a non-blocking error rather than a block. The contract has no hook-provenance dimension, so exit-2 still models settings hooks and command-exit-2-missing-file stays exit-2; how the missing-file text is matched is unspecified.")
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
            .positive_from(
                "mcp-skill",
                json!({"command_name":"docs:pdf","command_args":"report.pdf","prompt":"/docs:pdf report.pdf"}),
                &[CHANGELOG],
            )
            .output(
                "suppress-original-prompt",
                json!({"decision":"block","reason":"Unavailable.","hookSpecificOutput":{"hookEventName":"UserPromptExpansion","suppressOriginalPrompt":true}}),
                &[SDK],
            )
            .on_failure_block()
            .input_source(SDK)
            .output_source(SDK)
            .uncertainty("reason and exit-2 stderr are shown to the user.")
            .uncertainty("Skills that MCP servers provide (beta) are named <server>:<skill>, prefixed for connectors and plugin servers (for example claude_ai_Docs:pdf), and reach command_name in that form; the documentation dates them to v2.1.331 and the changelog to 2.1.332, and the expansion_type and command_source such a skill produces are undocumented, so the mcp-skill positive is synthesized.")
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
            .positive(
                "non-interactive-single-call",
                json!({"index":0,"final":true,"delta":"Here is the plan:\n1. Build\n2. Test"}),
            )
            .positive(
                "interactive-empty-final",
                json!({"index":2,"final":true,"delta":""}),
            )
            .uncertainty("In non-interactive runs, including Agent SDK queries and claude -p, MessageDisplay runs once per assistant message with index 0, final true, and the entire message in delta; an empty final delta happens only in interactive runs. The Agent SDK MessageDisplayHookInput docs still describe per-flush batching, and the reference gives no version.")
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
            .deprecated_decision("Deprecated for PreToolUse: approve maps to permissionDecision allow and block to deny. Use hookSpecificOutput.permissionDecision instead.")
            .continue_description("false stops Claude after the hook runs, even when the tool call fails or completes while Claude is still streaming, but does not block this tool call: to block it, also return permissionDecision deny. It does not apply when the permission check refuses the call.")
            .on_failure_block()
            .positive("mcp-tool", mcp_tool_call(false))
            .positive(
                "unrecognized-mode-and-effort",
                json!({"permission_mode":"reviewOnly","effort":{"level":"ultra","source":"setting"}}),
            )
            .positive_from(
                "monitor-call",
                json!({"tool_name":"Monitor","tool_input":{"description":"watch app log","timeout_ms":300000,"command":"tail -f app.log"}}),
                &[SOURCE, SDK],
            )
            .positive_from(
                "monitor-ws",
                json!({"tool_name":"Monitor","tool_input":{"description":"ws","timeout_ms":60000,"ws":{"url":"wss://example.test/feed"}}}),
                &[SOURCE, SDK],
            )
            .positive_from(
                "monitor-command-via-bash-matcher",
                json!({"tool_name":"Bash","tool_input":{"command":"tail -f app.log"}}),
                &[SOURCE],
            )
            .positive_from(
                "code-mode-repl",
                json!({"tool_name":"REPL","tool_input":{"code":"(await $`cat notes.txt`.text())"}}),
                &[CODE_MODE],
            )
            .positive_from(
                "code-mode-cd-prefix",
                json!({"tool_input":{"command":"cd 'app' && git push"}}),
                &[CODE_MODE],
            )
            .positive_from(
                "powershell-tool",
                json!({"tool_name":"PowerShell","tool_input":{"command":"Get-ChildItem -Recurse"}}),
                &[SOURCE],
            )
            .positive_from(
                "webfetch-offset",
                json!({"tool_name":"WebFetch","tool_input":{"url":"https://example.test/docs","prompt":"Summarize the API","offset":20000}}),
                &[SOURCE],
            )
            .official_output(
                "official-decision-control",
                r#"{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "allow",
    "permissionDecisionReason": "My reason here",
    "updatedInput": {
      "field_to_modify": "new value"
    },
    "additionalContext": "Current environment: production. Proceed with caution."
  }
}"#,
            )
            .output(
                "updated-input-without-decision",
                json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","updatedInput":{"command":"cargo test --offline"}}}),
                &[SOURCE, PERMISSIONS],
            )
            .output(
                "updated-input-unknown-key",
                json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow","updatedInput":{"command":"ls","bogus":true}}}),
                &[SOURCE, CHANGELOG],
            )
            .output(
                "deny-and-stop",
                json!({"continue":false,"stopReason":"Policy stop.","hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"Blocked."}}),
                &[SOURCE],
            )
            .output(
                "deprecated-decision-block",
                json!({"decision":"block","reason":"policy"}),
                &[SOURCE],
            )
            .output_negative(
                "updated-input-not-object",
                json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow","updatedInput":"cargo test"}}),
                "/hookSpecificOutput/updatedInput",
                "type",
            )
            .output_negative(
                "deprecated-decision-deny",
                json!({"decision":"deny"}),
                "/decision",
                "enum",
            )
            .process(
                "command-updated-input-without-decision",
                "structured",
                0,
                br#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","updatedInput":{"command":"cargo test --offline"}}}"#,
                b"",
            )
            .process(
                "command-deprecated-decision-block",
                "structured",
                0,
                br#"{"decision":"block","reason":"policy"}"#,
                b"",
            )
            .process(
                "command-deprecated-decision-approve",
                "structured",
                0,
                br#"{"decision":"approve"}"#,
                b"",
            )
            .process(
                "command-continue-false-without-deny",
                "structured",
                0,
                br#"{"continue":false,"stopReason":"Stop after this call."}"#,
                b"",
            )
            .process(
                "command-exit-2-continue-false",
                "exit-2",
                2,
                br#"{"continue":false}"#,
                b"blocked by hook",
            )
            .process(
                "command-exit-2-missing-file",
                "exit-2",
                2,
                b"",
                b"/bin/sh: /repo/.claude/hooks/check.sh: No such file or directory\n",
            )
            .process(
                "command-nonzero-allow-json",
                "nonzero-structured",
                1,
                br#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow"}}"#,
                b"",
            )
            .process_on(
                "command-on-failure-block",
                "command-on-failure-block-nonzero-allow-json",
                "failure-blocked",
                1,
                br#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow"}}"#,
                b"",
            )
            .process_on(
                "command-on-failure-block",
                "command-on-failure-block-json-lines-with-field",
                "invalid-json",
                0,
                b"{\"note\":1}\n{\"systemMessage\":\"hi\"}\n",
                b"",
            )
            .process_on(
                "http",
                "http-structured-empty-object",
                "structured",
                200,
                b"{}",
                b"",
            )
            .input_source(SDK)
            .output_source(SDK)
            .uncertainty(MCP_SERVER_NOTE)
            .uncertainty("permission_mode and effort.level are open strings: the reference lists their current values, which the schema keeps as examples, but the Agent SDK types both as string, and effort may gain members.")
            .uncertainty("updatedInput replaces the entire tool input, and Claude Code evaluates permission rules and a Bash command's auto-background eligibility against the returned input: allow auto-approves it, ask shows it to the user, and defer ignores it. With no permissionDecision the rewrite still applies to the normal permission flow, but in auto mode Claude Code can deny the changed call (a hook changed this call's input after auto mode reviewed it); if Claude then repeats the changed input exactly, the hook runs again and any updatedInput it returns is dropped. Settings and plugin command hooks may rewrite Bash, PowerShell, Monitor, Edit, Write, and NotebookEdit calls without that denial in the sessions that show it. In plan mode updatedInput is still applied. That the rewrite applies without a decision outside auto mode is inferred from these statements.")
            .uncertainty("Since Claude Code 2.1.300, updatedInput is validated like Claude's own input: unknown keys are dropped and a rejected input denies the call (changelog only); whether Claude learns why is undocumented. Since 2.1.326, rewrites from several hooks are kept in more cases, and an approved rewrite stays sandboxed unless it sets dangerouslyDisableSandbox: true (changelog only).")
            .uncertainty("allow does not skip approval for actions no mode auto-approves; reads from network paths such as UNC paths and /net automounts (Claude Code v2.1.292 or later); tools an Agent SDK app marks always_ask, unless the permission mode approves the call; MCP tools marked anthropic/requiresUserInteraction, with or without updatedInput; connector tools an organization set to ask; in plan mode, any call other than ExitPlanMode that isn't read-only (fixed in v2.1.299); shell writes to files the file tools refuse (v2.1.287); and MCP calls in Cowork cloud auto mode, where allow counts as no answer (v2.1.314). Deny and ask rules are still evaluated whatever the hook returns. If the tool's own permission check errors after an allow, the call is decided as if no hook answered (v2.1.323).")
            .uncertainty("When several PreToolUse hooks return different decisions, precedence is deny over defer over ask over allow; since Claude Code v2.1.321 a defer that cannot be honored no longer discards another hook's ask. An ask prompt is labeled [settings], [plugin:<name>], or [skill] by the hook's source. In some setups without managed settings, a mod's tool.check approval can approve a call a PreToolUse hook blocked.")
            .uncertainty("permissionDecisionReason is shown to Claude for deny, to the user for ask, and written only to the debug log for allow and defer; in a -p run with no permission host that denies an ask, Claude reads the ask reason in the tool result.")
            .uncertainty("A PreToolUse hook with a Bash matcher (including Bash|PowerShell, ^Bash$, and Bash.*) also runs on shell commands Claude starts with the Monitor tool (Claude Code v2.1.314 or later), receiving a Bash-shaped call whose allow and updatedInput have no effect; it can still deny or ask. A matcher that includes Monitor (Bash|Monitor, .*, or none) receives the Monitor call's own input (description, timeout_ms, and command or ws), where allow and updatedInput apply unless the hook's if names Bash; a rewrite must keep description and timeout_ms because updatedInput replaces the whole input. A Bash matcher never fires for a WebSocket watch. Whether the Bash-shaped call's tool_use_id is the Monitor call's id, and whether any field marks it, is undocumented.")
            .uncertainty("In Code Mode (an experimental opt-in since Claude Code 2.1.323), the code Claude runs reaches PreToolUse as the REPL tool with the JavaScript in tool_input.code, and tool calls made from the code reach PreToolUse under their own names, so a shell command can arrive prefixed with cd '<dir>' && . On v2.1.329 or later, for command and http hooks, a timeout with onFailure unset refuses the call (the hook did not finish) while another failure goes on to permission checks; onFailure: block refuses both and continue lets both proceed. Between 2.1.323 and 2.1.329 any hook failure on a call from code refused it.")
            .uncertainty("A timed-out command, http, or mcp_tool hook does not block the tool call unless the hook sets onFailure: block, or in Code Mode with onFailure unset (v2.1.329 or later), or on Windows without Git Bash for a shell-form command hook. An Agent SDK callback hook that exceeds its timeout blocks the call.")
            .uncertainty("On Windows without Git Bash, a shell-form PreToolUse command hook that fails with onFailure unset blocks the tool call (Claude Code v2.1.299, extended in v2.1.309 and v2.1.310): any exit code other than 0 or 2, even with JSON that doesn't deny, then counts as a failure, so the hook takes the command-on-failure-block outcomes. Exec form (args) without onFailure: block, or an explicit onFailure: continue, restores the command outcomes.")
            .uncertainty("Claude Code blocks the call without a hook verdict when matching the hooks fails or the tool input cannot be serialized to JSON (Claude Code v2.1.288 or later), and when an mcp_tool hook's answer exceeds 16 MiB (v2.1.321 or later) or arrives without its text.")
            .uncertainty("On claude -p --resume, a deferred call fires PreToolUse again, and a repeated defer with a resume prompt lets Claude answer without running the call; CLAUDE_CODE_DISABLE_DEFERRED_TOOL_RESUME=1 skips the deferred call (v2.1.328 or later). A deferred call whose tool is unavailable on resume fires no PreToolUse and ends with tool_deferred_unavailable; since v2.1.319 it stays pending unless CLAUDE_CODE_DROP_UNAVAILABLE_DEFERRED_TOOL=1.")
            .uncertainty("Claude receives exit-2 stderr as the denial reason prefixed with the event, tool, and command, as in PreToolUse:Bash hook error: [<command>]: <stderr>; under onFailure: block the text after the prefix starts with failed; blocking because onFailure is \"block\". Since Claude Code v2.1.326 a hook exiting 127 is reported as a check that may not have run.")
            .uncertainty("PreToolUse matches every tool name except EndConversation, including PowerShell and Workflow. Files referenced with @ in a prompt are inserted without a tool call and fire no PreToolUse unless CLAUDE_CODE_DISABLE_FILE_MENTIONS=1 (v2.1.299 or later); file-tool file_path values are absolute.")
            .uncertainty("The exit-2 effect holds for schema-valid JSON on stdout: exit 2 with continue false still blocks the call. A schema-valid JSON object printed with exit 0 and continue false but no deny stops Claude after the hook while the tool call still runs.")
            .uncertainty("The deprecated top-level decision and reason are still mapped by Claude Code (approve to allow, block to deny), so the output schema accepts them; Claude Code 2.1.326 made decision block hold in more cases without naming the events (changelog only). HookKit never emits them.")
            .uncertainty("If a script prints an allow and then starts a background process without redirecting that process's stdout and stderr, Claude Code blocks the tool call (stdio went quiet; refusing to honor the suspect capture); a deny printed by a background process after Claude Code stops reading, which happens up to 10 seconds after the hook's own process exits, does not block the call."),
        Seed::new(
            "PreSkillShellCommand",
            "pre_skill_shell_command",
            "tool",
            PreSkillShell,
        )
        .fields(vec![
            required(
                "tool_name",
                json!({"type":"string","examples":["Bash","PowerShell"],"description":"The shell tool that runs the command."}),
                json!("Bash"),
            ),
            required(
                "tool_input",
                json!({"type":"object","required":["command"],"properties":{"command":{"type":"string","description":"The command as it would run, with arguments filled in."}},"additionalProperties":true}),
                json!({"command":"git log -1 --format=%H"}),
            ),
            required(
                "tool_use_id",
                json!({"type":"string","description":"Identifies this command; it names no tool call in the transcript."}),
                json!("skillsh_1"),
            ),
            required(
                "command_name",
                json!({"type":"string","description":"The skill or command whose text holds the shell command."}),
                json!("release-notes"),
            ),
            optional(
                "command_source",
                json!({"type":"string","examples":["skills","syncedSkills","plugin"],"description":"Where the skill or command came from; omitted when unknown. The vocabulary is undocumented."}),
                json!("plugin"),
            ),
            required(
                "invoked_by",
                json!({"type":"string","examples":["user","claude","other"],"description":"Who started the load. user: typed, or sent by an SDK host. claude: the Skill tool, or a turn that Claude scheduled. other: anything else, such as a subagent's preload or a channel message, or not known. Open string: an unrecognized value must be treated as other."}),
                json!("user"),
            ),
        ])
        .structured(json!({"hookSpecificOutput":{"hookEventName":"PreSkillShellCommand","permissionDecision":"deny","permissionDecisionReason":"curl is not allowed in skills."}}))
        .exit2_json(
            "deny",
            "agent-context",
            "deny",
            json!({"hookSpecificOutput":{"hookEventName":"PreSkillShellCommand","permissionDecision":"allow"}}),
        )
        .handlers(THREE)
        .permission_mode("default")
        .deprecated_decision("Typed by the Agent SDK only as the generic approve or block and not documented for this event: block keeps the command from running and approve has no effect.")
        .on_failure_block()
        .base_sources(&[CHANGELOG, SDK, SOURCE])
        .input_sources(&[SDK, CHANGELOG])
        .output_sources(&[SDK, CHANGELOG])
        .representative("tool_name", json!("PowerShell"))
        .representative("tool_input", json!({"command":"git log -5 --oneline"}))
        .representative("invoked_by", json!("claude"))
        .positive(
            "subagent-preload",
            json!({"invoked_by":"other","agent_id":"agent-1","agent_type":"Explore"}),
        )
        .positive("unrecognized-invoked-by", json!({"invoked_by":"scheduler"}))
        .negative(
            "missing-command",
            json!({"tool_input":{}}),
            &[],
            "/tool_input",
            "required",
        )
        .negative(
            "tool-input-string",
            json!({"tool_input":"git log"}),
            &[],
            "/tool_input",
            "type",
        )
        .negative(
            "missing-command-name",
            json!({}),
            &["command_name"],
            "",
            "required",
        )
        .negative(
            "missing-invoked-by",
            json!({}),
            &["invoked_by"],
            "",
            "required",
        )
        .negative(
            "invoked-by-number",
            json!({"invoked_by":1}),
            &[],
            "/invoked_by",
            "type",
        )
        .output(
            "ask-refuses",
            json!({"hookSpecificOutput":{"hookEventName":"PreSkillShellCommand","permissionDecision":"ask","permissionDecisionReason":"Confirm network access."}}),
            &[SDK],
        )
        .output(
            "allow-no-op",
            json!({"hookSpecificOutput":{"hookEventName":"PreSkillShellCommand","permissionDecision":"allow"}}),
            &[SDK],
        )
        .output(
            "updated-input-refuses",
            json!({"hookSpecificOutput":{"hookEventName":"PreSkillShellCommand","updatedInput":{"command":"true"}}}),
            &[SDK],
        )
        .output_negative(
            "additional-context-rejected",
            json!({"hookSpecificOutput":{"hookEventName":"PreSkillShellCommand","additionalContext":"Not accepted."}}),
            "/hookSpecificOutput",
            "additionalProperties",
        )
        .output_negative(
            "approve-rejected",
            json!({"hookSpecificOutput":{"hookEventName":"PreSkillShellCommand","permissionDecision":"approve"}}),
            "/hookSpecificOutput/permissionDecision",
            "enum",
        )
        .process(
            "command-allow",
            "structured",
            0,
            br#"{"hookSpecificOutput":{"hookEventName":"PreSkillShellCommand","permissionDecision":"allow"}}"#,
            b"",
        )
        .process(
            "command-ask-refuses",
            "structured",
            0,
            br#"{"hookSpecificOutput":{"hookEventName":"PreSkillShellCommand","permissionDecision":"ask","permissionDecisionReason":"Confirm network access."}}"#,
            b"",
        )
        .process(
            "command-updated-input-refuses",
            "structured",
            0,
            br#"{"hookSpecificOutput":{"hookEventName":"PreSkillShellCommand","updatedInput":{"command":"true"}}}"#,
            b"",
        )
        .process(
            "command-exit-2-skill-policy",
            "exit-2",
            2,
            b"",
            b"curl is not allowed in skills",
        )
        .uncertainty("PreSkillShellCommand (Claude Code 2.1.332 or later) can refuse a skill's or custom command's inline shell commands as the skill loads. The hooks reference does not document it; the contract rests on the 2.1.332 changelog and the Agent SDK 0.3.332 types, applies the reference's generic exit-code, brace, HTTP, and onFailure rules, and every claim is low confidence. On Claude Code 2.1.331 and older, a settings file naming the event opens a dialog at start.")
        .uncertainty("A hook allows a command only by giving no refusal: allow adds nothing, while deny, ask, and defer all keep the command from running, and updatedInput is never applied, so any value also refuses it. The Agent SDK types no additionalContext for this event.")
        .uncertainty("invoked_by is modeled as an open string although the Agent SDK closes it to user, claude, and other, so a future value does not reject the payload; command_source is omitted when unknown and its vocabulary is undocumented (the examples come from inspection of the 2.1.332 CLI); tool_use_id names no tool call in the transcript and its format is undocumented.")
        .uncertainty("Behavior observed only by inspecting the minified Claude Code 2.1.332 CLI during the audit, not cited as a source: matchers filter tool_name (Bash or PowerShell) and the if field applies as on PreToolUse; the default timeout is 600 seconds; prompt and agent hooks are not skipped but refuse every matching command; hookSpecificOutput named PreToolUse, which this output schema rejects, and PermissionRequest-shaped answers are also accepted; and top-level decision block refuses while approve has no effect.")
        .uncertainty("A refused command is replaced in the loaded skill text by [not run: blocked by hook: <reason>], so exit-2 stderr or the reason reaches Claude inside the skill text, and the user sees a Not run in /<skill> notice with the reason cut to 500 characters and the command to 80. The agent-context stderr role of exit-2 is therefore an inference, and whether systemMessage reaches the user is undocumented.")
        .uncertainty("A non-blocking failure (exit code other than 0 or 2, invalid JSON, or a timeout) lets the command run with a Ran anyway notice, unless the hook sets onFailure: block, which this event honors. continue false aborts the whole skill load (Operation stopped by hook) rather than refusing one command.")
        .uncertainty("The ordering against UserPromptSubmit and UserPromptExpansion is undocumented; that a UserPromptSubmit block keeps this event from firing for that command is inferred from the UserPromptSubmit reference text."),
        Seed::new("PermissionRequest", "permission_request", "tool", Permission)
            .fields(vec![
                string("tool_name", "Bash"),
                required("tool_input", json!({}), json!({"command":"cargo test"})),
                optional("permission_suggestions", json!({"type":"array","description":"Permission updates Claude Code suggests for this request; not an exact list of the options the dialog shows."}), json!([])),
                mcp_server_field_described(
                    "Present for MCP tools (Claude Code v2.1.274 or later) and, since Claude Code 2.1.332, for the ask raised before a skill that an MCP server provides loads, where it names that server. The source vocabulary is open.",
                ),
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
            .on_failure_block_as("deny", "agent-context")
            .positive(
                "mcp-tool",
                json!({"tool_name":"mcp__github__create_issue","tool_input":{"title":"Flaky test"},"mcp_server":{"name":"github","source":"user"}}),
            )
            .positive_from(
                "mcp-skill-ask",
                json!({"tool_name":"Skill","tool_input":{"skill":"docs:search"},"mcp_server":{"name":"docs","source":"claudeai"},"tool_kind":"mcp_skill","mcp_skill":{"name":"docs:search","version":"v3"}}),
                &[CHANGELOG, SDK],
            )
            .official_positive(
                "official-example",
                r#"{
  "session_id": "abc123",
  "transcript_path": "/Users/.../.claude/projects/.../00893aaf-19fa-41d2-8238-13269b9b3ca0.jsonl",
  "cwd": "/Users/...",
  "permission_mode": "default",
  "hook_event_name": "PermissionRequest",
  "tool_name": "Bash",
  "tool_input": {
    "command": "rm -rf node_modules",
    "description": "Remove node_modules directory"
  },
  "permission_suggestions": [
    {
      "type": "addRules",
      "rules": [{ "toolName": "Bash", "ruleContent": "rm -rf node_modules" }],
      "behavior": "allow",
      "destination": "localSettings"
    }
  ]
}"#,
            )
            .output(
                "set-mode-manual",
                json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow","updatedPermissions":[{"type":"setMode","mode":"manual","destination":"session"}]}}}),
                &[SOURCE],
            )
            .output_negative(
                "updated-input-not-object",
                json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow","updatedInput":"cargo test"}}}),
                "/hookSpecificOutput/decision/updatedInput",
                "type",
            )
            .process(
                "command-set-mode-manual",
                "structured",
                0,
                br#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow","updatedPermissions":[{"type":"setMode","mode":"manual","destination":"session"}]}}}"#,
                b"",
            )
            .process_on(
                "command-on-failure-block",
                "command-on-failure-block-nonzero-empty",
                "failure-blocked",
                1,
                b"",
                b"",
            )
            .input_source(SDK)
            .output_source(SDK)
            .uncertainty("Exit status 2 is not honored: without a decision object the permission flow proceeds unchanged and stderr is discarded; a decision object printed with any exit code still applies.")
            .uncertainty("agent handlers are skipped on PermissionRequest since Claude Code v2.1.280.")
            .uncertainty("updatedInput and updatedPermissions apply only to allow; message and interrupt apply only to deny. A rewrite is re-evaluated against deny and ask rules; since Claude Code 2.1.323 a rewrite that fails the permission check is asked again or refused, and since 2.1.328 a rewrite to a sandbox-excluded command runs outside the sandbox (changelog only).")
            .uncertainty("Under onFailure: block a failure denies the request rather than taking the exit-2 row, and the deny carries the failure message, which the command-on-failure-block binding records as agent-context; where that message is surfaced is not documented.")
            .uncertainty("For a call that reaches a --permission-prompt-tool or an Agent SDK canUseTool callback, the hooks run alongside that host and whichever decides first applies. In a -p run outside dontAsk with neither host, the hooks also fire for calls that would otherwise be auto-denied. They never fire for a sandboxed command's network request or for EndConversation.")
            .uncertainty("permission_suggestions is not the dialog's option list: some dialogs ignore it, may withhold an option whose suggestion stays in the array, or offer options with no entry. setMode accepts manual as an alias for default, although the Agent SDK PermissionMode type omits it.")
            .uncertainty("behavior allow cannot approve what a deny rule refuses, unless a mod overrides the rule. Since Claude Code 2.1.331 Skill(name:*) allow rules from updatedPermissions are saved outside local settings; in Cowork cloud sessions setMode in an allow is ignored (2.1.300) and auto-mode hooks cannot approve shell commands or subagents (2.1.299) (changelog only).")
            .uncertainty("A PermissionRequest hook with a Bash matcher also runs on shell commands Claude starts with the Monitor tool (Claude Code v2.1.314 or later), but its allow and updatedInput have no effect on them; a matcher that includes Monitor receives the Monitor call's own input, where they apply.")
            .uncertainty("On Windows without Git Bash, a shell-form PermissionRequest command hook that fails with onFailure unset blocks the call (Claude Code v2.1.299, extended in v2.1.309 and v2.1.310), so it takes the command-on-failure-block outcomes; exec form without onFailure: block, or an explicit onFailure: continue, restores the command outcomes.")
            .uncertainty("Claude Code blocks the call without a hook verdict when matching the hooks fails or the tool input cannot be serialized to JSON (Claude Code v2.1.288 or later).")
            .uncertainty("How continue false interacts with a PermissionRequest decision is undocumented.")
            .uncertainty("The tool_name and tool_input of the ask raised before an MCP-served skill loads are undocumented; the mcp-skill-ask positive is synthesized from the Agent SDK 0.3.332 type change.")
            .uncertainty(PERMISSION_MCP_SERVER_NOTE),
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
            .positive_from(
                "managed-failed-call",
                json!({"tool_response":"Exit code 101\nerror: test failed"}),
                &[SOURCE],
            )
            .positive_from(
                "code-mode-repl-result",
                json!({"tool_name":"REPL","tool_input":{"code":"(await $`cat notes.txt`.text())"},"tool_response":{"code":"(await $`cat notes.txt`.text())","result":"deploy token: abc123","stdout":"","stderr":"","toolsUsed":["Bash"]}}),
                &[CODE_MODE],
            )
            .continue_description("false stops Claude after the hook runs, even when the tool call failed or completed while Claude was still streaming a response.")
            .on_failure_block()
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
            .process_on(
                "command-on-failure-block",
                "command-on-failure-block-lint-crashed",
                "failure-blocked",
                1,
                b"",
                b"lint crashed",
            )
            .uncertainty("classifierContext (Claude Code v2.1.236 or later) is capped at 2,000 UTF-16 code units shared by every hook for one call (the reference says characters), is ignored for async hooks, and is unused for calls the classifier transcript omits; the cap is not encoded as a schema limit. For hooks from settings files, plugins, skills, and agent frontmatter the classifier treats the note as unverified context that never establishes user intent; only an in-process Agent SDK callback may relay user intent, and restored notes after a resume are unverified.")
            .uncertainty("updatedToolOutput for a built-in tool must match that tool's output shape; otherwise Claude Code ignores it. In Cowork cloud sessions Claude Code ignores updatedToolOutput and updatedMCPToolOutput for some tools, such as AskUserQuestion and ExitPlanMode (open list; Claude Code v2.1.321 or later), and in cloud scheduled tasks a hook from the user's own settings cannot replace ExitPlanMode's plan (2.1.323, changelog only).")
            .uncertainty("In the main conversation, PostToolUse hooks from managed settings also run after some failed calls, and tool_response is then the error text as a string; since Claude Code 2.1.328 they no longer run for calls refused before they ran. A string tool_response therefore does not prove the call succeeded.")
            .uncertainty("After a Ctrl+Enter turn end while a tool runs, the hooks run on the tool's result and Claude Code kills a command hook still running after 10 seconds, whatever its timeout (Claude Code v2.1.319 or later; earlier releases did not start the hooks); Enter keeps the normal timeout and Esc cancels the hooks.")
            .uncertainty("In Code Mode, a hook matching REPL runs when code finishes, including background code, and when code throws; tool_response then carries code, result, stdout, stderr, and toolsUsed, and optionally error, images, documents, imagePagesFailed, uncutOutputNote (v2.1.331 or later), and refusedCalls.")
            .uncertainty("A PostToolUse hook with a Bash matcher does not run for Monitor commands.")
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
            .on_failure_block()
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
            .on_failure_block()
            .positive_from(
                "refused-call-in-batch",
                json!({"tool_calls":[{"tool_name":"Read","tool_input":{"file_path":"/repo/a"},"tool_use_id":"toolu_1","tool_response":"1\tcontent"},{"tool_name":"Bash","tool_input":{"command":"rm -rf build"},"tool_use_id":"toolu_2","tool_response":"Permission to use Bash has been denied."}]}),
                &[CHANGELOG],
            )
            .input_source(SDK)
            .uncertainty("The reference example always carries tool_response in each tool_calls item, but the Agent SDK types it optional in both 0.3.223 and 0.3.332, so items keep it optional.")
            .uncertainty("tool_calls items do not prove that a call ran: per the Claude Code 2.1.328 changelog, PostToolBatch also gets most calls refused before they ran, not marked as refused, and neither the reference nor the Agent SDK PostToolBatchToolCall type has a refusal flag. That such an item's tool_response is the refusal text is inferred.")
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
            .uncertainty("retry true only tells the model it may retry; Claude Code ignores it for no-verdict denials. Since Claude Code 2.1.331 the retry message tells Claude to make the refused call again only if the user asked for that call (changelog only); the reference still says the model may retry.")
            .uncertainty("The exit-code table says exit code and stderr are ignored because the denial already occurred; other nonzero codes are modeled the same way.")
            .uncertainty(MCP_SERVER_NOTE),
        Seed::new("Notification", "notification", "notification", NoControl)
            .fields(vec![
                string("message", "Permission required"),
                optional_string("title", "Claude Code"),
                required(
                    "notification_type",
                    json!({"type":"string","examples":["permission_prompt","idle_prompt","auth_success","auth_storage_failure","elicitation_dialog","elicitation_url_dialog","elicitation_complete","elicitation_response","agent_needs_input","agent_completed","quota_auto_resume_fired","quota_auto_resume_stale","quota_auto_resume_disabled","plugin_notification"]}),
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
            .positive_from(
                "auth-storage-failure",
                json!({"message":"Signed in, but the credentials could not be saved","notification_type":"auth_storage_failure"}),
                &[SOURCE],
            )
            .positive_from(
                "plugin-notification",
                json!({"message":"CI finished: 2 failures","title":"ci-watch","notification_type":"plugin_notification"}),
                &[SOURCE],
            )
            .output(
                "discarded-universal-fields",
                json!({"systemMessage":"Permission notification emitted.","continue":true}),
                &[SOURCE],
            )
            .input_source(SDK)
            .output_source(SDK)
            .uncertainty("notification_type is modeled as an open string: the Agent SDK types it as string, and the reference keeps adding values, most recently auth_storage_failure (Claude Code v2.1.288 or later; fires in place of auth_success when the new credentials cannot be saved) and plugin_notification (a mod's $.ui.notify, whose title defaults to the mod's name; undated, probably 2.1.295). The schema lists the documented values as examples.")
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
            .positive_from("background-monitor", background_monitor(), &[SOURCE])
            .positive_from("background-mcp-task", background_mcp_task(), &[SOURCE])
            .on_failure_ignored()
            .exit2_missing_file()
            .exit2_continue_false()
            .output_negative(
                "block-without-reason",
                json!({"decision":"block"}),
                "",
                "required",
            )
            .input_source(SDK)
            .uncertainty(STOP_EXIT_2_EXCEPTIONS)
            .uncertainty(BACKGROUND_TASKS_NOTE)
            .uncertainty("Since Claude Code 2.1.331 SubagentStop waits for the subagent's last reply to reach its transcript (changelog only).")
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
            .positive("teammate-agent", json!({"agent_id":"agent-1","agent_type":"reviewer"}))
            .on_failure_block()
            .uncertainty("decision block or exit 2 deletes the task and returns the message to Claude as the tool error; continue false is ignored and Claude keeps working.")
            .uncertainty(AGENT_ID_TASK)
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
            .positive("teammate-agent", json!({"agent_id":"agent-1","agent_type":"reviewer"}))
            .on_failure_ignored()
            .exit2_missing_file()
            .uncertainty("continue false stops the teammate only when a teammate finishing its turn triggered the event; it is ignored when the TaskUpdate tool triggered it, while exit 2 still blocks the completion. The reference says continue false takes precedence over exit 2 on events such as Stop and SubagentStop without naming TaskCompleted, so whether it overrides exit 2 here is not modeled.")
            .uncertainty("Exit 2 with nothing on stdout and stderr saying a file is missing, such as No such file or directory, is a non-blocking error rather than a block (exit-2-missing-file, probably Claude Code 2.1.327 or later); how Claude Code matches the text is unspecified, and any stdout disables the rule.")
            .uncertainty(AGENT_ID_TASK)
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
            .positive_from("background-monitor", background_monitor(), &[SOURCE])
            .positive_from("background-mcp-task", background_mcp_task(), &[SOURCE])
            .official_positive(
                "official-example",
                r#"{
  "session_id": "abc123",
  "transcript_path": "~/.claude/projects/.../00893aaf-19fa-41d2-8238-13269b9b3ca0.jsonl",
  "cwd": "/Users/...",
  "permission_mode": "default",
  "hook_event_name": "Stop",
  "stop_hook_active": true,
  "last_assistant_message": "I've completed the refactoring. Here's a summary...",
  "background_tasks": [
    {
      "id": "task-001",
      "type": "shell",
      "status": "running",
      "description": "tail logs",
      "command": "tail -f /var/log/syslog"
    }
  ],
  "session_crons": [
    {
      "id": "cron-001",
      "schedule": "0 9 * * 1-5",
      "recurring": true,
      "prompt": "check the build"
    }
  ]
}"#,
            )
            .on_failure_ignored()
            .exit2_missing_file()
            .exit2_continue_false()
            .output_negative(
                "block-without-reason",
                json!({"decision":"block"}),
                "",
                "required",
            )
            .input_source(SDK)
            .uncertainty(STOP_EXIT_2_EXCEPTIONS)
            .uncertainty(BACKGROUND_TASKS_NOTE)
            .uncertainty("Since Claude Code 2.1.331 Stop hooks in Claude Desktop no longer miss the last reply in the transcript (changelog only).")
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
            .uncertainty("error includes verification_required and overloaded, which the Agent SDK 0.3.332 SDKAssistantMessageError type lists but the hooks reference does not document. The reference's error list and StopFailure matcher list now omit overloaded and report an HTTP 529 overloaded response as server_error; overloaded stays in the closed enum because releases that send it may still exist.")
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
            .positive("teammate-agent", json!({"agent_id":"agent-1","agent_type":"reviewer"}))
            .on_failure_ignored()
            .input_source(SDK)
            .uncertainty("team_name is deprecated upstream and will be removed in a future release, but the reference and Agent SDK still describe it as always sent, so it remains required.")
            .uncertainty(AGENT_ID_TASK)
            .uncertainty("Since Claude Code 2.1.290 TeammateIdle no longer fires from a teammate's subagents or forks (changelog only).")
            .uncertainty("The reference says continue false takes precedence over exit 2 on events such as Stop and SubagentStop without naming TeammateIdle, so whether it overrides exit 2 here is not modeled."),
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
            .on_failure_block()
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
            .uncertainty("The decision-control summary says WorktreeRemove JSON output is discarded, so stdout is modeled as ignored; the exit-code section now states as a rule for every event that discards hook output that side-effect fields such as terminalSequence still fire on any exit code, which conflicts with that summary for this event.")
            .uncertainty("A failed removal writes the hook command and stderr to the debug log; agent view quotes the start of stderr when a background-session delete is refused.")
            .uncertainty("An HTTP hook applies the same failure contract: a non-2xx status, or a 2xx body that is neither empty nor a JSON object, fails the removal when the directory remains. The reference does not say whether a 2xx JSON object that fails the universal output schema also counts as a failure.")
            .uncertainty("Without any WorktreeRemove hook, Claude Code falls back to git worktree remove --force on the WorktreeCreate path.")
            .uncertainty("A plugin's WorktreeCreate and WorktreeRemove hooks run with --worktree only since Claude Code v2.1.300 and still not with --worktree and --tmux, because the worktree is created before plugins load."),
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
            .compaction_instructions()
            .on_failure_block()
            .representative("trigger", json!("manual"))
            .representative("custom_instructions", json!("Preserve test results"))
            .positive("manual-without-instructions", json!({"trigger":"manual"}))
            .positive_from(
                "subagent-compaction",
                json!({"agent_id":"agent-1","agent_type":"Explore"}),
                &[SOURCE, CHANGELOG],
            )
            .official_positive(
                "official-example",
                r#"{
  "session_id": "abc123",
  "transcript_path": "/Users/.../.claude/projects/.../00893aaf-19fa-41d2-8238-13269b9b3ca0.jsonl",
  "cwd": "/Users/...",
  "hook_event_name": "PreCompact",
  "trigger": "manual",
  "custom_instructions": null
}"#,
            )
            .process(
                "command-compaction-instructions-open-brace",
                "invalid-json",
                0,
                b"{keep all}",
                b"",
            )
            .process(
                "command-text-over-cap",
                "compaction-instructions",
                0,
                &[b'a'; 10_001],
                b"",
            )
            .input_source(SDK)
            .uncertainty("custom_instructions is null for auto compaction and for a manual /compact without instructions; the Agent SDK typed it string or null already at 0.3.223.")
            .uncertainty("Exit-2 stderr is shown to the user for a manual /compact; the reference does not say where it goes for auto compaction or where a JSON reason is surfaced.")
            .uncertainty("systemMessage and continue are discarded.")
            .uncertainty("When a PreCompact hook exits 0 and doesn't block, Claude Code uses its plain-text stdout as instructions for the compaction summary; the 2.1.285 snapshot modeled it as a logged no-op. How hook instructions combine with the user's /compact text in custom_instructions is undocumented.")
            .uncertainty("Claude Code cuts PreCompact stdout over 10,000 characters per hook at the cap and appends a marker (Claude Code v2.1.314 or later) instead of saving it to a file, so command-text-over-cap remains compaction-instructions.")
            .uncertainty("Inspection of the Claude Code 2.1.285 and 2.1.332 CLI builds during the audit, not cited as a source, suggests that the raw stdout of any succeeded, non-blocking run, including a structured exit-0 JSON object such as {}, and a 2xx JSON HTTP body are also appended as instructions, that the user sees the text echoed, and that hook instructions are ignored when a subagent compacts; none of this is documented, so structured stays event-specific-control.")
            .uncertainty("Since Claude Code 2.1.323, PreCompact and PostCompact carry agent_id and agent_type when a subagent compacts (changelog only)."),
        Seed::new("PostCompact", "post_compact", "compaction", NoControl)
            .fields(vec![
                required("trigger", json!({"enum":["manual","auto"]}), json!("auto")),
                string("compact_summary", "Summary"),
            ])
            .json(Json::SideEffects)
            .nonzero(Nonzero::Split {
                effect: "nonblocking-error",
                stderr_role: "user-message",
            })
            .positive_from(
                "subagent-compaction",
                json!({"agent_id":"agent-1","agent_type":"Explore"}),
                &[SOURCE, CHANGELOG],
            )
            .output(
                "discarded-universal-fields",
                json!({"systemMessage":"Compaction complete.","continue":true}),
                &[SOURCE],
            )
            .process(
                "command-nonzero-terminal-sequence",
                "nonzero-structured",
                1,
                b"{\"terminalSequence\":\"\\u0007\"}",
                b"x",
            )
            .uncertainty("systemMessage and continue are discarded; terminalSequence is documented to keep working on events that discard those fields.")
            .uncertainty("Exit status 2 shows stderr to the user only. Claude Code ignores the JSON of an event that discards hook output on any exit code apart from side-effect fields such as terminalSequence, which still fire; the reference states this by example (StopFailure), so nonzero exits are split into nonzero-structured (terminal-sequence-only) and nonzero-unstructured. Whether the stderr notice is still shown when the JSON is schema-valid is undocumented.")
            .uncertainty("Since Claude Code 2.1.323, PreCompact and PostCompact carry agent_id and agent_type when a subagent compacts (changelog only)."),
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
            .on_failure_block()
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
            .nonzero(Nonzero::Split {
                effect: "nonblocking-error",
                stderr_role: "user-message",
            })
            .process(
                "command-nonzero-terminal-sequence",
                "nonzero-structured",
                1,
                b"{\"terminalSequence\":\"\\u0007\"}",
                b"x",
            )
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
            .uncertainty("Exit status 2 shows stderr to the user only. Claude Code ignores the JSON of an event that discards hook output on any exit code apart from side-effect fields such as terminalSequence, which still fire; the reference states this by example (StopFailure), so nonzero exits are split into nonzero-structured (terminal-sequence-only) and nonzero-unstructured. Whether the stderr notice is still shown when the JSON is schema-valid is undocumented.")
            .uncertainty("terminalSequence is emitted only while an interactive interface is open, which may no longer hold when the session ends."),
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
            .nonzero(Nonzero::JsonIgnored {
                effect: "nonblocking-error",
                stderr_role: "user-message",
            })
            .top_level_block()
            .on_failure_block()
            .output(
                "decision-block",
                json!({"decision":"block","reason":"Declined by policy."}),
                &[SOURCE, CHANGELOG],
            )
            .output(
                "decision-block-overrides-action",
                json!({"decision":"block","reason":"r","hookSpecificOutput":{"hookEventName":"Elicitation","action":"accept"}}),
                &[SOURCE],
            )
            .process(
                "command-decision-block",
                "structured",
                0,
                br#"{"decision":"block","reason":"Declined by policy."}"#,
                b"",
            )
            .process(
                "command-nonzero-decision-block",
                "nonzero-json-ignored",
                1,
                br#"{"decision":"block","reason":"r"}"#,
                b"",
            )
            .output_source(CHANGELOG)
            .uncertainty(ELICITATION_DECISION)
            .uncertainty("Exit status 2 declines the elicitation and no dialog appears; stderr is shown nowhere and an exit-2 hookSpecificOutput is ignored.")
            .uncertainty(ELICITATION_NONZERO)
            .uncertainty("systemMessage and continue are discarded, whichever action the hook returns. For a URL-mode request, an accept skips the dialog, so the URL never opens."),
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
            .nonzero(Nonzero::JsonIgnored {
                effect: "nonblocking-error",
                stderr_role: "user-message",
            })
            .top_level_block()
            .on_failure_block()
            .output(
                "decision-block",
                json!({"decision":"block","reason":"Declined by policy."}),
                &[SOURCE, CHANGELOG],
            )
            .output(
                "decision-block-overrides-action",
                json!({"decision":"block","reason":"r","hookSpecificOutput":{"hookEventName":"ElicitationResult","action":"accept","content":{"name":"Ada"}}}),
                &[SOURCE],
            )
            .output(
                "cancel",
                json!({"hookSpecificOutput":{"hookEventName":"ElicitationResult","action":"cancel"}}),
                &[SOURCE],
            )
            .process(
                "command-decision-block",
                "structured",
                0,
                br#"{"decision":"block","reason":"Declined by policy."}"#,
                b"",
            )
            .process(
                "command-nonzero-decision-block",
                "nonzero-json-ignored",
                1,
                br#"{"decision":"block","reason":"r"}"#,
                b"",
            )
            .output_source(CHANGELOG)
            .uncertainty(ELICITATION_DECISION)
            .uncertainty("Exit status 2 changes the effective action to decline; stderr is shown nowhere and an exit-2 hookSpecificOutput is ignored.")
            .uncertainty(ELICITATION_NONZERO)
            .uncertainty("action cancel sends cancel to the server; the reference says to return decline to withhold the values the user submitted, which implies, without stating it, that cancel may still forward them.")
            .uncertainty("systemMessage and continue are discarded."),
    ]
}

fn context(event: &str) -> Value {
    json!({"hookSpecificOutput":{"hookEventName":event,"additionalContext":"Hook-provided context."}})
}

fn mcp_server_field() -> Field {
    mcp_server_field_described(
        "Present only for MCP tools (Claude Code v2.1.274 or later). The source vocabulary is open.",
    )
}

fn mcp_server_field_described(description: &'static str) -> Field {
    conditional(optional(
        "mcp_server",
        json!({
            "type":"object",
            "description":description,
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
        fields.push(required(
            "tool_response",
            json!({"description":"The tool's result in its native shape; for a failed call that a managed-settings hook sees, the error text as a string."}),
            json!({"stdout":"ok"}),
        ));
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
                    "type":{
                        "type":"string",
                        "examples":["shell","subagent","monitor","workflow","teammate","cloud session","MCP task"],
                        "description":"Friendly task-type label; falls back to the raw discriminant for unrecognized types. A Monitor's script is labeled monitor since Claude Code v2.1.329."
                    },
                    "status":{"type":"string"},
                    "description":{"type":"string"},
                    "command":{"type":"string","description":"Present only for shell tasks and monitor tasks that run a command."},
                    "agent_type":{"type":"string","description":"Present only for subagent tasks."},
                    "server":{"type":"string","description":"Present only for monitor and MCP task tasks."},
                    "tool":{"type":"string","description":"Present only for monitor and MCP task tasks."},
                    "name":{"type":"string","description":"Present only for workflow tasks."}
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

/// Overrides for a Stop or SubagentStop input with a running Monitor task.
fn background_monitor() -> Value {
    json!({"background_tasks":[{"id":"t2","type":"monitor","status":"running","description":"Watch build log","command":"tail -f build.log"}]})
}

/// Overrides for a Stop or SubagentStop input with a running MCP task.
fn background_mcp_task() -> Value {
    json!({"background_tasks":[{"id":"t3","type":"MCP task","status":"running","description":"Index repository","server":"search","tool":"index_repo"}]})
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
    add_universal(&mut top, seed.continue_description);
    if seed.top_level_block {
        top.insert("decision".into(), json!({"const":"block"}));
        let mut reason = json!({"type":"string"});
        if let Some(description) = seed.reason_description {
            reason["description"] = json!(description);
        }
        top.insert("reason".into(), reason);
    }
    if let Some(description) = seed.deprecated_decision {
        assert!(
            !seed.top_level_block,
            "{}: one top-level decision",
            seed.wire
        );
        top.insert(
            "decision".into(),
            json!({"enum":["approve","block"],"description":description}),
        );
        top.insert(
            "reason".into(),
            json!({"type":"string","description":"Deprecated together with decision."}),
        );
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
            json!({"additionalContext":{"type":"string"},"suppressOriginalPrompt":{"type":"boolean","description":"Typed by the Agent SDK (0.3.285 and 0.3.332) but not documented for this event by the reference."}})
        }
        Profile::Display => json!({"displayContent":{"type":"string"}}),
        Profile::PreTool => {
            json!({"permissionDecision":{"enum":["allow","deny","ask","defer"]},"permissionDecisionReason":{"type":"string"},"updatedInput":{"type":"object","description":"Replaces the entire tool input; permission rules are evaluated against it."},"additionalContext":{"type":"string"}})
        }
        Profile::PreSkillShell => {
            json!({"permissionDecision":{"enum":["allow","deny","ask","defer"],"description":"allow adds nothing: the command runs unless a hook refuses it. deny, ask, and defer all keep it from running."},"permissionDecisionReason":{"type":"string"},"updatedInput":{"type":"object","description":"Never applied. Any value keeps the command from running."}})
        }
        Profile::PermissionRequest => {
            json!({"decision":{"type":"object","required":["behavior"],"properties":{"behavior":{"enum":["allow","deny"]},"updatedInput":{"type":"object","description":"For allow only: replaces the entire tool input, which is re-evaluated against deny and ask rules."},"updatedPermissions":{"type":"array"},"message":{"type":"string"},"interrupt":{"type":"boolean"}},"additionalProperties":false}})
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

fn add_universal(top: &mut Map<String, Value>, continue_description: Option<&str>) {
    let mut continue_schema = json!({"type":"boolean"});
    if let Some(description) = continue_description {
        continue_schema["description"] = json!(description);
    }
    top.insert("continue".into(), continue_schema);
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

fn process_binding(outcomes: Vec<Value>) -> Value {
    json!({"kind":"process","request":{"channel":"stdin","framing":"single-document-at-eof","content_kind":"json"},"outcomes":outcomes})
}

fn http_binding(outcomes: Vec<Value>) -> Value {
    json!({"kind":"http","method":"POST","request_content_type":"application/json","response_content_type":"application/json","request":{"channel":"body","framing":"one-http-message","content_kind":"json"},"outcomes":outcomes})
}

fn contract(seed: &Seed) -> Value {
    let http = seed.handlers.contains(&"http");
    let mut bindings = Map::new();
    bindings.insert("command".into(), process_binding(command_outcomes(seed)));
    if http {
        bindings.insert("http".into(), http_binding(http_outcomes(seed)));
    }
    if matches!(seed.on_failure, OnFailure::Blocks { .. }) {
        bindings.insert(
            "command-on-failure-block".into(),
            process_binding(block_command_outcomes(seed)),
        );
        if http {
            bindings.insert(
                "http-on-failure-block".into(),
                http_binding(block_http_outcomes(seed)),
            );
        }
    }
    if seed.base_sources != [SOURCE] {
        for binding in bindings.values_mut() {
            for outcome in binding["outcomes"].as_array_mut().expect("outcomes") {
                outcome["sources"] = json!(seed.base_sources);
            }
        }
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
        uncertainties.push(POWERSHELL_EXIT_2);
    }
    if http && !matches!(seed.json, Json::Absent { .. }) && seed.failure_effect() != "nonblocking-error"
    {
        uncertainties.push(HTTP_FAILURE_CONTRACT);
    }
    if http {
        uncertainties.push(HTTP_UNSTATUSED);
    }
    match seed.on_failure {
        OnFailure::Blocks { .. } => {
            uncertainties.push(ON_FAILURE_BLOCKS);
            uncertainties.push(ON_FAILURE_BINDING_NOTES);
        }
        OnFailure::NoChange => uncertainties.push(ON_FAILURE_NO_CHANGE),
        OnFailure::Ignored => uncertainties.push(ON_FAILURE_IGNORED),
    }
    if seed.json == Json::Control {
        if !seed.text_context {
            uncertainties.push(JSON_LINES_RULE);
        }
        if !seed.compaction_instructions {
            uncertainties.push(OUTPUT_CAP);
        }
        uncertainties.push(ESCAPING);
    }
    if !matches!(seed.json, Json::Absent { .. }) {
        uncertainties.push(LONG_LOG);
    }
    uncertainties.push(ASYNC_RULES);
    if seed.wire != "PreToolUse" {
        uncertainties.push(SUSPECT_CAPTURE);
    }
    value["uncertainties"] = json!(uncertainties);
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

/// Exit-0 stdout that Claude Code reads as plain text rather than JSON.
fn plain_text_channel(seed: &Seed, role: &str) -> Value {
    let mut plain_text = channel("required", role, "text");
    plain_text["semantic_format"] = json!(if seed.json == Json::Control {
        "not-brace-delimited-or-fieldless-json-lines"
    } else {
        "plain-text-unless-brace-delimited"
    });
    plain_text["trailing_newline"] = json!("allowed");
    plain_text
}

/// The id of the exit-0 plain-text outcome.
fn text_outcome_id(seed: &Seed) -> &'static str {
    if seed.text_context {
        "text-context"
    } else if seed.compaction_instructions {
        "compaction-instructions"
    } else {
        "plain-text"
    }
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
    if seed.text_context {
        outcomes.push(outcome(
            "text-context",
            "provide-context",
            zero.clone(),
            plain_text_channel(seed, "protocol-value"),
            diagnostics.clone(),
        ));
    } else if seed.compaction_instructions {
        outcomes.push(outcome(
            "compaction-instructions",
            "provide-compaction-instructions",
            zero.clone(),
            plain_text_channel(seed, "agent-context"),
            diagnostics.clone(),
        ));
    } else {
        // Every other event writes plain-text stdout to the debug log.
        outcomes.push(outcome(
            "plain-text",
            "no-op",
            zero.clone(),
            plain_text_channel(seed, "diagnostics"),
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
        stdout["semantic_format"] = json!("brace-delimited-except-fieldless-json-lines");
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
        if seed.exit2_missing_file {
            // Listed before exit-2, which it narrows.
            let mut stderr = channel("required", "user-message", "text");
            stderr["semantic_format"] = json!("missing-file-diagnostic");
            outcomes.push(outcome(
                "exit-2-missing-file",
                "nonblocking-error",
                two.clone(),
                channel("forbidden", "none", "empty"),
                stderr,
            ));
        }
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
        if seed.exit2_continue_false {
            // Listed before exit-2-structured, which it narrows.
            let mut stdout = json_out.clone();
            stdout["semantic_format"] = json!("continue-false");
            let mut continue_false = outcome(
                "exit-2-continue-false",
                "stop",
                two.clone(),
                stdout,
                channel("optional", "ignored", "text"),
            );
            continue_false["sources"] = json!([SOURCE]);
            outcomes.push(continue_false);
        }
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
        Nonzero::JsonIgnored {
            effect,
            stderr_role,
        } => {
            outcomes.push(outcome(
                "nonzero-json-ignored",
                "no-op",
                nonzero.clone(),
                json_out,
                channel("optional", stderr_role, "text"),
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

/// Outcomes of a command hook that sets `onFailure: "block"`: invalid JSON
/// and every exit code other than 0 or 2 take the failure effect.
fn block_command_outcomes(seed: &Seed) -> Vec<Value> {
    let OnFailure::Blocks {
        effect,
        stderr_role,
    } = seed.on_failure
    else {
        unreachable!("{}: no on-failure-block binding", seed.wire);
    };
    let mut outcomes = Vec::new();
    for mut value in command_outcomes(seed) {
        match value["id"].as_str().expect("outcome id") {
            "invalid-json" => {
                value["effect"] = json!(effect);
                value["stderr"] = channel("optional", stderr_role, "text");
                outcomes.push(value);
            }
            "nonzero-structured" | "nonzero-unstructured" | "nonzero-json-ignored" | "nonzero" => {}
            _ => outcomes.push(value),
        }
    }
    let mut blocked = outcome(
        "failure-blocked",
        effect,
        json!({"range":{"min":1,"max":255}}),
        channel("optional", "ignored", "opaque"),
        channel("optional", stderr_role, "text"),
    );
    blocked["sources"] = json!([SOURCE, CHANGELOG]);
    outcomes.push(blocked);
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
    // A 2xx JSON object body that fails the output schema is a failed hook too.
    let mut schema_invalid_body = channel("required", "ignored", "opaque");
    schema_invalid_body["semantic_format"] = json!("json-object-failing-output-schema");
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
            success.clone(),
            non_json_body,
            no_stderr.clone(),
        ),
        outcome(
            "schema-invalid-body",
            failed,
            success,
            schema_invalid_body,
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

/// Outcomes of an HTTP hook that sets `onFailure: "block"`: every failed
/// response takes the failure effect.
fn block_http_outcomes(seed: &Seed) -> Vec<Value> {
    let OnFailure::Blocks { effect, .. } = seed.on_failure else {
        unreachable!("{}: no on-failure-block binding", seed.wire);
    };
    http_outcomes(seed)
        .into_iter()
        .map(|mut value| {
            if matches!(
                value["id"].as_str(),
                Some("non-json-body" | "schema-invalid-body" | "non-success")
            ) {
                value["effect"] = json!(effect);
                value["sources"] = json!([SOURCE, CHANGELOG]);
            }
            value
        })
        .collect()
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
    // Fixtures of an event the hooks reference documents cite it; those of
    // an undocumented event cite the schema's own sources.
    let documented = seed.base_sources == [SOURCE];
    let input_base = if documented {
        vec![SOURCE]
    } else {
        seed.input_sources.clone()
    };
    let output_base = if documented {
        vec![SOURCE]
    } else {
        seed.output_sources.clone()
    };
    let mut positive = vec![
        json!({"id":"minimal","origin":"synthesized","sources":input_base,"value":minimal}),
        json!({"id":"representative","origin":"synthesized","sources":seed.input_sources,"value":representative}),
    ];
    for input in &seed.positives {
        let value = if input.origin == "official" {
            input.value.clone()
        } else {
            let mut value = minimal.clone();
            for (name, field) in input.value.as_object().expect("positive overrides") {
                value.insert(name.clone(), field.clone());
            }
            Value::Object(value)
        };
        let sources = input.sources.as_ref().unwrap_or(&seed.input_sources);
        positive.push(json!({"id":input.id,"origin":input.origin,"sources":sources,"value":value}));
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
    let mut negative = vec![
        json!({"id":"wrong-discriminator","origin":"regression","sources":input_base,"value":wrong,"expected_pointer":"/hook_event_name","expected_keyword":"const"}),
        json!({"id":format!("missing-{missing_field}"),"origin":"synthesized","sources":input_base,"value":missing,"expected_pointer":"","expected_keyword":"required"}),
    ];
    for input in &seed.negatives {
        let mut value = minimal.clone();
        for (name, field) in input.overrides.as_object().expect("negative overrides") {
            value.insert(name.clone(), field.clone());
        }
        for name in input.remove {
            assert!(
                value.remove(*name).is_some(),
                "{}: {name} absent",
                seed.wire
            );
        }
        negative.push(json!({"id":input.id,"origin":"synthesized","sources":seed.input_sources,"value":value,"expected_pointer":input.pointer,"expected_keyword":input.keyword}));
    }
    let mut document = json!({"format_version":1,"input":{"positive":positive,"negative":negative},"process":process_cases(seed)});
    if !matches!(seed.json, Json::Absent { .. }) {
        let mut outputs = vec![
            json!({"id":"structured","schema":"command-response","origin":"synthesized","sources":output_base,"value":seed.structured}),
        ];
        if let Some(Exit2 {
            json: Exit2Json::Read { fixture, .. },
            ..
        }) = &seed.exit2
        {
            outputs.push(json!({"id":"exit-2-structured","schema":"command-response","origin":"synthesized","sources":output_base,"value":fixture}));
        }
        for example in &seed.outputs {
            outputs.push(json!({"id":example.id,"schema":"command-response","origin":example.origin,"sources":example.sources,"value":example.value}));
        }
        document["output"] = json!(outputs);
    }
    if !seed.output_negatives.is_empty() {
        document["output_negative"] = seed
            .output_negatives
            .iter()
            .map(|negative| json!({"id":negative.id,"schema":"command-response","origin":"regression","sources":output_base,"value":negative.value,"expected_pointer":negative.pointer,"expected_keyword":negative.keyword}))
            .collect();
    }
    document
}

/// Exit-0 stdout for the generated plain-text case of each event.
fn plain_text_bytes(seed: &Seed) -> &'static [u8] {
    if seed.text_context {
        b"Hook-provided context."
    } else if seed.compaction_instructions {
        b"Preserve the failing test names.\n"
    } else {
        b"Hook finished.\n"
    }
}

fn process_cases(seed: &Seed) -> Vec<Value> {
    let structured = serde_json::to_vec(&seed.structured).unwrap();
    let text_outcome = text_outcome_id(seed);
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
        if seed.text_context {
            process.push(case(
                "command-text",
                "command",
                "text-context",
                0,
                plain_text_bytes(seed),
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
            process.push(case(
                "command-text-json-array",
                "command",
                "text-context",
                0,
                b"[\"a\",\"b\"]",
                b"",
            ));
            process.push(case(
                "command-text-json-string",
                "command",
                "text-context",
                0,
                b"\"quoted\"",
                b"",
            ));
        } else {
            let id = if seed.compaction_instructions {
                "command-compaction-instructions"
            } else {
                "command-plain-text"
            };
            process.push(case(
                id,
                "command",
                text_outcome,
                0,
                plain_text_bytes(seed),
                b"",
            ));
            if seed.json == Json::Control {
                process.push(case(
                    "command-plain-text-json-lines",
                    "command",
                    text_outcome,
                    0,
                    b"{\"note\":1}\n{\"note\":2}\n",
                    b"",
                ));
                process.push(case(
                    "command-text-json-array",
                    "command",
                    text_outcome,
                    0,
                    b"[\"a\",\"b\"]",
                    b"",
                ));
            }
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
            process.push(case(
                "command-json-lines-with-field",
                "command",
                "invalid-json",
                0,
                b"{\"note\":1}\n{\"systemMessage\":\"hi\"}\n",
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
            if let Some(stdout) = &json_stdout {
                process.push(case(
                    "command-exit-2-structured",
                    "command",
                    "exit-2-structured",
                    2,
                    stdout,
                    stderr,
                ));
            }
            if seed.exit2_missing_file {
                process.push(case(
                    "command-exit-2-missing-file",
                    "command",
                    "exit-2-missing-file",
                    2,
                    b"",
                    b"/bin/sh: /repo/.claude/hooks/check.sh: No such file or directory\n",
                ));
                process.push(case(
                    "command-exit-2-missing-file-python",
                    "command",
                    "exit-2-missing-file",
                    2,
                    b"",
                    b"python3: can't open file '/repo/x.py': [Errno 2] No such file or directory",
                ));
                // Non-empty stdout disables the missing-file rule.
                process.push(case(
                    "command-exit-2-missing-file-with-stdout",
                    "command",
                    "exit-2-structured",
                    2,
                    json_stdout.as_deref().expect("exit-2 JSON fixture"),
                    b"/bin/sh: /repo/.claude/hooks/check.sh: No such file or directory\n",
                ));
                process.push(case(
                    "command-exit-2-other-stderr",
                    "command",
                    "exit-2",
                    2,
                    b"",
                    b"tests failed",
                ));
            }
            if seed.exit2_continue_false {
                process.push(case(
                    "command-exit-2-continue-false",
                    "command",
                    "exit-2-continue-false",
                    2,
                    br#"{"continue":false,"stopReason":"Done."}"#,
                    b"keep going",
                ));
            }
        }
        match seed.nonzero {
            Nonzero::Split { .. } | Nonzero::JsonIgnored { .. } => {
                let structured_outcome = if matches!(seed.nonzero, Nonzero::JsonIgnored { .. }) {
                    "nonzero-json-ignored"
                } else {
                    "nonzero-structured"
                };
                process.push(case(
                    "command-nonzero-structured",
                    "command",
                    structured_outcome,
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
    if let OnFailure::Blocks { .. } = seed.on_failure {
        let binding = "command-on-failure-block";
        let exit2 = seed.exit2.as_ref().expect("gating exit-2 row");
        let stderr: &[u8] = if exit2.blocks {
            b"blocked by hook"
        } else {
            b"hook failed"
        };
        process.push(case(
            "command-on-failure-block-structured",
            binding,
            "structured",
            0,
            &structured,
            b"",
        ));
        process.push(case(
            "command-on-failure-block-no-op",
            binding,
            "no-op",
            0,
            b"",
            b"",
        ));
        process.push(case(
            &format!("command-on-failure-block-{text_outcome}"),
            binding,
            text_outcome,
            0,
            plain_text_bytes(seed),
            b"",
        ));
        process.push(case(
            "command-on-failure-block-invalid-json",
            binding,
            "invalid-json",
            0,
            b"{not json}",
            b"",
        ));
        process.push(case(
            "command-on-failure-block-schema-invalid-json",
            binding,
            "invalid-json",
            0,
            b"{\"continue\":\"yes\"}",
            b"",
        ));
        process.push(case(
            "command-on-failure-block-exit-2",
            binding,
            "exit-2",
            2,
            b"",
            stderr,
        ));
        process.push(case(
            "command-on-failure-block-nonzero-structured",
            binding,
            "failure-blocked",
            1,
            &structured,
            b"",
        ));
        process.push(case(
            "command-on-failure-block-nonzero-unstructured",
            binding,
            "failure-blocked",
            1,
            b"",
            b"hook failed",
        ));
    }
    for extra in &seed.process {
        process.push(case(
            extra.id,
            extra.binding,
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
            let mut bindings = vec!["http"];
            if matches!(seed.on_failure, OnFailure::Blocks { .. }) {
                bindings.push("http-on-failure-block");
            }
            for binding in bindings {
                let id = |suffix: &str| format!("{binding}-{suffix}");
                process.push(case(
                    &id("structured"),
                    binding,
                    "structured",
                    200,
                    &structured,
                    b"",
                ));
                process.push(case(&id("empty"), binding, "empty-body", 204, b"", b""));
                process.push(case(
                    &id("non-json"),
                    binding,
                    "non-json-body",
                    200,
                    b"plain text body",
                    b"",
                ));
                process.push(case(
                    &id("schema-invalid"),
                    binding,
                    "schema-invalid-body",
                    200,
                    b"{\"continue\":\"yes\"}",
                    b"",
                ));
                process.push(case(&id("error"), binding, "non-success", 500, b"", b""));
            }
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
