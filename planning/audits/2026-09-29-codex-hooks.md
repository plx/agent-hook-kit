# Codex hook contract refresh

- Audit date: 2026-09-29
- Previous selected snapshot: `commit-1e59dc5-r1` (openai/codex `1e59dc5bdaa30c5cc8a488753b3a89c18f77c1bc`, first stable release `rust-v0.147.0`)
- Successor snapshot: `commit-ff6aec9-r1`
- Pinned upstream revision: `ff6aec96948b70d94983af2641a6b67c94faeff5` (tag `rust-v0.159.2`, the latest stable release on the audit date)
- Official hook reference: <https://learn.chatgpt.com/docs/hooks.md> (<https://developers.openai.com/codex/hooks.md> serves identical bytes)
- Vendored schemas: `contracts/vendor/codex/ff6aec96948b70d94983af2641a6b67c94faeff5/generated`

This audit re-read the current hook reference, diffed `codex-rs/hooks` and the hook call sites in `codex-rs/core` from the previous pin to the latest stable release, compared every generated schema byte for byte, and folded in the verified findings of the `codex-schemas-events` and `codex-engine-env-docs` drift studies. Corrected verdicts were applied as corrected; nothing was refuted.

## Pin selection

The successor pins the release tag rather than `main`. At audit time `main` was `9ef9cb1d9fc6013f6c1994346e0ee93ad9e6f986`. Its `codex-rs/hooks/schema/generated` tree is byte-identical to `rust-v0.159.2`, and `codex-rs/hooks/src` differs only by `bd4204efc2` ("Compile hook matchers during discovery", present only in `rust-v0.161.0-alpha.3`), which keeps the match-all, exact-name, pipe-alternative, and regex semantics. The other post-tag hooks commit, `fbc169827e`, was already backported to 0.159.2 as #49385. The hooks reference also says to use the page, not `main` schemas, as the release behavior reference.

## Sources and hashes

| Source id | Reference | Evidence |
| --- | --- | --- |
| `codex-hooks-reference` | `https://learn.chatgpt.com/docs/hooks.md` | SHA-256 `724881d81f1a9fdb35ea42d4851a11b1f6606bbc6e9d2d05c1e8c466fc25c630` of the raw `curl -sL` bytes (46,965 bytes, `text/markdown`), stable across repeated fetches; `content-hash-only` |
| `codex-generated-schemas` | `codex-rs/hooks/schema/generated` at `ff6aec9` | 23 files vendored with `MANIFEST.sha256`; `vendored` |
| `codex-hooks-source` | `codex-rs/hooks` at `ff6aec9` | `pinned-revision` |
| `codex-core-source` | `codex-rs/core` at `ff6aec9` (new) | `hook_runtime.rs`, `session/session.rs`, `tasks/mod.rs`; `pinned-revision` |

The rendered HTML page, `https://learn.chatgpt.com/docs/hooks`, returned 513,086 bytes with SHA-256 `5163d2e08d4774991ee06a01aaf19fbe15f3c7769887bf9910225837b85f52d9`. The successor cites the Markdown endpoint because it has no site chrome. The previous snapshot's documentation hash, `b5cf66c8…`, cannot be reproduced; the nearest Wayback capture, from 2026-08-02, decodes to `cf450a1f…`. OpenAI documentation is not vendored because it carries no redistribution license.

Of the 23 vendored schema files, 20 are byte-identical to `1e59dc5`. The other three are:

- `interrupt.command.input.schema.json`, new, `be37402db3524543efe1e4058e6b9a3a53f7dd8f7791a9b75bb2accf7842196e`;
- `interrupt.command.output.schema.json`, new, `d9a3fe9ed8ffd9e920d8bf1e49d2c1af49cf63b0c3cf526e568511dc998f824a`;
- `session-start.command.input.schema.json`, `690c0eef…` → `54168cf0bb3641bbc55dcdc58aa3803651d4f24499339061f3e9bb0ef9095633` (adds `fork`).

The vendor refresh script had been stuck at `9e552e9`, and its file list omitted `session-end.command.input.schema.json`. It now pins `ff6aec9` and lists the complete generated directory. The `1e59dc5` and `9e552e9` vendor trees stay in place for the frozen snapshots derived from them.

## Result

The command-event inventory grows from 11 to 12 events. `Interrupt` is new. `SessionStart.source` gains `fork`. No other input field, output field, or enum value changed, and all baseline events keep their payloads, including optional `agent_id`/`agent_type` on tool, compaction, and `UserPromptSubmit` events and the constant `SessionEnd.reason = "other"`.

The successor also corrects process-outcome modeling that was already wrong at `1e59dc5` (see below). Every prior outcome and fixture ID is retained, so existing conformance cases map one-to-one.

| Event | Wire change since `1e59dc5` | Successor contract change | Class |
| --- | --- | --- | --- |
| `SessionStart` | `source` adds `fork` (≥ 0.155.0); resume with supplied history now reports `resume`, not `startup` | enum + `fork-source` positive, `unknown-source` negative; docs-lag uncertainty | additive; breaking for consumers that close over the old enum |
| `SessionEnd` | none | exit-0 stdout modeled as ignored; visible-stderr `failure` outcome | ledger correction |
| `SubagentStart` | none | `no-op`, `failure` outcomes | ledger correction |
| `PreToolUse` | `cwd` is the step-local environment cwd (≥ 0.157.0); `model`/`permission_mode` come from step settings (≥ 0.156.0) | non-blank deny and legacy block reasons in the schema; `agent_id`/`agent_type` named; `context` and `legacy-block` official examples | behavioral + ledger correction |
| `PermissionRequest` | same cwd/model/mode change (review context) | `no-op`, `failure`; `allow` official example | behavioral |
| `PostToolUse` | same cwd/model/mode change | non-blank block reason unless `continue: false` | behavioral + ledger correction |
| `PreCompact` / `PostCompact` | none | `no-op`, visible-stderr `failure` | ledger correction |
| `UserPromptSubmit` | none | non-blank block reason; `context` and `block` official examples | ledger correction |
| `SubagentStop` | none | non-blank block reason | ledger correction |
| `Stop` | managed Stop hooks also fire for memory-consolidation turns (≥ 0.150.0) | non-blank block reason; memory-consolidation and interrupt uncertainties | behavioral + ledger correction |
| `Interrupt` | new (≥ 0.150.0) | complete event directory | additive |

### Interrupt

`Interrupt` runs when an active main-thread turn is aborted as interrupted. It runs after Codex flushes the transcript and before it emits `TurnAborted`. It never runs for subagents or idle threads, and configured matchers are ignored. The pinned core source and integration tests also dispatch it for self-aborts reported as interrupted, such as a manual compact stopped by a `PreCompact` `continue: false`; the reference describes only user interrupts.

- Input (all required): `session_id`, `turn_id`, `transcript_path` (string or null), `cwd`, `hook_event_name: "Interrupt"`, `model`, and `permission_mode`. It has no agent fields.
- Output: empty stdout, or a JSON object whose only member is an optional string `systemMessage`, surfaced as a warning. The output wire is `deny_unknown_fields`, so `continue`, `stopReason`, `suppressOutput`, and `decision` fail the run, and so does plain text.
- Exit codes: every non-zero exit, including 2, is a failure with the stderr discarded.
- Timeout: defaults to one second and is clamped to one through three seconds, including for async handlers.
- Handlers: `mcp_tool` handlers are supported.

The runtime also accepts `systemMessage: null`, but the generated schema declares a string, so the contract keeps the string.

## Process-outcome corrections

These were true at `1e59dc5` as well and are fixed in the successor rather than being upstream drift:

- **Exit-0 stderr is never read.** Every stderr read in `codex-rs/hooks` sits in a non-zero-exit arm, so exit-0 outcomes now model stderr as `role: ignored`, `content_kind: opaque` instead of `diagnostics`.
- **Non-zero exits fail open.** A new `failure` outcome (exit 1–255, effect `nonblocking-error`) covers them. `PreCompact`, `PostCompact`, and `SessionEnd` report the trimmed stderr as the failure message (`role: diagnostics`). Every other event records only `hook exited with code N` and discards stderr (`role: ignored`).
- **Exit 2 needs non-blank stderr.** For events where exit 2 blocks or continues (`PreToolUse`, `PermissionRequest`, `PostToolUse`, `UserPromptSubmit`, `Stop`, `SubagentStop`), exit 2 selects the blocking outcome only when trimmed stderr is non-empty. The `exit-2-blank-stderr` process fixture maps exit 2 with `" \n"` stderr to `failure`. For the other events, `exit-2-failure` shows that exit 2 has no special meaning.
- **Exit-2 stdout is ignored, not forbidden.**
- **Empty stdout is an explicit `no-op` outcome everywhere.** `SessionEnd` ignores all exit-0 stdout.
- **Blank block reasons are rejected.** Codex fails a synchronous run whose `decision: block` has a missing or whitespace-only `reason` (unless `continue: false` takes precedence), and a `PreToolUse` `permissionDecision: deny` without a non-blank `permissionDecisionReason`. The output schemas add these rules as `if`/`then` constraints with `pattern: "\\S"`, and `output_negative` fixtures pin them. A `PermissionRequest` deny with a blank message is not rejected; Codex substitutes a default message. The rest of the parsed-but-unsupported surface (for example `ask`, `approve`, and `PreToolUse` `continue: false`) stays in the schemas as vendored and is described in uncertainties.
- **Text context must not look like JSON.** Codex treats any stdout whose first non-whitespace character is `{` or `[` as JSON. This is now recorded for the `text-context` outcomes of `SessionStart`, `SubagentStart`, and `UserPromptSubmit`.

## Handler, binding, and runtime notes (documentation only)

The following are recorded as `handler_kinds`, uncertainties, or here, without claiming HookKit implementation:

- **Async command hooks** (≥ 0.148.0; previously skipped with a warning).
  - `async: true` runs in the background for every event except `SessionEnd`, which is always synchronous.
  - Control effects are ignored, exit 2 is a plain failure, and only `additionalContext` and `systemMessage` are delivered, at the next safe point. At most eight run concurrently per session.
  - Stdin carries no execution-mode marker, so every outcome in the successor describes synchronous handlers.
- **`mcp_tool` handlers** (≥ 0.148.0/0.149.0).
  - They take `server`, `tool`, an `input` template with `${field.path}` expansion, `timeout`, and `statusMessage`.
  - They always run synchronously, their tool results are parsed like exit-0 stdout, and they are rejected on `SessionEnd`.
  - `handler_kinds` is `[command, mcp_tool]` for every event except `SessionEnd`, which is `[command]`. `prompt` and `agent` handlers are still parsed but skipped.
- **Cloud orchestration**: command, prompt, and agent handlers, and hooks from local configuration or plugins, are not dispatched when a Work thread uses cloud orchestration; only admin-managed remote `mcp_tool` hooks run there.
- **Process lifecycle**:
  - Since 0.155.0 hooks start in a new session and process group without a controlling terminal (a Job Object with `CREATE_NO_WINDOW` on Windows), and the timeout also covers stdin delivery; stdin and output are pumped concurrently.
  - On timeout, I/O failure, or turn abort (after a 100 ms grace), Codex SIGKILLs the whole process group.
  - Descendants survive once the leader exits and its output pipes close within the timeout. A descendant that holds stdout or stderr open makes the run time out.
- **Tool-hook scope**: `PreToolUse`, `PermissionRequest`, and `PostToolUse` receive the step-local environment cwd, which may differ from the turn cwd reported by `SessionStart`, `Stop`, or `Interrupt`. The reference still says "session cwd".
- **Memory-consolidation Stop**: internal memory-consolidation turns run only managed-source and executor-scoped Stop hooks. The payload is an ordinary Stop payload, and a block ends the memory worker with an error.
- **Required managed hooks**: since 0.148.0 an invalid matcher, empty command, or unsupported handler in a required managed hook fails session startup.
- **Built-in cleanup hooks**: allowlisted bundled-plugin MCP cleanup hooks for `Stop`, `SubagentStop`, and `Interrupt` are trusted and hidden from hook listings. They run asynchronously only when executor-scoped.
- **Legacy `notify`**: the command now gets the same environment replay and credential scrub.
- **Unchanged**: output spilling (~2,500 approximate tokens by default, `additionalContextLimit`), the 600-second default timeout, matcher semantics, and concurrent launch of matching hooks.

## Command-process environment

Environment changes do not alter event JSON. They belong to the next command-environment supplement revision, which must also map `Interrupt`:

- `Interrupt` gets the same `plugin-paths` profile as every other event (`PLUGIN_ROOT`, `PLUGIN_DATA`, `CLAUDE_PLUGIN_ROOT`, `CLAUDE_PLUGIN_DATA`, set only for plugin-discovered hooks).
- Since 0.149.0 hooks receive a replay of the environment Codex captured when the session hook registry was created, not the live environment. Non-plugin handler configuration still cannot set variables.
- `CODEX_EXEC_SERVER_NOISE_AUTH_TOKEN`, `NODE_REPL_AUTH_TOKEN`, `OPENAI_FEDERATION_RULE_ID`, `OPENAI_IDENTITY_TOKEN_FILE`, and `OPENAI_WORKLOAD_IDENTITY_CONTEXT` are removed case-insensitively from both the snapshot and the overrides.
- `CODEX_SESSION_ID` and `CODEX_THREAD_ID` are set only for shell-tool processes, not hooks.
- Plugin paths can change between turns of one session after a plugin refresh.

## Classification

- **Additive**: the `Interrupt` event, `SessionStart.source = "fork"`, and new optional fixtures and official output examples.
- **Behavioral**: tool-event cwd/model/permission-mode sourcing, resume-with-history reporting `resume`, async execution, memory-consolidation Stop dispatch, and process-lifecycle changes.
- **Breaking for strict consumers**: the old closed `source` enum rejects `fork`. HookKit currently maps unknown sources to `Startup`, so forks are misrecorded. The narrowed output schemas reject blank block reasons that the previous catalog accepted but Codex always failed.
- **Documentation-only**: `mcp_tool` handlers, cloud orchestration, managed-requirement failures, built-in cleanup hooks, and legacy `notify` environment handling.

The successor was frozen with `cargo xtask contracts freeze codex commit-ff6aec9-r1`. Selecting it in `contracts/registry.yaml`, the successor command-environment supplement, and the matching Rust changes are left to the integration step.
