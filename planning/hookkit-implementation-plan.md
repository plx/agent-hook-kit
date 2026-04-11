
# Hookkit: Implementation Plan
## Shovel-Ready Build Plan

Version: draft 0.1

## 1. Delivery strategy

Build `hookkit` in five phases.

The main tactical decision is to ship a useful command-hook runtime early, not wait for full event coverage across every harness.

## 2. Phase breakdown

## Phase 0 — Source review, fixture capture, crate skeleton

### Objectives

- lock down scope,
- assemble representative JSON fixtures,
- set up workspace structure,
- prove parse-and-emit flow for one event per harness.

### Tasks

1. Create workspace and empty crates:
   - `hookkit-core`
   - `hookkit-runtime`
   - `hookkit-claude`
   - `hookkit-codex`
   - `hookkit-gemini`
   - `hookkit-common`

2. Create `fixtures/` with real JSON samples for:
   - Claude `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`
   - Codex `SessionStart`, `PreToolUse`, `PostToolUse`, `UserPromptSubmit`, `Stop`
   - Gemini `SessionStart`, `BeforeAgent`, `BeforeTool`, `AfterTool`, `AfterAgent`

3. Capture expected outputs for:
   - allow / empty output,
   - block/deny,
   - context injection,
   - stop/continue behavior.

4. Add CI:
   - `cargo fmt --check`
   - `cargo clippy --all-targets --all-features -D warnings`
   - `cargo test --workspace`

### Exit criteria

- workspace compiles,
- one fixture parses for each harness,
- one output object serializes correctly for each harness.

## Phase 1 — Native parsing and emission for the core event set

### Objectives

Ship a usable version for the hook types most likely to matter first.

### Scope

#### Claude
- `SessionStart`
- `UserPromptSubmit`
- `PreToolUse`
- `PostToolUse`
- `PostToolUseFailure`
- `PermissionDenied`
- `Stop`
- `Notification`
- `SessionEnd`

#### Codex
- `SessionStart`
- `PreToolUse`
- `PostToolUse`
- `UserPromptSubmit`
- `Stop`

#### Gemini
- `SessionStart`
- `SessionEnd`
- `BeforeAgent`
- `AfterAgent`
- `BeforeTool`
- `AfterTool`
- `Notification`
- `PreCompress`

### Tasks by crate

#### `hookkit-core`
- define `Harness`
- define shared error types
- define raw payload wrapper
- add JSON helper utilities
- add path helper aliases (`camino::Utf8PathBuf`)
- add internal capability enums

#### `hookkit-claude`
- implement top-level input enum
- implement output envelope and builders
- type common top-level fields
- type `Bash`, `Write`, and `Edit` tool inputs
- preserve unknown tool payloads as raw JSON
- implement parser dispatch on `hook_event_name`

#### `hookkit-codex`
- implement current documented event set
- type Bash command payloads for `PreToolUse` and `PostToolUse`
- model only documented supported outputs as ergonomic builders
- add explicit unsupported-capability errors for parsed-but-not-supported features

#### `hookkit-gemini`
- implement current documented event set
- model `BeforeTool` and `AfterTool`
- model `BeforeAgent` and `AfterAgent`
- preserve raw tool payloads
- type minimal `tool_response`
- implement `SessionStart`, `SessionEnd`, `Notification`, `PreCompress`

#### `hookkit-runtime`
- stdin reader
- stdout serializer
- stderr writer
- exit code mapping
- `run_native()`
- `run_common()` placeholder with only a few wrappers at first

### Testing

- fixture round-trip tests
- per-event golden serialization tests
- output validation tests
- integration tests running tiny binaries with JSON piped into stdin

### Exit criteria

- all phase-1 events parse and emit correctly,
- invalid output combinations are rejected before writing stdout,
- a minimal CLI example works for each harness.

## Phase 2 — Common wrappers and semantic helpers

### Objectives

Make cross-harness business logic practical without destroying native fidelity.

### Scope

Implement common wrappers for:

- session start
- prompt submit
- pre-tool use
- post-tool use
- stop
- notification
- session end
- pre-compress / pre-compact

### Tasks

1. Add `hookkit-common` wrapper enums.
2. Add shared convenience accessors:
   - `tool_name()`
   - `session_id()`
   - `cwd()`
   - `last_assistant_message()`
3. Add common output structs with conversion functions back to native outputs.
4. Add explicit unsupported-capability conversion errors.
5. Add small message intent helpers:
   - `UserNotice`
   - `AgentContext`
   - `AgentFeedback`
   - `TailToolCall`

### Important guardrail

Do not add a “universal everything” struct. Keep wrapper enums harness-tagged.

### Exit criteria

- one shared post-tool handler can target Claude, Codex, and Gemini,
- one shared prompt-submit handler can target Claude, Codex, and Gemini,
- unsupported conversions fail explicitly and are covered by tests.

## Phase 3 — Runtime polish and example executables

### Objectives

Prove that real hook tools become easier to build.

### Example executables to ship

#### Example 1: `shared-posttool-autofix`
Behavior:
- respond to a post-tool edit/write event,
- run formatter,
- run linter/autofix if applicable,
- remain quiet on clean success,
- show user a concise status,
- show agent concise guidance only when manual work remains,
- spill verbose diagnostics to a temp file artifact.

#### Example 2: `codex-bash-guard`
Behavior:
- inspect a Codex `PreToolUse` Bash command,
- deny destructive patterns,
- emit the documented Codex-supported block format.

#### Example 3: `gemini-beforetool-policy`
Behavior:
- deny certain tool argument patterns,
- optionally rewrite tool input,
- demonstrate `hookSpecificOutput.tool_input`.

#### Example 4: `claude-sessionstart-context`
Behavior:
- add additional context at session start,
- optionally write environment exports through `CLAUDE_ENV_FILE` in a separate example.

### Runtime improvements

- add a small artifact manager module,
- add helper methods for retrieving session/turn/tool keys,
- add structured logging to stderr,
- add a `--dump-parsed` debug mode for development.

### Exit criteria

- examples are documented,
- examples are used in integration tests,
- examples demonstrate user-vs-agent message splitting.

## Phase 4 — Advanced Claude support

### Objectives

Support the Claude-specific features that make it the richest target.

### Scope

Add:

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
- `InstructionsLoaded`
- `WorktreeCreate`
- `WorktreeRemove`
- `Elicitation`
- `ElicitationResult`
- `StopFailure`

### Special implementation notes

#### `PermissionRequest`
- model nested decision objects carefully,
- support updated-input and permissions suggestions,
- keep output validation strict.

#### `FileChanged`
- add dynamic `watchPaths` support,
- validate absolute paths.

#### `WorktreeCreate`
- model special “path return” semantics as its own output builder,
- do not force it through allow/block abstractions.

#### `CwdChanged` / `SessionStart`
- add optional helper for `CLAUDE_ENV_FILE`.

### Exit criteria

- advanced Claude events are fully typed,
- special-case emitters are covered by golden tests,
- example documentation explains which events remain Claude-only.

## Phase 5 — Gemini model-layer depth and Codex evolution

### Objectives

Complete the backend-specific value that wrapper enums cannot provide.

### Scope

#### Gemini
- fully type `LLMRequest`
- fully type `LLMResponse`
- support `BeforeModel`
- support `AfterModel`
- support `BeforeToolSelection`
- add helper builders for synthetic LLM response and tool filtering

#### Codex
- watch the docs/changelog for expanded event or tool support
- add versioned capabilities if/when more than Bash becomes available
- add `SessionStart` source support for `/clear` if needed by the version being targeted

### Exit criteria

- Gemini model-layer hooks are first-class,
- Codex feature gating is explicit and tested.

## 3. Recommended implementation order inside each crate

For every harness crate:

1. top-level enums
2. common top-level fields
3. one event at a time
4. serialization tests for that event
5. output builder for that event
6. validation rules for that event
7. integration test for that event

Avoid designing every type up front before the first end-to-end path works.

## 4. Test plan

## 4.1 Unit tests

- parse valid fixture JSON
- reject invalid JSON
- reject event/type mismatches
- verify unknown-field preservation
- verify native output JSON shape

## 4.2 Golden tests

Create stable snapshots for:

- Claude `PreToolUse` deny
- Claude `PostToolUse` block with `additionalContext`
- Claude `Stop` continue
- Codex `PreToolUse` deny
- Codex `Stop` continuation
- Gemini `BeforeTool` rewrite
- Gemini `AfterTool` deny/replace
- Gemini `AfterAgent` retry

## 4.3 Integration tests

For each example binary:

- pipe fixture JSON into stdin,
- capture stdout/stderr/exit code,
- compare against expected behavior.

## 4.4 Compatibility tests

- ensure unknown events become `Unknown`
- ensure unknown fields survive parse/serialize
- ensure unsupported-capability conversion returns the correct error

## 4.5 Property tests

Add selectively where useful:

- unknown-field round-trip preservation,
- no emitted JSON contains nulls for omitted optional fields,
- output validation forbids contradictory control states.

## 5. Dependency recommendations

Minimal initial dependency set:

```toml
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"
camino = { version = "1", features = ["serde1"] }
clap = { version = "4", features = ["derive"] }   # runtime/examples/templates
tempfile = "3"                                     # artifacts/examples
```

Optional later additions:

- `insta` for snapshot testing
- `schemars` if you want to generate your own schemas
- `indexmap` if stable insertion order matters for emitted JSON
- `time` or `chrono` if timestamps become first-class

## 6. Example backlog tickets

### Core tickets
- HK-001: Create workspace and crate skeleton
- HK-002: Add shared error model
- HK-003: Add runtime stdin/stdout/exit plumbing
- HK-004: Add Claude native parser dispatch
- HK-005: Add Codex native parser dispatch
- HK-006: Add Gemini native parser dispatch

### Claude tickets
- HK-020: Claude `SessionStart`
- HK-021: Claude `UserPromptSubmit`
- HK-022: Claude `PreToolUse`
- HK-023: Claude `PostToolUse`
- HK-024: Claude `Stop`
- HK-025: Claude `PermissionDenied`
- HK-026: Claude `Notification`
- HK-027: Claude `SessionEnd`

### Codex tickets
- HK-040: Codex `SessionStart`
- HK-041: Codex `PreToolUse`
- HK-042: Codex `PostToolUse`
- HK-043: Codex `UserPromptSubmit`
- HK-044: Codex `Stop`
- HK-045: Codex capability gating

### Gemini tickets
- HK-060: Gemini `SessionStart`
- HK-061: Gemini `BeforeAgent`
- HK-062: Gemini `BeforeTool`
- HK-063: Gemini `AfterTool`
- HK-064: Gemini `AfterAgent`
- HK-065: Gemini `SessionEnd`
- HK-066: Gemini `Notification`
- HK-067: Gemini `PreCompress`

### Common-layer tickets
- HK-080: Add common wrapper enums
- HK-081: Add shared accessors
- HK-082: Add common output conversions
- HK-083: Add unsupported-capability errors
- HK-084: Add message helpers

### Example tickets
- HK-100: shared post-tool autofix example
- HK-101: Codex Bash deny example
- HK-102: Gemini before-tool guard example
- HK-103: Claude session-start context example

## 7. Risks and mitigations

## Risk 1: over-normalizing too early
**Mitigation:** native-first design, wrappers second.

## Risk 2: Codex support shifts quickly
**Mitigation:** centralize Codex capability flags and version tests.

## Risk 3: Gemini model-hook types are deeper than tool hooks
**Mitigation:** defer full model typing to Phase 5 while preserving raw JSON now.

## Risk 4: output semantics are easy to get subtly wrong
**Mitigation:** runtime-owned validation plus golden tests.

## Risk 5: verbose diagnostics bloat agent context
**Mitigation:** add artifact helpers early in Phase 3.

## 8. Definition of done for a release candidate

A release candidate should satisfy all of the following:

- documented crate APIs,
- all phase-1 events supported,
- common wrapper layer available for the aligned event set,
- three example hook executables included,
- integration tests green,
- CI green on Linux and macOS,
- README includes quick-start examples for Claude, Codex, and Gemini.

## 9. Recommended release sequence

### Release 0.1.0
- native parsing/emission for phase-1 events
- runtime helpers
- basic examples

### Release 0.2.0
- common wrappers
- shared message helpers
- artifact utilities

### Release 0.3.0
- advanced Claude support
- Gemini model-layer hooks
- Codex capability evolution

## 10. Immediate next steps

If implementation starts now, the first concrete tasks should be:

1. create workspace and crate skeleton,
2. add one end-to-end path for `Claude SessionStart`,
3. add one end-to-end path for `Codex PreToolUse`,
4. add one end-to-end path for `Gemini BeforeTool`,
5. freeze fixture format and testing style before expanding event coverage.

That sequence gets the architecture under test immediately and avoids speculative overdesign.
