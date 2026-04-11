
# Hookkit: Rust Hook Harness Library / Starter Kit
## Implementation-Oriented Specification

Version: draft 0.1  
Working name: `hookkit`

## 1. Purpose

`hookkit` is a Rust workspace for building robust hook executables for coding-agent harnesses such as Claude Code, Codex, and Gemini CLI.

The core problem it solves is not “run a shell script when a hook fires.” The core problem is:

1. parse harness-specific JSON input into strongly typed Rust values,
2. preserve enough native detail to emit harness-correct responses,
3. offer cross-harness wrapper enums for aligned lifecycle events,
4. provide reusable runtime plumbing for stdin/stdout/exit-code behavior, and
5. make higher-level hook tools practical to write repeatedly without redoing the same shell and JSON glue.

The intended user workflow is:

- write a Rust executable or library crate for a hook,
- select a harness at invocation time (`--claude`, `--codex`, `--gemini`),
- parse stdin JSON into a native or common event value,
- run hook logic against typed input,
- emit the correct JSON and/or exit code for the target harness.

## 2. Product goals

### 2.1 Required goals

- **Harness-aware input parsing**
  - Caller specifies the harness.
  - Event type is inferred from the payload, not from a second CLI flag.
- **Native types first**
  - Every supported hook event gets a native input/output type in its harness module.
- **Cross-harness wrappers**
  - Semantically aligned events get common wrapper enums.
- **Emission correctness**
  - Library handles stdout JSON, stderr, and exit code semantics correctly.
- **Message audience separation**
  - The library must make it easy to keep user-visible messages distinct from model-visible context/feedback.
- **Forward compatibility**
  - Unknown fields and unsupported event variants must be preserved rather than destroyed.
- **CLI-plumbing reuse**
  - Building a new hook tool should mostly mean implementing the hook’s business logic.

### 2.2 Stretch goals

- **Reusable clap-based templates**
  - Support rapidly building many hook executables with shared parsing/emission behavior.
- **Artifact helpers**
  - Provide utilities for temp files, spill-to-file diagnostics, turn/session scoping, and metadata handoff between related hook passes.
- **Policy helpers**
  - Small reusable building blocks for common workflows like format/lint/autofix/report.

## 3. Non-goals

Version 1 should explicitly avoid these:

- parsing entire harness configuration files (`settings.json`, `hooks.json`, `config.toml`) beyond what is needed for examples,
- hiding all harness differences behind a single lossy “universal hook” struct,
- inventing capabilities that the harness does not support,
- silently downgrading unsupported outputs,
- runtime plugin loading for arbitrary hook implementations,
- trying to own Claude prompt hooks, Claude agent hooks, or Gemini internal orchestration semantics beyond typed payload modeling where needed.

## 4. Design principles

### 4.1 Native-first, not normalized-first

The docs show that the three harnesses overlap, but not cleanly:

- Claude exposes the broadest hook surface, including tool, session, permission, task, subagent, notification, config, worktree, compact, and elicitation hooks.
- Codex currently exposes a narrower, experimental surface, with current `PreToolUse` and `PostToolUse` behavior centered on `Bash`.
- Gemini overlaps on session/prompt/tool/stop flows, but also has model-layer hooks like `BeforeModel`, `AfterModel`, and `BeforeToolSelection`. 

Therefore the library should be built in layers:

1. **native harness layer**
2. **common wrapper layer**
3. **optional semantic helper layer**

The native layer is the source of truth.

### 4.2 Capability-aware conversions

When converting from a common output type to a harness-native output type, unsupported behavior must produce an explicit error such as:

- `UnsupportedCapability`
- `UnsupportedForEvent`
- `UnsupportedForHarnessVersion`

The library should never “pretend” a capability succeeded when the target harness only parses-but-ignores it.

### 4.3 Preserve raw payloads

Every parsed input event should retain the original JSON payload, or at minimum preserve unknown fields with `#[serde(flatten)]`, so that:

- forward-compatible parsing is possible,
- callers can inspect fields not yet modeled natively,
- round-trip and compatibility testing are easier.

### 4.4 Prefer JSON emission over plain stdout

Even where a harness allows plain stdout as context, the runtime should default to JSON output when structured output is intended. Plain stdout should be an explicit opt-in fallback.

### 4.5 Make message routing explicit

The library should help authors answer:

- is this for the **user only**,
- for the **agent/model only**,
- or for **both** via different channels?

This is critical for workflows like:
- quiet success,
- user-visible summaries,
- agent-visible corrective guidance,
- user-visible detailed diagnostics that do not bloat context.

## 5. Workspace layout

Recommended workspace:

```text
hookkit/
  Cargo.toml
  crates/
    hookkit-core/
    hookkit-runtime/
    hookkit-claude/
    hookkit-codex/
    hookkit-gemini/
    hookkit-common/
    hookkit-templates/      # stretch goal
  examples/
    claude-posttool-lint/
    codex-pretool-bash-policy/
    gemini-beforetool-guard/
    shared-posttool-autofix/
  fixtures/
    claude/
    codex/
    gemini/
```

### 5.1 Crate roles

#### `hookkit-core`
Shared foundational types:

- `Harness`
- `HookEventKey`
- errors
- JSON helpers
- raw payload wrapper
- capability types
- message intent types
- stable utility traits

#### `hookkit-runtime`
I/O and execution plumbing:

- read stdin
- parse native/common event
- write stdout JSON
- map structured results to exit code / stderr
- convenience `main()` helpers

#### `hookkit-claude`
Claude native input/output types and conversions.

#### `hookkit-codex`
Codex native input/output types and conversions.

#### `hookkit-gemini`
Gemini native input/output types and conversions.

#### `hookkit-common`
Cross-harness wrapper enums and optional semantic outputs.

#### `hookkit-templates`
Optional clap/build-time templates for reusable executables.

## 6. Core type system

## 6.1 Harness identity

```rust
pub enum Harness {
    Claude,
    Codex,
    Gemini,
}
```

## 6.2 Event key

```rust
pub enum HookEventKey {
    SessionStart,
    SessionEnd,
    PromptSubmit,
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    Stop,
    Notification,
    PermissionRequest,
    PermissionDenied,
    BeforeModel,
    AfterModel,
    BeforeToolSelection,
    PreCompress,
    Other(&'static str),
}
```

This enum is only for internal categorization. Native event enums remain more precise.

## 6.3 Raw JSON preservation

Every top-level parsed event should contain:

```rust
pub struct RawPayload(pub serde_json::Value);
```

and each native event struct should also preserve unknown top-level fields:

```rust
#[derive(Deserialize, Serialize)]
pub struct CommonTopLevel {
    pub session_id: String,
    pub transcript_path: Option<Utf8PathBuf>,
    pub cwd: Utf8PathBuf,
    pub hook_event_name: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}
```

## 6.4 Native input enum shape

Each harness crate should expose a top-level input enum:

```rust
pub enum ClaudeHookInput {
    SessionStart(claude::SessionStart),
    UserPromptSubmit(claude::UserPromptSubmit),
    PreToolUse(claude::PreToolUse),
    PostToolUse(claude::PostToolUse),
    PostToolUseFailure(claude::PostToolUseFailure),
    PermissionRequest(claude::PermissionRequest),
    PermissionDenied(claude::PermissionDenied),
    Stop(claude::Stop),
    StopFailure(claude::StopFailure),
    Notification(claude::Notification),
    // ...
    Unknown { event_name: String, raw: RawPayload },
}
```

Equivalent top-level enums exist for Codex and Gemini.

## 6.5 Native output enum shape

Each harness crate should expose:

```rust
pub enum ClaudeHookOutput {
    Empty,
    Json(claude::OutputEnvelope),
    BlockingError { stderr: String },
    Warning { stderr: String, exit_code: i32 },
}
```

Equivalent enums exist for Codex and Gemini.

The runtime converts these to actual process behavior.

## 7. Harness parsing contract

## 7.1 Input contract

Parsing must require a **harness hint** but must **not** require an event hint.

```rust
pub fn parse_stdin_native(harness: Harness) -> Result<NativeHookInput, ParseError>;
```

Implementation steps:

1. read stdin as bytes,
2. parse as `serde_json::Value`,
3. read `hook_event_name`,
4. dispatch to the harness-specific event parser,
5. if event name is recognized but strict typed parsing fails, return a typed parse error,
6. if event name is unknown, return `Unknown { event_name, raw }`.

## 7.2 Why event inference is safe

All three harnesses include a top-level hook event name in the payload:

- Claude includes `hook_event_name` in common input fields.
- Codex includes `hook_event_name` in common input fields.
- Gemini includes `hook_event_name` in its base input schema.

So once the harness is known, event inference is straightforward and should be the default behavior.

## 8. Native harness modules

## 8.1 Claude native surface

Claude should be modeled as the most feature-rich backend.

### 8.1.1 Phase-1 Claude events
These should be first-class in version 1:

- `SessionStart`
- `UserPromptSubmit`
- `PreToolUse`
- `PostToolUse`
- `PostToolUseFailure`
- `PermissionDenied`
- `Stop`
- `Notification`
- `SessionEnd`

### 8.1.2 Phase-2 Claude events
Add next:

- `PermissionRequest`
- `SubagentStart`
- `SubagentStop`
- `TaskCreated`
- `TaskCompleted`
- `TeammateIdle`
- `ConfigChange`
- `CwdChanged`
- `FileChanged`
- `PreCompact`
- `PostCompact`

### 8.1.3 Phase-3 Claude events
Add later:

- `InstructionsLoaded`
- `WorktreeCreate`
- `WorktreeRemove`
- `Elicitation`
- `ElicitationResult`
- `StopFailure`

### 8.1.4 Claude output modeling rules

Claude has multiple output idioms, so native types must reflect that precisely:

- top-level universal fields like `continue`, `stopReason`, `systemMessage`, `suppressOutput`,
- top-level `decision`/`reason` for events like `UserPromptSubmit`, `PostToolUse`, `Stop`,
- `hookSpecificOutput.permissionDecision` for `PreToolUse`,
- `hookSpecificOutput.retry` for `PermissionDenied`,
- special output models for `WorktreeCreate`, `FileChanged`, and elicitation hooks.

Do **not** compress these into one flat generic struct internally.

## 8.2 Codex native surface

Codex must be treated as a constrained backend.

### 8.2.1 Version-1 Codex events

- `SessionStart`
- `PreToolUse`
- `PostToolUse`
- `UserPromptSubmit`
- `Stop`

### 8.2.2 Codex-specific constraints to encode in types

The Codex backend should expose what the docs say is currently supported, not what is merely parsed.

Examples:

- `PreToolUse` can deny, but “allow”, “ask”, `updatedInput`, and `additionalContext` are not currently supported and fail open.
- `PreToolUse` / `PostToolUse` currently center on `Bash`.
- `Stop` requires JSON on stdout when exiting 0.

The output builders should therefore make the supported path ergonomic and the unsupported path explicit.

### 8.2.3 Codex version gating

Add an internal `CodexFeatureSet` that can evolve over time.

```rust
pub struct CodexFeatureSet {
    pub bash_only_tool_hooks: bool,
    pub pretool_allow_supported: bool,
    pub updated_input_supported: bool,
    // ...
}
```

Default to the current documented behavior.

## 8.3 Gemini native surface

Gemini deserves its own rich native model because it has both agent-loop and model-layer hooks.

### 8.3.1 Version-1 Gemini events

- `SessionStart`
- `SessionEnd`
- `BeforeAgent`
- `AfterAgent`
- `BeforeTool`
- `AfterTool`
- `BeforeModel`
- `AfterModel`
- `BeforeToolSelection`
- `Notification`
- `PreCompress`

### 8.3.2 Gemini stable model types

Gemini exposes stable `LLMRequest` and `LLMResponse` shapes in its hook reference, so those should be modeled as first-class typed structs rather than raw `Value`.

### 8.3.3 Gemini output modeling rules

Gemini native outputs need dedicated types for:

- tool input rewriting in `BeforeTool`,
- tool-result replacement, context append, and tail-call requests in `AfterTool`,
- model request override / synthetic model response in `BeforeModel`,
- response replacement in `AfterModel`,
- tool filtering in `BeforeToolSelection`,
- retry / stop semantics in `AfterAgent`.

These should not be forced through a Claude-shaped abstraction.

## 9. Tool payload strategy

Tool payloads vary a lot across harnesses. The library should use a layered approach.

## 9.1 Phase-1 strategy

For event types that involve tools:

- always type the outer event,
- type the most common/high-value tool payloads,
- preserve a raw fallback for everything else.

Example:

```rust
pub enum ClaudeToolInput {
    Bash(ClaudeBashToolInput),
    Write(ClaudeWriteToolInput),
    Edit(ClaudeEditToolInput),
    AskUserQuestion(ClaudeAskUserQuestionInput),
    ExitPlanMode(ClaudeExitPlanModeInput),
    Unknown { tool_name: String, raw: serde_json::Value },
}
```

Equivalent approach for Gemini and Codex.

## 9.2 Why not fully type every tool immediately

This workspace is for hook executables, not a full reimplementation of every harness tool protocol. Phase 1 should prioritize the tool inputs needed for the likely high-value hooks:

- `Bash` / shell
- `Write`
- `Edit`
- Gemini tool names commonly used in file and command hooks

Everything else can use raw JSON plus convenience accessors until demand justifies more typing.

## 10. Common wrapper layer

The `hookkit-common` crate should not attempt one universal struct. It should provide semantically aligned wrapper enums.

## 10.1 Recommended common wrappers

```rust
pub enum CommonHookInput {
    SessionStart(CommonSessionStartInput),
    PromptSubmit(CommonPromptSubmitInput),
    PreToolUse(CommonPreToolUseInput),
    PostToolUse(CommonPostToolUseInput),
    Stop(CommonStopInput),
    Notification(CommonNotificationInput),
    SessionEnd(CommonSessionEndInput),
    PreCompress(CommonPreCompressInput),
}
```

Each common input is itself an enum over harness-native variants.

### 10.1.1 Example

```rust
pub enum CommonPostToolUseInput {
    Claude(claude::PostToolUse),
    Codex(codex::PostToolUse),
    Gemini(gemini::AfterTool),
}
```

## 10.2 What should map

### Good common mappings

- Claude `SessionStart` / Codex `SessionStart` / Gemini `SessionStart`
- Claude `UserPromptSubmit` / Codex `UserPromptSubmit` / Gemini `BeforeAgent`
- Claude `PreToolUse` / Codex `PreToolUse` / Gemini `BeforeTool`
- Claude `PostToolUse` / Codex `PostToolUse` / Gemini `AfterTool`
- Claude `Stop` / Codex `Stop` / Gemini `AfterAgent`
- Claude `Notification` / Gemini `Notification`
- Claude `SessionEnd` / Gemini `SessionEnd`
- Claude `PreCompact` / Gemini `PreCompress`

### Events that should stay native-only at first

- Claude `PermissionRequest`
- Claude `PermissionDenied`
- Claude `WorktreeCreate`
- Claude `WorktreeRemove`
- Claude `Elicitation`
- Claude `ElicitationResult`
- Claude `FileChanged`
- Claude `ConfigChange`
- Claude `TaskCreated`
- Claude `TaskCompleted`
- Claude `TeammateIdle`
- Gemini `BeforeModel`
- Gemini `AfterModel`
- Gemini `BeforeToolSelection`

## 10.3 Common wrappers must be lossless

The wrapper types must expose:

- shared convenience accessors,
- access to the underlying harness-native value,
- no loss of harness-specific fields.

Example:

```rust
impl CommonPreToolUseInput {
    pub fn tool_name(&self) -> &str { ... }
    pub fn raw_tool_input(&self) -> &serde_json::Value { ... }
    pub fn as_claude(&self) -> Option<&claude::PreToolUse> { ... }
    pub fn as_gemini(&self) -> Option<&gemini::BeforeTool> { ... }
}
```

## 11. Common output layer

The output side should mirror the same pattern:

```rust
pub enum CommonHookOutput {
    SessionStart(CommonSessionStartOutput),
    PromptSubmit(CommonPromptSubmitOutput),
    PreToolUse(CommonPreToolUseOutput),
    PostToolUse(CommonPostToolUseOutput),
    Stop(CommonStopOutput),
    Notification(CommonNotificationOutput),
    SessionEnd(CommonSessionEndOutput),
    PreCompress(CommonPreCompressOutput),
}
```

## 11.1 Common output types should be intent-oriented

Example:

```rust
pub struct CommonPostToolUseOutput {
    pub user_notice: Option<UserNotice>,
    pub agent_context: Vec<String>,
    pub agent_feedback_block: Option<String>,
    pub replace_tool_result: Option<serde_json::Value>,
    pub tail_tool_call: Option<TailToolCall>,
    pub continue_session: Option<bool>,
    pub stop_reason: Option<String>,
}
```

Conversion rules:

- converting to a harness-native output either succeeds,
- or returns a precise unsupported-capability error.

## 11.2 Message helper types

```rust
pub struct UserNotice {
    pub text: String,
    pub level: NoticeLevel,
}

pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

pub struct TailToolCall {
    pub name: String,
    pub args: serde_json::Value,
}
```

Keep these helpers small and purpose-built.

## 12. Runtime contract

## 12.1 Command-hook runtime API

Recommended runtime entry point:

```rust
pub fn run_native<H>(
    harness: Harness,
    handler: impl FnOnce(NativeHookInput, &RuntimeContext) -> Result<NativeHookOutput>
) -> std::process::ExitCode;
```

And:

```rust
pub fn run_common(
    harness: Harness,
    handler: impl FnOnce(CommonHookInput, &RuntimeContext) -> Result<CommonHookOutput>
) -> std::process::ExitCode;
```

## 12.2 Runtime context

```rust
pub struct RuntimeContext {
    pub harness: Harness,
    pub raw_input: serde_json::Value,
    pub stdin_bytes: Vec<u8>,
    pub cwd: Utf8PathBuf,
}
```

## 12.3 Emission rules

The runtime must own:

- `stdout` JSON serialization,
- `stderr` emission,
- exit-code mapping,
- output validation before emission.

Validation examples:

- reject plain stdout for Codex `Stop`,
- reject Claude `PreToolUse` output that uses top-level `decision` instead of `hookSpecificOutput.permissionDecision`,
- reject Gemini `BeforeToolSelection` outputs that try to use unsupported `decision`,
- reject unsupported output combinations before any bytes are printed.

## 13. Error model

Recommended error hierarchy:

```rust
pub enum HookkitError {
    Io(io::Error),
    InvalidJson(serde_json::Error),
    MissingHookEventName,
    UnknownEvent { harness: Harness, event_name: String },
    ParseFailure { harness: Harness, event_name: String, source: serde_json::Error },
    UnsupportedCapability { harness: Harness, event: HookEventKey, capability: &'static str },
    InvalidOutputCombination { harness: Harness, event: HookEventKey, message: String },
}
```

## 14. Output validation rules

The library should validate the following before emission:

- mutually exclusive top-level and hook-specific control paths,
- missing required companion fields (`reason` when blocking, etc.),
- use of unsupported fields for a target event,
- invalid empty JSON objects when a harness requires a specific shape,
- tool rewrite objects that are not JSON objects,
- non-absolute paths where absolute paths are required (`WorktreeCreate`, dynamic watch paths, etc.).

## 15. Artifact and temp-file helpers

This is not part of the minimal parser/emitter core, but it is a high-value adjacent module.

Recommended crate/module:

```text
hookkit-runtime::artifacts
```

Capabilities:

- create temp files scoped to session / turn / tool-use id,
- emit stable human-readable filenames,
- store metadata sidecars,
- optionally serialize diagnostics as JSON and text,
- reopen prior artifacts by key.

Example key type:

```rust
pub struct ArtifactKey {
    pub session_id: String,
    pub turn_id: Option<String>,
    pub tool_use_id: Option<String>,
    pub label: String,
}
```

This directly supports autofix/lint pipelines where:

- pass 1 runs formatter/linter,
- diagnostics are written once,
- pass 2 or a follow-up skill references the saved artifact path instead of rerunning the linter.

## 16. Recommended high-value helper patterns

These are helper modules layered on top of the core library, not part of the strict core API.

## 16.1 Autofix-first post-tool hook helper

Provide a small helper that standardizes:

- success with no noise,
- autofix happened,
- manual issues remain,
- user-visible detailed report,
- model-visible short corrective instruction.

Example result model:

```rust
pub enum AutofixOutcome {
    Clean,
    AutoFixed {
        files_changed: Vec<Utf8PathBuf>,
        summary: String,
    },
    ManualActionRequired {
        summary: String,
        diagnostics_artifact: Utf8PathBuf,
        suggested_followup: String,
    },
}
```

## 16.2 Mid-edit lint profile helper

Optional module for:

- selecting weaker lint configurations for in-progress edits,
- suppressing “unused” style findings when desired,
- choosing formatter/linter by file extension and repository state.

This belongs in a support crate or examples, not the core parser/emitter crate.

## 17. Security and robustness expectations

All example hook executables should follow these rules:

- quote shell arguments if shelling out,
- avoid passing raw JSON through shells unless necessary,
- use direct process spawning over `bash -c` when possible,
- validate file paths,
- cap context size,
- spill verbose diagnostics to artifacts,
- never print non-JSON text to stdout unless the harness/event explicitly relies on plain stdout and the runtime is told to do so.

## 18. Recommended public API sketch

```rust
use hookkit_core::Harness;
use hookkit_common::{CommonHookInput, CommonHookOutput};
use hookkit_runtime::run_common;

fn main() -> std::process::ExitCode {
    run_common(Harness::Claude, |input, ctx| {
        match input {
            CommonHookInput::PostToolUse(ev) => handle_post_tool_use(ev, ctx),
            _ => Ok(CommonHookOutput::empty()),
        }
    })
}

fn handle_post_tool_use(
    ev: hookkit_common::CommonPostToolUseInput,
    _ctx: &hookkit_runtime::RuntimeContext,
) -> hookkit_core::Result<CommonHookOutput> {
    // Business logic here
    Ok(CommonHookOutput::empty())
}
```

## 19. Acceptance criteria

Version 1 is complete when all of the following are true:

1. A caller can choose `Harness::{Claude,Codex,Gemini}` and parse stdin without separately specifying the event name.
2. Native input enums exist for the version-1 event set of each harness.
3. Native output builders exist for the version-1 event set of each harness.
4. Common wrapper enums exist for:
   - session start,
   - prompt submit,
   - pre-tool,
   - post-tool,
   - stop,
   - notification where applicable.
5. The runtime can emit harness-correct stdout/stderr/exit behavior for all version-1 events.
6. Unsupported capability use returns explicit errors.
7. Real fixture JSON is covered by golden tests.
8. At least three example executables compile and pass integration tests:
   - Claude post-tool autofix/lint,
   - Codex pre-tool Bash policy,
   - Gemini before-tool guard.

## 20. Recommendation summary

The recommended architecture is:

- **native harness crates first**,
- **common wrapper enums second**,
- **small semantic helpers third**,
- **runtime-owned emission validation always**.

That structure best matches the documented differences among Claude, Codex, and Gemini while still making reusable hook executables practical to build.
