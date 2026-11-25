# Codex hook contract refresh: `commit-3d86e43-r1`

- Audit date: 2026-11-25
- Previous selected snapshot: `commit-ff6aec9-r2` (openai/codex
  `ff6aec96948b70d94983af2641a6b67c94faeff5`, tag `rust-v0.159.2`;
  selected in `620d5f1`)
- Proposed successor: `commit-3d86e43-r1`, **draft (not yet frozen or
  selected)**
- Proposed pinned upstream revision:
  `3d86e43b075adeba2193ec5c22ac92f55afd82e6` (tag `rust-v0.172.0`, the
  latest stable release on the audit date)
- Official hook reference: <https://learn.chatgpt.com/docs/hooks.md>
  (<https://developers.openai.com/codex/hooks.md> serves identical bytes)
- Vendored schemas: `contracts/vendor/codex/3d86e43b075adeba2193ec5c22ac92f55afd82e6/generated`
  (already committed in `ff3ab73`, 24 schemas plus `MANIFEST.sha256`)
- Selected supplement: `command-environments-2026-09-30-r2`; it needs a
  successor that targets `commit-3d86e43-r1`
- Native crate: `crates/hookkit-codex` (implements `commit-ff6aec9-r2`)

The reference body retrieved on 2026-09-30 was hashed but never captured,
and `web.archive.org` is unreachable from this environment. Drift in the
reference was therefore found by comparing today's text with what
`commit-ff6aec9-r2` encodes (schemas, fixtures, outcomes, uncertainties,
and `sources.yaml` limitations) and with the 2026-09-29 and 2026-09-30
audit notes. Source drift was found directly: `codex-rs/hooks`,
`codex-rs/protocol`, and the hook call sites in `codex-rs/core` were
diffed from `ff6aec9` to `3d86e43`
(`codex-hooks-ff6aec9..3d86e43.diff` covers `codex-rs/hooks`), and every
release-boundary claim was checked with `git tag --contains`. Doc line
numbers refer to the staged copy `codex-hooks.md`, whose hash is in
"Sources and hashes"; source citations use `path@rev:line`.

Three audit slices (reference text `CX-DOC`, event source `CX-SRC`, and
engine and environment `CX-ENG`) produced 43 findings. Each was
adversarially verified; none was refuted outright, but many were narrowed
or corrected, and the slices overlapped heavily. This note uses the
verified wording, merges duplicates, re-checks the verifiers' "missed"
items against the pinned source, and records the narrowed claims at the
end.

## Pin selection

The successor pins the release tag, not `main`. At audit time `main` was
`87de5e932dd03bb9efece4a6dd3eab4c5de45701` (2026-11-25). It is not a
descendant of `3d86e43`: the release branch diverged at merge-base
`39c29c3bb6` (2026-11-19, contained in the tag), and the release branch
has no `codex-rs/hooks` commits after it. `git diff --stat 3d86e43
87de5e93 -- codex-rs/hooks/schema` is empty, so the generated wire is
identical on `main`. The hooks commits that exist only on `main` and in
0.173/0.174 alpha tags are listed under "Main-only and pre-release
changes" and are not encoded. The reference itself says to use the page,
not the `main` schemas, as the release behavior reference
(`codex-hooks.md:1151-1152`). Where the page lags the tag, the tag's
generated schema and source control, as in earlier refreshes.

## Sources and hashes

| Source id | Reference | Evidence |
| --- | --- | --- |
| `codex-hooks-reference` | `https://learn.chatgpt.com/docs/hooks.md` | SHA-256 `37cdbfc3b9d19925f88af3e680399fd61bd3d15979961302160f97698cc5a841` (49,306 bytes); previously `724881d81f1a9fdb35ea42d4851a11b1f6606bbc6e9d2d05c1e8c466fc25c630` (46,965 bytes); the `developers.openai.com` copy is byte-identical (`cmp`); `content-hash-only` |
| `codex-generated-schemas` | `codex-rs/hooks/schema/generated` at `3d86e43` | 24 files vendored with `MANIFEST.sha256` (`ff3ab73`); `vendored` |
| `codex-hooks-source` | `codex-rs/hooks` at `3d86e43` | `pinned-revision` |
| `codex-core-source` | `codex-rs/core` at `3d86e43` | `pinned-revision` |
| supplement `codex-protocol-source` | `codex-rs/protocol` at `3d86e43` | `pinned-revision` |
| supplement `codex-config-source` | `codex-rs/config` and `codex-rs/protocol/src/hooks.rs` at `3d86e43` | `pinned-revision` |

Of the 24 vendored schema files, the 23 shared with the `ff6aec9` tree
are byte-identical to it, and every file matches `git show
3d86e43:codex-rs/hooks/schema/generated/<file>`. `git diff --stat ff6aec9
3d86e43 -- codex-rs/hooks/schema` shows one change: the new
`model-request.command.input.schema.json` (+179 lines, blob `a78e125a`,
SHA-256 `5182c45dad9a6f6ece6ded590052323ccd80aaa9ae3649e04a107936fc7a13b0`).
No model-request output schema exists. The vendor refresh is therefore
done; only the generator, the snapshot, the supplement, and the crates
still lack the new events.

Of the three `codex-hooks-reference` limitations in
`commit-ff6aec9-r2/sources.yaml:10-13`, the "omitting the SessionStart
`fork` source" clause is resolved (`codex-hooks.md:438`, `:454`,
`:668-671`). The "session cwd for tool hooks" wording (`:200`, `:489`) and
the PermissionRequest "fail closed" wording (`:901-903`) still apply, and
the cwd one is now further from the source (see "Tool-event `cwd`").

## Result

The command-event inventory grows from 12 to **14** events:
`PreModelRequest` and `PostModelRequest` are new
(`hooks/src/lib.rs@3d86e43:29-44`, `HOOK_EVENT_NAMES: [&str; 14]`). No
event was removed or renamed, and the 22 generated input and output
schemas of the existing 12 events are byte-identical. The headline
changes are:

- **New observation-only events `PreModelRequest` and
  `PostModelRequest`** (0.164.0; `kind`, `request_id`, `attempt_number`,
  `thread_id`, and `response_id` since 0.171.0). They have a process
  contract unlike every other Codex event: stdout is never parsed, and
  non-blank stderr is surfaced as an error entry at every exit code,
  including 0. The reference documents only the 0.164.0-0.170.0 field
  set.
- **Tool-event stdin `cwd` is now the local command's execution
  directory** (0.166.0) for `exec_command` (Bash) calls, local Bash,
  `write_stdin`, and network-access approvals, and their `PostToolUse`
  completions. The hook process still starts in the step-local
  environment cwd, so the two can now differ.
- **Environment-scoped managed hooks** (0.165.0) run on an execution
  environment's executor with rewritten stdin (`cwd` = environment cwd,
  transcript paths null), a different shell and environment policy, and
  cannot grant a `PermissionRequest`.
- **`PreToolUse` `updatedInput` may not change a tool's target selector**
  (0.165.0, MCP `environment_id` 0.170.0), and **direct Guardian review**
  dispatches `PreToolUse` alone for recorded calls that never execute
  (0.167.0, more tools 0.168.0).
- **Plugin hook approval is bound to the plugin bundle's contents**
  (0.164.0): a changed, symlinked, or unreadable bundle makes every run
  fail before launch.
- **Command-environment drift**: the credential scrub list grew from five
  to nine names, and `CODEX_TOOL_CALL_ID` is a new shell-tool-only label.
- **Corrections of the selected snapshot**: `permission_mode` only ever
  carries `default` or `bypassPermissions`; Interrupt `mcp_tool` handlers
  share the 1-3 s clamp; the exit-0-stderr invariant must be narrowed.

| Event | Wire or behavior change since `ff6aec9` | Successor contract change | Class |
| --- | --- | --- | --- |
| `PreModelRequest` (new) | new event (0.164.0); fields added in 0.171.0 | complete event directory; `outputs: []`; `no-op` and `failure` outcomes | additive |
| `PostModelRequest` (new) | new event; `status`, `error`, `token_usage`, `response_id` | complete event directory; same outcomes; MCP pricing uncertainty | additive |
| `SessionStart` | none (reference caught up on `fork` and mid-turn compact) | drop stale docs-lag clause; `compact-source` positive fixture | documentation-only |
| `SessionEnd` | none (reference now lists triggers) | trigger uncertainty; 30 min vs 60 s limitation | documentation-only |
| `SubagentStart` | none | shared uncertainties only | none |
| `PreToolUse` | stdin `cwd` = resolved local workdir (0.166.0); selector rule (0.165.0/0.170.0); Guardian direct review (0.167.0/0.168.0) | rewritten cwd uncertainty; selector, Guardian, and code-mode uncertainties; synthesized cwd positives | behavioral |
| `PermissionRequest` | per-action stdin `cwd` (0.166.0); environment-scoped `allow` ignored (0.165.0) | rewritten cwd uncertainty; provenance uncertainty; `write_stdin` positive | behavioral |
| `PostToolUse` | stdin `cwd` = original execution cwd, even in a later turn (0.166.0) | rewritten cwd uncertainty; code-mode block uncertainty | behavioral |
| `PreCompact` / `PostCompact` | none | shared uncertainties only | none |
| `UserPromptSubmit` | none | shared uncertainties only | none |
| `SubagentStop` | none (`agent_transcript_path` null for environment-scoped hooks) | shared uncertainties only | none |
| `Stop` | none (environment-scoped hooks join memory-consolidation Stop) | shared uncertainties only | none |
| `Interrupt` | none | timeout uncertainty extended to `mcp_tool` | correction |

The shared uncertainties added to every event cover environment-scoped
hooks, plugin content trust, the `mcp_tool` runtime, `permission_mode`
values (on the nine events that carry it), and the root `session_id`
(on events that can fire in subagents).

## Event inventory change

`commit-ff6aec9-r2/snapshot.yaml:9-45` lists 12 events, `SessionStart`
through `Interrupt`. The successor adds:

- `events/pre-model-request` (`wire_name: PreModelRequest`, `rust_key:
  pre_model_request`, `category: model`, `inferability: definitive`,
  discriminator `/hook_event_name` const `PreModelRequest`);
- `events/post-model-request` (`wire_name: PostModelRequest`, `rust_key:
  post_model_request`, `category: model`, same discriminator shape).

`hookkit-core` already has an event category for model events
(`crates/hookkit-core/src/event.rs:16-17`). The expected `cargo xtask
contracts diff codex/commit-ff6aec9-r2 codex/commit-3d86e43-r1` result is
2 events added, 12 changed (uncertainty text, plus new positive fixtures
on `SessionStart`, `PreToolUse`, `PermissionRequest`, and `PostToolUse`),
0 schema changes for the existing 12, and `sources.yaml` changed.

## Confirmed drift by event

### `PreModelRequest` and `PostModelRequest` (new events)

Merged from CX-DOC-1, CX-DOC-2, CX-DOC-3, CX-SRC-1 through CX-SRC-7,
CX-ENG-6, and the verifiers' notes on pairing and budget gating.

Reference evidence:

- `codex-hooks.md:26`: "| For each model request attempt    |
  `PreModelRequest`, `PostModelRequest` |"
- `codex-hooks.md:974-978`: "`PreModelRequest` runs before each attempt.
  `PostModelRequest` reports the outcome. Both receive the model and
  `turn_id`, plus `agent_id` and `agent_type` when available. Hook output
  is diagnostic only. It can't change the model's context or block a
  request."
- `codex-hooks.md:980-986`: "`PostModelRequest` includes three additional
  fields:" `status` ("`completed`, `failed`, or `cancelled`."), `error`,
  and `token_usage`.
- `codex-hooks.md:434`: "| `PreModelRequest`, `PostModelRequest` | not
  supported | Both events run for each model request attempt |" (matcher
  table).
- `codex-hooks.md:496-499`: the `permission_mode` list names nine events,
  not the model-request events.

Source evidence (`3d86e43`):

- `hooks/src/events/model_request.rs:22-36`: `#[serde(tag =
  "hook_event_name")] pub enum ModelRequestHookEvent { PreModelRequest,
  PostModelRequest { status, error, token_usage, response_id } }`.
- `model_request.rs:118-143`: `ModelRequestHookInput` with
  `#[serde(skip)] lifecycle_id`, `kind: SamplingKind`, "/// Identifies a
  generation across harness retries and model fallback. pub request_id:
  String", "/// One-based attempt number within this generation. pub
  attempt_number: u32", `session_id`, "/// Identifies the issuing thread
  within its shared session." `thread_id`, `turn_id`, `cwd`,
  `transcript_path`, `model`, and `agent_id`/`agent_type` with
  `skip_serializing_if = "Option::is_none"`. No `permission_mode`.
- `model_request.rs:110-116`: `SamplingKind` is `sampling`,
  `foreground_compaction`, or `background_compaction`; `:57-61`:
  `ModelRequestStatus` is `completed`, `failed`, or `cancelled`.
- `core/src/hook_runtime/sampling.rs:1-3`: "A sampling loop keeps one
  request ID across retries. Each call to open a response stream pairs
  its own hooks; transport recovery within that call does not start
  another hook pair."
- `sampling.rs:65` increments `attempts` before the input is built
  (`attempt_number: self.attempts`), so the first attempt is 1;
  `:78-92` builds the input with `cwd: tool_hook_cwd(&step.environments,
  &step.turn)` and `thread_id: session.thread_id()`; `:95`
  `session.check_rollout_budget()?` runs before any hook; `:106-109`
  awaits pre-hooks before `send.await`.
- Constructors: `core/src/session/turn.rs:2062` (`Sampling`);
  `core/src/compact.rs:276-277` and `compact_remote_v2_installation.rs:134-135`
  (`ForegroundCompaction`); `compact_remote_v2_attempt.rs:357-358`
  (`BackgroundCompaction`, reached only through the `AsyncCompaction`
  feature, `Stage::UnderDevelopment`, `default_enabled: false`).
- Generated schema `model-request.command.input.schema.json`: root
  `required` `[attempt_number, cwd, kind, model, request_id, session_id,
  thread_id, transcript_path, turn_id]`; a root `oneOf` keyed on
  `hook_event_name`; the Post branch requires `[error, hook_event_name,
  response_id, status, token_usage]`; `attempt_number` is `{format:
  uint32, minimum: 0}`; `TokenUsage` requires `input_tokens`,
  `cached_input_tokens`, `output_tokens`, `reasoning_output_tokens`, and
  `total_tokens` (int64) and leaves `cache_write_input_tokens` optional
  with default 0. It is the only one of the 24 vendored schemas without
  `additionalProperties: false`: `ModelRequestHookInput` carries no
  `#[serde(deny_unknown_fields)]` and flattens the event enum, so
  schemars emits none. The serialized key set is still closed.
- History (from the generated schema at each tag): absent at
  `rust-v0.163.0`; present at `rust-v0.164.0` (`70b3bb6d88`, #55144) with
  `[agent_id, agent_type, cwd, model, session_id, transcript_path,
  turn_id]` and Post `[error, status, token_usage]`; `kind`,
  `request_id`, `attempt_number`, `thread_id`, and Post `response_id`
  first at `rust-v0.171.0` (`81b995a6c9` #61562, `3b8bd2ee7e` #61587).
- Upstream test `core/tests/suite/hooks.rs:6103-6158`
  (`model_request_hooks_report_failures_and_retries_but_cannot_veto`)
  asserts one `request_id` across retries, `attempt_number == index + 1`,
  `thread_id == session_configured.thread_id`, and `kind == "sampling"`.

Input contract (both events):

- Required: `session_id` (the root session identity, see "All events"),
  `thread_id` (the issuing thread; equals `agent_id` in a thread-spawned
  subagent), `turn_id`, `cwd` (step-local environment cwd, falling back
  to the turn cwd, or the environment cwd for environment-scoped
  handlers), `transcript_path` (string or null), `model`, `kind`,
  `request_id` (a UUID v4 shared across retries and model fallback of one
  generation), `attempt_number` (one-based `u32`, incremented on each
  open), `hook_event_name`.
- Optional: `agent_id` and `agent_type`, present only for thread-spawned
  subagents (`hook_runtime.rs@3d86e43:1101-1108`; `agent_type` falls back
  to the default role name).
- `PostModelRequest` adds required `status`, `error` (string or null),
  `response_id` (string or null), and `token_usage` (`TokenUsage` or
  null). `protocol/src/protocol.rs@3d86e43:2598-2618` always serializes
  `cache_write_input_tokens` (`#[serde(default)]`, never skipped), and
  `codex_rollout_budget_units` is `skip_serializing`.

`PostModelRequest` value rules from source (`model_request.rs:64-100`):

- `completed`: `error` null, `response_id` the response id,
  `token_usage` the provider usage or null.
- `failed`: `error` is `CodexErr::to_string()`. `ResponseIncomplete`
  carries `response_id` and `token_usage` (Display "Incomplete response
  returned, reason: {reason}", `protocol/src/error.rs:102`); every other
  failure sets both null. The EOF path
  (`core/src/client/response_delivery.rs:158-173`) builds
  `CodexErr::Stream("stream closed before response.completed")`, whose
  Display is "stream disconnected before completion: {0}"
  (`error.rs:130-131`), so hooks receive "stream disconnected before
  completion: stream closed before response.completed". `ContentFilter`
  has its own fixed text and is not `ResponseIncomplete`, so both
  members are null.
- `cancelled`: `TurnAborted` and `Interrupted` failures, and attempts
  dropped before settling, set `error`, `token_usage`, and
  `response_id` all null.

Dispatch and timing:

- Matchers are ignored: `events/common.rs@3d86e43:143-147`
  `matcher_pattern_for_event` returns `None`, `model_request.rs:153-157`
  and `:197-201` pass `matcher_input: None`, and the events are not in
  `HOOK_EVENT_NAMES_WITH_MATCHERS` (still 9). The reference agrees
  (`:434`).
- `async: true` is honored; only `SessionEnd` forces synchronous
  execution (`engine/discovery.rs:687`). `execution_mode_for_event`
  returns `Sync` when any handler is synchronous
  (`registry.rs:359-367`). The timeout defaults to 600 s with a 1 s
  minimum and no clamp (`discovery.rs:959-981`). Scope is `Turn`.
- A synchronous `PreModelRequest` delays the request by up to its
  timeout. A synchronous `PostModelRequest` holds back the terminal
  event (completed or error) and, under `response_process_mode`
  `AfterCompleteResponse`, the buffered items until it finishes
  (`response_delivery.rs:64-115`); in the default mode streaming items and
  tool dispatch proceed (`81b995a6c9`: "Allow streaming output and tool
  execution to proceed before the hook finishes").
- When the turn stops waiting, unfinished synchronous post runs are
  marked `Stopped` with "Hook cancelled because the turn was interrupted"
  (`sampling.rs:141-190`). Dropping an unsettled attempt schedules
  `cancelled` on the async executor even for synchronous handlers
  (`sampling.rs:206-215`), which shares the per-session background
  concurrency limit (eight per `codex-hooks.md:649-650`) and is aborted
  at runtime shutdown, so a late `cancelled` may never run.
- Pairing: `lifecycle_id` is `#[serde(skip)]`, so hooks can correlate
  only on (`request_id`, `attempt_number`). Neither event fires when the
  rollout budget is exhausted before the attempt (`sampling.rs:93-95`).
  `PostModelRequest` runs without a `PreModelRequest` when only post
  handlers are configured (`pre_mode` is `None` at `:106`), and post runs
  only when post handlers exist.
- Memory-consolidation turns use `for_memory_consolidation`, which keeps
  managed-source and executor-scoped handlers and adds no
  environment-scoped hooks (`hook_runtime.rs@3d86e43:1117-1131`,
  `engine/mod.rs:176-194`); other turns add environment-scoped hooks
  (`with_environment_hooks`). Plugin hooks therefore never run on
  memory-consolidation turns. Upstream test:
  `memory_consolidation_skips_user_model_request_hooks`
  (`hooks.rs:6600`).
- `additionalContextLimit` on these handlers is ignored with a load
  warning (`discovery.rs:695-711`).

Process outcomes (`model_request.rs:275-301`):

```rust
let status = if result.error.is_none() && result.exit_code == Some(0) {
    HookRunStatus::Completed
} else {
    HookRunStatus::Failed
};
// Observer stdout is diagnostic, including any JSON that looks like a control directive.
if !result.stdout.trim().is_empty() {
    tracing::debug!(stdout = %result.stdout, "model request hook output");
}
if let Some(text) = common::trimmed_non_empty(&result.stderr) {
    entries.push(HookOutputEntry { kind: HookOutputEntryKind::Error, text });
}
```

- stdout is never parsed at any exit code. Plain text, invalid JSON, and
  control-looking JSON such as `{"continue":false,"decision":"block"}`
  are only debug-logged; the upstream test writes exactly such a
  directive (`hooks.rs:6071-6086`) and asserts it never reaches the
  request (`:6125-6131`).
- Trimmed non-blank stderr becomes an `Error` entry **regardless of exit
  code**, so an exit-0 run is `Completed` but shows an error entry. This
  breaks the invariant "Codex never reads stderr from a command that
  exits 0", which the 2026-09-29 audit states for all events
  ("Every stderr read in `codex-rs/hooks` sits in a non-zero-exit arm")
  and which every existing contract repeats. The invariant still holds
  for the other 12 events, whose parsers changed only in input
  serialization and context metadata.
- Every non-zero exit, exit 2 included, and a death without an exit
  status (`command_runner.rs:353-356`) is `Failed`. No "hook exited with
  code N" text is added, so a failed run with blank stderr has no entries.
  A timeout, spawn error, or plugin trust error adds its own error text.
  Nothing can block or veto. Async runs deliver their `Error` entries
  too (`schedule_async_hook`).

`mcp_tool` pricing side channel (`model_request.rs:1-2`, `:182-188`,
`:205-215`, `:302-325`; `c8c3f70893`, first in `rust-v0.171.0`): a
synchronous `mcp_tool` `PostModelRequest` handler whose run completes may
return `structuredContent` `{"extra_codex_rollout_budget_units": <f64>}`
(default 0) when the event is billable, that is `completed`, or `failed`
with `token_usage` present. A non-object, a wrong type, or a negative or
non-finite value sets `invalid_price`; sibling prices are kept, but
usage posting then fails with `CodexErr::UsageAccountingFailed("Invalid
post-hook budget units: …")` (`core/src/session/rollout_budget.rs:139-144`).
Command handlers can never price: both the local and environment command
paths set `structured_content: None` (`command_runner.rs:271`, `:436`).
Executor-scoped `mcp_tool` handlers always run async
(`engine/mod.rs:196-206`) and are not summed, so they cannot price
either. The MCP result text is ignored like command stdout. The reference
says only "Hook output is diagnostic only" (`:977-978`).

Reference lag: `codex-hooks.md:974-986` describes exactly the
`rust-v0.164.0`-`0.170.0` schema. It omits `kind`, `request_id`,
`attempt_number`, `thread_id`, and `response_id`, calls the Post
additions "three additional fields", and mentions neither exit-0 stderr
visibility, the per-open pairing, nor MCP pricing. Because the omitted
fields ship in the `rust-v0.172.0` tag, not only on `main`, the
`:1151-1152` "release behavior reference" sentence does not make the page
control; this is a docs lag, not a contradiction.

Classification: **additive** (two events).

Proposed contract (`events/pre-model-request`,
`events/post-model-request`):

- `schemas.input`: derived from the single vendored file. For each wire
  name, merge the root `properties` and `required` with the `oneOf`
  branch whose `hook_event_name` enum matches, make `hook_event_name` a
  required `const`, and drop `oneOf`. Keep the vendored openness (no
  `additionalProperties: false`) and the vendored `attempt_number`
  `minimum: 0`; record both as uncertainties (the struct is closed, and
  Codex starts at 1) rather than narrowing, because nothing at runtime
  rejects either and the 0.171.0 additions show the key set grows.
  Sources `[codex-hooks-reference, codex-generated-schemas,
  codex-hooks-source, codex-core-source]`, origin `derived`, assurance
  `source-reviewed`.
- `schemas.outputs: []`. `handler_kinds: [command, mcp_tool]`.
- `bindings.command` request: stdin, `single-document-at-eof`, JSON.
- Outcomes (two, so exit ranges do not overlap):
  - `no-op`: effect `none`, exit `{exact: 0}`, stdout `{presence:
    optional, role: ignored, content_kind: opaque}`, stderr `{presence:
    optional, role: diagnostics, content_kind: text, encoding: utf-8}`.
    The exit-0 `diagnostics` role has precedent in frozen snapshots
    (`codex/commit-1e59dc5-r1/events/pre-tool-use`, Antigravity
    `docs-2026-08-04-r1`).
  - `failure`: effect `nonblocking-error`, exit `{range: {min: 1, max:
    255}}`, stdout optional/ignored/opaque, stderr optional/diagnostics/
    text.
  - No exit-2 outcome and no structured or control outcome.
- Uncertainties: observation-only, stdout (including control JSON) is
  ignored at every exit; stderr is shown as an error entry even at exit
  0; no "hook exited with code N" text; matchers ignored; async allowed;
  600 s default timeout with no clamp; a synchronous pre hook delays the
  request; a synchronous post hook holds back the terminal event (and
  buffered items under `AfterCompleteResponse`) but not streaming or tool
  dispatch by default; interrupted synchronous post runs are `Stopped`;
  dropped attempts schedule an async `cancelled` that may be lost at
  shutdown; one hook pair per stream open, none once the rollout budget
  is exhausted, correlation only by (`request_id`, `attempt_number`),
  and Post without Pre when only post handlers exist;
  `background_compaction` requires the under-development
  `AsyncCompaction` feature; no `permission_mode`; `cwd` is the
  step-local environment cwd; memory-consolidation turns run only
  managed and executor handlers; `additionalContextLimit` is ignored;
  MCP result text is ignored (replacing the generic "parsed like exit-0
  stdout" wording); on Post, the MCP-only pricing channel and its
  failure mode; the Post value rules above.
- `sources.yaml`: a `codex-hooks-reference` limitation that the page
  documents the pre-0.171 field set and omits exit-0 stderr visibility
  and MCP pricing; cite `:1151-1152` and state that the tag's schema
  controls.
- Fixtures: `minimal` and `representative` positives, negatives, and
  process cases as listed in the test plan.

Native impact: `hookkit-codex` knows neither event. `CatalogEvent`
(`catalog.rs:61-72`), `from_name` (`:126-140`), `fields()`
(`:180-249`), `protocol::events()` (`protocol.rs:19-34`), the `Event`
selector (`:703-731`), `AnyInput` (`:753-755`), `AnyCommandOutput`
(`:765-767`), and `Codex::decode` (`:804-815`) have no arm, so decoding
falls through to `HookkitError::UnrecognizedEvent` (the three enums are
`#[non_exhaustive]`, so adding variants is additive). A HookKit binary
registered for these events today fails every model request attempt. On
the selected-harness path `failure_event` still identifies the wire name
through `resolution.rs:398-418` `discriminated_event`, which falls back
to the `/hook_event_name` string; `codex/PreModelRequest` is not in
`CODEX_EXIT_TWO_GATES` (`failure.rs:274`), so even `FailClosed` lowers
to `NotGating` and exit 1. Only when no invocation can be read at all
does the Codex lowering return exit 2 (`failure.rs:303-304`). Either way
Codex records a plain failed run, and because these events surface
stderr, the user sees the HookKit error line on every attempt.
`FieldKind` (`catalog.rs:75-80`) has only `String`, `NullableString`,
`Bool`, and `Any`, so `attempt_number` and `token_usage` need new kinds.
The output type must not reuse the JSON control macros
(`common_controls!`, `continuation_controls!`,
`system_message_control!`), whose stdout Codex ignores here.
`crates/hookkit-codex/src/lib.rs:9-14` ("Codex never reads stderr from a
hook that exits 0 ... shows that run's stderr only for `PreCompact`,
`PostCompact`, and `SessionEnd`") becomes false for these events.

### Tool-event `cwd` (`PreToolUse`, `PermissionRequest`, `PostToolUse`)

Merged from CX-DOC-4, CX-SRC-8, CX-SRC-9, CX-SRC-10, and CX-ENG-1.

Evidence:

- `hooks/src/events/common.rs@3d86e43:24-29`: `pub(crate) fn
  command_input_cwd(cwd: &Path, tool_cwd: Option<&PathUri>) -> String {
  tool_cwd.map_or_else(|| cwd.display().to_string(),
  PathUri::inferred_native_path_string) }`, used at
  `pre_tool_use.rs:189`, `permission_request.rs:183`, and
  `post_tool_use.rs:162`.
- `pre_tool_use.rs@3d86e43:30-32`: "/// Local tool cwd for hook stdin.
  Remote commands leave this unset to retain the host cwd. The hook
  process still runs in `cwd`." and `:178-180`: "Local commands can
  expose their execution cwd here, independently of the directory used
  to launch hooks. Remote commands retain the host cwd; remote cwd
  reporting is deferred."
- `core/src/tools/handlers/unified_exec/exec_command.rs@3d86e43:344-360`
  (`pre_tool_use_cwd`: "Hook scripts execute on the host. Remote command
  paths cannot replace the host cwd in hook input") and
  `unified_exec.rs:91-111` (`resolve_cwd`: `environment.cwd().join(workdir)`;
  empty or absent `workdir` means the environment cwd). Only
  `exec_command` overrides the default `Ok(None)` (`registry.rs:188-196`).
- `core/src/tools/approvals.rs@3d86e43:188-282`: `ExecCommand` uses
  `PermissionRequestPayload::bash(.., cwd)` (`sandboxing.rs:136-158`);
  `WriteStdin` uses the exec session's cwd; `NetworkAccess` uses the
  trigger's cwd or the action cwd; `ApplyPatch`, `McpToolCall`, and
  `RequestPermissions` use `None`; `:270-280`: "Command hooks run on the
  host. Keep remote paths in the action's original tool input, but never
  use them as the hook's top-level cwd."
- `core/src/tools/context.rs@3d86e43:458-461`: "/// Local execution cwd
  retained for PostToolUse, independent of the collecting turn."; only
  `ExecCommandToolOutput` overrides `post_tool_use_cwd` (`:502-504`;
  default `None` in `tools/src/tool_output.rs:55-60`).
- The hook process cwd is unchanged: `hook_runtime.rs@3d86e43:265-270`
  `tool_hook_cwd` (step local-environment cwd, else turn cwd), used as
  `command.current_dir` (`command_runner.rs:284`); identical to
  `@ff6aec9:248-253`.
- An unresolvable `workdir` or environment now fails the call before any
  `PreToolUse` runs (`registry.rs:1181-1183`, `PreToolUseError::Failed`;
  test `invalid_execution_cwd_records_failure_without_running_hooks`).
- Upstream tests: `core/tests/suite/hooks_cwd_tests.rs` ("Tool hooks
  report local command cwd and retain host cwd for remote commands.";
  `completion_in_another_turn`, `stdin_approval_in_another_turn`).
- Commits `fe15bd38de` (#57198) and `beb34ce8d0` (#57203), both first in
  stable `rust-v0.166.0` (in `0.165.0-alpha.3`, not in `0.165.0`).
- Reference (unchanged wording): `codex-hooks.md:489` "| `cwd` |
  `string` | Working directory for the session |"; `:200` "Commands run
  with the session `cwd` as their working directory."

Since 0.166.0, stdin `cwd` is the resolved local execution directory for:
`exec_command` (Bash) `PreToolUse`; local Bash, `write_stdin`, and
network-access `PermissionRequest`s; and `PostToolUse` of unified-exec
output, including completions delivered by a later `write_stdin` poll or
in another turn (the original command's directory is retained). The path
is rendered in its native convention (`PathUri::inferred_native_path_string`;
a unit test maps `file:///C:/repo-B` to `C:\repo-B`, though in practice a
local command uses the host's convention). Remote-environment commands,
`apply_patch`, MCP, `request_permissions`, and other function tools keep
the step-local cwd. `exec_command` `tool_input` is still only
`{command}` (`exec_command.rs:331-342`), so `cwd` is the only way a hook
learns the workdir. The selected snapshot's uncertainty
(`pre-tool-use/contract.yaml:61`, `permission-request/contract.yaml:125`,
`post-tool-use/contract.yaml:126`: "`cwd` and the process working
directory are the step's local-environment cwd") is wrong for these
cases. The generated schemas are byte-identical; `cwd` stays a string.

Related source notes:

- `write_stdin` `PermissionRequest` `tool_input.cwd` is a `PathUri`
  serialized as its URI string (`utils/path-uri/src/lib.rs@3d86e43:687-694`),
  e.g. `file:///repo/nested`, while the top-level `cwd` is native. This
  holds since `ff6aec9`, and the snapshot never mentions `write_stdin` as
  a `PermissionRequest` tool name.
- The zsh-fork `ApprovalAction::Execve` variant, which produced
  shlex-joined Bash `PermissionRequest`s without a description
  (`approvals.rs@ff6aec9:198-202`), was removed with the patched zsh
  backend in `cd85a26cae` (#52160, `rust-v0.163.0`). Per-subcommand
  execve escalations no longer produce `PermissionRequest`s. The snapshot
  never described that path; no schema impact.

Classification: **behavioral**; breaking for consumers that treat
tool-event `cwd` as the project root.

Proposed contract: replace the cwd uncertainty on all three events with
the per-case rule above, state that the hook process always starts in the
step-local cwd (so stdin `cwd` and the process cwd can differ), that an
unresolvable workdir fails the call before `PreToolUse`, and keep the
"reference still says session cwd" note and the 0.156.0 `model`/
`permission_mode` sentence. Add synthesized positives (test plan).

Native impact:

- Rustdoc is wrong: `crates/hookkit-codex/src/protocol.rs:68-72` ("Since
  Codex 0.157.0 `cwd` (and the hook's process working directory) is the
  step's local-environment directory"), `:78` and `:345` ("Step-local
  working directory of the tool call"), `:338`, and `catalog.rs:261-263`
  (`PermissionRequest` "reports the step-local directory").
- `NativeContext.workspace_roots` is seeded from tool-event `cwd`
  (`protocol.rs:324`, `:633`; `catalog.rs:422`). Consequences:
  `hookkit-session-state` `record_project_observation`
  (`crates/hookkit-session-state/src/metadata.rs:599-616`) keys projects
  by the sorted roots, so each distinct workdir creates a new project
  observation; `hookkit-tool-runner/src/post_tool.rs:224-226` starts
  Codex config discovery from `workspace_roots().first()`, which can now
  be a subdirectory or `/tmp`; and `examples/codex-claude-rules/src/main.rs:76-79`
  uses stdin `cwd` as `project_root` when `--project-root` is absent and
  looks for rules at `project_root/.claude/rules` (`:140`), so repository
  rules are missed for workdir commands.
- `hookkit-shell` `CODEX_BASH_PROFILE` rustdoc
  (`crates/hookkit-shell/src/adapters.rs:19-29`) says the hook `cwd` "is
  only the default working directory". Since 0.166.0 it already is the
  resolved workdir for local commands. Keep `UnverifiedFallback`
  (remote commands still report the host cwd with no stdin marker), but
  fix the doc. The `/workdir` branch (test
  `codex_workdir_is_resolved_against_the_hook_cwd`, `:258`) never fires
  for real Codex payloads; if a workdir ever appeared in `tool_input` it
  would be applied twice.
- `hookkit-file-activity` resolves relative paths against the hook
  process cwd (`current_utf8_dir`, `src/lib.rs:1047`, `:1821`, `:2039`),
  which can now disagree with stdin `cwd`.

### `PreToolUse`: `updatedInput` selectors and direct Guardian review

Merged from CX-DOC-6 and CX-ENG-8.

Evidence:

- `core/src/tools/registry.rs@3d86e43:216-311`
  (`with_updated_function_input`, `preserve_hook_selector`): "`if let
  Some(value) = object.get(field) && value != &expected { return
  Err(FunctionCallError::RespondToModel(format!("PreToolUse hook cannot
  change the tool environment ({field})"))); } if !expected.is_null() {
  object.insert(field.to_string(), expected); }`". A rewrite failure maps
  to `PreToolUseError::Failed` (`:1198-1212`). No such function exists at
  `ff6aec9`.
- `core/src/tools/handlers/apply_patch.rs@3d86e43:377-401`.
- Commits `4610b573e5` (#56343, `rust-v0.165.0`): "Preserve environment,
  MCP server, and terminal session selectors across hook input rewrites,
  rejecting attempts to switch targets."; `a4b4082b5f` (#58368,
  `rust-v0.167.0`): "Run the shared `PreToolUse` hook handling before
  evaluating approval requirements ... return an error when a hook blocks
  the call".
- Reference: `codex-hooks.md:835-839` says only "other `updatedInput`
  shapes are reported as errors."

Rules since 0.165.0: a rewrite may omit or repeat a call's target
selector but may not change it. Selectors are the tool-declared
environment argument fields (`ToolEnvironments::Arguments`, e.g.
`environment_id`), MCP-resource `server` (whitespace-trimmed; a blank
value normalizes to null and then mismatches), terminal `session_id`, the
`apply_patch` patch `environment_id` (error without a field suffix), and,
since **0.170.0**, MCP `environment_id` under `EnvironmentQualified`
exposure. A changed selector fails the tool call with a model-visible
error, not a failed hook run; an omitted non-null selector is
re-inserted. For such tools a non-object `updatedInput` fails with
"expected an object for rewritten tool arguments". A rewritten
`apply_patch` must now parse ("apply_patch verification failed: …").
`exec_command` rewrites only `cmd` from `command`, so it has no selector
exposure.

Direct Guardian review: `CodexThread::guardian_review` ("Review the final
injected tool call without executing it", `codex_thread.rs:1118-1134`)
was itself added in 0.167.0 (#58028). Since then it runs
`apply_pre_tool_use` for a recorded `exec_command`; since 0.168.0 also
for `apply_patch` (#58899, #58931), `request_permissions` (#58903), and
MCP calls (#59735). It runs inside a speculative turn with a fresh UUIDv7
`turn_id` (`recorded_tool_call.rs:92-97`, `:139-145`), the call never
executes, so no `PermissionRequest` or `PostToolUse` follows, and a block
reaches the API caller as `InvalidRequest`, not the model.

Classification: **behavioral**.

Proposed contract: two `PreToolUse` uncertainties (selector rule with
versions; Guardian direct review with a synthetic `turn_id` and no
follow-up events). No schema change: the JSON schema cannot relate
`updatedInput` to the input.

Native impact: `PreToolUseOutput::rewrite` (`protocol.rs:178`) accepts any
`serde_json::Map`, so the non-object case is already impossible. Add
rustdoc telling callers to omit or echo selector fields; optionally add a
helper that copies `environment_id`, `server`, and `session_id` from the
original `tool_input`. Session aggregation that keys on `turn_id` must
tolerate an orphan `PreToolUse`.

### `PreToolUse` and `PostToolUse`: code mode (documentation catch-up)

From CX-DOC-13. `codex-hooks.md:958-970` adds "Tool calls from code
mode": a `PreToolUse` block rejects the JavaScript tool promise before
the tool runs; an `updatedInput` runs the rewritten call and resolves
with its result; a `PostToolUse` `decision: "block"` or exit 2 rejects
the promise with the hook reason after the tool ran; `continue: false`
"uses the hook feedback for the model-visible result, but doesn't reject
the nested tool promise." The behavior is unchanged since `ff6aec9`
(upstream tests `core/tests/suite/hooks.rs@ff6aec9:4114`, `:4196`,
`:4359`, `:4368`; `registry.rs@3d86e43:1085-1100`). The doc's
`continue: false` row is imprecise for nested calls: code mode reads only
`code_mode_result()` (`code_mode/mod.rs@3d86e43:435`), and
`PostToolUseFeedbackOutput::code_mode_result` returns
`self.original.code_mode_result(payload)` (`registry.rs@3d86e43:392-394`,
identical at `ff6aec9:250-252`), so the promise resolves with the
**original** tool result and the hook's text never reaches the script.
No upstream test covers that case.

Classification: **documentation-only**. Proposed contract: add the
block and rewrite effects to `pre-tool-use`, the block/exit-2 rejection
to `post-tool-use`, and sharpen the existing `post-tool-use/contract.yaml:130`
"resolves instead of rejecting" to "resolves with the original tool
result". Native: matching rustdoc on `PreToolUseOutput::block`/`deny`/
`rewrite` (`protocol.rs:161-182`), `PostToolUseOutput` block constructors
(`:458`, `:499`), and `with_continue` (`:533`). No wire fixture can show
code mode.

### `PermissionRequest`: environment-scoped `allow` is ignored

From CX-SRC-12 and CX-ENG-7.
`hooks/src/events/permission_request.rs@3d86e43:229-239`:

```rust
output_parser::PermissionRequestDecision::Allow => {
    // Environment authority may restrict the controller's policy, never grant it.
    if !matches!(
        handler.source_path,
        crate::engine::HandlerSourcePath::Environment { .. }
    ) {
        decision = Some(PermissionRequestDecision::Allow);
    }
}
```

Since 0.165.0 (`75a4146411`, "prevent environment hooks from granting
permission requests"), an `allow` from an environment-scoped handler
(command or `mcp_tool`) leaves the run `Completed` with no decision, so
the request falls through to other hooks, Guardian, or the user. A
structured deny (`:240-247`) and exit 2 with non-blank stderr
(`:259-266`) still deny. This is the only environment-source check under
`hooks/src/events`, so an environment-scoped `PreToolUse` can still
allow and rewrite. Stdin carries no marker, so no fixture can show it.

Classification: **behavioral**. Proposed contract: a `PermissionRequest`
uncertainty; outcomes unchanged (they describe the process, not its
provenance). Native: rustdoc on `PermissionRequestOutput::allow()`
(`catalog.rs:754`).

### `SessionStart` (documentation catch-up)

From CX-DOC-11. The reference now lists `fork` (`codex-hooks.md:438`,
`:454`, `:668-671`: "A forked thread uses `source: "fork"`. Resuming a
thread with supplied history still uses `source: "resume"`.") and
documents that compact-matching `SessionStart` hooks run "before the next
model request ... also ... when automatic compaction happens in the
middle of a turn", and that `continue: false` ends the turn
(`:689-694`). The behavior existed at `ff6aec9`
(`session/turn.rs@ff6aec9:634`, `@3d86e43:822`), and subagents never run
`SessionStart` for compact (`hook_runtime.rs@3d86e43:154`). The input
schema is byte-identical.

Proposed contract: drop the clause "The hooks reference still lists only
startup, resume, clear, and compact" from
`session-start/contract.yaml:126` and the `fork` clause from the
`codex-hooks-reference` limitation; add an uncertainty for mid-turn
compact dispatch on root sessions only; add a `compact-source` positive
(the schema has `additionalProperties: false` and no `turn_id`, so the
fixture is a plain `source: compact` payload; none exists today).
Native: none (`SessionStartSource` already has `Fork`).

### `SessionEnd` (documentation catch-up and a doc-vs-source conflict)

From CX-DOC-15. `codex-hooks.md:698-702`: "It runs for the main thread
when you archive or delete a conversation that's still open, when Codex
closes normally, or after a conversation has been idle and isn't open in
any connected client for 30 minutes." and `:704-706`: "Switching away
from a conversation or calling `thread/unsubscribe` doesn't end the
session right away". The idle-unload mechanism exists at both pins
(`app-server/src/request_processors/thread_lifecycle.rs@3d86e43:34-68`;
unload shuts the thread down, and shutdown runs
`run_session_end_hooks`, `core/src/session/handlers.rs@3d86e43:420`). The
delay is **not** 30 minutes in source: `core/src/config/mod.rs@3d86e43:4324-4325`
`Duration::from_secs(cfg.thread_unload_delay_secs.unwrap_or(/*default*/ 60))`
(same at `@ff6aec9:3903-3904`), overridable in `config.toml` or per
thread (`thread_processor.rs@3d86e43:5563-5572`). The doc presumably
describes a client-chosen value.

Proposed contract: a documentation-level uncertainty listing the
triggers with `thread_unload_delay_secs` (default 60 s, configurable),
stating that unsubscribe or a client switch does not fire it immediately
and that it may never arrive if the process is killed; a
`codex-hooks-reference` limitation recording the 30 min vs 60 s
conflict. Environment-scoped hooks now also reach `SessionEnd` (see
below). Native: session-state cleanup keyed on `SessionEnd` must
tolerate late or missing delivery.

### `Interrupt`: `mcp_tool` timeout clamp (correction)

From CX-DOC-8 item 2. `engine/discovery.rs` applies
`normalize_command_hook` to `mcp_tool` handlers too, so Interrupt MCP
hooks default to 1 s clamped to 1-3 s. This holds at `ff6aec9` as well
(test `interrupt_mcp_tool_hooks_are_supported_and_timeout_is_clamped`).
`interrupt/contract.yaml:110` says only "Command handlers default to a
one-second timeout". The reference's general notes state the rule; only
its MCP field table (`codex-hooks.md:274`, "Defaults to `600`") omits it.

Classification: **correction** of the prior snapshot. Proposed contract:
"Command and `mcp_tool` handlers default to a one-second timeout clamped
to one through three seconds"; a limitation that `:274` omits the clamp.
No other Interrupt change: dispatch is still only for
`TurnAbortReason::Interrupted` on the main thread
(`core/src/tasks/mod.rs@3d86e43:1059-1062`); the only addition is
environment-scoped hooks from the last step's environments, which do not
run when the turn has no last-known step context
(`hook_runtime.rs@3d86e43:514-521`).

### All events

#### Environment-scoped managed hooks

Merged from CX-DOC-5, CX-SRC-11, and CX-ENG-2 (0.165.0: `7a5860da35`
#56302, `75a4146411` #56304, `4610b573e5` #56343; listing in 0.166.0;
output delivery and launch-in-timeout `dd2df0aa9d` #58996 in 0.168.0).

- Discovery (`engine/discovery.rs@3d86e43:533-597`
  `append_environment_hooks`): hooks from an environment attachment's
  requirements are admitted as `HookSource::System`, `is_managed: true`,
  `HookRequirement::Required`, with `env: HashMap::new()` and
  `commandWindows` chosen by the environment cwd's path convention
  (`:542-556`). They also pass `allow_managed_hooks_only` and
  memory-consolidation filtering, and a load failure is a required-hook
  load error.
- Stdin rewrite (`engine/mod.rs@3d86e43:104-119`, applied for sync and
  async handlers in `dispatcher.rs:170-185`): "/// Use the environment's
  working directory and withhold controller-owned transcript paths." `cwd`
  becomes the environment cwd in its native form (overriding any tool
  cwd), `transcript_path` becomes null, and `agent_transcript_path`, when
  present, becomes null. Stdin carries no environment-scope marker.
- Execution (`command_runner.rs@3d86e43:251-281`;
  `core/src/hook_environment_executor.rs@3d86e43:123-236`): commands run
  on the owning executor through the exec server, in the environment's
  shell. For sh, bash, and zsh the wrapper is `IFS= read -r
  codex_hook_input; printf '%s' "$codex_hook_input" | (\n{command}\n)`, so
  the hook receives the same bytes as a local hook (local hooks also get
  compact JSON with no trailing newline, `command_runner.rs:317-326`).
  PowerShell gets `[Console]::In.ReadLine() | {command}` (trailing newline
  and encoding unverified); `Cmd` is rejected ("environment command hooks
  require PowerShell on Windows"). The environment comes from the
  environment's `shell_environment_policy` without a Codex snapshot
  replay; the exec server drops `NON_INHERITABLE_ENV_VARS` and, with
  `metadata: None`, unsets `CODEX_TOOL_CALL_ID`
  (`exec-server/src/local_process.rs:790-814`). There are no plugin
  variables. stdout and stderr are each silently capped at 1 MiB
  (`MAX_HOOK_OUTPUT_BYTES`, `:35`), and one deadline covers launch, write,
  and read.
- Scope: they join every lifecycle event, including `SessionEnd` and
  `Interrupt` (which previously ran only `Local` handlers), memory-
  consolidation `Stop` (`hook_runtime.rs:446`), and model-request events
  except on memory-consolidation turns. Tool events bound to environments
  run only those environments' hooks; environment-agnostic calls
  (`ToolEnvironments::All`, MCP without an owner) run every attached
  environment's tool hooks (`hook_environment_executor.rs:66-83`,
  `tools/context.rs:89-110`). `PermissionRequest` always targets the
  action's single environment. Interrupt without a last-known step adds
  none.
- `PermissionRequest` `allow` is ignored (see above).
- Reference: the only mention is `codex-hooks.md:338`, under cloud
  orchestration ("It does not support ... environment-scoped hooks").

Classification: **behavioral** (documentation-level for the catalog).
Proposed contract: one shared uncertainty on all 14 events with the
points above; no schema change (`transcript_path` and
`agent_transcript_path` are already nullable, `cwd` is a string).
Native: `crates/hookkit-codex/src/environment.rs:1-26` and
`docs/command-environments.md` describe only local hooks; a null
`transcript_path` does not mean the session has no transcript.

#### Plugin hook approval bound to bundle contents

Merged from CX-DOC-7, CX-SRC-13, and CX-ENG-3 (`2bf2bdf82c` #54619,
first in `rust-v0.164.0`; bundled Memory hook trusted without saved
hashes since `39a7b53fa4`, `rust-v0.171.0`).

- Before every execution of a non-builtin plugin hook (command, sync or
  async, or `mcp_tool`) that has a saved approval, unless
  `--dangerously-bypass-hook-trust` is set (`discovery.rs:855-856`),
  Codex fingerprints the whole plugin root (`codex-plugin-bundle-v1`,
  SHA-256 over sorted relative paths, kinds, lengths, and contents,
  skipping only `__pycache__`, `.venv`, `*.pyc`, and `*.egg-info` at any
  depth; `plugin_fingerprint.rs@3d86e43:37-47`) inside the handler's
  timeout (`command_runner.rs:228-249`; `mcp_runner.rs:44-49`).
- Launch is refused, with no exit code and telemetry outcome
  `trust_error`, when the declaration hash or content hash differs from
  the approval ("plugin contents changed; hook approval required",
  `plugin_trust.rs:40-44`), when an entry is a symlink or special file
  ("unsupported plugin entry type", `plugin_fingerprint.rs:82-86`), when
  the root is not a real directory, a symlinked root included ("plugin
  root is not a directory", `:27-29`), when a file changes mid-scan
  (`:79-81`), or when the scan times out (command: "hook timed out after
  Ns"; `mcp_tool`: "hook trust verification timed out"). The run fails
  open like any failed run, with a visible error entry.
- Scan time is subtracted from the command or MCP timeout
  (`command_runner.rs:336-339`; `mcp_runner.rs:93-97`), which matters on
  the 1-3 s `SessionEnd` and Interrupt budgets.
- Runtime discovery now registers a plugin hook with any saved approval
  even when its declaration changed (`discovery.rs@3d86e43:928-931`,
  `|| (source.plugin_content_hash.is_none() && plugin_trust.is_some())`),
  and it fails at execution; at `ff6aec9` such a hook was skipped at
  discovery. Listing mode hashes eagerly and marks it modified. Legacy
  approvals without `trusted_content_hash` pin the contents seen at
  first execution (`plugin_trust.rs:55-68`).
- Reference: `codex-hooks.md:418-419` "If a plugin update changes a
  bundled script, Codex requires a new review even if the hook definition
  stays the same." conflicts with `:69-72` ("new or changed hooks are
  marked for review and skipped until trusted"): at runtime they are not
  skipped, they produce a failed run on every match. The MCP `timeout`
  described as an "active execution timeout" (`:274`) also includes the
  scan.

Classification: **behavioral**, documentation-level for the catalog (no
process runs, so nothing can be fixtured). Proposed contract: one shared
uncertainty; a `codex-hooks-reference` limitation for the `:69-72`
conflict; a supplement `plugin-paths` limitation that mutable state
belongs in `PLUGIN_DATA`, never under `PLUGIN_ROOT`. Native: rustdoc on
`CodexPluginEnvironment` (`environment.rs:32-39`) and README packaging
guidance (no symlinks, no build trees or logs under the plugin root,
rebuilding a bundled binary forces re-review). The repository ships no
Codex plugin manifest today, so there is nothing else to change.

#### `mcp_tool` runtime (documentation only)

Merged from CX-DOC-8 and CX-ENG-9. HookKit models only the command
binding, so these change uncertainties and limitations, not
`handler_kinds` (still `[command, mcp_tool]`, `SessionEnd` `[command]`).

- New in 0.164.0 (`cc44ffb3e2` #55263: "Wait for the selected server
  when executing an MCP hook, leaving unrelated cached servers dormant,
  and apply the hook timeout to the entire operation."): hook calls use
  `McpToolStartup::Wait` and may start a dormant cached server within
  the hook timeout (`hook_mcp_executor.rs@3d86e43:44-112`); at
  `ff6aec9` the executor passed `wait_for_server: false`
  (`hook_mcp_executor.rs@ff6aec9:36`). `codex-hooks.md:306` ("Hooks use
  an existing MCP connection. They don't start or reconnect servers.")
  is stale. Only allowlisted bundled REPL `turn_ended` cleanup skips
  dormant servers (`8afe928a76`, 0.171.0).
- New in 0.171.0 (`f993175205` #62080): bundled App hooks must target
  the host Apps server, and the connector and current policy must allow
  automatic execution; they are treated as builtin
  (`discovery.rs:823-832`).
- Unchanged but unrecorded: the hook output parser sees only the
  result's text content blocks joined with `\n`, treated as exit-0
  stdout; `isError: true` fails the run; `structuredContent` is ignored
  except for `PostModelRequest` pricing (`mcp_runner.rs@3d86e43:103-123`,
  same logic at `hook_mcp_executor.rs@ff6aec9:39-51`). Executor-scoped
  handlers, MCP included, always run async, cannot apply control effects,
  and are hidden from hook reporting (`engine/mod.rs:88-93`,
  `:196-206`), so the snapshot's "always synchronously" is imprecise for
  that source. Async `mcp_tool` handlers are otherwise impossible:
  `execution_mode` maps every non-executor `McpTool` to `Sync`.
- Environment-scoped `mcp_tool` hooks are routed by `environment_id`
  (`mcp_runner.rs:51-55`).

Proposed contract: extend the shared `mcp_tool` uncertainty with these
points; a `codex-hooks-reference` limitation citing `:306` as stale since
0.164.0. Native: none.

#### `permission_mode` values (correction)

From CX-DOC-10. Both pins derive `permission_mode` only from the
approval policy: `core/src/hook_runtime.rs@3d86e43:1079-1087` (same
function `@ff6aec9:1032-1040`), `AskForApproval::Never =>
"bypassPermissions"`, `UnlessTrusted | OnRequest | Granular(_) =>
"default"`, used by every producer (`:166`, `:215`, `:291`, `:335`,
`:439`, `:536`, `:724`). `acceptEdits`, `plan`, and `dontAsk` are never
emitted, although the reference (`codex-hooks.md:496-499`) and the
generated schemas list all five. All nine snapshot input schemas that
carry the field enumerate the five values (`pre-tool-use` keeps only
`enum`, without `type`).

Classification: **correction** (pre-existing). Proposed contract: keep
the five-value enum (generated superset) and add an uncertainty to the
nine events. Native: document on `PermissionMode`
(`protocol.rs:47-63`) that Codex currently emits only `Default` and
`BypassPermissions`, so plan mode cannot be inferred from it.

#### `session_id` is the root session identity

From CX-DOC-14. `codex-hooks.md:487`: "Subagent hooks use the parent
session id." Every producer sets `session_id: sess.session_id()`, which
"Returns the identity shared by the root thread and all descendant
threads" (`core/src/session/session.rs@3d86e43:673-676`, `@ff6aec9:649-652`),
so for nested thread-spawned agents it is the **root** id, not the
immediate parent's. Subagents are told apart by `agent_id`
(`sess.thread_id()`, `hook_runtime.rs:1101-1108`) and, on model-request
events, `thread_id`. Unchanged source; documentation catch-up. Proposed
contract: an uncertainty on every event that can fire on a subagent
thread (tool events, compaction, `UserPromptSubmit`, `SubagentStart`,
`SubagentStop`, model-request events). Native: `NativeContext.session_id`
(`protocol.rs:325`, `:634`; `catalog.rs:423`) groups subagent activity
under the root session, and `NativeContext` has no agent field
(`hookkit-core/src/context.rs:185-210`).

#### Matchers and tool coverage (documentation catch-up)

From CX-DOC-12. The reference's new "Matcher patterns" and "Tool
coverage" tables (`codex-hooks.md:421-477`) describe unchanged source.
`core/src/tools/hook_names.rs` is identical between the pins
(`apply_patch` aliases `Write` and `Edit`; `spawn_agent` alias `Agent`;
unified exec reports `Bash`). Matching moved into a precompiled
`HookMatcher` (`engine/matcher.rs@3d86e43:11-40`) with the
`ff6aec9` rules: `""` or `"*"` matches all; a pattern of only
`[A-Za-z0-9_|]` is an exact match on `|` alternatives (`Bash` does not
match `BashOutput`); anything else is an unanchored regex; an invalid
regex skips the group, which is a load failure for required managed
hooks. The reference's "regex string" (`:423`) is imprecise.
`write_stdin` never runs `PreToolUse` but can deliver the original
command's `PostToolUse`; hosted tools such as `WebSearch` bypass tool
hooks (`:465-474`). Matchers are ignored for `UserPromptSubmit`, `Stop`,
`Interrupt`, and both model-request events.

Proposed contract: tool-event uncertainties for canonical names versus
matcher-only aliases, unified exec as `Bash`, delayed `PostToolUse`, and
excluded hosted tools; a shared exact-versus-regex matcher uncertainty.
Native: none in parsing; generated matchers should use plain names
deliberately, and stdin `tool_name` stays `apply_patch` when matched as
`Edit`.

#### Output spilling (documentation catch-up)

From CX-DOC-9. Unchanged since `ff6aec9` apart from provenance metadata,
but never recorded. Model-visible `additionalContext` above the handler's
`additionalContextLimit` (default 2,500 approximate tokens,
`output_spill.rs@3d86e43:13`; 0 disables spilling) is written to
`<temp_dir>/hook_outputs/<thread_id>/<uuid>.txt`, and the model gets a
head/tail preview with the path, or a plain truncation if the write fails
(`:70-91`). The limit is honored only for `PreToolUse`, `PostToolUse`,
`SessionStart`, `UserPromptSubmit`, and `SubagentStart`; other events
warn and ignore it (`discovery.rs:695-711`). Stop and SubagentStop
continuations and tool feedback always use the default
(`output_spill.rs:106-113`). The reference says `<session_id>`
(`codex-hooks.md:543`); the source keys the directory by the registry's
thread id (`command_runner.rs:94`, `Hooks::new(thread_id)`), which
differs in subagents.

Proposed contract: an uncertainty on the text-context and structured
outcomes of the five events and on Stop/SubagentStop block reasons; a
`<session_id>` vs thread id limitation. Native: rustdoc on the context
and block-reason builders (`protocol.rs:183`, `:212`, `:441`, `:484`;
`catalog.rs` `context_text_event!`).

#### Configuration, trust, and managed-hook prose (documentation only)

From CX-DOC-16 and CX-DOC-17. The trust review and `/hooks`,
`--dangerously-bypass-hook-trust`, managed hooks trusted by policy,
`allow_managed_hooks_only` (`discovery.rs@3d86e43:108-115`; the new
early return for plugin sources is equivalent), `managed_dir` and
`windows_managed_dir`, linked-worktree project hooks, the dual-format
warning, the `hooks` feature key, and the optional top-level hooks.json
`description` (still `config/src/hook_config.rs@3d86e43:14-16`) all
exist at both pins. `HookEventsToml`, `MatcherGroup`, and
`HookHandlerConfig` moved to `protocol/src/hooks.rs` and are re-exported,
unchanged except for the two new event keys. The cloud-orchestration
rewrite (`codex-hooks.md:338-340`: "only Global MCP hooks", no
environment-scoped or `SessionEnd` MCP hooks, "A `PreToolUse` error,
timeout, or malformed response can let the tool run") is consistent with
the snapshot; optionally refine that uncertainty with those words. The
file now ends with "### Plain-text aliases" and the single line
"- string | null" (`:1157-1159`), a renderer glossary with no protocol
meaning.

## Command-process environment

Merged from CX-DOC-18, CX-ENG-4, CX-ENG-5, and the verifiers' notes.
Environment changes do not alter event JSON; they belong in the successor
command-environment supplement.

- **New events.** `append_plugin_hook_sources` still inserts
  `PLUGIN_ROOT`, `CLAUDE_PLUGIN_ROOT`, `PLUGIN_DATA`, and
  `CLAUDE_PLUGIN_DATA` for every plugin-discovered handler whatever its
  event (`discovery.rs@3d86e43:312-320`), `HookEventsToml` accepts both
  model-request keys (`protocol/src/hooks.rs:10-11`, `:113-136`), and
  model-request handlers run through the same
  `dispatcher::execute_handler` path (`dispatcher.rs:242-247`). Map
  `PreModelRequest` and `PostModelRequest` to `[plugin-paths]`; plugin
  hooks are dropped only on memory-consolidation turns.
- **Credential scrub list: five to nine names.**
  `protocol/src/shell_environment.rs@3d86e43:19-29`:
  `CODEX_EXEC_SERVER_NOISE_AUTH_TOKEN`, `NODE_REPL_AUTH_TOKEN`,
  `CODEX_GUARDIAN_DECISIONS_API_KEY` (`rust-v0.162.0`, `9552906b2b`
  #50019), `OPENAI_FEDERATION_RULE_ID`, `OPENAI_IDENTITY_TOKEN_FILE`,
  `OPENAI_IDENTITY_TOKEN_SOURCE`, `OPENAI_SPIFFE_AUDIENCE`,
  `OPENAI_SPIFFE_ID` (the last three `rust-v0.171.0`, `6242064da0`
  #61207), `OPENAI_WORKLOAD_IDENTITY_CONTEXT`. `SPIFFE_ENDPOINT_SOCKET`
  is defined (`:15`) but not scrubbed. The local runner still removes
  them case-insensitively from the replay and the handler variables
  (`command_runner.rs@3d86e43:488-499`), legacy notify does the same
  (`registry.rs:424`), and the exec server applies the list to
  environment-scoped hooks. The selected supplement
  (`sources.yaml:117`), `crates/hookkit-codex/src/environment.rs:6-10`,
  and `docs/command-environments.md:147-151` still say five.
- **`CODEX_TOOL_CALL_ID`** (`rust-v0.163.0`, `9b738582b1` #52119;
  `shell_environment.rs:8`, `:37-48`) labels command processes: set by
  `exec_command` (`core/src/unified_exec/process_manager.rs:1494`), by
  exec-server launch metadata (`exec-server/src/local_process.rs:795-815`),
  and for code-mode nested executions; shell snapshots exclude it. Hooks
  never set it. A local hook sees it only ambiently through the
  `std::env::vars_os()` snapshot (`hooks/src/registry.rs:84`), naming an
  outer call; an environment-scoped hook has it unset. Hook tool-call
  identity is the JSON `tool_use_id`.
- **Handler config location.** The `codex-config-source` limitation
  (`sources.yaml:128`) cites `src/hook_config.rs`; at `3d86e43`
  `HookHandlerConfig` lives in `codex-rs/protocol/src/hooks.rs:148-188`
  and still has no `env`.
- **Local only.** The replayed-snapshot and scrub rules
  (`sources.yaml:106`, `:117`) describe locally executed hooks.
  Environment-scoped hooks get the environment's shell environment
  policy, no plugin variables, and the exec server's scrub. The existing
  `plugin-paths` activation ("command hook was discovered from a Codex
  plugin", `supplement.yaml:603`) already excludes them, so no new
  profile is needed.
- **Plugin trust.** Writing under `PLUGIN_ROOT` invalidates the hook's
  own approval (see "All events").

Proposed supplement (for example `command-environments-2026-11-25-r1`,
shared with the other harness successors): retarget Codex to
`commit-3d86e43-r1`; add the two event profiles (Codex 12 to 14 mapped
events); move `codex-hooks-source`, `codex-protocol-source`, and
`codex-config-source` to revision `3d86e43`; record the new reference
hash citing `724881d8…c630` per `contracts/MAINTENANCE.md`; list the nine
scrubbed names with versions and note `SPIFFE_ENDPOINT_SOCKET`; extend
the "not hook identity" limitation with `CODEX_TOOL_CALL_ID`; limit the
replay and scrub limitations to local hooks; add the plugin-trust
limitation to `plugin-paths`.

## Main-only and pre-release changes (not pinned)

From CX-DOC-19 and CX-SRC-14. These `codex-rs/hooks` commits are on
`main` `87de5e93` and only in alpha tags; they must not be encoded in a
snapshot pinned to `rust-v0.172.0`. None changes the generated schemas,
and the reference mentions none of them.

- `a1afa12273` (#62831, from `rust-v0.173.0-alpha.3`): per-handler
  `onFailure: continue|block` for command and `mcp_tool` hooks. With
  `block`, a failed synchronous run (non-zero exit, timeout, invalid
  output, input-serialization failure) takes the event's blocking path
  with "<Event> hook failed. This hook is configured to block
  continuation on failure.": `PreToolUse` block, `PermissionRequest`
  deny, `PostToolUse` withheld result, `Stop`/`SubagentStop` block plus
  continuation, `stop_reason` for `UserPromptSubmit`, `SessionStart`,
  `SubagentStart`, and compaction, and a no-effect `Blocked` on
  Interrupt. Discovery rejects `block` for `SessionEnd`, the
  model-request events, and async commands; executor-scoped hooks
  downgrade it; it is part of the trust hash. At `rust-v0.172.0`,
  `HookHandlerConfig` has no `onFailure` and no `deny_unknown_fields`,
  so the key is silently ignored.
- `65d69f25b4` (#62559, from `rust-v0.173.0-alpha.1`): overflowing
  model-request price sums are rejected before posting (`HookCharge`
  becomes an enum).
- `5b10c107ba` (#63474, from `rust-v0.174.0-alpha.1`): synchronous MCP
  `SessionStart` hooks may return `codex/encryptedContent` text blocks,
  forwarded as encrypted `function_call_output` (cap 40,000 bytes).
- `ff146c344d` (#63766, `rust-v0.174.0-alpha.5`): structured
  dynamic-tool results in code mode; "Matching post-tool-use hooks can
  receive the structured result for successful calls", so `PostToolUse`
  `tool_response` may change shape.
- `98835f5eba` (#63523): prompts accepted before an interrupted turn
  setup are preserved, and an interrupted blocking start hook is retried
  on the next turn.
- `17b00277cc`: `BUILD.bazel` only.

Re-check at the next stable tag, especially `onFailure: block` (Codex
would fail closed on a HookKit exit 1, so the `failure` outcome of every
synchronous event except `SessionEnd` and the model-request events would
become configuration-dependent) and the `PostToolUse` `tool_response`
shape. Do not emit `onFailure` from HookKit templates.

## Corrections to the selected snapshot and earlier audits

- **Exit-0 stderr.** "Codex never reads stderr from a command that exits
  0" (every contract; 2026-09-29 audit; `hookkit-codex/src/lib.rs:9-14`)
  must be scoped to the original 12 events.
- **`permission_mode`** values never include `acceptEdits`, `plan`, or
  `dontAsk` (pre-existing).
- **Interrupt `mcp_tool`** handlers share the 1-3 s clamp
  (pre-existing).
- **"`mcp_tool` handlers ... always synchronously"** is imprecise for
  executor-scoped handlers (pre-existing).
- **Code-mode `continue: false`** resolves with the original tool result,
  not merely "instead of rejecting".
- **`codex-hooks-reference` limitations**: drop the `fork` clause; add
  the model-request field lag, the stale MCP server-start sentence, the
  Interrupt MCP timeout omission, the trust "skipped" conflict, the
  `<session_id>` spill directory, and the SessionEnd 30 min vs 60 s
  conflict.
- **Supplement**: the scrub list, the handler-config citation, and the
  local-only scope of the replay rules (see above).

## Evidence and classification

| Source | Bytes | SHA-256 (2026-11-25) | Recorded by `commit-ff6aec9-r2` |
| --- | --- | --- | --- |
| `learn.chatgpt.com/docs/hooks.md` | 49306 | `37cdbfc3b9d19925f88af3e680399fd61bd3d15979961302160f97698cc5a841` | 46965, `724881d81f1a9fdb35ea42d4851a11b1f6606bbc6e9d2d05c1e8c466fc25c630` |
| `developers.openai.com/codex/hooks.md` | 49306 | identical bytes | identical bytes |
| `model-request.command.input.schema.json` at `3d86e43` | 179 lines | `5182c45dad9a6f6ece6ded590052323ccd80aaa9ae3649e04a107936fc7a13b0` | not present |
| other 23 generated schemas at `3d86e43` | | byte-identical to the `ff6aec9` vendor tree | vendored at `ff6aec9` |

The successor is classified as **additive** for the wire: every input
and output that validated against `commit-ff6aec9-r2` still validates,
and the only new wire is the two model-request events. The tool-event
`cwd` value, environment-scoped hooks, the `PermissionRequest`
provenance filter, the selector rule, Guardian direct review, and plugin
trust are **behavioral**. Output spilling, code mode, matchers and tool
coverage, the SessionStart and SessionEnd prose, `session_id`, and the
configuration prose are **documentation-only**. `permission_mode`
values, the Interrupt MCP clamp, and the exit-0 stderr scope are
**corrections**. The native Rust change is additive: `Event`,
`AnyInput`, and `AnyCommandOutput` are `#[non_exhaustive]`.

`sources.yaml` re-reviewed files for the successor: `hooks/src/lib.rs`,
`events/model_request.rs`, `events/common.rs`,
`events/{pre_tool_use,permission_request,post_tool_use}.rs`,
`engine/{mod,discovery,dispatcher,command_runner,mcp_runner}.rs`,
`engine/{plugin_trust,plugin_fingerprint,matcher}.rs`,
`output_spill.rs`, `core/src/hook_runtime.rs`,
`core/src/hook_runtime/sampling.rs`, `core/src/client/response_delivery.rs`,
`core/src/hook_environment_executor.rs`, `core/src/hook_mcp_executor.rs`,
`core/src/tools/{registry,approvals,context,sandboxing}.rs`,
`core/src/tools/handlers/unified_exec/exec_command.rs`,
`core/src/session/rollout_budget.rs`, and
`protocol/src/{shell_environment,hooks,protocol,error}.rs`.

`FROZEN_LEDGER` already records `commit-ff6aec9-r2`
(`xtask/src/main.rs:2156-2158`) and `command-environments-2026-09-30-r2`
(`:2181-2183`). Deselecting them needs no new entry, and existing
entries must not be edited; only the frozen successor snapshot and
supplement need entries.

## Items checked and unchanged

- **Existing wire.** The 22 generated input and output schemas of the 12
  existing events are byte-identical between `ff6aec9` and `3d86e43`
  (and on `main`); no input field, type, optionality, or enum changed
  for `SessionStart` (sources `startup|resume|clear|compact|fork`),
  `SessionEnd` (`reason` `other`), `SubagentStart`, `PreToolUse`,
  `PermissionRequest`, `PostToolUse`, `PreCompact`/`PostCompact`
  (`trigger` `manual|auto`), `UserPromptSubmit`, `SubagentStop`, `Stop`,
  or `Interrupt`.
- **Output parsing.** `engine/output_parser.rs`, `engine/schema_loader.rs`,
  `legacy_notify.rs`, and `declarations.rs` have no diff. Blank-reason
  rejection, the `PreToolUse` unsupported fields, `PostToolUse`
  `continue: false` precedence (LCAE-M1), `PermissionRequest` reserved
  fields discarding the whole decision (LCAE-4; the reference still says
  "fail closed", `:901-903`), the Interrupt `deny_unknown_fields` output,
  text-context behavior, and the Stop/SubagentStop block-reason rule all
  hold. The `events/*.rs` diffs for the 12 events only switch to
  `serde_json::to_value`, add test fields, and attach context metadata;
  the only parse change is the environment-scoped `allow` filter.
- **Exit codes.** Exit 2 with non-blank trimmed stderr still blocks or
  continues only on `PreToolUse`, `PermissionRequest`, `PostToolUse`,
  `UserPromptSubmit`, `Stop`, and `SubagentStop`; other non-zero exits
  fail open; failure stderr is reported only for `PreCompact`,
  `PostCompact`, and `SessionEnd` (`compact.rs:313-319`,
  `session_end.rs:110-117`).
- **Compaction stop.** `PreCompact`/`PostCompact` `continue: false` still
  returns `TurnAborted`, reported as interrupted (LCAE-5;
  `compact.rs:201-231`, `compact_remote_v2.rs:207-261`,
  `compact_token_budget.rs:50-72`).
- **Interrupt** dispatch, input, output, timeout, and ignored matcher,
  apart from environment-scoped hooks.
- **Timeouts.** 600 s default (minimum 1) everywhere except `SessionEnd`
  and Interrupt (1 s, clamped 1-3 s), `discovery.rs:957-981`.
- **Async command hooks.** `SessionEnd` forced synchronous, at most eight
  concurrent per session (`command_runner.rs:54`), only `Context`,
  `Warning`, and `Error` entries delivered. The async path now goes
  through `execute_handler`, so plugin trust and environment routing also
  apply to `async: true` hooks; semantics are otherwise unchanged.
- **Local process.** `$SHELL -lc` or `%COMSPEC% /C`, new session or Job
  Object, SIGKILL of the group on timeout, I/O failure, or drop,
  descendants kept after a clean exit, no output cap
  (`command_runner.rs:283-419`, `:448-501`); compact single-line stdin
  with no trailing newline, then EOF (`:317-326`). Stdin is now built
  with `serde_json::to_value(...).to_string()`; the shipped binary pulls
  in `codex-tui`, which enables serde_json `preserve_order`
  (`tui/Cargo.toml:108`), so key order still follows struct order.
- **Step settings.** Tool-event `model` and `permission_mode` still come
  from the step's captured settings or the review context
  (`hook_runtime.rs@3d86e43:208-215`, `:285-292`, `:329-335`).
- **Memory-consolidation Stop** filtering is behavior-identical (moved to
  `ConfiguredHandler::runs_for_memory_consolidation`, `Unknown` kept
  fail-closed).
- **`UserPromptSubmit` `prompt`** is still the concatenation of text parts
  (`ResolvedUserInput::as_text` collect versus the old `join("")`).
- **Delayed `PostToolUse` via `write_stdin`** existed at `ff6aec9`; only
  its cwd retention is new.
- **Handler kinds and config fields.** `command` and `mcp_tool`
  supported; `prompt` and `agent` parsed and skipped; `mcp_tool` on
  `SessionEnd` a load failure; no `env` setting; TOML-representable MCP
  input validation.
- **Built-in executor-scoped cleanup hooks** still run async, are hidden,
  and run only when no synchronous handler blocked
  (`dispatcher.rs:160-169`, `:205-229`).
- **Cloud orchestration**: command hooks still not dispatched.
- **`PostToolUse` `tool_input`/`tool_response`** shapes at the tag.

## Open questions and uncertainties

- **Derived schema openness.** This note recommends keeping the
  model-request inputs open and `attempt_number` at `minimum: 0`, as
  vendored, and recording the closed struct and one-based numbering as
  uncertainties. The alternative is a source-derived
  `additionalProperties: false` and `minimum: 1` (origin `derived`,
  assurance `source-reviewed`), which would make a Post-only member on a
  `PreModelRequest` a negative. Decide before freezing.
- **Value-rule narrowing.** Whether to add `if`/`then` rules for
  `PostModelRequest` (`cancelled` means all three members null;
  `completed` means `error` null and `response_id` a string) as
  source-derived narrowing like the block-reason rules. Recommended: no,
  record them as uncertainties, because Codex is the producer and the
  rules are not schema-enforced upstream.
- **Open value sets.** `kind` already grew in 0.171.0. Recommended: keep
  the vendored schema enums for `kind` and `status`, parse both into
  open native enums with `Unknown` arms, and list `/kind` (both events)
  and `/status` (Post) in `OPEN_VALUE_SET_NEGATIVES`.
- **Workspace roots.** Should `hookkit-codex` stop seeding
  `NativeContext.workspace_roots` from tool-event `cwd`, and expose it as
  a command directory, deriving project roots from turn-scoped events?
  Recommended: yes, see the implementation plan.
- **Shell cwd origin.** Whether `hookkit-shell` should gain an origin such
  as "harness-reported command cwd unless remote" for Codex, or keep
  `UnverifiedFallback` (recommended for now, because remote commands
  report the host cwd with no marker).
- **PowerShell environment hooks.** `[Console]::In.ReadLine() | {command}`
  likely adds a trailing newline and, on Windows PowerShell 5.1, may use
  a non-UTF-8 `$OutputEncoding`; a script-block hook receives pipeline
  input, not stdin. Unverified; record as an uncertainty, not a rule.
- **Environment hooks and `CODEX_THREAD_ID`.** With `metadata: None` the
  exec server unsets `CODEX_TOOL_CALL_ID`; whether a `CODEX_THREAD_ID`
  from the environment's policy survives was not verified.
- **Background compaction ordering.** With `AsyncCompaction` enabled,
  `background_compaction` pairs run before `PreCompact` runs at adoption,
  possibly for discarded candidates. Feature-gated and off by default;
  mention only in the model-request uncertainty for now.
- **Environment-scoped `PreToolUse`** can still allow and rewrite while
  `PermissionRequest` allow is filtered. Whether that asymmetry is
  intended upstream is unknown; record it, do not model it.
- **Live observation.** No model-request run, environment-scoped hook, or
  plugin trust failure has been observed live. A safe live matrix should
  confirm exit-0 stderr visibility, ignored JSON stdout, the 60 s idle
  `SessionEnd`, and the trust failure before HookKit marks the
  model-request events native in `contracts/status/stabilization-v1.yaml`.

## Claims narrowed or rejected during verification

No finding was refuted outright. These claims did not survive as stated:

- **"Local replay and credential scrubbing are unchanged"** (CX-DOC-18):
  the scrub list grew from five to nine names.
- **"Add a FROZEN_LEDGER entry for `commit-ff6aec9-r2` when deselecting
  it"** (CX-DOC-20): the entry already exists; adding one would duplicate
  it. The same finding's list of snapshot-id literals was incomplete
  (see the plan).
- **"Vendor the new schema and add its MANIFEST entry"** (CX-SRC-1,
  CX-SRC-7): already done in `ff3ab73`.
- **"schemars drops `additionalProperties`"** (CX-SRC-7): the struct never
  had `deny_unknown_fields`.
- **"A HookKit binary on these events under FailClosed exits 2"**
  (CX-DOC-1 verifier caveat): on the selected-harness path the wire name
  is identified, so it lowers to `NotGating` and exit 1; exit 2 occurs
  only when no invocation can be read.
- **"Single-line stdin without a trailing newline" as an
  environment-hook difference** (CX-DOC-5): local hooks get the same
  framing. The "empty conditional profile" for environment hooks is
  unnecessary; `plugin-paths` is already conditional.
- **"`mcp_tool` handlers are always synchronous"** (CX-SRC-4):
  executor-scoped handlers run async and cannot price.
- **PostModelRequest EOF error text** (CX-SRC-2): the wire value is the
  `Display` string "stream disconnected before completion: stream closed
  before response.completed".
- **`write_stdin` fixture `tool_input.cwd` as a native path** (CX-SRC-9):
  it is a `file://` URI.
- **"Interrupt MCP clamp" and "text blocks joined" as new drift**
  (CX-DOC-8): both pre-date `ff6aec9`; the text goes to the hook output
  parser, not the model.
- **"SessionEnd after 30 minutes idle"** (CX-DOC-15): the source default
  is 60 s and configurable.
- **"Only pre-tool-use and permission-request enumerate
  `permission_mode`"** (CX-DOC-10): all nine schemas do.
- **"`description` moved to `protocol/src/hooks.rs`"** (CX-DOC-16): it is
  still in `config/src/hook_config.rs`; only handler types moved. The
  same finding's "no contract-relevant source change" missed plugin
  content trust.
- **MCP `environment_id` selector since 0.165.0** (CX-ENG-8): since
  0.170.0. "Previously the hooks did not run on that path": the Guardian
  review API itself is new in 0.167.0.
- **`CODEX_TOOL_CALL_ID`'s "only caller"** (CX-ENG-5): the exec server
  and code mode also set or clear it.
- **"A changed plugin bundle is skipped until re-trusted"** (one
  verifier note): at runtime the hook is registered and fails on each
  run; only listing mode marks it modified. The reference's "skipped"
  wording is recorded as a limitation.
- **`onFailure` scope** (CX-SRC-14): it reaches every synchronous event
  except `SessionEnd` and the model-request events, not only
  `PreToolUse` and `PermissionRequest`.
- **Citation fixes**: `PermissionRequestOutput::allow()` is at
  `catalog.rs:754`, not `:643`; there is no "classic shell" handler at
  `3d86e43` (CX-SRC-8); the plugin-manifest packaging work in CX-DOC-7
  has no target in this repository.
