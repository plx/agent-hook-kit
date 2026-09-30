# Hookkit Common Model v0
## Semantic Common Views And Capability-Aware Lowering

Version: draft 0.1

## 1. Purpose

This spec defines the missing common model layer for `agent-hook-kit`.

The current architecture has the correct high-level shape:

1. a CLI flag selects the harness format (`--claude`, `--codex`, `--gemini`);
2. native JSON is parsed into harness-specific Rust types;
3. native types are wrapped in `hookkit-common` enums;
4. hook logic receives `CommonHookInput`;
5. hook logic returns `CommonHookOutput`;
6. the runtime lowers common output into harness-native output and emits stdout, stderr, and exit codes.

The gap is that "common" is currently mostly a tagged native wrapper plus a few shared accessors. This is lossless, but not semantic enough for reusable hook tools. The `ruff-agent-hook` prototype exposed two concrete weaknesses:

- input logic still had to inspect raw JSON to find post-tool result paths because `CommonPostToolUseInput` did not expose a normalized tool-result view;
- output logic still had to branch on `Harness::Codex` because unsupported post-tool agent feedback is handled by hook-specific code rather than centralized lowering policy.

This spec adds a second common layer:

1. **Lossless native wrapper**: preserve full harness-native input/output escape hatches.
2. **Normalized semantic view**: expose shared concepts that hook authors actually need.
3. **Capability-aware lowering**: let hook authors return semantic output intents and let the library choose the correct harness strategy or a documented fallback.

## 2. Goals

- Keep native harness models as the source of truth.
- Keep common wrappers lossless.
- Add event-specific semantic views over common wrappers.
- Make common hook logic practical without repeatedly matching on every harness.
- Preserve raw payload access for unmodeled fields and future harness changes.
- Centralize output lowering and unsupported-capability behavior.
- Make message audience explicit: user-only, agent-only, or both.
- Provide an audit method for identifying similar gaps in every common/shared hook type.

## 3. Non-Goals

- Do not replace native harness modules with one universal struct.
- Do not pretend every harness supports every semantic intent.
- Do not silently drop important output unless the caller explicitly selects a best-effort policy.
- Do not parse arbitrary shell commands into exact modified-file sets in v0.
- Do not require every harness-specific edge case to be modeled before adding useful semantic helpers.

## 4. Layering Model

The library should expose three layers.

### 4.1 Native Layer

Native crates remain authoritative:

- `hookkit-claude`
- `hookkit-codex`
- `hookkit-gemini`

Native event structs model harness payloads as accurately as possible and preserve unknown fields with `#[serde(flatten)]` or raw payloads.

### 4.2 Common Wrapper Layer

Common wrapper enums preserve native detail:

```rust
pub enum CommonPostToolUseInput {
    Claude(claude::PostToolUse),
    Codex(codex::PostToolUse),
    Gemini(gemini::AfterTool),
}
```

This layer is lossless and provides escape hatches:

```rust
impl CommonPostToolUseInput {
    pub fn as_claude(&self) -> Option<&claude::PostToolUse>;
    pub fn as_codex(&self) -> Option<&codex::PostToolUse>;
    pub fn as_gemini(&self) -> Option<&gemini::AfterTool>;
}
```

### 4.3 Semantic View Layer

Each common wrapper may expose a normalized view:

```rust
impl CommonPostToolUseInput {
    pub fn view(&self) -> CommonPostToolUseView<'_>;
}
```

Views are allowed to borrow from native values and raw JSON. They should avoid unnecessary cloning.

The view is not required to be perfectly complete. It must clearly mark derived values as exact, likely, or unknown where appropriate.

## 5. Common Input Contract

### 5.1 Shared Metadata

All semantic event views should expose event metadata:

```rust
pub struct CommonEventMeta<'a> {
    pub harness: Harness,
    pub event: HookEventKey,
    pub session_id: Option<&'a str>,
    pub cwd: Option<&'a str>,
    pub turn_id: Option<&'a str>,
    pub tool_use_id: Option<&'a str>,
    pub raw_input: &'a serde_json::Value,
}
```

Rules:

- `session_id` and `cwd` may be optional in the semantic view even if current native structs require them, because forward-compatible unknown events may not have them.
- `raw_input` should be available either through the view or through `RuntimeContext`.
- `tool_use_id` should normalize `toolUseId`, `tool_use_id`, and harness-specific equivalents.

### 5.2 Raw And Native Escape Hatches

Every common wrapper should keep:

- native enum variants;
- raw JSON access through `RuntimeContext`;
- `as_harness()` helpers for native-specific handling.

Semantic helpers should reduce ordinary harness branching, not forbid it.

### 5.3 Derived Values

Semantic views may derive values from:

- typed native fields;
- raw JSON fields;
- known tool input/output shapes;
- generic conservative heuristics.

Derived values must carry source and confidence when correctness matters.

```rust
pub enum DerivedConfidence {
    Exact,
    Likely,
    Unknown,
}

pub enum DerivedSource {
    NativeField(&'static str),
    ToolInputField(&'static str),
    ToolResultField(&'static str),
    RawField(String),
    Heuristic,
}
```

## 6. PostToolUse Input View

`PostToolUse` is the first worked example because formatter/linter hooks need it.

### 6.1 Type Shape

```rust
pub struct CommonPostToolUseView<'a> {
    pub meta: CommonEventMeta<'a>,
    pub tool: CommonToolUseView<'a>,
    pub result: CommonToolResultView<'a>,
    pub path_candidates: Vec<PathCandidate<'a>>,
    pub native: CommonPostToolUseNative<'a>,
}

pub enum CommonPostToolUseNative<'a> {
    Claude(&'a claude::PostToolUse),
    Codex(&'a codex::PostToolUse),
    Gemini(&'a gemini::AfterTool),
}
```

### 6.2 Tool View

```rust
pub struct CommonToolUseView<'a> {
    pub name: Option<&'a str>,
    pub input: Option<&'a serde_json::Value>,
}
```

This directly normalizes:

- Claude `tool_name` / `toolName`, `tool_input` / `toolInput`;
- Codex `toolName`, `toolInput`;
- Gemini `toolName`, `toolInput`.

### 6.3 Tool Result View

```rust
pub struct CommonToolResultView<'a> {
    pub value: Option<&'a serde_json::Value>,
    pub status: ToolExecutionStatus,
    pub stdout: Option<&'a str>,
    pub stderr: Option<&'a str>,
    pub exit_code: Option<i64>,
}

pub enum ToolExecutionStatus {
    Success,
    Failure,
    Unknown,
}
```

Normalization should cover common native names:

- Claude `tool_response` / `toolResponse`;
- Codex `tool_result` / `toolResult`;
- Gemini `tool_response` / `toolResponse`.

`status` rules:

- explicit success booleans map to `Success` or `Failure`;
- `exitCode == 0` maps to `Success`;
- nonzero `exitCode` maps to `Failure`;
- missing fields map to `Unknown`.

### 6.4 Path Candidates

Formatter/linter hooks need candidate modified paths, but harnesses and tools do not expose this uniformly.

```rust
pub struct PathCandidate<'a> {
    pub path: &'a str,
    pub absolute_path: PathBuf,
    pub project_relative_path: Option<PathBuf>,
    pub role: PathRole,
    pub source: DerivedSource,
    pub confidence: DerivedConfidence,
}

pub enum PathRole {
    ModifiedFile,
    ReadFile,
    Directory,
    Unknown,
}
```

V0 should implement conservative extraction:

- exact candidates from known file-edit fields:
  - `file_path`
  - `filePath`
  - `path`, only for known file-writing tools or tool results that identify a file;
  - `target_file`
  - `targetFile`
  - `absolute_path`
  - `absolutePath`
- candidate source should distinguish tool input from tool result;
- relative paths should resolve from event `cwd`;
- project-relative paths should be computed from a caller-provided project root when available;
- duplicates should be removed after path normalization.

V0 should avoid claiming shell command parsing is exact. For shell/Bash tools, path extraction from raw command text is out of scope unless a later helper explicitly models it as heuristic.

### 6.5 Accessor API

Expose simple accessors for common cases:

```rust
impl CommonPostToolUseInput {
    pub fn tool_result(&self) -> Option<&serde_json::Value>;
    pub fn execution_status(&self) -> ToolExecutionStatus;
    pub fn path_candidates(&self, cwd: &Path, project_root: Option<&Path>) -> Vec<PathCandidate<'_>>;
    pub fn modified_files(&self, cwd: &Path, project_root: Option<&Path>) -> Vec<PathCandidate<'_>>;
}
```

The full `view()` method should use these helpers internally.

## 7. Common Output Contract

Common output should express semantic intent first.

### 7.1 Message Audience

```rust
pub enum MessageAudience {
    User,
    Agent,
    Both,
}

pub struct UserNotice {
    pub text: String,
    pub level: NoticeLevel,
}

pub struct AgentFeedback {
    pub text: String,
    pub severity: FeedbackSeverity,
}
```

Rules:

- user-only details should not automatically enter model context;
- agent feedback should be concise by default;
- detailed diagnostics should be represented as artifacts or user notices, not blindly injected into agent context.

### 7.2 Artifacts And Diagnostics

Manual-fix workflows need first-class diagnostics:

```rust
pub struct DiagnosticArtifact {
    pub absolute_path: PathBuf,
    pub project_relative_path: Option<PathBuf>,
    pub media_type: String,
    pub summary: Option<String>,
}

pub struct DiagnosticReport {
    pub title: String,
    pub text: String,
    pub artifact: Option<DiagnosticArtifact>,
}
```

The artifact manager can remain in `hookkit-runtime`, but common outputs should be able to refer to artifacts once created.

### 7.3 PostToolUse Output Intent

Current `CommonPostToolUseOutput` should evolve toward:

```rust
pub struct CommonPostToolUseOutput {
    pub notices: Vec<UserNotice>,
    pub agent_feedback: Vec<AgentFeedback>,
    pub diagnostics: Vec<DiagnosticReport>,
    pub replace_tool_result: Option<serde_json::Value>,
    pub tail_tool_call: Option<TailToolCall>,
    pub session_control: Option<SessionControl>,
    pub lowering: LoweringPolicy,
}
```

Builder methods should remain ergonomic:

```rust
CommonPostToolUseOutput::new()
    .with_user_notice(UserNotice::info("ruff: file is clean"))
    .with_agent_feedback("ruff: autofixed formatting")
    .with_diagnostic_artifact(artifact);
```

### 7.4 Lowering Policy

Unsupported output behavior should be centralized:

```rust
pub enum LoweringPolicy {
    Strict,
    BestEffort,
    BestEffortWithWarnings,
}
```

Default should be `Strict` for library APIs and `BestEffortWithWarnings` may be appropriate for CLI templates/examples.

Rules:

- `Strict`: unsupported intents return `UnsupportedCapability`.
- `BestEffort`: unsupported non-critical intents are dropped or redirected according to the capability matrix.
- `BestEffortWithWarnings`: same as best-effort, but emits user-visible warnings to stderr.

Individual intents should declare whether dropping is allowed:

```rust
pub enum IntentRequirement {
    Required,
    Optional,
}
```

For example:

- blocking a dangerous tool is required;
- replacing a tool result is required;
- user-only clean status may fall back to stderr;
- agent feedback after autofix may be optional for a harness that cannot receive post-tool context.

## 8. Capability Matrix

The lowering layer should maintain an explicit per-harness/event matrix.

### 8.1 PostToolUse v0 Capabilities

| Intent | Claude PostToolUse | Codex PostToolUse | Gemini AfterTool |
| --- | --- | --- | --- |
| Empty success | supported | supported | supported |
| Agent feedback/additional context | supported | unsupported | supported |
| User notice structured channel | unsupported | unsupported | unsupported |
| User notice stderr fallback | supported by runtime policy | supported by runtime policy | supported by runtime policy |
| Replace tool result | unsupported | unsupported | supported |
| Tail tool call | unsupported | unsupported | unsupported |
| Continue/stop control | partially supported by Claude envelope | unsupported | unsupported |

The important change is not that all harnesses gain support. The change is that the lowering layer owns this table.

> **Update (2026-09-30):** the stderr fallback in this table never reached
> Claude Code or Codex users. Neither harness shows stderr from a hook that
> exits 0: Claude Code writes it only to its debug log, and Codex discards it.
> Both harnesses' `PostToolUse` responses carry a top-level `systemMessage`
> that is shown to the user, so the "User notice structured channel" row is
> now **supported** (`systemMessage`) for Claude and Codex.
> `post-tool-use-agent-hook` and `examples/shared-posttool-autofix` send user
> notices there, and Codex `PostToolUse` agent feedback is also supported
> through `hookSpecificOutput.additionalContext`. The table above is kept as
> the original design record; see `crates/hookkit-tool-runner/RUNNER_DESIGN.md`
> for the current lowering.

### 8.2 Lowering Result

Lowering should return more than just native output:

```rust
pub struct LoweredOutput {
    pub native: NativeHookOutput,
    pub stderr_messages: Vec<String>,
    pub warnings: Vec<LoweringWarning>,
}

pub struct LoweringWarning {
    pub harness: Harness,
    pub event: HookEventKey,
    pub intent: &'static str,
    pub action: LoweringAction,
}

pub enum LoweringAction {
    Dropped,
    RedirectedToStderr,
    Converted,
}
```

The runtime can then emit warnings/messages without every hook doing `eprintln!`.

## 9. Worked Example: Ruff Post-Tool Hook

### 9.1 Desired Hook Logic

The hook core should not need to branch on harness for normal behavior:

```rust
fn handle_post_tool(input: CommonPostToolUseInput, ctx: &RuntimeContext) -> CommonHookOutput {
    let view = input.view();
    let paths = view.modified_files(ctx.cwd(), Some(project_root));

    if paths.is_empty() {
        return CommonHookOutput::empty();
    }

    let outcome = run_ruff(paths);

    match outcome {
        Clean { summary } => CommonPostToolUseOutput::new()
            .with_user_notice(UserNotice::info(summary))
            .into(),
        AutoFixed { agent, user } => CommonPostToolUseOutput::new()
            .with_user_notice(UserNotice::info(user))
            .with_agent_feedback(agent)
            .with_lowering_policy(LoweringPolicy::BestEffortWithWarnings)
            .into(),
        Manual { user, agent, artifact } => CommonPostToolUseOutput::new()
            .with_user_notice(UserNotice::warning(user))
            .with_diagnostic_artifact(artifact)
            .with_agent_feedback(agent)
            .with_lowering_policy(LoweringPolicy::BestEffortWithWarnings)
            .into(),
    }
}
```

### 9.2 Expected Lowering

Claude:

- user notices fall back to stderr (superseded 2026-09-30: user notices now
  lower to the top-level `systemMessage`, because Claude Code writes exit-0
  stderr only to its debug log; see the note under §8.1);
- agent feedback lowers to `hookSpecificOutput.additionalContext`;
- diagnostics artifact appears in stderr/user notice and can be referenced in agent feedback.

Codex:

- user notices fall back to stderr (superseded 2026-09-30: user notices now
  lower to `systemMessage`, because Codex discards exit-0 stderr);
- agent feedback is unsupported for `PostToolUse` (superseded: Codex
  `PostToolUse` now accepts `hookSpecificOutput.additionalContext`);
- under `BestEffortWithWarnings`, agent feedback is dropped with a warning;
- under `Strict`, lowering fails.

Gemini:

- user notices fall back to stderr;
- agent feedback lowers to `hookSpecificOutput.additionalContext`;
- replace-tool-result remains available when no additional context is emitted.

## 10. Audit Template For Other Common Hook Types

Every common hook type should be reviewed with the same checklist.

### 10.1 Input Audit

For each common input wrapper:

- Which native event variants map into it?
- Which fields are common across all harnesses?
- Which fields are common across two harnesses but missing in the third?
- Which fields are currently only accessible through raw/native escape hatches?
- Which values should be semantic views rather than direct fields?
- Which derived values need confidence/source metadata?
- Are unknown fields preserved?
- Does the view expose enough information for a realistic cross-harness hook?

### 10.2 Output Audit

For each common output type:

- What semantic intents can the type express?
- Which native outputs can represent each intent?
- Which intents currently fail during lowering?
- Which failures should remain strict?
- Which failures should have stderr or no-op fallback?
- Which output messages are user-only, agent-only, or both?
- Can a hook author request best-effort behavior without matching on `Harness`?
- Are diagnostics/artifacts first-class or ad hoc strings?

### 10.3 Test Audit

For each semantic helper and lowering behavior:

- native fixture parses into common wrapper;
- common wrapper produces semantic view;
- view exposes raw/native escape hatches;
- each supported output intent lowers to stable native JSON;
- unsupported required intents fail under `Strict`;
- unsupported optional intents follow fallback policy under `BestEffortWithWarnings`;
- stderr/user messages do not contaminate stdout JSON.

## 11. Likely Gaps By Event Type

### 11.1 PostToolUse

Known gaps:

- missing normalized `tool_result`;
- missing normalized execution status;
- missing candidate changed-file extraction;
- user notices are modeled but not lowerable;
- Codex unsupported post-tool agent context forces hook-level branching;
- diagnostics/artifacts are ad hoc.

Priority: highest. This is the primary formatter/linter hook path.

### 11.2 PreToolUse

Likely gaps:

- rewrite/deny/allow capability differences need centralized policy;
- Codex allow is parsed but not supported;
- tool input rewrite is not supported equally;
- user-facing denial details versus agent-facing denial reason need clearer audience modeling.

Priority: high. This is the primary policy/guard hook path.

### 11.3 PromptSubmit

Likely gaps:

- Claude supports richer context/title/system-message behavior than the common output exposes;
- prompt expansion and prompt submit may need distinct semantic models;
- user-visible notice behavior is not consistently modeled.

Priority: medium.

### 11.4 Stop

Likely gaps:

- continue/retry/allow semantics are mostly aligned but should be expressed as session-control intents;
- user notice versus agent retry reason needs explicit separation;
- Codex stop requires JSON on success, which should remain a lowering concern.

Priority: medium.

### 11.5 Notification And Session Events

Likely gaps:

- common `user_notice` exists for some events but lowering often rejects it;
- if no harness supports structured notice emission, stderr fallback should be explicit;
- session-start context injection is Claude-only today and should be represented as capability-gated intent.

Priority: low to medium.

### 11.6 Gemini Model-Layer Hooks

Likely gaps:

- `BeforeModel`, `AfterModel`, and `BeforeToolSelection` are currently native-oriented;
- common semantic views should wait until there is a concrete cross-harness use case, since other harnesses may not have equivalents.

Priority: low for v0.

## 12. Migration Plan

### Phase 1: PostToolUse Semantic View

- Add `CommonEventMeta`.
- Add `CommonToolUseView`.
- Add `CommonToolResultView`.
- Add `PathCandidate`.
- Implement `CommonPostToolUseInput::view()`.
- Replace raw JSON path extraction in `ruff-agent-hook` with semantic accessors.

### Phase 2: PostToolUse Output Lowering

- Extend `CommonPostToolUseOutput` with notices, diagnostics, and lowering policy.
- Add `LoweredOutput`.
- Move stderr fallback and unsupported optional-agent-feedback behavior into runtime/common lowering.
- Remove `Harness::Codex` branching from `ruff-agent-hook` output logic.

### Phase 3: Capability Matrix

- Introduce an explicit capability table for each common output event.
- Add strict/best-effort tests for each post-tool intent.
- Document the matrix in code comments and README/developer docs.

### Phase 4: Audit Other Events

- Apply the audit template to `PreToolUse`, `PromptSubmit`, `Stop`, and session/notification events.
- Add semantic views only where they are driven by realistic hook examples.
- Avoid generalizing model-layer hooks prematurely.

## 13. Acceptance Criteria

The v0 common model is complete enough when:

- a formatter/linter post-tool hook can identify modified file candidates without inspecting raw JSON directly;
- the hook can return user notices, agent feedback, and diagnostic artifact references without matching on harness;
- Claude and Gemini receive agent feedback through their native additional-context mechanisms;
- Codex post-tool limitations are handled by lowering policy rather than hook-specific branches;
- unsupported required intents still fail loudly;
- unsupported optional intents have documented fallback behavior;
- tests cover strict and best-effort lowering for all `PostToolUse` output intents.

## 14. Open Questions

- Should `LoweringPolicy` live on every common output, in `RuntimeContext`, or in a runtime configuration object?
- Should `UserNotice` fallback to stderr be the default for CLI hooks, or should callers opt into it?
- Should path extraction be part of `hookkit-common`, or should there be a separate `hookkit-common::paths` helper module?
- Should diagnostics artifacts be created by `hookkit-runtime` before output construction, or should common output support lazy artifact creation?
- Should common views borrow raw JSON only, or own normalized data to simplify lifetime-heavy hook code?
