# Claude Code hook contract refresh: `docs-2026-11-25-r1`

- Audit date: 2026-11-25
- Previous selected snapshot: `docs-2026-09-30-r1` (Claude Code 2.1.285;
  selected in `620d5f1`)
- Proposed successor: `docs-2026-11-25-r1`, **draft (not yet frozen or
  selected)** when this note was written; since frozen and selected (see
  "Final status")
- Proposed pin: Claude Code 2.1.332, the changelog head at
  `anthropics/claude-code@d66368052c9303ce022c017a5b59d62ebd57d1e7` and the
  npm `latest` tag on the audit date (npm `stable` is 2.1.326)
- Official hook reference: <https://code.claude.com/docs/en/hooks.md>
- Corroborating types: `@anthropic-ai/claude-agent-sdk` 0.3.332 (`sdk.d.ts`),
  compared with the pinned 0.3.285
- Selected supplement: `command-environments-2026-09-30-r2`; it needs a
  successor that targets `docs-2026-11-25-r1`
- Native crate: `crates/hookkit-claude` (implements `docs-2026-09-30-r1`)

The reference body pinned on 2026-09-30 (`5129d583…`) was hashed but never
captured, and `web.archive.org` is unreachable from this environment. Drift
in the reference was therefore found by comparing today's text with what
`docs-2026-09-30-r1` encodes (schemas, fixtures, outcomes, uncertainties,
and `sources.yaml` limitations) and with the 2026-09-29 and 2026-09-30
audit notes. Where a statement has no changelog entry, this note says
whether it is new behavior, a documentation catch-up, or a snapshot-time
error, and how sure that call is. The changelog was diffed from the pinned
`2282079d` (through 2.1.285) to `d6636805` (through 2.1.332), and the Agent
SDK `sdk.d.ts` from 0.3.285 to 0.3.332 (with 0.3.326 and 0.3.331 checked
for version boundaries). Two minified CLI builds, the `claude` binaries in
`@anthropic-ai/claude-agent-sdk-linux-x64` 0.3.285 and 0.3.332, were
grepped to settle questions the reference and the SDK leave open; that
evidence is marked "CLI bundle" and is never the only support for an
outcome this note proposes at more than `low` confidence.

Doc line numbers refer to the staged copy of `hooks.md` (`cc-hooks.md`,
hash below); changelog line numbers refer to `CHANGELOG.md` at `d6636805`,
with the release section in brackets. Native citations are to `HEAD`
(`1ea6068`); `crates/hookkit-runtime/src/failure.rs` has uncommitted
Antigravity edits in the working tree that shift its lines by about four,
so its citations also name the symbol.

Four audit slices (shared process rules `CC-SHARED`, per-event reference
text `CC-A`, changelog `CC-CL`, and SDK types `CC-SDK`) produced 70 kept
findings. Each was adversarially verified. None was refuted outright, but
many were narrowed or corrected, and the slices overlapped heavily. This
note uses the verified wording, merges duplicates, re-checks the verifiers'
"missed" items against the evidence, and records the narrowed claims at the
end. A fifth slice on the 22 events `PermissionDenied` through
`ElicitationResult` (`CC-B`, 17 findings: 10 confirmed, 7 modified, none
refuted, plus 8 items its verifier found missing) was verified separately
and merged after the main synthesis. Nine of its findings duplicate items
the other slices had already carried (Notification values, `overloaded`,
`continue: false` over exit 2, PreCompact instructions, the PostCompact and
SessionEnd split, `onFailure`, `agent_id`, `background_tasks`, the
PermissionDenied wording). Its new or corrected items are marked `CC-B-n`
or "CC-B missed" in the sections below: the `TaskCompleted` and
`TeammateIdle` top-level block, corrections to the missing-file rule, idle
compaction, Elicitation remainders, subagent matchers, worktree firing
rules, and firing-scope notes for six events. Those changes are applied in
the draft and planned in `plan-claude-code-b.json`.

## Pin selection

The 2026-09-29 refresh pinned the changelog head (2.1.285) while npm
`stable` was 2.1.280, and this note follows that precedent: the proposed
pin is 2.1.332. On 2026-11-25 the npm dist-tags are `latest` = `next` =
2.1.332 and `stable` = 2.1.326. The pin matters in three places:

- `PreSkillShellCommand` exists only from 2.1.332 (changelog `:6`). On
  2.1.331 and older, a settings file naming it opens a dialog at start.
- `onFailure` on `mcp_tool` hooks (2.1.328), Code Mode's removal of
  `UserPromptSubmit` firing (2.1.328), and the PermissionDenied `retry`
  wording (2.1.331) are above `stable`.
- The fetched reference cites versions up to v2.1.328; `code-mode.md`
  cites v2.1.329 and v2.1.331. It never mentions `PreSkillShellCommand`,
  so the reference lags the 2.1.332 release.

Every encoded behavior newer than 2.1.326 carries its version in an
uncertainty, so a consumer on `stable` can tell what does not apply. If
review prefers to pin `stable`, drop the `PreSkillShellCommand` event and
record it as a post-pin addition in `sources.yaml` limitations; nothing
else in this note depends on the choice.

## Sources and hashes

| Source id | Reference | Evidence |
| --- | --- | --- |
| `claude-hooks-reference` | `https://code.claude.com/docs/en/hooks.md` | SHA-256 `194272edd023a34be015d351c402c3ca166d84ebfd9b50215b204f09553e3099` (293,618 bytes), stable across retrievals on 2026-11-25; previously `5129d58370ed4d7910c6e86537e0592b2902ea5bf0a8682721c5484f5d46f566` (248,570 bytes); `content-hash-only` |
| `claude-code-changelog` | `https://raw.githubusercontent.com/anthropics/claude-code/d66368052c9303ce022c017a5b59d62ebd57d1e7/CHANGELOG.md` | SHA-256 `4192cfdae7e1401a199984dcc4910cdb8491550eadb2150a541d55d4464aa5ad` (1,423,161 bytes, through 2.1.332); previously `2282079d…` / `0937ba0c…f217` (through 2.1.285); `pinned-revision` |
| `claude-agent-sdk-types` | `https://registry.npmjs.org/@anthropic-ai/claude-agent-sdk/-/claude-agent-sdk-0.3.332.tgz` | tarball SHA-256 `622bbf059f41467d249f3d5b49ebbb5b79d8e655f0a148f1b26e1a9860865aa0`; `package/sdk.d.ts` `5041abd88b30c03567f960ca0214b4622adf86147474785f4cc3f7cd8cfa1c0d`; previously 0.3.285 (`5c68fdbf…` / `3bc04bbd…`); `pinned-revision`, secondary |
| proposed `claude-code-cli-bundle` | `https://registry.npmjs.org/@anthropic-ai/claude-agent-sdk-linux-x64/-/claude-agent-sdk-linux-x64-0.3.332.tgz` | tarball SHA-256 `e56d25934eb652425f593f784b949c9d6ae0817b9b0cb1b495acbfa934fb99ee` (sha1 matches npm `dist.shasum`); minified CLI 2.1.332; 0.3.285 build (`06b4e450…`) used for the PreCompact comparison; see "Open questions" on whether to cite it |
| proposed `claude-code-mode` | `https://code.claude.com/docs/en/code-mode.md` | SHA-256 `3ee4d085c4cdacede22e9d314b74ba78c0a8dc520dbca0682863c73ece4c4881` (32,787 bytes); new source |
| proposed `claude-permissions` | `https://code.claude.com/docs/en/permissions.md` | SHA-256 `687a80f8ae55138a03648fbd54fa1c109291656a512a8ed28c1c3f2825460615` (94,797 bytes); new source for allow exceptions |
| supplement `claude-environment-reference` | `https://code.claude.com/docs/en/env-vars.md` | SHA-256 `30cd22fe95a40ced8d175d368d5e915441014255f40965e681d2a3dcbdff317e` (194,781 bytes); previously `f49dad1f…` (156,688 bytes) |

Other pages fetched for corroboration (`curl --fail --silent --show-error
--location --compressed`, 2026-11-25): `mcp.md` `785996c2…`, `skills.md`
`a4e18f19…`, `tools-reference.md` `fed59082…`, `settings-reference.md`
`f87b28f0…`, `agent-sdk/hooks.md` `5ac6b7a5…`. The supplement's remaining
Claude sources all changed hash and are listed under "Command-process
environment".

The pinned `sources.yaml` limitations that change:

- `claude-agent-sdk-types`: "type UserPromptSubmit source and
  session_title inputs that the reference does not document" is now half
  wrong; the reference documents `session_title` (`:1548`). `source` is
  still SDK-only.
- `claude-code-changelog`: "no hook change follows 2.1.285" is superseded.
- The SDK limitation should add that 0.3.332 calls `onFailure`
  `'continue' (default)` (`sdk.d.ts:7960`), which the reference contradicts
  on Windows without Git Bash and in Code Mode.

## Result

The documented inventory is unchanged at 33 events (`cc-hooks.md:1283-3919`,
33 `### <Event>` sections). With the proposed pin, the successor adds a
34th, **`PreSkillShellCommand`**, sourced from the changelog and the SDK
because the reference does not document it. No event was removed or
renamed, and SDK 0.3.285 → 0.3.332 changes no existing event's input or
`hookSpecificOutput` type. The headline changes are:

- **`onFailure` (`continue` | `block`) on command and HTTP hooks (2.1.295),
  Agent SDK callbacks (2.1.317), and `mcp_tool` hooks (2.1.328).** With
  `block`, a hook that cannot start, times out, exits with a code other
  than 0 or 2 (even with schema-valid allowing JSON), or prints unparseable
  or schema-invalid JSON takes the event's exit-2 effect, and on
  `PermissionRequest` denies. Every failure outcome in the snapshot
  silently assumes the field is unset.
- **Unset is not always `continue`.** On Windows without Git Bash, a
  shell-form `PreToolUse` or `PermissionRequest` command hook fails closed
  (2.1.299, 2.1.309, 2.1.310); in Code Mode a `PreToolUse` timeout refuses
  the call (2.1.329).
- **Exit-2 exceptions newly documented** in the shared exit-code section: a
  missing-file exit 2 on `Stop`, `SubagentStop`, `TaskCompleted`, and a
  plugin's `UserPromptSubmit` is a non-blocking error; `continue: false`
  beats exit 2 on `Stop` and `SubagentStop`; `Elicitation` and
  `ElicitationResult` ignore `hookSpecificOutput` on every nonzero exit. The
  CLI builds show all three already applied at 2.1.285.
- **Corrections of the selected snapshot.**
  - `PreCompact` plain-text stdout becomes compaction instructions (true at
    2.1.285 too).
  - JSON-lines output with no field set is plain text on the 27
    non-text-context events, not invalid JSON.
  - `PreToolUse` deprecated top-level `decision` is still mapped, but the
    schema rejects it.
  - `TaskCompleted` and `TeammateIdle` honor a documented top-level
    `decision: "block"`, which their output schemas rejected.
- **Per-event semantics** that the reference now spells out or that
  changed: `updatedInput` without a decision, `allow` exceptions and
  multi-hook precedence, Bash matchers covering Monitor commands (2.1.314),
  managed `PostToolUse` firing on failed calls, `UserPromptSubmit` running
  before skill expansion (2.1.299) and receiving `session_title`, Code
  Mode's `REPL` tool, and `MessageDisplay`'s single non-interactive call.
- **Command environment:** no variable added or removed for the existing
  events; every Claude doc source the supplement cites changed hash, and
  `CLAUDE_ENV_FILE` is unavailable to hooks that run in PowerShell.

| Event | Wire or behavior change since `docs-2026-09-30-r1` | Successor contract change | Class |
| --- | --- | --- | --- |
| `PreSkillShellCommand` (new) | new guard event (2.1.332); changelog and SDK only | complete event directory, `confidence: low`; supplement profile | additive |
| the 30 events where `onFailure` applies | `onFailure: block` turns failures into the exit-2 effect | `command-on-failure-block` / `http-on-failure-block` bindings on 14 gating events; uncertainty elsewhere | behavioral |
| 27 non-text-context events | none (rule now explicit) | `plain-text` covers JSON-lines with no field; new process cases | correction |
| `PreToolUse` | `updatedInput` validation (2.1.300), Monitor (2.1.314), allow exceptions, Code Mode `REPL`, Windows default | rewritten `updatedInput`, timeout, and `continue` uncertainties; deprecated `decision` accepted; positives | behavioral, correction |
| `PermissionRequest` | Windows default; Monitor; Cowork limits; `mcp_skill` ask (2.1.332) | uncertainties; `mcp_server` description widened | behavioral |
| `UserPromptSubmit` | runs before expansion (2.1.299); `session_title` documented; missing-file exit 2 for plugins; Code Mode (≤2.1.327) | `session_title` property; `exit-2-missing-file` uncertainty; firing uncertainties | additive, behavioral |
| `PostToolUse` | managed hooks on failed calls (string `tool_response`); interrupt budget (2.1.319); Cowork exclusions | uncertainties; string-response positive | behavioral |
| `PostToolBatch` | refused calls appear unmarked (stated in 2.1.328) | uncertainty; synthetic positive | documentation |
| `PreCompact` | plain-text stdout is compaction instructions, cut at 10,000 chars (2.1.314) | `compaction-instructions` outcome replaces `plain-text` | correction |
| `Stop`, `SubagentStop` | missing-file exit 2 and `continue: false` over exit 2 (both at 2.1.285 already); Monitor tasks typed `monitor` (2.1.329); SubagentStop plugin-scoped `agent_type`, web-fetch managed-only, repeated firing fixed (2.1.311) | `exit-2-missing-file`, `exit-2-continue-false` outcomes; `background_tasks` examples; matcher and firing uncertainties; `plugin-agent` positive | correction, documentation |
| `TaskCompleted`, `TeammateIdle` | top-level `decision: "block"` documented (honored at 2.1.285); missing-file exit 2 (TaskCompleted documented, TeammateIdle CLI only); `agent_id` (2.1.290) | top-level `decision`/`reason` accepted; `block` examples and process cases; TaskCompleted `exit-2-missing-file`; positives | correction |
| `Elicitation`, `ElicitationResult` | `hookSpecificOutput` ignored on any nonzero exit; `decision` now documented; ElicitationResult runs on user decline and cancel, skipped after an Elicitation answer, `content` replaces the whole object | `nonzero-json-ignored` replaces `nonzero-structured`; uncertainties rewritten; decline/cancel positives; official output | correction, documentation |
| `PostCompact`, `SessionEnd` | `terminalSequence` fires on any exit; SessionEnd budget ignores plugin timeouts | nonzero split; budget uncertainty | documentation (resolves uncertainty) |
| `PreCompact`, `PostCompact` | `auto` also covers idle compaction (2.1.299 terminal, 2.1.314 VS Code and Desktop); proactive vs recovery block | `trigger` description; uncertainties | documentation |
| `Notification` | `auth_storage_failure` (2.1.288), `plugin_notification`; `idle_prompt` timing (2.1.288 fix) | examples; timing uncertainty | additive |
| `SubagentStart` | plugin-scoped `agent_type`; web-fetch managed-only | uncertainty; `plugin-agent` positive | documentation |
| `ConfigChange`, `CwdChanged`, `DirectoryAdded`, `PreModelSwitch`, `PostModelSwitch` | firing scope spelled out (non-firing saves, main-conversation-only `cd`, `/add-dir` exclusions, model-switch triggers) | uncertainties | documentation |
| `StopFailure` | reference drops `overloaded`; 529 reports `server_error` | uncertainty; enum kept | documentation |
| `MessageDisplay` | one call per message in non-interactive runs | uncertainty; positive | documentation |
| `SessionStart`, `InstructionsLoaded`, `PreCompact`, `PostCompact`, `TaskCreated`, `TeammateIdle` | `agent_id`/`agent_type` sent in more cases (2.1.288, 2.1.290, 2.1.323) | positives; uncertainties | additive |
| `PermissionDenied` | `retry` message narrowed (2.1.331) | uncertainty | behavioral (wording only) |
| `WorktreeCreate`, `WorktreeRemove` | symlink refusal on EnterWorktree since 2.1.325; plugin hooks with `--worktree` since 2.1.300; `terminalSequence` conflict; new trigger contexts, kept subagent and workflow worktrees, ExitWorktree `discard_changes`, background-delete checks, failure reporting no longer in the debug log | uncertainties | correction, documentation |
| `Setup` | reference now says command-only | uncertainty rewritten | documentation |

## Event inventory change

`docs-2026-09-30-r1/snapshot.yaml` lists 33 events. The proposed successor
adds:

- `events/pre-skill-shell-command` (`wire_name: PreSkillShellCommand`,
  `rust_key: pre_skill_shell_command`, `category: tool`, `inferability:
  definitive`, discriminator `/hook_event_name` const
  `PreSkillShellCommand`).

Evidence: changelog `:6` [2.1.332] "Added a `PreSkillShellCommand` hook
event that can refuse a skill or command's inline shell commands as it
loads; on 2.1.331 and older, a settings file naming it opens a dialog at
start"; SDK 0.3.332 `sdk.d.ts:1036` (`HOOK_EVENTS`), `:1069` (`HookEvent`),
`:1071` (`HookInput`), `:2959-2998` (input and `hookSpecificOutput`),
`:10723` (the `SyncHookJSONOutput` union). It is absent from SDK 0.3.285,
0.3.326, and 0.3.331, from `cc-hooks.md` (0 matches), and from `skills.md`,
`agent-sdk/hooks.md`, and `hooks-guide.md`. The expected
`cargo xtask contracts diff claude-code/docs-2026-09-30-r1
claude-code/docs-2026-11-25-r1` result is 1 event added, 0 removed, every
existing event changed (uncertainty text at minimum), and `sources.yaml`
changed.

## Confirmed drift: shared process model

### `onFailure` (all events except the Stop family)

Merged from CC-SHARED-1, CC-A-1, CC-CL-2, CC-SDK-4, and the `onFailure`
correction to CC-CL-20.

Reference evidence:

- `cc-hooks.md:458` (command), `:510` (http): "| `onFailure` | no |
  Whether the action that triggered the hook goes ahead when the hook
  fails: `"continue"` or `"block"`. … Requires Claude Code v2.1.295 or
  later |"; `:550` (mcp_tool).
- `:940`: "To block the action instead, set `"onFailure": "block"` on a
  `command`, `http`, or `mcp_tool` hook. Requires Claude Code v2.1.295 or
  later. On `mcp_tool` hooks, `onFailure` requires v2.1.328 or later."
- `:979-986`, the failure list: "**Can't start**"; "**Exit code other than
  0 or 2**: counts for a command hook even if it printed JSON that allows
  the action, such as `permissionDecision: "allow"`. To return a JSON
  decision, exit 0"; "**HTTP error**"; "**MCP failure**" (on gating events
  "a non-empty answer also counts unless it's `{}` or JSON that sets a hook
  output field other than `reason`"); "**Timeout**"; "**Invalid output**:
  the JSON output can't be parsed or fails schema validation. For an HTTP
  hook, a 2xx body that is neither empty nor a JSON object also counts.
  Plain-text stdout from a command hook isn't a failure".
- `:988`: "With `"block"` set, a failure does what exit code 2 does on that
  event, except on `PermissionRequest`, where it denies the request."
- `:990-993`: no effect on "`Stop`, `SubagentStop`, `TaskCompleted`, and
  `TeammateIdle` hooks" or on "Background command hooks: command hooks that
  set `async` or `asyncRewake`".
- `:813` (exit-code table, other nonzero code with valid JSON): "For hooks
  whose failure blocks the action, the run counts as a failure instead".
- `:906`: a timed-out PreToolUse hook doesn't block "except for hooks whose
  failure blocks the action"; `:1534` says the same for UserPromptSubmit;
  `:1074` for HTTP.
- `:970-977`: the message becomes `failed; blocking because onFailure is
  "block"` (or `timed out`) followed by the program's output.

Changelog: `:3250` [2.1.295] "Added `onFailure: "block"` for command and
HTTP hooks: a hook that can't start, times out, or exits with an unexpected
code blocks the action instead of letting it through"; `:2069` [2.1.317]
Agent SDK callbacks; `:575` [2.1.328] `mcp_tool`. SDK 0.3.332 types
`onFailure?: 'continue' | 'block'` on command (`sdk.d.ts:7958-7962`), http
(`:8061-8065`), and mcp_tool (`:8111-8115`) handlers and on callback
matchers (`:1053-1063`), plus `hook_on_failure_supported` in the initialize
response (`:5132-5134`). Command and http `onFailure` and callback
`onFailure` are already in 0.3.326; the mcp_tool field first appears in
0.3.331. CLI bundle 2.1.332: `Mkr=new
Set(["Stop","SubagentStop","TaskCompleted","TeammateIdle"])` is the ignore
set; `Bkn` also ignores mcp_tool `onFailure` on `SessionStart` and `Setup`;
`fXo` adds `permissionRequestResult:{behavior:"deny"}` on
PermissionRequest.

Discrepancies to record, not to encode:

- SDK `:7960` says `'continue' (default)` with no platform exception; the
  reference treats unset differently on Windows without Git Bash and in
  Code Mode (next two sections).
- SDK `:8114` adds `Setup` and `SessionStart` to the mcp_tool ignore list.
  The CLI agrees with the SDK; the reference simply omits it. This matters
  after `/clear` or compaction, when SessionStart mcp_tool hooks do run.
- SDK `:8061-8065` (http) repeats the command text "exited with a code other
  than 0 or 2", a copy error.
- SDK `:8113` "Any other value reads as 'block'" appears only in the SDK.
- The SDK's "counts as exit code 2, so the action the event guards (a tool
  call, a permission request, a prompt) is blocked" is loose wording, not a
  contradiction of `:988`: it names the permission request, and the CLI
  denies there.

Snapshot: every standard-model contract's `invalid-json`,
`nonzero-structured` (28 contracts), `nonzero-unstructured`, HTTP
`non-json-body`, and HTTP `non-success` outcomes describe `onFailure` unset
or `continue`. The PreToolUse uncertainty "A timed-out command, http, or
mcp_tool hook does not block the tool call." (`pre-tool-use/contract.yaml`,
generated at `generate-claude-contracts.rs:693`) is now conditional.
`contracts/meta/v1` cannot key an outcome on handler configuration:
`Binding` and `Outcome` are `deny_unknown_fields`
(`xtask/src/main.rs:292-326`), though binding ids are free-form.

Classification: behavioral (2.1.295 for command and http; 2.1.328 for
mcp_tool).

Proposed contract change (format v1, no metadata extension):

- Every command and http binding on the 30 events where the field applies
  gains an uncertainty: "These outcomes describe `onFailure` unset or
  `continue`. With `onFailure: block` (v2.1.295+; mcp_tool v2.1.328+;
  ignored for `async`/`asyncRewake`), a start failure, a timeout, an exit
  code other than 0 or 2, or invalid output takes this event's exit-2
  effect; see `command-on-failure-block`." On the four exempt events
  (`Stop`, `SubagentStop`, `TaskCompleted`, `TeammateIdle`) the uncertainty
  says the field is ignored.
- Sibling bindings `command-on-failure-block` (kind `process`) and
  `http-on-failure-block` (kind `http`) on the 14 events where block
  changes an outcome: `PreToolUse`, `PermissionRequest`,
  `PreSkillShellCommand`, `UserPromptSubmit`, `UserPromptExpansion`,
  `PostToolUse`, `PostToolUseFailure`, `PostToolBatch`, `TaskCreated`,
  `PreCompact`, `PreModelSwitch`, `ConfigChange`, `Elicitation`, and
  `ElicitationResult`. On the other 16 events where the field applies the
  exit-2 row is the same user notice or log line as a non-blocking error
  (or, for the worktree events, any failure already fails), so they get
  the uncertainty only, stating that block changes nothing there.
- Outcomes of `command-on-failure-block`: `structured`, `no-op`,
  `plain-text`/`text-context`/`compaction-instructions`, `exit-2`, and
  `exit-2-structured` copied from `command`; `invalid-json` takes the
  event's exit-2 effect; `nonzero-structured` and `nonzero-unstructured`
  merge into `failure-blocked` (exit 1-255, any stdout; exact `exit-2`
  takes precedence) with the exit-2 effect: `deny` (PreToolUse,
  PreSkillShellCommand, UserPromptSubmit, UserPromptExpansion, PreCompact,
  PreModelSwitch, ConfigChange), `deny` as a decision (PermissionRequest,
  not exit-2's `ignored`), `provide-context` with stderr agent-context
  (PostToolUse, PostToolUseFailure), `stop` (PostToolBatch), `rollback`
  (TaskCreated), `decline` (Elicitation, ElicitationResult). The stderr
  role follows the event's exit-2 row; the message carries the
  `failed; blocking because onFailure is "block"` prefix.
- Outcomes of `http-on-failure-block`: `structured` and `empty-body` copied;
  `non-json-body`, `non-success`, and the new `schema-invalid-body` (next
  section but one) take the exit-2 effect.
- Uncertainties on both: start failure and timeout cannot be expressed by
  an exit or status selector and also take the exit-2 effect; the message
  prefix; whether a schema-valid JSON object printed with exit 1 still
  applies its fields alongside the failure (undocumented).
- Rewrite the PreToolUse timeout uncertainty to: "A timed-out command,
  http, or mcp_tool hook does not block the tool call unless the hook sets
  `onFailure: block`, or in Code Mode with `onFailure` unset (v2.1.329+),
  or on Windows without Git Bash for a shell-form command hook. An Agent
  SDK callback that times out blocks the call (`cc-hooks.md:1801`)."
  Add the same shape to `UserPromptSubmit` (`:1534`, `:1536`), which has
  no timeout uncertainty today.
- Status overlay: default `harness: claude-code`, `binding:
  command-on-failure-block`, `level: catalog-only`; `http-on-failure-block`
  `unsupported`.
- Generator: a shared `ON_FAILURE` uncertainty constant and a
  `Seed::on_failure(effect)` that emits the sibling binding.

A `handler_options` qualifier on `Outcome` would express this more
directly but needs a meta and xtask format change; it is listed under
"Open questions".

Native impact (no emitted bytes change; HookKit typed constructors never
pair JSON with a nonzero exit other than 2,
`crates/hookkit-claude/tests/snapshot_contract.rs:942-990`):

- `nonblocking_error!` rustdoc (`crates/hookkit-claude/src/catalog.rs:1072-1084`,
  "The action proceeds and Claude does not see the text") is false under
  `block` on the 11 gating events that use it: PreToolUse (`:1988`),
  PermissionRequest (`:1787`), UserPromptSubmit (`:2207`),
  UserPromptExpansion (`:2165`), PostToolBatch (`:1485`), PreCompact
  (`:1461`), ConfigChange (`:1437`), TaskCreated (`:1609`), Elicitation
  (`:1721`), ElicitationResult (`:1749`), and PreModelSwitch
  (`model_switch.rs:176`). On PostToolUseFailure (`catalog.rs:1514`) and
  `PostToolUseOutput::nonblocking_error` (`protocol.rs:676-680`) it is
  false in a weaker way: stderr reaches Claude, nothing is blocked.
  `SessionStartOutput::nonblocking_error` (`protocol.rs:282-287`) and the
  other notice events stay correct.
- `crates/hookkit-claude/src/lib.rs:36-46` says `nonblocking_error` is a
  notice and that Claude Code reads JSON on every exit code; add that a
  JSON decision must exit 0 under `onFailure: block`.
- `crates/hookkit-runtime/src/failure.rs`: the module docs (`:12-17`) and
  `FailurePolicy::NonBlocking` (`:74-89`) describe fail-open, and
  `FailureResponse::fails_closed` comes only from `nonzero_exit_blocks`
  (`CLAUDE_NONZERO_GATES`, the worktree events). Under `block` exit 1
  blocks. Document that these describe `onFailure` unset or `continue`
  and that `fails_closed` is a lower bound; recommend `"onFailure":
  "block"` alongside `FailurePolicy::FailClosed`, because only the harness
  can fail closed on a missing binary, a timeout, or `panic = "abort"`.
  `crates/hookkit-runtime/src/aligned.rs:13-16` repeats the fail-open
  claim.
- README, template catalog, and scaffold settings snippets for policy hooks
  should show `"onFailure": "block"` and exec form (`args`). HookKit writes
  no settings and has no typed handler-config model; do not emit
  `onFailure` from templates unless it is modeled.

Synthetic fixtures (process cases on `command-on-failure-block` unless
noted):

- PreToolUse `on-failure-block-nonzero-allow-json`: exit 1, stdout
  `{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow"}}`
  → `failure-blocked` (deny). Control: the same bytes on `command` →
  `nonzero-structured`.
- PreToolUse `on-failure-block-nonzero-unstructured`: exit 1, stderr
  `boom` → `failure-blocked`.
- PreToolUse `on-failure-block-invalid-json`: exit 0, stdout `{not json}`
  → `invalid-json` (deny); `{"continue":"yes"}` → `invalid-json` (deny).
- PreToolUse `on-failure-block-plain-text`: exit 0, stdout
  `Hook finished.\n` → `plain-text` (no-op). Negative: plain text is not a
  failure.
- PermissionRequest `on-failure-block-nonzero`: exit 1, empty stdout →
  `failure-blocked` (deny decision).
- PostToolUse: exit 1, stderr `lint crashed` → `failure-blocked`
  (provide-context). PostToolBatch: exit 1 → `stop`.
- HTTP PreToolUse `http-on-failure-block-error` (500), `…-non-json` (200,
  `ok`), `…-schema-invalid` (200, `{"continue":"yes"}`) → deny.
- Generator test: no `command-on-failure-block` binding on `Stop`,
  `SubagentStop`, `TaskCompleted`, `TeammateIdle`, or the 16 other events.
- `hookkit-runtime` unit test documenting that
  `failure_response(NonBlocking, claude-code, PreToolUse)` exits 1 and
  reports `fails_closed() == false`, with a doc note that `onFailure:
  block` makes it block.

### Unset `onFailure` on Windows without Git Bash

Merged from CC-SHARED-2, CC-A-2, CC-CL-3, and the verifiers' PowerShell
exit-2 note.

- `cc-hooks.md:946`: "On Windows without Git Bash, a shell-form
  `PreToolUse` or `PermissionRequest` command hook that fails with
  `onFailure` unset can instead block the tool call."
- `:1009`: "Without Git Bash, any exit code other than 0 or 2 from that hook
  counts as a failure, even when it prints JSON that doesn't deny the
  call." `:1011`: add `"onFailure": "continue"` to let the call go ahead.
  `:1014`: exec form (`args`) without `"onFailure": "block"` "can go ahead
  when the program fails". `:1017`: the pre-v2.1.309 message named the
  `shell` setting.
- `:457`: `shell` "Defaults to `"bash"`, or to `"powershell"` on Windows when
  Git Bash isn't installed … Ignored when `args` is set".
- Changelog `:3108` [2.1.299] hooks that set no shell; `:2534` [2.1.309]
  `"shell": "powershell"` too, and "Fixed hooks that run in PowerShell not
  blocking when the hook's program exits with code 2"; `:2480` [2.1.310]
  commands that are only quoted strings; `:2167` [2.1.316] PowerShell exit 2
  with a leading type cast, `param`, `using`, or attribute.
- CLI bundle 2.1.332 has the message `blocking because Git Bash wasn't
  found, so only exit code 0 or 2 counts as this hook's verdict`; the
  string is absent from the 2.1.285 build.

Classification: behavioral (platform-conditional default). Before 2.1.309,
exit 2 from any PowerShell-run hook did not block, so on Windows every
exit-2 outcome needs 2.1.309+ (2.1.316+ for the leading-cast cases).

Proposed contract change: a PreToolUse and PermissionRequest uncertainty
stating the rule, its versions, that exec form and explicit `continue`
restore the base `command` outcomes, and that the condition selects the
`command-on-failure-block` outcomes. A shared uncertainty (on every event
with an exit-2 outcome) for the PowerShell exit-2 floor. A `sources.yaml`
limitation for the SDK's unconditional `'continue' (default)`.

Native impact: HookKit binaries registered in shell form on such hosts turn
`FailurePolicy::NonBlocking` and `PreToolUseOutput::nonblocking_error` /
`PermissionRequestOutput::nonblocking_error` into gates. Add a platform
caveat to the `nonblocking_error` docs for those two events, to
`failure.rs` `failure_response` docs ("Claude Code and Codex proceed with
the pending action"), and to the README: register HookKit binaries in exec
form (`"command": "<hook>.exe", "args": []`) and set `onFailure`
explicitly. No code change.

Fixtures: the bytes cannot encode the platform, so the cases are the
`command-on-failure-block` ones above, referenced from the uncertainty.

### Code Mode (`PreToolUse`, `PostToolUse`, `UserPromptSubmit`)

Merged from CC-CL-4 and the verifiers' Code Mode notes.

- `code-mode.md:180`: "write a `PreToolUse` hook that matches `REPL`, the
  tool that runs the code, and read the JavaScript from `tool_input.code`";
  `:174`, `:178`: tool calls made from code reach PreToolUse under their
  own names, and "a `git push` in a folder named `app` can reach your hook
  as `cd 'app' && git push`".
- `:251-297`: a PostToolUse hook matching `REPL` "runs when background code
  finishes and when code throws an error"; `tool_response` carries `code`,
  `result`, `stdout`, `stderr`, `toolsUsed`, and optionally `error`,
  `images`, `documents`, `imagePagesFailed`, `uncutOutputNote` (omit on
  v2.1.331+), and `refusedCalls`.
- `:366-370`, on v2.1.329 or later for command and http PreToolUse hooks:
  `onFailure` not set → timeout "Refused", other failure "Goes on to
  permission checks"; `"block"` → both refused; `"continue"` → both go on.
  `:355-360`: the refusal texts `the hook did not finish` and `the hook
  failed`.
- `cc-hooks.md:910`, `:997`: "If you use Code Mode, timeouts on
  `PreToolUse` work differently."
- `cc-hooks.md:1530`, `:1579`; `code-mode.md:308`, `:379`: before v2.1.328
  UserPromptSubmit also ran on code output (input `source: poll_event`) and
  a blocking `reason` reached Claude's context.
- Changelog `:1409` [2.1.323] Code Mode added (experimental opt-in);
  `:696` [2.1.328]; `:546` [2.1.329]. Between 2.1.323 and 2.1.328 any
  PreToolUse hook error on a call from code refused the call.

`REPL` never appears in `cc-hooks.md`. SDK 0.3.285 already mentioned
"inner REPL calls" in the `classifierContext` docs, so the tool name
predates the public release; SDK 0.3.332 drops that phrase.

Classification: behavioral (2.1.323-2.1.329). No schema change:
`tool_input` and `tool_response` are open.

Proposed contract change: PreToolUse and PostToolUse uncertainties naming
`REPL` and the shapes above, the `cd '<dir>' && ` prefix, and the Code Mode
failure table; a UserPromptSubmit uncertainty for the 2.1.323-2.1.327
window. Cite `code-mode.md` as a source.

Native impact: none to parsing. `hookkit-shell` Bash analysis should
tolerate the `cd 'x' && ` prefix (document it; the parser already handles
`&&` lists).

Fixtures: PreToolUse positive `code-mode-repl` (`tool_name: "REPL"`,
`tool_input: {"code":"(await $\`cat notes.txt\`.text())"}`); PostToolUse
positive `code-mode-repl-result` (`tool_response:
{"code":"…","result":"deploy token: abc123","stdout":"","stderr":"","toolsUsed":["Bash"]}`);
PreToolUse positive `code-mode-cd-prefix` (`tool_input:
{"command":"cd 'app' && git push"}`); UserPromptSubmit positive
`poll-event-source` (`source: "poll_event"`).

### Exit 2 with a missing-file message

Merged from CC-SHARED-3, CC-A-3, CC-B-5 (modified), and the CC-B
verifier's TeammateIdle note.

- `cc-hooks.md:821`: "**`Stop`, `SubagentStop`, `TaskCompleted`, and a
  plugin's `UserPromptSubmit` hook**: when your hook exits 2 with nothing on
  stdout and its stderr says a file is missing, such as `No such file or
  directory`, Claude Code treats the run as a non-blocking error."
- `:992`: "Claude can't repair a hook that won't run".
- Changelog `:786` [2.1.327] "Fixed a Stop hook that failed to run being
  shown as having reported a problem" changes only how the run is shown;
  it is not the rule's origin (below).
- CLI bundle: the 2.1.285 build (`gFn`) and the 2.1.332 build (`b_o`,
  exported as `exitTwoMeansMissingScript`) both contain
  `(e==="Stop"||e==="SubagentStop"||e==="TaskCompleted"||e==="TeammateIdle"||Boolean(pluginId)&&e==="UserPromptSubmit")&&!stdout.trim()&&/no such file|can't open/i.test(stderr)`,
  and return false when the hook's stdio was cut before end of stream
  (`stdioIncomplete` or `stdioClosedWithoutEnd`; 2.1.332 then logs "the
  exit-2 block stands"). The 2.1.285 build already emits
  `hook_non_blocking_error` "Hook script appears to be missing … Treating
  as non-blocking".

The case is realistic: `python3 missing.py` exits 2 with `[Errno 2] No
such file or directory`. The reference does not say how the text is
matched. The CLI answers it: a case-insensitive `no such file` or `can't
open` substring on stderr, with stdout empty after trimming whitespace. A
localized shell message, dash's `cannot open … No such file` (still
matched through the second half), or node's `Cannot find module` (not
matched) behave accordingly. The CLI also applies the rule to
`TeammateIdle`, which the reference omits.

Snapshot: `exit-2` is `continue` (Stop, SubagentStop, TeammateIdle) or
`deny` (TaskCompleted, UserPromptSubmit) for any stderr; no plugin-origin
dimension exists.

Classification: correction (a snapshot-time omission). The rule is in the
2.1.285 build, so the earlier "probably 2.1.327" call is withdrawn.

Proposed contract change:

- `Stop`, `SubagentStop`, and `TaskCompleted` gain `exit-2-missing-file`,
  listed before `exit-2`:
  - exit exactly 2;
  - stdout `optional`, `opaque`, `semantic_format:
    empty-or-whitespace-only`;
  - stderr required, `semantic_format: contains-no-such-file-or-cant-open`;
  - effect `nonblocking-error`, stderr role user-message.
- A shared uncertainty records the missing version, the CLI-only match
  rule, and the stdio-cut exception.
- `TeammateIdle` gets an uncertainty only. The rule rests on CLI
  inspection alone, so it is not cataloged, and there is no "stays
  continue" control either.
- `UserPromptSubmit` gets an uncertainty only, because the rule applies
  only to plugin-sourced hooks and the contract has no provenance
  dimension.

Native impact:

- `StopOutput::blocking_error` (`catalog.rs:1544`), `SubagentStopOutput`
  (`:1579`), `TaskCompletedOutput` (`:1622`), `TeammateIdleOutput`
  (`:1644`), and `UserPromptSubmitOutput::blocking_error` (`:2201`) emit
  exit 2 with empty stdout and caller stderr. A wrapper whose message
  quotes a tool's `No such file or directory` or `can't open` is silently
  downgraded. Document it on those constructors and point to
  `into_blocking_error` with a JSON block, which keeps stdout non-empty.
  TaskCompleted and TeammateIdle need the new block builders for that (see
  their section below). Changing `blocking_error`'s bytes would move it
  from `command-exit-2` to `exit-2-structured`, so do not.
- `hookkit-runtime` `FailClosed` lowers `UserPromptSubmit` (and an
  identified-but-unknown event with no name) to exit 2 with empty stdout
  and `hookkit: <program> <hook> failed: <cause chain>` on stderr
  (`failure.rs` `CLAUDE_EXIT_TWO_GATES`, `:249-262`, and the `ExitTwo` arm,
  `:390-395`). A Rust `io::ErrorKind::NotFound` displays `No such file or
  directory (os error 2)`, so a plugin-shipped fail-closed prompt gate
  fails open. Lower Claude `UserPromptSubmit` to exit 2 with
  `{"decision":"block","reason":<diagnostic>}` on stdout as well. Stop,
  SubagentStop, and TaskCompleted are deliberately not gates
  (`failure.rs:245-248`), so nothing changes there.

Fixtures: Stop `command-exit-2-missing-file` (exit 2, stderr
`/bin/sh: /repo/.claude/hooks/stop.sh: No such file or directory\n`) →
`exit-2-missing-file`; Stop `command-exit-2-missing-file-python` (stderr
`python3: can't open file '/repo/x.py': [Errno 2] No such file or
directory`) → `exit-2-missing-file`; Stop with stdout
`{"decision":"block","reason":"Run tests."}` and the same stderr →
`exit-2-structured` (negative: non-empty stdout disables the rule); Stop
`command-exit-2-missing-file-blank-stdout` (stdout `\n`, the same stderr)
→ `exit-2-missing-file`; Stop stderr `tests failed` → `exit-2`; the same
cases on SubagentStop and TaskCompleted; PreToolUse exit 2 with `No such
file or directory` → `exit-2` (deny; the rule is scoped). Runtime test:
`FailClosed` UserPromptSubmit lowering with a `NotFound` cause yields
non-empty JSON stdout and exit 2.

### `continue: false` with exit 2, and `continue` on `PreToolUse`

Merged from CC-SHARED-4, CC-A-9, and CC-SHARED-16.

- `cc-hooks.md:852`: "**JSON that sets `continue: false`**: on events such
  as `Stop` and `SubagentStop`, the field takes precedence over exit code 2,
  so Claude or the subagent ends its turn. … On `PreToolUse`, the tool call
  stays blocked with or without the field."
- `:1032`: "If your hook exits 2 and also sets `continue: false`, check
  Exit code 2 first, because the result can differ from your event's row."
- `:1109`: "`continue` … It doesn't block a `PreToolUse` tool call, so to do
  that, also return `permissionDecision: "deny"`"; `:1121`: the stop applies
  even when the call fails or completes mid-stream, "It doesn't apply when
  the permission check refuses a `PreToolUse` call."
- Changelog `:3174` [2.1.296] managed PreToolUse deny with `continue:
  false` now ends the turn; `:1523` [2.1.323] interrupted-turn resume no
  longer re-runs a turn `continue: false` stopped.

No changelog entry introduces the `:852` precedence or the `:1109`
statement, so both are probably snapshot-time omissions. CC-B-4 confirms
it for `:852`. The 2.1.285 CLI already set `preventContinuation` from any
JSON `continue: false`, and its Stop-hook loop handles that flag beside
`blockingError` in the same way as the 2.1.332 build.

Snapshot: Stop and SubagentStop `exit-2-structured` is `continue` for any
schema-valid JSON; the PreToolUse and PostToolUse output schemas have an
undescribed `continue`. TaskCompleted already records that `continue:
false` is ignored when TaskUpdate triggered the event.

Classification: correction (probable omission).

Proposed contract change:

- Stop and SubagentStop: `exit-2-continue-false` (exit 2, stdout JSON with
  `semantic_format: continue-false`, effect `stop`), listed before
  `exit-2-structured`. TeammateIdle and TaskCompleted: an uncertainty that
  `:852` names Stop and SubagentStop only "such as", keeping TaskCompleted's
  TaskUpdate split.
- PreToolUse output schema: describe `continue` per `:1109` and `:1121` in
  that schema, not in the shared `add_universal`, which feeds all events.
  PermissionRequest: an uncertainty that its interaction with `continue:
  false` is undocumented.

Native impact:

- `into_blocking_error!` (`catalog.rs:1019-1044`) accepts any JSON, so
  `StopOutput::no_op().with_continue(false)?.into_blocking_error("x")`
  builds, and its rustdoc says "Claude Code still blocks". Reject
  `continue: false` in `into_blocking_error` for `StopOutput` and
  `SubagentStopOutput`, or document the precedence.
- `with_continue` rustdoc (`catalog.rs:785-792`, "taking precedence over
  event-specific decisions") is wrong for `PreToolUseOutput` (`:1989`): an
  allow plus `continue: false` still runs the tool. Make it per-event, or
  add a `deny(..).with_continue(false)` recipe.

Fixtures: Stop exit 2, stdout `{"continue":false,"stopReason":"Done."}`,
stderr `keep going` → `exit-2-continue-false`; Stop exit 2 with
`{"decision":"block","reason":"Run tests."}` → `exit-2-structured`
(control); PreToolUse exit 2 with `{"continue":false}` → `exit-2` (deny);
PreToolUse output example `deny-and-stop`
(`{"continue":false,"stopReason":"Policy stop.","hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"Blocked."}}`);
PreToolUse process `command-continue-false-without-deny` (exit 0,
`{"continue":false,"stopReason":"Stop after this call."}`) → `structured`,
with the uncertainty that the tool still runs. Rust: the Stop
`into_blocking_error` case above is rejected.

### Background processes that hold stdio open

Merged from CC-SHARED-5 and CC-A-10.

- `cc-hooks.md:917`: "If you don't redirect them, a hook that isn't async
  takes up to 10 seconds to finish after its own process exits."
- `:919`: a deny that a background process prints after Claude Code has
  stopped reading doesn't block the tool call.
- `:921`: "If your script prints an allow, then starts a background process
  without redirecting that process's stdout and stderr, Claude Code blocks
  the tool call. The hook error includes `stdio went quiet` and `refusing
  to honor the suspect capture`."
- Changelog `:4104` [2.1.285] made synchronous hooks finish shortly after
  their own process exits. That half is at the pin, so it is an omission;
  the suspect-capture block has no changelog entry.

Classification: behavioral (suspect capture, undated) plus an omission.
The scope beyond tool-call events is not stated.

Proposed contract change: a PreToolUse uncertainty (and a shared one
marked "scope beyond PreToolUse unknown"). The metadata cannot express
timing, so no process case.

Native impact: none for emission. The spawn sites already set child stdio
explicitly: `crates/hookkit-tool-runner/src/exec.rs:437-439`, `:503-505`,
`:893-895`, and `crates/hookkit-pkl-config/src/eval.rs:637-639`. Add a
`hookkit-runtime` doc note that handler-spawned children must not inherit
the hook's stdout or stderr, and a `hookkit-tool-runner` regression test
that a tool which backgrounds `sleep 30 &` does not keep the hook's own
stdout pipe open.

### JSON-lines stdout on the non-text-context events

Merged from CC-SHARED-10, CC-A-18, and the verifiers' notes.

- `cc-hooks.md:831`: "For a hook that isn't async, your stdout counts as
  JSON output only when you print one JSON object … with nothing around it
  but whitespace."
- `:833`: output not starting with `{`, or not ending with `}`, is plain
  text: "A JSON array and a quoted JSON string are plain text by this
  rule."
- `:834`: "**Two or more lines that each parse as JSON on their own, the
  first starting with `{` and the last ending with `}`**: plain text when no
  line is a JSON output object that sets a field. When one line is, the
  output is a parse failure".

The rule is in the generic "Exit code 0" section. `TEXT_JSON_RULE`
(`generate-claude-contracts.rs:29`) already encodes both branches, but it
is attached only to the four text-context events (`:513`, `:606`, `:645`,
`:1207`). On the other 27 events (PreToolUse among them) `plain-text` is
`plain-text-unless-brace-delimited` and `invalid-json` is `brace-delimited`
(`pre-tool-use/contract.yaml:61`, `:82`) with no exception, so
`{"note":1}\n{"note":2}\n` reads as `invalid-json` (a non-blocking error
notice) where upstream makes it plain text, debug-logged, no effect. No
process case covers arrays, quoted strings, or a field-setting line
anywhere.

Classification: correction (the rule probably predates the pin).

Proposed contract change: attach a generic JSON-lines uncertainty to every
command binding with `plain-text` and `invalid-json`, and make the
semantic-format labels say that `plain-text` includes JSON-lines whose
lines set no output field and `invalid-json` includes JSON-lines where one
does. Extend `TEXT_JSON_RULE` to name arrays and quoted strings.

Native impact: none for emitters. `parsed_as_json` (`catalog.rs:397-400`)
treats any trimmed `{…}` text as JSON, so `text_context` reroutes both
JSON-lines shapes to `additionalContext`. Every new case id must be
declared or classified in `undeclared_reason`
(`tests/snapshot_contract.rs:942-990`), or the assertion that every case is
covered fails. `command-text-json-array` and `command-text-json-string` on
the text-context events can be declared and reproduced exactly by
`text_context(...)`.

Fixtures: on SessionStart, UserPromptSubmit, UserPromptExpansion, and
PostModelSwitch: `command-json-lines-with-field`
(`{"note":1}\n{"systemMessage":"hi"}\n`) → `invalid-json`
(context dropped); `command-text-json-array` (`["a","b"]`) →
`text-context`; `command-text-json-string` (`"quoted"`) → `text-context`.
On PreToolUse (and generated for every non-text-context event):
`command-plain-text-json-lines` (`{"note":1}\n{"note":2}\n`) →
`plain-text`; `command-json-lines-with-field` → `invalid-json`;
`command-text-json-array` → `plain-text`.

### Output-discarding events and `terminalSequence`

Merged from CC-SHARED-8 and the verifiers' WorktreeRemove note.

- `cc-hooks.md:823`: "**Events that discard hook output, such as
  `StopFailure`**: Claude Code ignores your JSON on any exit code, apart
  from side-effect fields like `terminalSequence`, which still fire."
- `:1137`: the field "works on events that discard `systemMessage` and
  `continue`"; `:1139`: only while an interactive interface is open;
  `:1140`: only from a hook that doesn't run in the background.
- `:3444` (PostCompact) and `:3708` (SessionEnd) document discarding.
- `:1219` (decision table): WorktreeRemove "JSON output is discarded".

PostCompact and SessionEnd use one merged `nonzero` outcome with the
uncertainty "the reference does not say whether terminalSequence fires on
a nonzero exit" (`Nonzero::Merged` at generator `:1125`, `:1215`).
InstructionsLoaded, Notification, and StopFailure already split. The
WorktreeRemove uncertainty says the general statement "is not restated
for WorktreeRemove"; `:823` now states it as an exit-code rule for every
discarding event while `:1219` still says only "discarded".

Classification: documentation (resolves an uncertainty; covers PostCompact
and SessionEnd by example only).

Proposed contract change: PostCompact and SessionEnd switch to
`Nonzero::Split`: `nonzero-structured` (effect
`terminal-sequence-only`, stderr optional) and `nonzero-unstructured`
(`nonblocking-error`, stderr user-message), with uncertainties that the
reference does not say whether the stderr notice shows when the JSON is
schema-valid, and (SessionEnd) that `terminalSequence` needs an open
interactive interface. WorktreeRemove: record the `:823` / `:1219`
conflict; keep stdout ignored.

Native impact: none (`PostCompactOutput::terminal_sequence` and
`SessionEndOutput::terminal_sequence` emit on exit 0).

Fixtures: PostCompact and SessionEnd exit 1, stdout
`{"terminalSequence":"\u0007"}`, stderr `x` → `nonzero-structured`;
PostCompact exit 1, empty stdout → `nonzero-unstructured`.

### Async and `asyncRewake` command hooks

Merged from CC-SHARED-11 and CC-CL-12.

- `cc-hooks.md:4183`: response fields like `decision`, `permissionDecision`,
  and `continue` have no effect.
- `:4219-4221`: after exit, `additionalContext` and `systemMessage` go to
  Claude on the next turn; "neither field is shown to you."
- `:4223-4226`: "Print the JSON response alone on stdout or on one line of
  its own … Spanning several lines requires Claude Code v2.1.295 or
  later." `:4228`: wrong-typed fields are dropped one by one.
- `:4210`: "Claude Code doesn't enforce `timeout` on it. Claude Code still
  enforces `timeout` on a hook you run with `asyncRewake`."
- `:4214`: in `-p`, an async hook still running at teardown is killed and
  finalized `cancelled`.
- `:456`, `:4287`: `asyncRewake` exit 2 wakes Claude immediately, even when
  idle, with stderr, or stdout if stderr is empty.
- `:4289`: unchanged async SessionStart output is skipped on resume
  (changelog `:3319` [2.1.295]); `:3327` [2.1.295] multi-line async JSON;
  `:3884` [2.1.287] a missing `asyncRewake` script is reported once.
- `:993`, `:1140`: `onFailure` and `terminalSequence` do not apply.

Async hooks existed at 2.1.285 and the 2026-09-29 audit deliberately left
them out of scope. Its blanket "not subject to `timeout`" is wrong for
`asyncRewake`.

Classification: mostly correction (pre-existing modeling gap) plus
behavioral items at 2.1.295.

Proposed contract change: a shared uncertainty on every event stating
these rules and versions; a `command-async` binding variant is optional and
waits on the same metadata decision as `onFailure`.

Native impact: HookKit emits compact single-line JSON, which satisfies both
the synchronous and async framing. Under `async: true` the decision
builders, `with_continue`, `with_terminal_sequence`, and plain-text
`text_context` do nothing, and `with_system_message` reaches Claude, not
the user (the `text_context` point is inferred from `:4221`). Add rustdoc
notes; the fail-closed lowering is meaningless for async hooks.

Fixtures (only if `command-async` is adopted): PostToolUse exit 0, stdout
`log line\n{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"Tests passed"}}\n`
→ delivered; pretty-printed JSON alone → delivered;
`{"systemMessage":5,"hookSpecificOutput":{…}}` → delivered with
`systemMessage` dropped; `{"decision":"block","reason":"r"}` → no-op;
`asyncRewake` exit 2 with stderr `tests failed`, or with empty stderr and
stdout `tests failed` → wake.

### HTTP response handling

From CC-SHARED-12.

- `cc-hooks.md:1079`: "**2xx with a JSON object body**: parsed using the
  same JSON output schema as command hooks. A body that fails schema
  validation is a non-blocking error"; `:1081` "**Non-2xx status**:
  non-blocking error"; `:1082` "**Connection failure**: non-blocking
  error"; `:1083` timeout cancels the hook; `:1074` `onFailure: block`.
- Changelog `:1223`-`:1224` [2.1.326] managed allowlists and ignored
  `Content-Length`/`Transfer-Encoding` headers (configuration only);
  `:1475` [2.1.323] HTTP 500 with `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB`.

The snapshot's HTTP outcomes (generator `http_outcomes`, `:1779-1847`) are
`structured`, `empty-body`, `non-json-body` (`not-a-json-object`), and
`non-success` (400-599). A 2xx JSON object that fails the schema matches
none of them.

Classification: correction (ledger-only; HookKit implements no HTTP
binding).

Proposed contract change: an HTTP `schema-invalid-body` outcome (200-299,
a JSON object failing the output schema) with the event's failure effect,
kept as an uncertainty on WorktreeCreate and WorktreeRemove; an uncertainty
that connection failure and timeout have no status; an uncertainty that
3xx handling is undocumented, instead of widening `non-success`. Do not
model 1xx.

Fixtures: PreToolUse HTTP 200 with `{"continue":"yes"}` →
`schema-invalid-body`; 200 with `{}` → `structured` (control).

### Output caps, escaping, and truncation

Merged from CC-SHARED-15, CC-A-17, and CC-CL-21.

- `cc-hooks.md:1095-1099`: `additionalContext`, `systemMessage`, and
  `initialUserMessage` strings and plain stdout are each capped at 10,000
  characters; over the cap the output is saved to a file in the session
  directory and replaced by the path plus a preview of up to 2,000
  characters; Claude is not asked to read the file; the cap has no
  setting. `:1098` exempts PreCompact stdout, which is cut with a marker
  (`:3440`).
- `:1189`: "If your string contains a `<system-reminder>` or
  `</system-reminder>` tag, Claude receives the string with that tag's `<`
  replaced by `&lt;`."
- Changelog `:9313` [2.1.89] the cap; `:3522` [2.1.292] the escaping;
  `:3182` [2.1.296] text resembling plugin hint tags no longer altered;
  `:2236` [2.1.314] PreCompact cut; `:1732` [2.1.321] each hook gets its own
  overflow file; `:3988` [2.1.286] non-text values no longer cause API 400s;
  `:19` [2.1.332] "text over about 50,000 characters is now shortened" when
  a hook blocks or fails with a very large log.

Classification: the cap is a pre-existing omission (2.1.89); the escaping
is behavioral (2.1.292); the 50,000-character shortening is ambiguous about
which text it cuts.

Proposed contract change: a shared uncertainty on every event carrying
these fields or text-context stdout (cap, file spill, preview, no read
prompt); a second uncertainty for the escaping (2.1.292+); the PreCompact
cut lives with PreCompact; the 2.1.332 shortening as an ambiguous
uncertainty. No `maxLength`: longer text is accepted and spilled.

Native impact: no behavior change. Document the cap and escaping on
`text_context`, `with_context` / `with_additional_context` (`catalog.rs:914`,
`:923`, `:948`, `:2008`, `:2111`; `protocol.rs:240`, `:621`),
`SessionStartOutput::with_initial_user_message`, and `with_system_message`.

Fixtures: Rust tests that `PostToolUseOutput::with_context` of 10,001 `a`
characters and of `<system-reminder>x</system-reminder>` emit the string
unchanged and validate against `post-tool-use/output.command.schema.json`.

### Universal output fields

From CC-SHARED-16 (narrowed).

The universal field set is unchanged (`cc-hooks.md:1107-1113`;
`SyncHookJSONOutput` identical apart from the `hookSpecificOutput` union).
New wording: `systemMessage` can arrive as an `SDKInformationalMessage`
(`:1112`); `continue` on PreToolUse is covered above. A `terminalSequence`
with anything outside the allowlist is ignored entirely (`:1113`), which
SDK 0.3.285 already said ("anything else is dropped"), so it is not drift.
Optional native change: validate `with_terminal_sequence`
(`catalog.rs:814-825`) against the allowlist.

### Harness-side refusals without a verdict

From CC-CL-11.

- Changelog `:3839` [2.1.288]: "Fixed PreToolUse and PermissionRequest hooks
  being skipped when matching them failed or the tool's input could not be
  serialized to JSON; the call is now blocked".
- `:1775` [2.1.321]: a PreToolUse `mcp_tool` answer over 16 MiB now refuses
  the call; `cc-hooks.md:578-585`: on PreToolUse an answer without its text
  refuses the call, elsewhere it is a non-blocking error.

Classification: behavioral. Proposed change: PreToolUse and
PermissionRequest uncertainties; no hook input is delivered, so no
fixture. No native change.

### Exit-2 message format

From the verifiers' notes. `cc-hooks.md:872-876`: Claude receives exit-2
stderr as `PreToolUse:Bash hook error: [<command>]: <stderr>`; under
`onFailure: block` it is `… failed; blocking because onFailure is "block"`
or `timed out` followed by the program's output (`:970-977`). Changelog
`:1145` [2.1.326]: a PreToolUse hook exiting 127 now reports that its
check may not have run. The snapshot's agent-context stderr role does not
mention the prefix. Proposed change: a PreToolUse uncertainty (may predate
the pin). No native change.

## Confirmed drift by event

### `PreSkillShellCommand` (new event)

Merged from CC-SHARED-14, CC-CL-1, CC-SDK-1, CC-SDK-2, CC-SDK-3, and the
verifiers' CLI notes.

Input (SDK `sdk.d.ts:2959-2985`): `BaseHookInput & { hook_event_name:
'PreSkillShellCommand'; tool_name: string` ("The shell tool that runs the
command: Bash or PowerShell")`; tool_input: { command: string }` ("The
command as it would run, with arguments filled in")`; tool_use_id: string`
("Identifies this command. It names no tool call in the transcript.")`;
command_name: string` ("The skill or command whose text holds the shell
command")`; command_source?: string; invoked_by: 'user' | 'claude' |
'other'` ("user: typed, or sent by an SDK host. claude: the Skill tool, or a
turn that Claude scheduled. other: anything else … or not known.")`}`.
CLI bundle: `lXo` builds `{...Sh(…), hook_event_name:"PreSkillShellCommand",
tool_name, tool_input:{command}, tool_use_id, command_name,
command_source: h.renderedCommand?.source, invoked_by:
h.skillInvokedBy??"other"}`, so the common fields (including
`scratchpad_dir`, which the SDK omits) come from the shared builder,
`invoked_by` is always present, and an undefined `command_source` is
dropped.

Output (SDK `:2986-2998`): `hookSpecificOutput { hookEventName:
'PreSkillShellCommand'; permissionDecision?: allow | deny | ask | defer;
permissionDecisionReason?: string; updatedInput?: Record<string, unknown>
}`. "allow adds nothing: the command runs unless a hook refuses it. deny,
ask and defer all keep it from running." `updatedInput`: "Never applied.
Any value keeps the command from running." No `additionalContext`.

CLI bundle 2.1.332, process semantics the reference and SDK leave open:

- Matcher: `u_n` returns `e.tool_name` for this event, so matchers filter
  `Bash` / `PowerShell`; the `if` field applies as for PreToolUse
  (`qXo`), and tool-alias rewriting is skipped.
- Default timeout 600 s (`rm=600000`).
- Guard set: `FV=new Set(["PreToolUse","PermissionRequest","PreSkillShellCommand"])`.
  Exit 2 (`blockingError`) refuses the command, with stderr minus the
  `[cmd]:` prefix as the reason.
- A non-blocking failure (exit 1, invalid JSON, timeout) lets the command
  run with a `Ran anyway in /<skill>: <cmd> (hook failed: …|hook timed
  out)` notice; `onFailure: block` applies (not in `Mkr`).
- `prompt` and `agent` hooks are not skipped silently: a matching one
  yields a blocking error with the reason `prompt and agent hooks don't run
  on PreSkillShellCommand`, so it refuses every matching command.
- `TWn` also accepts `hookSpecificOutput` named `PreToolUse` (mapped the
  same way: ask and defer refuse with "…the hook asked for approval, which
  a skill can't wait for"; `updatedInput` refuses with "the hook changed the
  command, which isn't supported here") and `PermissionRequest`-shaped
  answers (deny refuses; `interrupt` sets `continue: false`; allow with
  `updatedInput` or `updatedPermissions` refuses). Top-level `decision:
  "block"` refuses.
- A refused command is replaced in the loaded skill text by `[not run:
  blocked by hook: <reason>]` (passed through an escaper so it cannot
  trigger further inline shell), and the user sees `Not run in /<skill>:
  <cmd> (blocked by hook: <reason>)`; the shown reason is cut to 500
  characters and the command to 80; in sessions that may be bridged the
  hook text is blanked and only logged. `continue: false` aborts the whole
  load with `Operation stopped by hook`; an internal hook error refuses.

Classification: additive (2.1.332). `confidence: low` throughout; exit-code
rules are source-reviewed from a minified build.

Proposed contract change:

- `input.schema.json`: open object; required common fields plus
  `tool_name` (string; examples `Bash`, `PowerShell`), `tool_input` (open
  object requiring string `command`), `tool_use_id`, `command_name`, and
  `invoked_by`; optional `command_source` (examples `skills`,
  `syncedSkills`, `plugin`, from the CLI) and the common optionals. For
  `invoked_by`, prefer an open string with examples `user`, `claude`,
  `other` and a forward-compatibility uncertainty; the
  `ledger-claude-closed-common-enums` precedent rested on SDK `string`
  typing and does not transfer, so a closed enum is defensible too.
- `output.command.schema.json`: universal fields plus
  `hookSpecificOutput` (`additionalProperties: false`, `hookEventName`
  const, `permissionDecision` enum, `permissionDecisionReason`,
  `updatedInput` object). Top-level `decision`/`reason` only with a
  low-assurance note (the SDK types the generic `approve | block`; the CLI
  maps `block` and accepts `approve` with no effect).
- `contract.yaml`: `handler_kinds: [command, http, mcp_tool]`; the standard
  command outcome set with `structured` → `event-specific-control` (allow
  no-op; deny, ask, defer, and any `updatedInput` refuse), `plain-text` →
  `no-op`, `invalid-json` and `nonzero-unstructured` → `nonblocking-error`
  ("Ran anyway"), `exit-2` and `exit-2-structured` → `deny`,
  `nonzero-structured` → `event-specific-control`; HTTP mirrors
  PreToolUse; `command-on-failure-block` as above.
- Uncertainties: the reference is silent; version gate (2.1.331 and older
  show a settings dialog); matcher filters `tool_name`; `if` applies;
  600 s timeout; prompt/agent handlers refuse; `PreToolUse`- and
  `PermissionRequest`-shaped answers are accepted; refusal text and
  truncation; `continue: false` aborts the load; exit-2 stderr reaches
  Claude inside the skill text and the user as a notice (role recorded as
  agent-context, uncertain); ordering against UserPromptSubmit and
  UserPromptExpansion is undocumented (a UserPromptSubmit block stops the
  command, so this event presumably never fires then; inferred); served
  remote calls send `session_id` `served:<id>` and an empty
  `transcript_path` (true of every event).
- Supplement: an `event_profiles` entry mirroring PreToolUse (no
  `environment-file`).

Native impact today: `ClaudeCode::decode` (`protocol.rs:1358-1371`) falls
through to `catalog::decode` (`catalog.rs:2401`, `_ => return Ok(None)`),
so a real payload is `HookkitError::UnrecognizedEvent`. The runtime still
names it through the shared `/hook_event_name` discriminator
(`crates/hookkit-runtime/src/resolution.rs:390-418`, test
`unregistered_events_are_identified_by_the_shared_discriminator_pointer`),
and `fail_closed_lowering` maps a named Claude event outside
`CLAUDE_EXIT_TWO_GATES` to `NotGating`. Under both policies HookKit exits 1
with empty stdout, so a `FailClosed` HookKit guard on this event fails
open unless the user sets `onFailure: block`. (The `unknown … exit 2` row
in `failure.rs:344` applies only when no event name is present.)

Native changes: `Event` variant (`protocol.rs:1172-1240`); identification
descriptors (`protocol.rs:73-84`, `catalog.rs:2332-2363`); decode arm
(`catalog.rs:2371-2403`); an `event_spec!` with the required set above;
scoped accessors `command_name()`, `invoked_by()`, `command_source()`; a
`wire_enum` `SkillInvoker { User, Claude, Other, Unknown(String) }` in
`values.rs`; `PreSkillShellCommandOutput` with `no_op`, `deny(reason)`,
`ask(reason)` and `defer()` documented as refusals, `blocking_error`,
`into_blocking_error`, `nonblocking_error`, and universal builders, and
no `allow` that claims approval and no rewrite builder (do not copy
`PreToolUseOutput::allow` rustdoc, `catalog.rs:2015-2024`); `events()`
registration with `D::command::<PreSkillShellCommand>(EXIT_2)`;
`tests/snapshot_contract.rs:24` `EVENTS` to 34; `hookkit-runtime`
`CLAUDE_EXIT_TWO_GATES` plus the `failure_response` table; optionally a
`hookkit-shell` Pre-phase shell source (`tool_name` Bash/PowerShell,
`/command`).

Fixtures:

- Positive `minimal`: `{"session_id":"s1","transcript_path":"/t/s1.jsonl","cwd":"/repo","hook_event_name":"PreSkillShellCommand","tool_name":"Bash","tool_input":{"command":"git log -1 --format=%H"},"tool_use_id":"skillsh_1","command_name":"release-notes","invoked_by":"user"}`;
  `representative` adds `permission_mode`, `prompt_id`, `scratchpad_dir`,
  `command_source: "plugin"`, `tool_name: "PowerShell"`, `invoked_by:
  "claude"`; `subagent-preload` (`invoked_by: "other"`, no
  `command_source`, `agent_id`/`agent_type`); open-value `invoked_by:
  "scheduler"` if the field is open.
- Negatives: `missing-command` (`/tool_input`, required),
  `tool-input-string` (`/tool_input`, type), `missing-command-name` (``,
  required), `missing-invoked-by`, `invoked-by-number`
  (`/invoked_by`, type), `wrong-discriminator` (`/hook_event_name`,
  const).
- Outputs: `deny`, `ask-refuses`, `allow-no-op`, `updated-input-refuses`
  (`{"hookSpecificOutput":{"hookEventName":"PreSkillShellCommand","updatedInput":{"command":"true"}}}`).
  Output negatives: `additionalContext` inside `hookSpecificOutput`
  (additionalProperties), `permissionDecision: "approve"` (enum). Do not
  add a negative for `hookEventName: "PreToolUse"`; the CLI accepts it.
- Process: `command-structured` (deny), `command-allow` (no-op),
  `command-updated-input-refuses`, `command-no-op`, `command-plain-text`,
  `command-invalid-json`, `command-exit-2` (stderr `curl is not allowed in
  skills`), `command-exit-2-structured`, `command-nonzero-structured`,
  `command-nonzero-unstructured`, HTTP `http-structured`,
  `http-non-success`, `http-non-json`.
- Native: round-trip byte-equal with unknown fields kept; hint-less
  resolution is definitive; `SkillInvoker::from("future")` is `Unknown`.
- Runtime: before the change, `FailClosed` on a named PreSkillShellCommand
  payload exits 1 with `fails_closed() == false`; after it, exit 2 with
  `fails_closed() == true`.

### `PreToolUse`

#### `updatedInput`

Merged from CC-A-6 and CC-CL-9.

- `cc-hooks.md:2014`: "Replaces the entire input object … Claude Code
  evaluates permission rules and a Bash command's auto-background
  eligibility against the input your hook returns … With no
  `permissionDecision`, auto mode can deny the changed call".
- `:2041-2054`: the denial reads "a hook changed this call's input after
  auto mode reviewed it"; if Claude repeats the changed input exactly, the
  hook runs again and "Any `updatedInput` it returns is dropped". `:2056`:
  settings and plugin command hooks may rewrite Bash, PowerShell, Monitor,
  Edit, Write, and NotebookEdit without this denial.
- `permissions.md:687`: in plan mode `updatedInput` is still applied.
- Changelog (changelog-only): `:2948` [2.1.300] validated like Claude's own
  input, unknown keys dropped, rejected input denies the call; `:1144`
  [2.1.326] multi-hook rewrites kept in more cases; `:1199` [2.1.326]
  approved rewrites stay sandboxed unless `dangerouslyDisableSandbox:
  true`. `:3675` [2.1.290] is a fix toward semantics HookKit already
  documents.

The snapshot uncertainty (generator `:691`) "Whether Claude Code applies
updatedInput without a permissionDecision is undocumented" is superseded.
The reference states only the auto-mode consequence, so the replacement is
an inference from `:2014` and `:2043`.

Classification: correction (documentation) plus behavioral (2.1.300,
2.1.326).

Proposed contract change: replace the uncertainty with the no-decision
rule (permission rules run on the returned input; auto mode may deny;
exact repeat drops it; applied in plan mode); add uncertainties for the
2.1.300 validation and the 2.1.326 multi-hook and sandbox rules. Type
`updatedInput` as `object` in the PreToolUse and PermissionRequest output
schemas (SDK `Record<string, unknown>`, `sdk.d.ts:2717`, `:2728`, `:3011`);
an output-only tightening, marked as a schema change.

Native impact: rewrite `PreToolUseOutput::with_updated_input` rustdoc
(`catalog.rs:2088-2106`), which says the no-decision case is undocumented,
and the `with_context` note that refers to it (`:2000-2007`); mention
validation and sandbox rules on `PermissionRequestOutput::with_updated_input`
(`:1916-1924`). Both already take `Map<String, Value>`.

Fixtures: output `updated-input-without-decision`
(`{"hookSpecificOutput":{"hookEventName":"PreToolUse","updatedInput":{"command":"cargo test --offline"}}}`)
and process `command-updated-input-without-decision` (exit 0) →
`structured`; output `updated-input-unknown-key` (allow with
`{"command":"ls","bogus":true}`, schema-valid, uncertainty that `bogus` is
dropped); output negatives `updated-input-not-object` (`"cargo test"`) at
`/hookSpecificOutput/updatedInput` and
`/hookSpecificOutput/decision/updatedInput` (PermissionRequest), keyword
`type`. Native: `PreToolUseOutput::no_op().with_updated_input(map)`
validates.

#### `allow` exceptions and multi-hook precedence

Merged from CC-A-8 and CC-CL-10.

`allow` does not skip approval for:

- actions no mode auto-approves (already in the crate docs,
  `catalog.rs:1950-1952`, `:2015-2018`);
- reads from network paths, UNC on Windows and `/net` automounts on macOS
  and Linux (`cc-hooks.md:2012`; `permissions.md:539-545`; changelog
  `:3472` [2.1.292], security);
- tools an Agent SDK app marks `always_ask`, unless the permission mode
  approves them (`:2012`; possibly pre-pin);
- MCP tools marked `anthropic/requiresUserInteraction`, with or without
  `updatedInput` (`:2101`);
- in plan mode, any non-read-only call other than `ExitPlanMode`
  (`permissions.md:687`; `:3096` [2.1.299] fix);
- shell writes to files the file tools refuse (`:3955` [2.1.287]);
- connector tools an organization set to `ask` (`permissions.md:681`);
- in Cowork cloud auto mode, MCP calls (`:2303` [2.1.314], "counts as no
  answer");
- Monitor commands under a Bash matcher (next section).

If the tool's permission check errors after an allow, the call is decided
as if no hook answered (`:1624` [2.1.323]). Several hooks resolve by `deny
> defer > ask > allow` (`cc-hooks.md:2017`; possibly pre-pin), and since
`:1734` [2.1.321] an unhonorable `defer` no longer discards another hook's
`ask`. An `ask` prompt carries a `[settings]`, `[plugin:<name>]`, or
`[skill]` label (`:2021`; `:2859` [2.1.301]). An `ask` reason reaches
Claude in the tool result when a `-p` run with no permission host denies
the call (`:2013`). `permissions.md:661-671`: in some setups a mod's
`tool.check` can approve a call a PreToolUse hook blocked.

Classification: behavioral (versioned items) plus documentation.

Proposed contract change: PreToolUse uncertainties for the exceptions with
versions, the precedence, the `-p` ask route, and the mod override. No
schema change; the existing `structured` example already carries `ask`
with a reason.

Native impact: rustdoc on `PreToolPermissionDecision::Allow`
(`catalog.rs:1950-1952`), `PreToolUseOutput::allow` (`:2015-2025`), and
`ask` (`:2032-2036`, "shown to the user but not to Claude").

#### Bash matchers cover Monitor commands

Merged from CC-SHARED-13, CC-A-7, and CC-CL-5.

- `cc-hooks.md:282`: "`Bash` matches the Bash tool, and on `PreToolUse` and
  `PermissionRequest` also Monitor commands".
- `:392`: "A `PreToolUse` or `PermissionRequest` hook with a `Bash` matcher
  also runs on shell commands Claude starts with the Monitor tool, but its
  `allow` and `updatedInput` have no effect on them. Requires Claude Code
  v2.1.314 or later. … a `PostToolUse` hook with one doesn't run for
  Monitor."
- `:396`: a matcher including Monitor (`Bash|Monitor`, `.*`, or none) "gets
  the Monitor call's own input, and `allow` and `updatedInput` apply"; with
  an `if` naming Bash it still gets Monitor input but allow and rewrite have
  no effect. `:397`: "**`Bash` matcher**: the hook gets a Bash call. … The
  hook doesn't run if its `if` names Monitor". Bash matchers never fire for
  WebSocket watches.
- Changelog `:2247`, `:2304` [2.1.314].
- SDK `sdk-tools.d.ts:2976-2996` (0.3.332; same in 0.3.285): `MonitorInput`
  requires `description` and `timeout_ms`, with `command` or `ws`.

Classification: behavioral (2.1.314). Nothing in the documented input
marks a Bash-shaped Monitor call.

Proposed contract change: PreToolUse and PermissionRequest uncertainties
covering both matcher cases, the inert allow and rewrite, that the
Bash-shaped input's fields and `tool_use_id` are unspecified, and that the
rewrite must keep Monitor's `description` and `timeout_ms` because
`updatedInput` replaces the whole input. A PostToolUse uncertainty that a
Bash matcher does not fire for Monitor.

Native impact: rustdoc caveats on `PreToolUseOutput::allow` and
`with_updated_input` (`catalog.rs:2023`, `:2100`) and on
`PermissionRequestOutput::allow` and `with_updated_input` (`:1818`,
`:1916`). `hookkit-shell` `CLAUDE_BASH_PROFILE`
(`crates/hookkit-shell/src/adapters.rs:16-17`) matches only `Bash`, so under
`Bash|Monitor` or match-all a shell guard or file-activity inference skips
Monitor commands; add a Monitor profile (`"Monitor"`, `/command`; a
`ws`-only call is not a shell call). `hookkit-tool-access` must tolerate a
Bash Pre with no matching Post (`crates/hookkit-tool-access/src/call.rs:186-226`).

Fixtures: PreToolUse positive `monitor-call`
(`tool_name:"Monitor"`, `tool_input:{"description":"watch app log","timeout_ms":300000,"command":"tail -f app.log"}`);
`monitor-ws` (`{"description":"ws","timeout_ms":60000,"ws":{"url":"wss://example.test/feed"}}`);
`monitor-command-via-bash-matcher` (`tool_name:"Bash"`,
`{"command":"tail -f app.log"}`, origin synthesized). `hookkit-shell`
tests: a Monitor PreToolUse with `command` yields a shell match once the
profile exists; a `ws`-only call yields none. Tool-access: a Pre with no
Post expires cleanly.

#### `defer` on resume

From CC-CL-24. On `claude -p --resume` the same deferred call fires
PreToolUse again (`cc-hooks.md:2117`); a repeated defer with a prompt lets
Claude answer without running it, with no timeout or retry limit
(`:2134-2136`); `CLAUDE_CODE_DISABLE_DEFERRED_TOOL_RESUME=1` skips the call
(changelog `:578` [2.1.328]; `env-vars.md:253`); an unavailable deferred
tool fires no PreToolUse and yields `tool_deferred_unavailable`
(`:2148`), and since `:1865` [2.1.319] the call stays pending unless
`CLAUDE_CODE_DROP_UNAVAILABLE_DEFERRED_TOOL=1`. These are host settings,
not hook-process environment. Proposed change: one PreToolUse uncertainty;
rustdoc on `PreToolUseOutput::defer` (`catalog.rs:2039-2044`) that the hook
sees the same call again on resume.

#### Tool inventory

From CC-A-20. The matcher list now names `PowerShell` and `Workflow` and
excludes `EndConversation` (`cc-hooks.md:1786`, `:1792`); `@`-mentioned
files bypass PreToolUse unless `CLAUDE_CODE_DISABLE_FILE_MENTIONS=1`
(`:1797`; changelog `:2975` [2.1.299]); file-tool `file_path` is always
absolute, with backslashes on Windows (`:1809-1814`); PowerShell
`tool_input` and the advice to match `Bash|PowerShell` (`:1862-1879`);
WebFetch `offset` (`:1941`; `:3575` [2.1.290]). Documentation-only for the
schema. Positives `powershell-tool` and `webfetch-offset`; an optional
`hookkit-shell` Claude PowerShell profile (`ShellDialect::PowerShell`
exists) with a test that PowerShell is not matched by the Bash profile.

#### Deprecated top-level `decision`

From CC-CL-22. `cc-hooks.md:2039`: the deprecated `"approve"` and `"block"`
"map to `"allow"` and `"deny"` respectively"; changelog `:1192` [2.1.326]:
"Changed a hook's `"decision": "block"` to hold in more cases" (no event
named). The snapshot contradicts itself: its uncertainty says the fields
"are still mapped by Claude Code but are intentionally omitted", yet
`output.command.schema.json` is `additionalProperties: false` without
them, so exit 0 with `{"decision":"block"}` classifies as `invalid-json`
(non-blocking error) while Claude Code denies. Correction (pre-pin).

Proposed change: accept top-level `decision` (enum `approve | block`,
described as deprecated) and `reason` in the PreToolUse command output
schema; reword the uncertainty; record the 2.1.326 change as unscoped and
low-confidence. Other events whose schemas reject top-level `decision`
get at most an uncertainty. Process `command-deprecated-decision-block`
(exit 0, `{"decision":"block","reason":"policy"}`) → `structured` (deny);
`…-approve` → `structured` (allow); output negative `{"decision":"deny"}`
(enum). HookKit never emits the form.

### `PermissionRequest`

Merged from CC-A-19, CC-SDK-7, and the PermissionRequest parts above.

- Hooks race a `--permission-prompt-tool` or SDK `canUseTool` host; first
  decision wins (`cc-hooks.md:2161`).
- In `-p` with no host and outside dontAsk, they fire for calls that would
  otherwise be auto-denied (`:2164`, `:2178`; behavior since `:5420`
  [2.1.268]); never for sandboxed network requests (`:2168`) or
  `EndConversation` (`:2178`).
- `permission_suggestions` is not the dialog's option list (`:2176`).
- `setMode` accepts `manual` as an alias for `default` (`:2242`; already in
  SDK 0.3.285 `sdk.d.ts:6737`).
- A rewrite that fails the permission check is asked again or refused
  (`:1625` [2.1.323]); a rewrite to a sandbox-excluded command runs outside
  the sandbox (`:704` [2.1.328]); `Skill(name:*)` allow rules are saved
  outside local settings (`:190` [2.1.331]).
- Cowork cloud: `setMode` in an allow is ignored (`:2947` [2.1.300]); auto
  mode hooks can't approve shell commands or subagents (`:3137`
  [2.1.299]).
- `allow` "does not override a matching deny rule" now has a mod exception
  (`:2209`).
- Since 2.1.332 the ask raised before an MCP-served skill loads carries
  `@internal` fields the SDK strips to blank lines (`sdk.d.ts.diff:383-385`):
  CLI `BOe` spreads `tool_kind: "mcp_skill"`, `mcp_server: {name, source}`
  naming the skill's server, and `mcp_skill: {name, version, uri?,
  server_description?}`.

The last item falsifies the snapshot's `mcp_server` description "Present
only for MCP tools (Claude Code v2.1.274 or later)"
(`permission-request/input.schema.json:40`, and the uncertainty).

Classification: behavioral (versioned items), documentation, correction
(`mcp_server`).

Proposed contract change: uncertainties for each item; widen the
`mcp_server` description; no typed `tool_kind`/`mcp_skill` (internal). If
permission-update entries are ever typed, keep `mode` open (SDK
`PermissionMode` excludes `manual`) and `destination` open or including
`cliArg` (SDK `PermissionUpdateDestination`, `:2774`; the reference table
`:2258-2263` omits it). Windows, Monitor, and `updatedInput` items as
above.

Native impact: rustdoc on `PermissionRequestOutput::allow`
(`catalog.rs:1813-1820`) for the host race, Cowork limits, and mod
exception. No API change.

Fixtures: output `set-mode-manual`
(`{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow","updatedPermissions":[{"type":"setMode","mode":"manual","destination":"session"}]}}}`)
→ schema-valid, exit 0 → `structured`; input positive `mcp-skill-ask`
(`tool_name:"Skill"`, `tool_input:{"skill":"docs:search"}`,
`mcp_server:{"name":"docs","source":"claudeai"}`, `tool_kind:"mcp_skill"`,
`mcp_skill:{"name":"docs:search","version":"v3"}`, origin synthesized;
`tool_name`/`tool_input` of this ask are unknown); native re-serialization
equals the input.

### `UserPromptSubmit`

Merged from CC-A-4, CC-A-5, CC-CL-13, CC-SDK-5, and the verifiers' notes.

- `cc-hooks.md:1548`: "UserPromptSubmit hooks also receive `session_title`
  when the session has a custom title, with the same meaning as the
  SessionStart `session_title` field" (set by `--name`, `/rename`, a hook's
  `sessionTitle`, or `renameSession()`; generated titles never appear,
  `:1316-1318`). SDK `session_title?: string` since 0.3.285. `source`
  stays SDK-only, value set unchanged (0.3.332 only rewords
  `schedule_wakeup`, `sdk.d.ts.diff:2270-2273`).
- `:1525-1530`: it fires on scheduled tasks and `/loop`, background-subagent
  reports, cross-session messages, and (before v2.1.328) Code Mode output.
- `:1561`: for a typed skill or custom command, the hook receives the line
  as typed, and a block stops the command and its `!` shell commands
  (changelog `:3141` [2.1.299]). `:3142` [2.1.299]: a skill's own
  frontmatter UserPromptSubmit hook first runs on the next prompt
  (changelog-only).
- `:1532`: 30 s default timeout (unchanged). `:1534`: a timed-out command,
  HTTP, or MCP tool hook is canceled, its output including
  `additionalContext` is discarded with a transcript notice, and the prompt
  proceeds unless `onFailure: block`; `:1536`: an SDK callback that times
  out blocks the prompt.
- `:1540-1542`: Esc or an interrupt stops the prompt, which returns to the
  input; a simultaneous block stands (`:3186` [2.1.296]).
- No firing on replayed permission answers in remote sessions (`:1770`
  [2.1.321]).
- `:1579`: `reason` "Not added to context, except when the hook blocks Code
  Mode output before v2.1.328".
- The plugin-only missing-file exit-2 rule (above).

Snapshot: `input.schema.json` declares only `prompt`; the uncertainty
(generator `:604`) and `sources.yaml` limitation call `session_title`
undocumented.

Classification: additive (`session_title`, documentation catch-up),
behavioral (2.1.296, 2.1.299, 2.1.321, 2.1.328).

Proposed contract change: optional `session_title` (string, sources
`claude-hooks-reference` and `claude-agent-sdk-types`); narrow the SDK
uncertainty to `source`; uncertainties for firing sources (tied to
`source` values), typed slash-command lines and ordering before expansion
(marking the knock-on effect on UserPromptExpansion and
PreSkillShellCommand as inferred), frontmatter timing, interrupts, replay,
timeout, and the Code Mode reason exception.

Native impact: `CatalogInput` has no `session_title()` accessor scoped to
UserPromptSubmit (`catalog.rs:39-64`; `prompt()` at `:150-153`); add one
(and for SessionStart, for symmetry with `protocol.rs:120`). Document on
`prompt()` that slash-command prompts arrive unexpanded and prompts may be
machine-injected, and on `block`/`blocking_error` (`catalog.rs:2196-2205`)
that a block also stops the skill's inline shell. FailClosed lowering per
the missing-file section.

Fixtures: representative input gains `"session_title":"Review"`; positive
`slash-command-line` (`"prompt":"/deploy staging"`); positive `sdk-source`
(`"source":"loop_wakeup"`); negative `session-title-not-string`
(`"session_title":7`, `/session_title`, `type`). Native: `session_title()`
is `Some` on UserPromptSubmit and `None` on UserPromptExpansion.

### `PostToolUse`

Merged from CC-A-11, CC-A-12, CC-CL-14, and the verifiers' notes.

- `cc-hooks.md:2273`: "In the main conversation, `PostToolUse` hooks from
  managed settings also run after some failed calls"; `:2296`: "For a failed
  call, `tool_response` is the error text as a string." Changelog `:700`
  [2.1.328]: managed PostToolUse no longer fires for calls refused before
  they ran.
- `:2280-2292` and `:1946` [2.1.319]: after a Ctrl+Enter turn end the hooks
  run on the tool's result with 10 s to finish, and a command hook still
  running is killed "whatever its `timeout`"; Enter keeps the normal
  timeout; Esc cancels.
- `:2333` and `:1819` [2.1.321]: in Cowork cloud sessions
  `updatedToolOutput` and `updatedMCPToolOutput` are ignored "for some
  tools, such as AskUserQuestion and ExitPlanMode" (open list). `:1653`
  [2.1.323] (changelog-only): in cloud scheduled tasks a hook from the
  user's own settings cannot replace ExitPlanMode's plan.
- `:2376-2377`: `classifierContext` from settings, plugin, skill, or
  agent-frontmatter hooks is unverified context; only in-process SDK
  callbacks may relay user intent. (Rewrite pairing was already in SDK
  0.3.285, so it is an omission.)
- `REPL` `tool_response` (Code Mode section).

Snapshot: `tool_response` is required and typed `{}`, so a string is valid
today; no uncertainty covers any of this.

Classification: behavioral (2.1.319, 2.1.321, 2.1.323, 2.1.328), plus
omissions.

Proposed contract change: describe `tool_response`; uncertainties for each
item. Keep the 2,000 UTF-16-unit `classifierContext` wording from the SDK
and note that the reference says "characters".

Native impact: `PostToolUseInput.tool_response` is `serde_json::Value`
(`protocol.rs:498-499`); fix its doc ("Tool result in its native JSON
shape"). `crates/hookkit-tool-access/src/call.rs:180-184` says failed calls
are reported "only through `PostToolUseFailure`", which is wrong for
managed hooks; the Claude impls (`call.rs:229-245`) and
`hookkit-file-activity` (`src/lib.rs:752`, `:2264`) must not treat a
string-response PostToolUse as proof of a completed write. Rustdoc on
`with_classifier_context`, `with_updated_tool_output`, and
`with_updated_mcp_tool_output` (`protocol.rs:740-770`). `block`
(`protocol.rs:633-637`) is already correct.

Fixtures: positive `managed-failed-call` (`tool_name:"Bash"`,
`tool_input:{"command":"cargo test"}`, `tool_response:"Exit code 101\nerror: test failed"`);
native tests that `PostToolUse::parse` accepts it and that tool-access and
file-activity classify it as unknown or failed, not succeeded.

### `PostToolBatch`

From CC-A-13 (narrowed). Changelog `:700` [2.1.328] adds, about managed
PostToolUse, "`PostToolBatch` gets most, not marked as refused"; the
reference is silent (`cc-hooks.md:2452-2507`) and SDK
`PostToolBatchToolCall` has no refusal flag. That `tool_response` is the
refusal text is inferred from `:2484`. Documentation (version unknown;
first stated in 2.1.328). Proposed change: an uncertainty that items do not
prove execution. Native: rustdoc on `CatalogInput::tool_calls`
(`catalog.rs:250-260`). Fixture: synthetic positive `refused-call-in-batch`
with the common fields.

### `PreCompact`

Merged from CC-SHARED-9, CC-CL-8, CC-SDK-6, and the verifiers' notes.

- `cc-hooks.md:837-840`: "On these events, your plain-text stdout reaches
  Claude or the compaction summary: … **`PreCompact`**: your stdout becomes
  instructions for the compaction summary".
- `:3440`: "When a PreCompact hook exits 0 and doesn't block, Claude Code
  uses its stdout as instructions for the compaction summary. Keep that
  stdout under 10,000 characters per hook. Claude Code cuts longer output
  at the cap and appends a marker."
- `:1098`: exempt from the file spill. Changelog `:2236` [2.1.314] the cut.
- SDK 0.3.331+ `sdk.d.ts:8113`: "PreCompact and WorktreeCreate, where plain
  text is the hook's answer".
- CLI bundle: both the 2.1.285 and 2.1.332 builds merge the trimmed output
  of every succeeded, non-blocked run into the compaction
  `customInstructions` (`newCustomInstructions`, joined with blank lines);
  2.1.332 cuts each at `QDe=1e4` with a `[PreCompact hook [<cmd>] output
  cut: …]` marker and returns only `blockedBy` when a subagent compacts.
  The command result's output is raw stdout at exit 0 with no JSON
  filtering, and an HTTP 2xx JSON body is used as is. The user also sees
  `PreCompact [<cmd>] completed successfully: <text>`.

Snapshot: `plain-text` → `no-op`, stdout role `diagnostics`
(`pre-compact/contract.yaml:45-61`; seed at generator `:1091` with
`NoControl`); the 2026-09-30 audit lists PreCompact among events whose
plain text is only logged.

Classification: correction. The 2.1.285 build already used the text, so
the snapshot was wrong when it froze.

Proposed contract change: replace `plain-text` with
`compaction-instructions` (exit 0, `plain-text-unless-brace-delimited`,
effect `provide-compaction-instructions`, stdout role agent-context); keep
`invalid-json` for brace-delimited text. Uncertainties: the 10,000-character
cut and marker (2.1.314+); applies only when the hook doesn't block;
ignored for subagent compactions (CLI); the raw stdout of a non-blocking
structured exit-0 response, including `{}`, and a 2xx JSON HTTP body are
also passed as instructions (CLI, undocumented); the user-visible echo;
how hook instructions combine with the user's `/compact` text. Coordinate
with the `command-on-failure-block` outcome set.

Native impact:

- `PreCompactOutput` (`catalog.rs:1441-1463`) has no text constructor. Add
  `PreCompactOutput::compaction_instructions(text)`: writes the text
  verbatim at exit 0 and returns an error for trimmed text starting with
  `{` and ending with `}`, because PreCompact has no `hookSpecificOutput`
  to fall back to (do not reuse `text_context`'s `additionalContext`
  rerouting, which would emit schema-invalid JSON); warn or error past
  10,000 characters.
- `PreCompactOutput::no_op()` (`no_op!` at `catalog.rs:766-775`, applied at
  `:1449`) emits `{}`, which Claude Code feeds into the compaction
  instructions as the literal text `{}` and echoes to the user. Make it
  emit empty stdout on PreCompact, matching the snapshot's `command-no-op`
  fixture, and document that any JSON from this event (for example
  `with_terminal_sequence`) is also appended. This bug predates the pin.
- `tests/snapshot_contract.rs:961-963` classifies `command-plain-text` as
  "plain text that Claude Code only logs for this event"; reclassify for
  PreCompact.

Fixtures: process `command-compaction-instructions` (exit 0, `Preserve the
failing test names.\n`) → `compaction-instructions`;
`command-compaction-instructions-open-brace` (`{keep all}`) →
`invalid-json`; `command-text-over-cap` (10,001 `a`) →
`compaction-instructions` with the cut uncertainty; `command-no-op`
(empty stdout) unchanged; `command-exit-2` unchanged. Native: exact-bytes
test for `compaction_instructions`; `PreCompactOutput::no_op()` emits no
bytes.

#### Idle compaction (`PreCompact`, `PostCompact`)

From CC-B-7. The `auto` matcher row now covers "Auto-compact when the
conversation reaches the auto-compact window, and idle compaction"
(`cc-hooks.md:3415`; PostCompact `:3451`). Idle compaction was added in the
terminal in `:2976` [2.1.299] and in VS Code and Claude Desktop's Code tab in
`:2218` [2.1.314]. `:3419` spells out what a block does to automatic
compaction. If compaction was triggered proactively, before the context
limit (idle compaction included), Claude Code skips it and the
conversation continues. If it was triggered to recover from a context-limit
error the API already returned, the error surfaces and the request fails.
The snapshot's `trigger` enum is unchanged, and no uncertainty described
either point.

Classification: documentation, plus a behavioral widening of `auto` (2.1.299,
2.1.314). No schema change.

Applied:

- `trigger` gets a description on both events.
- A shared `COMPACTION_AUTO` uncertainty on both events.
- A PreCompact uncertainty for proactive versus recovery blocks.

Native: `CompactTrigger::Auto` rustdoc (`values.rs:320-321`, "at the
auto-compact window") should add idle compaction.

### `Stop` and `SubagentStop`

Missing-file and `continue: false` items as above. Also, from CC-CL-6:
`background_tasks` entries report a Monitor's script as `monitor`, not
`shell`, since `:552` [2.1.329], and `command` is present "only for `shell`
tasks and `monitor` tasks that run a command" (`cc-hooks.md:2877`, `:2880`,
`:2882-2883`). SDK 0.3.332 still says "Only present for 'shell' tasks"
(`sdk.d.ts:171-173`), so the SDK lags; `monitor` itself was already an SDK
example label at 0.3.285. `:176` [2.1.331]: Stop hooks in Claude Desktop no
longer miss the last reply in the transcript, and SubagentStop now waits
for it. Proposed change: `examples` and descriptions for
`background_tasks[].type` (`shell`, `subagent`, `monitor`, `workflow`,
`teammate`, `cloud session`, `MCP task`) and field presence, and an
uncertainty for the relabeling. Positives on Stop and SubagentStop:
`background-monitor`, with entry
`{"id":"t2","type":"monitor","status":"running","description":"Watch build log","command":"tail -f build.log"}`,
and `background-mcp-task` (`type: "MCP task"`, `server`, `tool`). No
native change (`background_tasks` is untyped).

CC-B additions (documentation; uncertainties only, applied):

- SubagentStop's `background_tasks` and `session_crons` "are scoped to the
  parent session, not the subagent" (`cc-hooks.md:2718`, CC-B-13).
- The reference still says the transcript "isn't guaranteed to include the
  final message at Stop time on all versions" (`:2868`), despite the
  2.1.331 fix; the Stop uncertainty says so (CC-B missed). The 2.1.332 CLI
  also passes `flushTranscriptFirst` for TaskCompleted and TeammateIdle
  (absent from 2.1.285). That is undocumented and not encoded.
- `:2388` [2.1.311]: "Fixed `SubagentStop` hooks running about twice a
  minute for each running subagent instead of once when it finishes"
  (CC-B missed). The reference never mentions it, and the changelog does
  not say which releases had the bug, so a consumer on 2.1.285-2.1.310 may
  see repeated SubagentStop events. Low-priority uncertainty.

### `SubagentStart` and `SubagentStop`: matchers and the web-fetch subagent

From CC-B-15 (confirmed). Documentation only; the changelog has no entry.

- A plugin's subagent reports the plugin-scoped `agent_type`, such as
  `my-plugin:reviewer`. "The colon places a plugin-scoped name on the
  regular-expression path, so anchor the matcher" (`cc-hooks.md:2662`; the
  matcher table at `:300` lists `^my-plugin:reviewer$`).
- For the built-in web-fetch subagent, Claude Code runs SubagentStart and
  SubagentStop hooks "from managed settings only" (`:2666`, `:2706`).
- "A `matcher` that names agent types doesn't match an empty `agent_type`.
  A hook whose matcher is omitted, `""`, or `"*"`, or is a regular
  expression that matches an empty string, runs for events with an empty
  `agent_type` too" (`:2714`; the related fix is pre-pin, `:4930`
  [2.1.275]).

Applied:

- A shared `SUBAGENT_MATCHER_NOTE` uncertainty on both events.
- The SubagentStop internal-agent uncertainty gains the empty-matcher rule.
- Synthetic positive `plugin-agent` (`agent_type: "my-plugin:reviewer"`) on
  both events.

Native: `CatalogInput::agent_type` rustdoc (`catalog.rs:117-121`) can note
the plugin-scoped form. No API change.

### `TaskCompleted` and `TeammateIdle`: top-level `decision: "block"`

From CC-B-3 (modified).

- `cc-hooks.md:2833` (TaskCompleted): "**JSON `{"decision": "block",
  "reason": "..."}`**: the task stays open. Claude receives `reason` and
  keeps working."
- `:3017` (TeammateIdle): "the teammate receives `reason` and continues
  working."
- `:1212` (decision table): "Exit code 2 or `decision: "block"` keeps the
  teammate working or the task open. With `continue: false`, the teammate
  goes idle."
- `:2834`, `:3018`: `continue: false` sends the teammate idle "even if
  another of your … hooks blocks". On a `TaskUpdate` call, TaskCompleted
  ignores `continue: false`.

The snapshot's two `output.command.schema.json` files allowed only
`continue`, `stopReason`, `suppressOutput`, `systemMessage`, and
`terminalSequence` with `additionalProperties: false`. A documented exit-0
block therefore classified as `invalid-json`, a non-blocking error, while
Claude Code keeps the task open.

This is not new behavior. The 2.1.285 CLI's generic JSON processor already
mapped `decision: "block"` to a blocking error for every event
(`case"block":B.permissionBehavior="deny",B.blockingError={blockingError:e.reason||"Blocked by hook",…}`),
and TaskCompleted and TeammateIdle consume `blockingError`. The 2026-09-29
audit added `decision` to TaskCreated only. Changelog `:1192` [2.1.326]
("`decision: "block"` to hold in more cases") names no event and is not
cited.

Classification: correction (relaxing).

Applied:

- `Seed::top_level_block()` on both events, without
  `block_reason_required`. `reason` is optional because the reference
  shows it but does not require it, and the CLI defaults the message.
- `reason` descriptions on both events.
- An output example `block` on both events.
- Process cases `command-block` (exit 0 → `structured`) and
  `command-exit-2-block` (exit 2 with the block JSON → `exit-2-structured`:
  `deny` on TaskCompleted, `continue` on TeammateIdle).
- Uncertainties for the correction, for `continue: false` winning over
  another hook's block, and (TaskCompleted) for the `TaskUpdate` split.
- Source: `claude-hooks-reference` only, with no version floor.

Native: `TaskCompletedOutput` (`catalog.rs:1613-1630`) and
`TeammateIdleOutput` (`:1632-1652`) have `blocking_error` and
`into_blocking_error` but no `block_builders!`. Add them, documenting that
another hook's `continue: false` overrides the block, and the TaskUpdate
exception. That also gives these events a JSON block to recommend over the
missing-file-prone `blocking_error`.

Native tests:

- `TaskCompletedOutput::block("Tests not passing.")` and
  `TeammateIdleOutput::block(..)` emit exact bytes at exit 0 and validate
  against the new schemas.
- `tests/snapshot_contract.rs` declares or classifies `command-block`,
  `command-exit-2-block`, and the missing-file `-blank-stdout` cases.

### `TaskCreated`, `TaskCompleted`, `TeammateIdle`, `InstructionsLoaded`, and compaction events: `agent_id`

From CC-CL-16. The common `agent_id` / `agent_type` / `effort` are sent in
more cases: InstructionsLoaded for subagent file-access loads, and
`effort` for lazy loads (`:3843` [2.1.288]); TaskCreated, TaskCompleted,
and TeammateIdle identify the subagent or in-process teammate ("May be
absent. Requires Claude Code v2.1.290 or later", `cc-hooks.md:2770`,
`:2826`, `:3010`; the changelog entry `:3713` concerns Agent results and
says TeammateIdle no longer fires from a teammate's subagents or forks);
PreCompact, PostCompact, and post-compaction SessionStart when a subagent
compacts (`:1548` [2.1.323]). All schemas already allow the fields.
Proposed change: positives on those seven events and versioned
uncertainties. Native: broaden `CatalogInput::agent_id` and
`SessionStartInput::agent_id` rustdoc. CC-B-12 adds CLI evidence that
`agent_type` accompanies `agent_id` on all three task events. The 2.1.285
build constructed their inputs with no agent fields; the 2.1.332 build
passes `{agentId, agentType}`. The "not a `name@team` address" framing is
dropped, because `:3713` concerns Agent results.

### `SessionStart`

From CC-A-15 (narrowed). The reference now spells out background
execution at launch, `--continue`/`--resume`, and `/clear`, with the first
response waiting and in-session `/resume` waiting (`cc-hooks.md:1299-1305`;
background resume dates from `:5429` [2.1.268]); output dropped after
`/clear` or a conversation switch; late-installing plugins'
`initialUserMessage` and `sessionTitle` ignored (`:1372`; `:3202`
[2.1.296]); `fork` covering the `/fork` background copy, `/branch`, and
move-to-background (`:1297`); generated titles absent from
`session_title` (`:1318`). Documentation of pre-pin behavior except the
`agent_id` item. Proposed change: uncertainties. Native: the
`SessionStartInput.agent_id`/`agent_type` rustdoc
(`protocol.rs:100-105`, "when a subagent session starts") is wrong, since
a subagent start fires SubagentStart; per `cc-hooks.md:768-769`, `agent_id`
identifies the subagent or teammate the hook fires in, and `agent_type`
is also present on the main thread with `--agent`. Fixture: positive
`subagent-compact` (`source:"compact"`, `agent_id`, `agent_type`).

### `MessageDisplay`

From CC-A-14. `cc-hooks.md:1670`: "In non-interactive runs, including Agent
SDK queries and `claude -p`, MessageDisplay runs once per assistant
message … `index` is `0`, `final` is `true`, and `delta` holds the entire
message." `:1682`: an empty final delta happens only in interactive runs.
The SDK `MessageDisplayHookInput` docs are unchanged and still describe
per-flush batching. Version unknown. Proposed change: an uncertainty;
positives `non-interactive-single-call` (common fields, `turn_id`,
`message_id`, `index:0`, `final:true`, `delta:"Here is the plan:\n1. Build\n2. Test"`)
and `interactive-empty-final` (`index:2`, `final:true`, `delta:""`).
Native: optional typed `index`/`final`/`delta` accessors
(`catalog.rs:1362-1378`).

### `Notification`

Merged from CC-SHARED-13 (part), CC-CL-7, and CC-SDK-8. New documented
values: `auth_storage_failure` (`cc-hooks.md:2576`; v2.1.288 per `:2588`;
fires in place of `auth_success`) and `plugin_notification` (`:2586`; a
mod's `$.ui.notify`, title defaults to the mod name; most likely 2.1.295,
`:3261`, and absent from the 2.1.285 CLI build). Both appear in the matcher
list (`:299`). Additive. Proposed change: add both to the examples
(generator `:836`) and the uncertainty. Native: `NotificationType`
(`values.rs:254-279`) gains `AuthStorageFailure` and `PluginNotification`
(today they parse as `Unknown(..)`). Positives `auth-storage-failure` and
`plugin-notification` (`"title":"ci-watch"`). CC-B-1 confirms all of this.

`idle_prompt` timing comes from the CC-B verifier's missed list, and the
slice's "Notification unchanged apart from CC-B-1" overlooked it.
`cc-hooks.md:2596-2600` says that `permission_prompt`, `idle_prompt`,
`elicitation_dialog`, and `elicitation_url_dialog` "share their timing with
desktop notifications". In terminal sessions they fire only when the user
seems away:

- `permission_prompt` and the elicitation types fire after about six
  seconds without typing.
- `idle_prompt` fires "about 60 seconds after Claude finishes responding,
  and only if you haven't typed since and no background agent … is still
  running".
- Claude Code doesn't send `idle_prompt` "while it waits for a claude.ai
  usage limit to reset".

Changelog `:3838` [2.1.288] fixed `idle_prompt` firing while background
agents ran.

Classification: behavioral (2.1.288) plus documentation. Applied: a timing
uncertainty. No schema change.

Native: the `NotificationType::IdlePrompt` rustdoc (`values.rs:257-258`,
"The prompt has been idle") can state the timing.

### `StopFailure`

From CC-B-2 (confirmed). `cc-hooks.md:2966` now lists 11 `error`
values without `overloaded` and adds "For an HTTP `529` overloaded
response, the value is `server_error`"; the matcher list (`:308`) also
omits it. SDK 0.3.332 `SDKAssistantMessageError` (`sdk.d.ts:4094`) and the
2.1.332 CLI zod enum still include `overloaded`. The snapshot's closed
enum keeps it, sourced from the earlier reference. Documentation (or a
quiet behavior change; unknown). Proposed change: keep `overloaded` in the
enum (removing it could reject real payloads) and move it into the
SDK-sourced uncertainty beside `verification_required`, with the 529 →
`server_error` statement. Native: keep `values.rs:288` `Overloaded`.

### `Elicitation` and `ElicitationResult`

Merged from CC-SHARED-6 and CC-SHARED-7.

- `cc-hooks.md:822`: "Claude Code applies your `hookSpecificOutput` when
  your hook exits 0, and ignores it on any other exit code"; `:853` for
  exit 2.
- `:1220`: "`decision: "block"` also declines"; `:3815`: "the block
  overrides an `action` in the same output"; `:3828`: neither the user nor
  the server sees why (stderr and `reason` are not shown); `:3830`: decision
  was ignored from v2.1.105 until v2.1.284; `:3808`: `reason`,
  `systemMessage`, and `continue` discarded; `:3817`: a decline from one
  hook overrides another's accept or cancel.
- `:3953` (ElicitationResult): "Cancel the request | `"action": "cancel"` |
  `cancel`. To withhold the values the user submitted, return
  `"decline"`."

Snapshot: `nonzero-structured` (exit 1-255, effect
`event-specific-control`) and the `command-nonzero-structured` case (exit
1 with `action: accept`) contradict `:822`. The uncertainties (generator
`:1261`, `:1292`) say the reference does not document `decision` and
"where a reason is surfaced is unknown".

Classification: correction (no changelog entry).

Proposed contract change: replace `nonzero-structured` with
`nonzero-json-ignored` (exit 1-255, stdout JSON, effect `no-op`: the dialog
proceeds), with an uncertainty that the reference does not say whether a
hook-error notice shows or whether top-level `decision: block` still
declines on exit 1 or 3-255; retarget `command-nonzero-structured`; add a
case `{"decision":"block","reason":"r"}` at exit 1 under the same outcome
with that uncertainty. Rewrite the decision uncertainties to cite the
reference (reason shown nowhere; block overrides `action`; multi-hook
decline precedence). ElicitationResult: an uncertainty that `cancel` may
forward the user's submitted values. Keep `exit-2`.

Native impact: none for emitters. Rustdoc on `ElicitationOutput` and
`ElicitationResultOutput` `block_builders!` (`catalog.rs:1710-1714`,
`:1738-1742`): the reference now documents the decision; the reason is
shown nowhere.

Fixtures: Elicitation exit 1 with the accept JSON → `nonzero-json-ignored`;
exit 0 with the same → `structured` (control); ElicitationResult exit 1
with `{"hookSpecificOutput":{"hookEventName":"ElicitationResult","action":"decline"}}`
→ `nonzero-json-ignored`; output example `decision-block-overrides-action`
(`{"decision":"block","reason":"r","hookSpecificOutput":{"hookEventName":"Elicitation","action":"accept"}}`)
→ decline.

CC-B-8 and CC-B-9 confirm the above. The CLI also checks `decision:
"block"` before `hookSpecificOutput` and returns `{}` when no `action` is
present. CC-B-9 adds CLI-only evidence on the open question. In the 2.1.285
build, `blocked` is computed from a stdout `decision: "block"` at any exit
code (`_n=en.decision==="block", xn=status===2||_n`), so exit 1 or 3-255
with a block probably still declines. `ELICITATION_NONZERO` now says so as
a CLI-inspection note. `command-nonzero-decision-block` stays
`nonzero-json-ignored` until the CLI is accepted as a source.

Remainders from CC-B-8 and the CC-B verifier's missed list (documentation;
the changelog has no entry):

- `cc-hooks.md:3923`: "When an Elicitation hook answers a request, Claude
  Code sends that answer to the server without running ElicitationResult
  hooks." Applied as an uncertainty on both events.
- `:3969` (ElicitationResult): "Your `content` replaces the user's whole
  `content` object, so include the fields you aren't changing. … Claude
  Code ignores a `hookSpecificOutput` that has no `action`."
- `:3971` (ElicitationResult): "ElicitationResult hooks also run when the
  user declines or cancels, and your `action` replaces theirs. Check that
  the input's `action` is `accept` before you return `accept`, or your hook
  turns a declined request into an accepted one."
- `:3806` (URL mode: an accept means the URL never opens) was already in
  the Elicitation uncertainty.

Applied, ElicitationResult:

- Uncertainties for `:3969` and `:3971`.
- The `cancel` uncertainty adds that whether an accept without `content`
  forwards the user's values is undocumented.
- Positives `user-declined` (`{"action":"decline","mode":"form"}`) and
  `user-cancelled` (`{"action":"cancel"}`).
- The official output `official-change-values` (the `alice@example.com`
  block).
- A synthesized output `content-without-action`
  (`{"hookSpecificOutput":{"hookEventName":"ElicitationResult","content":{"name":"Ada"}}}`).
  It is schema-valid, and its process case `command-content-without-action`
  (exit 0) → `structured` changes nothing.

Native:

- `elicitation_actions!` (`catalog.rs:1656-1704`) as instantiated for
  `ElicitationResultOutput` (`:1734-1738`) gives `accept` and
  `accept_without_content` no warning. Document that the hook also runs on
  a user decline or cancel and that its action replaces the user's (check
  the input `action` first), and that `content` replaces the whole object.
- `ElicitationOutput` can note that ElicitationResult hooks are skipped
  after it answers.

### `Setup`

From CC-SHARED-7. `cc-hooks.md:4039`: "`Setup` supports only `command`
hooks"; `:597`: mcp_tool on Setup is skipped. The snapshot uncertainty
(generator `:539`) says the list "still names mcp_tool"; rewrite it.
`handler_kinds: [command]` is unchanged.

### `PermissionDenied`

From CC-CL-15. Changelog `:212` [2.1.331]: "Changed what a
`PermissionDenied` hook's `retry: true` tells Claude: make the refused call
again only if you asked for that call"; the reference still says the model
"may retry" (`cc-hooks.md:2561`). Wire unchanged. Extend the uncertainty
(generator `:827`) and the `PermissionDeniedOutput::retry` rustdoc
(`catalog.rs:1409-1416`).

### `UserPromptExpansion`: MCP skills

From CC-CL-23 (narrowed). MCP servers can provide skills named
`<server>:<skill>`, prefixed for connectors and plugin servers
(`claude_ai_Docs:pdf`, `plugin_my-plugin_docs:pdf`; `mcp.md:1842`,
`:1850`). The docs say v2.1.331 (`mcp.md:1838`, `skills.md:325`); the
changelog lists it under 2.1.332 (`:5`). The `:` already occurred in
plugin-scoped names, so the matcher consequence is not new. Undocumented:
the Skill tool's `tool_input.skill` for an MCP skill, and the
`expansion_type` and `command_source` an MCP skill produces. Separately,
`:1911` [2.1.319] Desktop now honors matchers written with a connector's
terminal name. Documentation only: a synthetic positive `mcp-skill`
(`command_name:"docs:pdf"`, `prompt:"/docs:pdf report.pdf"`, no
`command_source`), an uncertainty, and the version contradiction.

### `WorktreeCreate` and `WorktreeRemove`

From CC-CL-19 (narrowed). The snapshot's "refused (Claude Code v2.1.216 or
later)" (`worktree-create/contract.yaml:195`) is no longer supported: the
reference states the rule with no version (`cc-hooks.md:3336`), and the
2.1.216 changelog section has no entry. The changelog adds a route floor:
EnterWorktree refuses a path through a symlink inside the project since
`:1373` [2.1.325]. Plugin WorktreeCreate and WorktreeRemove hooks run with
`--worktree` only since `:2907` [2.1.300] and still not with `--tmux`
(`cc-hooks.md:3282`); the plugin seed-dir case was fixed in `:1308`
[2.1.325]. Correction. Reword the uncertainty and add the plugin note;
keep or drop the v2.1.216 in `WorktreeCreateOutput::path` rustdoc
(`protocol.rs:991-1002`) to match. WorktreeRemove: the `:823` conflict
(above).

CC-B-16 (modified) and the CC-B verifier's missed list add firing and
reporting rules. The 2.1.325 symlink entry is a route floor for
EnterWorktree; the general rule is unversioned. Classification:
documentation, apart from the versioned changelog items. Uncertainty text
only, with no schema or outcome change, all applied.

WorktreeCreate (`cc-hooks.md:3270-3277`):

- The trigger list now includes three newly listed contexts: a workflow
  running an agent in its own worktree, a background session isolated in
  its own worktree, and `claude remote-control` in `worktree` mode.
- The hand-authored `worktree-create/contract.yaml` gains this list.

WorktreeRemove, firing:

- It fires (`:3340-3347`) when you exit an interactive worktree session
  and choose removal, and without a prompt when an unnamed interactive
  session exits with no changed or untracked files. Since `:2836`
  [2.1.301], quitting no longer removes a hook-made worktree that may hold
  unpushed commits without asking.
- It also fires when you delete a background session, when Claude calls
  `ExitWorktree`, and without a prompt when a remote-control worktree
  session ends cleanly or remote control is stopped while git finds no
  changes or new commits.
- Subagent and workflow-agent worktrees are kept, and WorktreeRemove does
  not run for them (`:3349`).
- `ExitWorktree` refuses a hook-created worktree unless Claude passes
  `discard_changes: true` (`:3351`). Related changes to the ExitWorktree
  refusal rules are in changelog `:1210` [2.1.326] (refuses while a
  background subagent or shell may still run there) and `:851` [2.1.327]
  (asks before removing possibly unsaved work).
- Change detection is git-only, so a non-git worktree looks clean
  (`:3353`). Claude Code never deletes a hook-created worktree's branch
  (`:3364`).
- For a background-session delete, Claude Code verifies the stored path
  before running the hook, refusing a symlink or a path through one below
  the repository root. It runs the hook on a non-empty worktree only after
  confirmation in agent view, and `claude rm` keeps the session and
  worktree instead (`:3366`).

WorktreeRemove, results and reporting:

- With no hook (`:3357-3360`), Claude Code removes the worktree as `git
  worktree remove --force` would at exit or through `ExitWorktree`, so a
  worktree git doesn't recognize stays on disk. Remote-control cleanup
  keeps the worktree and shows `worktree removal failed, kept: <path>`.
- After exit 0 there is no git fallback (`:3361`). A hook that deleted the
  directory before a nonzero exit counts as removed (`:3362`).
- Failures are reported in agent view (how the hook ended, the start of
  stderr, and whether deleting again removes the directory) or on the
  remote-control terminal (`:3403-3404`). The snapshot's "writes the hook
  command and stderr to the debug log" is no longer stated and was
  replaced.

Native: `WorktreeRemoveOutput::failed` rustdoc (`catalog.rs:1318-1328`)
still says Claude Code "writes the stderr to its debug log". Replace it
with the documented reporting, and add the no-fallback-after-exit-0 and
kept-subagent-worktree notes to `removed` (`:1311-1316`). Drop the
v2.1.216 in `WorktreeCreateOutput::path` (`protocol.rs:991-1002`).

### Firing scope: `ConfigChange`, `CwdChanged`, `DirectoryAdded`, `PreModelSwitch`, `PostModelSwitch`, `SessionEnd`

From CC-B-17 (confirmed) and the CC-B verifier's DirectoryAdded note, plus
changelog entries found while re-checking. The input enums and outcomes
already match, so these are uncertainties only, all applied:

- **ConfigChange**:
  - Changes take effect without running the hooks when saved with "Yes,
    and don't ask again", `/permissions`, or `/model` in the saving
    session (another session that loads the file runs its hooks). The
    same holds for server-managed settings, macOS managed preferences,
    Windows registry policy, and WSL `wslInheritsWindowsSettings`
    Windows-side files (`cc-hooks.md:3049-3053`, `:3106`).
  - Changelog `:3681` [2.1.290] fixed sandboxed Bash on Linux firing
    ConfigChange mid-command when the settings files were absent.
- **CwdChanged**: fires for "a shell command in the main conversation"
  (`:3112`). That subagent commands don't fire it is implied, not stated.
- **DirectoryAdded**:
  - It fires after the sandbox and permission refresh, and the hook runs
    unsandboxed (`:3155`).
  - It does not fire for the `--add-dir` startup flag (SessionStart covers
    it), the `/permissions` Workspace tab, or a directory already inside a
    working directory (`:3149-3153`).
  - Since `:1219` [2.1.326], `/add-dir` shows a trust prompt for an
    untrusted folder. Whether the event fires when trust is declined is
    undocumented.
- **PreModelSwitch**:
  - It also runs for fast mode, the `/config` Model setting, `Option+P`
    or `Alt+P`, and `apply_flag_settings` or Remote Control requests,
    mapped to `command`, `picker`, and `sdk` (`:3476-3480`, `:3563`).
  - A deny reason becomes the `set_model` error (`:3599`).
  - `ModelSwitchSource` (`values.rs:189-210`) already documents this
    mapping.
- **PostModelSwitch**: runs on `opusplan` plan-mode transitions (which
  `source` is unstated) and not for fallback-chain turns (`:3633-3636`).
- **SessionEnd**:
  - The 1.5 s budget rises to the highest per-hook `timeout`, up to 60 s,
    but "Timeouts set on plugin-provided hooks don't raise the budget".
    `CLAUDE_CODE_SESSIONEND_HOOKS_TIMEOUT_MS` overrides it
    (`:3710-3713`). The snapshot had no budget uncertainty at all.
  - Changelog `:2894` [2.1.300] fixed SessionEnd hooks sometimes not
    running at exit while a background command ran.

Checked and not encoded:

- `:1722` [2.1.321], a Windows FileChanged `watchPaths` fix.
- `:785` [2.1.327], a display fix for failing SubagentStop, SessionEnd,
  CwdChanged, and FileChanged hooks.

### Handler kinds and matchers

From CC-SHARED-18 and CC-CL-20. `handler_kinds` are unchanged for all 33
events (`cc-hooks.md:4000-4039`: 12 events with all five kinds;
PermissionRequest without `agent`; 18 events with command/http/mcp_tool;
Setup command-only; SessionStart command and mcp_tool), matching every
`contract.yaml` and the generator constants (`:21-25`). Changes to bindings
HookKit does not implement:

- prompt/agent: at most 12 of one event at a time (`cc-hooks.md:409`;
  `:2798` [2.1.302]); the changelog adds 24 per session plus 12 overdue and
  a 1 s minimum timeout (`:2358` [2.1.312]), which the reference omits;
  prompt `model` aliases (`:407` [2.1.330]); agent `model` honored (`:2911`
  [2.1.300]); agent hooks handle `ok: false` like a prompt hook with
  `continueOnBlock: true` and default to a 60 s timeout; PermissionDenied
  prompt and agent output is discarded (`:4100-4110`).
- HTTP: managed allowlists (`:1223`), ignored length headers (`:1224`).
- mcp_tool: `onFailure` (2.1.328), the oversize refusal (2.1.321), and
  auth/startup fixes (2.1.321-2.1.325).
- Command configuration: `once` (a run that blocks with exit 2 leaves the
  hook in place, `:427`), cross-file dedupe (`:411`), skill-hook limits of
  1,024 hooks and 16,384 matcher characters (`:714`; `:2740` [2.1.304]).

Matcher rules with no post-pin changelog entry, recorded as
`sources.yaml` limitations, not drift: `if` is evaluated only on the five
tool events and a handler with `if` never runs elsewhere (`:424`);
FileChanged and StopFailure use a narrower exact-match alphabet (`:287`).

## Command-process environment

Merged from CC-SHARED-17, CC-A-16, CC-CL-17, CC-CL-18, and the verifiers'
notes.

No hook-exported variable was added or removed for the 33 existing events.
`CLAUDE_PROJECT_DIR`, `CLAUDE_PLUGIN_ROOT`, `CLAUDE_PLUGIN_DATA`
(`cc-hooks.md:491`, exec and shell form), `CLAUDE_PLUGIN_OPTION_<KEY>`
(`:495`), `CLAUDE_CODE_REMOTE`, `CLAUDE_CODE_BRIDGE_SESSION_ID` (`:415`),
`CLAUDE_EFFORT` (`:761`, now also stated in the hooks reference), and
`CLAUDE_ENV_FILE` on SessionStart, Setup, CwdChanged, and FileChanged
(`:1441`; `env-vars.md:459`) are as the supplement models them. The
`env-vars.md` rows for `CLAUDECODE`, `CLAUDE_CODE_CHILD_SESSION` (now also
naming Monitor subprocesses), `CLAUDE_CODE_BRIDGE_SESSION_ID`,
`CLAUDE_PID`, `CLAUDE_EFFORT`, `CLAUDE_CODE_REMOTE_SESSION_ID`, and
`CLAUDE_ENV_FILE` still match the profiles.

New statements:

- `cc-hooks.md:1441`: `CLAUDE_ENV_FILE` is unavailable to "a hook that runs
  in PowerShell, whether through `"shell": "powershell"` or by default on
  Windows without Git Bash". `:457`: exec form is not a PowerShell hook.
- `:1432-1437` and `:3203` [2.1.296]: PowerShell *tool commands* receive
  the file's variables only when every line is blank, a comment, or a
  plain assignment, never in sandboxed PowerShell, and on Windows not for
  values containing `/` or `\`. Persistence fixes: `:3320` [2.1.295]
  `/resume` and `/branch`; `:2008` [2.1.318] cloud Setup; `:336` [2.1.330]
  `-p --continue`.
- `:775`: a session with the HIPAA configuration has Anthropic credentials
  removed from the hook environment. `:773`: there is no `$CLAUDE_MODEL`.
- `env-vars.md` `CLAUDE_CODE_SHELL_PREFIX`: PowerShell and exec-form hooks
  run without the prefix.
- Ambient changes: `:3106` [2.1.299] a user mod's `$.env.set` no longer
  changes managed command hooks (so it still can change other hooks);
  `:260` [2.1.331] VS Code no longer leaks `ELECTRON_RUN_AS_NODE`; `:3290`
  [2.1.295] background sessions no longer pass `FORCE_COLOR=3`.
- Self-hosted runner (`self-hosted-environments-configuration.md:42-56`):
  `CLAUDE_CODE_REMOTE_SLACK_THREAD_URL`, `…_THREAD_TS`, and
  `…_CHANNEL_ID` for Claude Tag sessions, each independently unset, and
  readable "in what the session runs, such as … Claude Code hooks". No
  changelog entry. `CLAUDE_CODE_REMOTE_SESSION_UUID` is not new (already
  recorded in the pinned `sources.yaml`). Changelog `:713` [2.1.328] says
  `CLAUDE_RUNNER_SESSION_UUID` and `CLAUDE_CODE_REMOTE_SESSION_UUID` are
  both set "for the session and its `checkout`, `post-session` and
  `spawn-runner` hooks", but the page lists `CLAUDE_RUNNER_SESSION_UUID`
  only for the runner's hooks, so "the session process" is unconfirmed.

Successor supplement sources (all fetched 2026-11-25 with
`curl --fail --silent --show-error --location --compressed`; each must
cite the earlier full hash in `limitations`):

| Source id | New SHA-256 (bytes) | Pinned |
| --- | --- | --- |
| `claude-hooks-reference` | `d16003851f60562eaa793e6270316b17550f8e24691214d7d798e5a503537c79` (301,732; the body that `docs-2026-11-25-r1` pins, see "Post-review corrections"; this note reviewed `194272ed…3099`, 293,618) | `5129d583…f566` |
| `claude-environment-reference` | `30cd22fe95a40ced8d175d368d5e915441014255f40965e681d2a3dcbdff317e` (194,781) | `f49dad1f…7b1a` |
| `claude-plugins-reference` | `bcd6278cef41259de55b509ae9d9f91c9d2fba1c369798ec1a6667738ae32986` (43,583) | `2ec613e8…c270` |
| `claude-plugin-components` | `1737d81c9c68f055970b0b78cf621946644daf940bda65a8c3125d4b35ae5d1b` (38,271) | `97fa2bab…8fed` |
| `claude-cross-session-messaging` | `d68cba02a8597a58825ae954e0c25265c0108e3926c13fb5b0c72028b70a37f0` (37,464) | `886e6885…3620` |
| `claude-cloud-environments` | `31e38896dbbeb74ebc5471b05bfd8f3d57134d2e2d7063945466032849a15720` (67,124) | `3e230231…9d1` |
| `claude-self-hosted-environments` | `33bed9e7f6541408584e9a6bd8ebb42bc4e3fb05d7131bfce8105e0980ff9ef9` (21,742) | `ec3fc5f3…5c9c` |
| `claude-self-hosted-environments-configuration` | `ac98c78146986c3ab14e895b711c395842b265a37f4902bd2ca8f56f2542af45` (106,366) | `a2521568…f4ff` |
| `claude-code-changelog` | `d6636805…`, `4192cfda…a5ad` | `2282079d…`, `0937ba0c…f217` |

The profile-relevant statements were spot-checked and still hold:
`plugins/components.md:364` ("every hook process receives
`CLAUDE_PLUGIN_ROOT` and `CLAUDE_PLUGIN_DATA` … plus
`CLAUDE_PLUGIN_OPTION_<KEY>`"); `cross-session-messaging.md:248`, `:255`
(socket exported to hooks, token alongside it); `cloud-environments.md:370`,
`:515-535` (`CLAUDE_CODE_REMOTE_SESSION_ID`, `CLAUDE_CODE_REMOTE=true`).
`self-hosted-environments.md` was hashed but not re-read for its version
statement; re-check it when the supplement is authored.

Proposed supplement change: no profile or variable added or removed for
the 33 events. Map `PreSkillShellCommand` with PreToolUse's profiles
(baseline, effort, trace-context, claude-process, cloud-session,
local-remote-control, messaging, messaging-token, plugin-paths,
plugin-options; no `environment-file`); nothing upstream states this, so
record it as inferred from the CLI's shared input builder and runner. Keep
`environment-file` optional. Add limitations for the PowerShell exclusion,
the PowerShell consumption rule, HIPAA credential removal, mod
`$.env.set`, the ELECTRON/FORCE_COLOR removals, and the self-hosted Slack
variables (model them only if runner-provided variables are judged native
hook environment, and then as optional self-hosted-only variables with
per-variable presence, not in the all-or-none `cloud-session` profile).
Add `claude-hooks-reference` to the effort profile's sources.

Native impact: none to parsing. `ENVIRONMENT_FILE_EVENTS` rustdoc
(`crates/hookkit-claude/src/environment.rs:44-58`) should add the
PowerShell exclusion and consumption rule. `ClaudeCommandEnvironment`
ignores inherited extras (`CLAUDE_CODE_REMOTE_SESSION_UUID`, Slack
variables). Existing tests already cover CLAUDE_ENV_FILE scoping
(`environment.rs:557-589`); new tests: SessionStart with `CLAUDE_EFFORT`
absent (effort `None`) and with an unknown level (preserved); a Cloud
environment with `CLAUDE_CODE_REMOTE_SESSION_UUID` and
`CLAUDE_CODE_REMOTE_SLACK_THREAD_TS` set and no thread URL still parses as
`Cloud { remote_session_id: "cse_x" }`; once PreSkillShellCommand exists,
`CLAUDE_ENV_FILE` is ignored on it.

## Corrections to the selected snapshot and earlier audits

The selected snapshot was already wrong, or is now contradicted by better
evidence, here:

- `PreCompact` `plain-text` is not a no-op; the 2.1.285 build already
  appended it to the compaction instructions.
- JSON-lines output with no field set is plain text on all 31 standard
  events, not just the four text-context ones.
- `PreToolUse` deprecated top-level `decision`/`reason` is mapped by Claude
  Code but rejected by the output schema.
- `Elicitation` and `ElicitationResult` ignore `hookSpecificOutput` on
  every nonzero exit, so `nonzero-structured` is wrong there.
- `PermissionRequest` `mcp_server` is not "Present only for MCP tools"
  (2.1.332 MCP-skill asks).
- `PreToolUse` `continue: false` does not block the call, and on `Stop` /
  `SubagentStop` it overrides exit 2.
- `TaskCompleted` and `TeammateIdle` honor a top-level `decision: "block"`,
  which their output schemas rejected (the 2.1.285 CLI already mapped it).
- The missing-file exit-2 rule on `Stop`, `SubagentStop`, `TaskCompleted`,
  and a plugin's `UserPromptSubmit` (and, per the CLI, `TeammateIdle`)
  already existed at 2.1.285.
- The `WorktreeCreate` v2.1.216 floor for the path rule is no longer
  supported by the reference, and the WorktreeRemove "stderr goes to the
  debug log" note is no longer stated.
- The 10,000-character cap (2.1.89) and the async/`asyncRewake` process
  rules predate the pin and were never recorded.
- Stale uncertainties: Setup "still names mcp_tool"; UserPromptSubmit
  `session_title` "not documented"; Elicitation `decision` "sourced only
  from the changelog"; PreToolUse `updatedInput` without a decision
  "undocumented"; PostCompact/SessionEnd `terminalSequence` "not said".

The 2026-09-29 audit's "Async command hooks … are not subject to
`timeout`" is wrong for `asyncRewake` (`cc-hooks.md:4210`).

Native documentation that is already wrong at the pin:
`SessionStartInput.agent_id` ("when a subagent session starts"),
`PreCompactOutput::no_op` (`{}` becomes instructions),
`hookkit-tool-access` "failed calls … only through PostToolUseFailure",
`with_continue` ("precedence over event-specific decisions") on
PreToolUse, the Elicitation `block_builders!` text ("the hooks reference
does not yet document it"), and `WorktreeRemoveOutput::failed` ("writes the
stderr to its debug log").

## Evidence and classification

- Additive: `PreSkillShellCommand`; UserPromptSubmit `session_title`;
  Notification values; `agent_id` cases; Stop `background_tasks` labels.
- Behavioral: `onFailure`; the Windows default; Code Mode; async
  multi-line JSON; escaping; `updatedInput` validation and sandbox rules;
  Monitor coverage; `allow` exceptions; UserPromptSubmit ordering,
  interrupts, replay; PostToolUse interrupts and Cowork; PermissionRequest
  Cowork and re-ask; PermissionDenied wording; harness refusals; idle
  compaction as `auto` (2.1.299, 2.1.314); `idle_prompt` with background
  agents (2.1.288); the firing fixes for SubagentStop (2.1.311), SessionEnd
  (2.1.300), ConfigChange (2.1.290), and WorktreeRemove (2.1.301).
- Correction: PreCompact instructions; JSON-lines; deprecated `decision`;
  Elicitation nonzero; `continue: false`; the missing-file exit 2 (in the
  2.1.285 build); TaskCompleted and TeammateIdle top-level block;
  `mcp_server`; WorktreeCreate floor; HTTP schema-invalid body; async and
  cap omissions.
- Documentation-only: PostCompact/SessionEnd `terminalSequence`;
  MessageDisplay single call; SessionStart semantics; PostToolBatch
  refused calls; StopFailure `overloaded`; Setup; tool inventory; handler
  details; environment hashes; ElicitationResult on user decline and
  cancel, the skip after an Elicitation answer, and whole-object
  `content`; subagent matchers and the web-fetch subagent; worktree firing
  and reporting; ConfigChange, CwdChanged, DirectoryAdded, model-switch,
  and SessionEnd firing scope.
- Breaking for existing HookKit outputs: none in bytes. Semantically,
  `PreCompactOutput::no_op()` already injected `{}`, and
  `nonblocking_error` and `FailurePolicy::NonBlocking` block under
  `onFailure: block` or the Windows default. Breaking for consumers of the
  ledger: PreCompact `plain-text` → `compaction-instructions` and the
  Elicitation outcome rename change outcome ids that conformance cases
  cite; typing `updatedInput` as `object` narrows two output schemas.
  `exit-2-missing-file` now admits whitespace-only stdout and renames its
  stderr `semantic_format` to `contains-no-such-file-or-cant-open`. The
  TaskCompleted and TeammateIdle output schemas widen; no outcome id
  changes.

## Items checked and unchanged

- Event inventory in the reference: the same 33 events
  (`cc-hooks.md:31-65`, `:1283-3919`); none removed or renamed.
- `handler_kinds` for all 33 events; agent hooks still skipped on
  PermissionRequest (`:4016`).
- Common input fields (`:753-771`): `session_id`, `prompt_id`,
  `transcript_path`, `cwd`, `scratchpad_dir`, `permission_mode` (Manual
  arrives as `default`, `:758`, `:760`; `values.rs:117-120`), `effort.level`,
  `hook_event_name`, `agent_id`, `agent_type`; `model` only on SessionStart.
  SDK `BaseHookInput` is byte-identical between 0.3.285 and 0.3.332 and
  still omits `scratchpad_dir`.
- The universal output field set (`:1107-1113`) and SDK
  `SyncHookJSONOutput` top-level fields.
- The core exit-code table cells (`:811-815`) apart from the `onFailure`
  qualifier: exit 2 with schema-invalid JSON still blocks with stderr; a
  JSON blocking reason replaces stderr (`:848`, `:854`).
- The brace rule (v2.1.248) and the four text-context events (`:839`);
  exit-0 stderr is never shown to Claude (`:842`).
- The per-event exit-2 table (`:1034-1068`) and the SessionStart,
  SubagentStart, and PostModelSwitch notice rule (`:1070`); every snapshot
  exit-2 effect and stderr role matches.
- HTTP basics: POST with JSON (`:512`); empty 2xx success; non-JSON 2xx
  and non-2xx non-blocking; per-event failure contracts (`:1076-1081`).
- Default timeouts: 600 s command/http/mcp_tool, 30 s prompt, 60 s agent;
  30 s UserPromptSubmit, PreModelSwitch, PostModelSwitch; 10 s
  MessageDisplay; a PreModelSwitch timeout blocks (`:425`, `:904`,
  `:1532`, `:1664`).
- mcp_tool skipped on SessionStart at launch and every Setup (`:597`).
- The decision-control table (`:1209-1223`), `additionalContext` placement
  (`:1179-1185`), workspace trust (`:4299-4306`), dedupe (`:411`), cwd
  fallback (`:413`).
- Input fields, required sets, and enums of every existing event,
  including SessionStart `source`, Setup `trigger`, InstructionsLoaded
  `memory_type`/`load_reason`, UserPromptExpansion `expansion_type`
  (`slash_command | mcp_prompt`), MessageDisplay, PreToolUse,
  PermissionRequest, PostToolUse, PostToolUseFailure, PostToolBatch,
  PermissionDenied, SubagentStart, SubagentStop, TaskCreated,
  TaskCompleted, Stop, TeammateIdle (`team_name` deprecated but sent),
  ConfigChange, CwdChanged, DirectoryAdded, FileChanged, WorktreeCreate,
  WorktreeRemove, PreCompact (`custom_instructions` string or null),
  PostCompact, PreModelSwitch, PostModelSwitch, SessionEnd (`reason`,
  legacy `bypass_permissions_disabled`), Elicitation, ElicitationResult.
  SDK 0.3.285 → 0.3.332 changes none of them (whitespace in
  PermissionRequest; comments in UserPromptSubmit and PostToolUse).
- Output fields of every existing event, including
  UserPromptExpansion `suppressOriginalPrompt` (still SDK-only) and the
  PostToolUseFailure decision-table inconsistency (`:1211` vs
  `:2437-2441`).
- Setup discards everything; InstructionsLoaded discards `systemMessage`
  and `continue` and ignores the exit code; MessageDisplay displays the
  original on failure (`:1478`, `:1517`, `:1664`, `:1782`).
- Representative-fixture `permission_mode` presence matches the upstream
  examples.
- `effort.level` and `CLAUDE_EFFORT` value sets; `permission_mode` value
  set (mode-selection changes in 2.1.302-2.1.326 only change which value
  appears).
- PreModelSwitch and PostModelSwitch inputs, outputs, and matcher; the
  Elicitation URL mode (already modeled).
- UI, telemetry, debug, and loading changes with no protocol effect
  (`/hooks` menu, transcript styling, crashed-hook display, `--debug`
  lines, trust dialog, OpenTelemetry fields, sandbox protection of hook
  files, malformed `plugin.json`).
- Mods' hook modules and the self-hosted runner's lifecycle hooks are
  separate systems and out of scope.
- SDK `core.d.ts` and `agentSdkTypes.d.ts` change only exports unrelated to
  hooks.
- No `hookkit-claude` typed constructor emits JSON with a nonzero exit
  other than 2 or brace-delimited invalid JSON, so the `onFailure`,
  Windows, and Elicitation-nonzero rules change no emitted bytes.

## Open questions and uncertainties

- **Pin.** 2.1.332 (npm `latest`, precedent) or 2.1.326 (npm `stable`)?
  Only `PreSkillShellCommand` depends on it.
- **CLI bundle as a source.** Several PreSkillShellCommand rules and the
  PreCompact `{}` behavior rest on a minified build. Cite it as
  `claude-code-cli-bundle` (`kind: official-package`, `authority:
  secondary`, `pinned-revision`) with a limitation that it was inspected
  by string search, or keep it audit-only and record those rules as
  uncertainties? Source `kind` is free-form in `meta/v1`, so either works
  without a format change.
- **Handler-configuration metadata.** Sibling bindings (proposed) or a
  `handler_options` qualifier on `Outcome` (format change; `Binding` and
  `Outcome` are `deny_unknown_fields`)? The same choice covers async hooks
  and the Windows default, and should be made jointly with the Codex and
  Antigravity successors.
- Under `onFailure: block`, does schema-valid JSON printed with exit 1
  still apply its fields alongside the failure?
- `onFailure: block` on events whose exit 2 stops nothing: notice, or
  ignored? (The SDK ignores mcp_tool on Setup and SessionStart only.)
- The missing-file match rule: the CLI answers it (case-insensitive `no
  such file` or `can't open` on stderr, stdout empty after trim, skipped
  when stdio was cut), but the reference does not. Should the CLI-only
  parts (whitespace-only stdout, TeammateIdle) stay in uncertainties, or
  become outcomes once the CLI bundle is cited? Why only a plugin's
  UserPromptSubmit?
- Does the suspect-capture block apply beyond PreToolUse? Is the 10 s
  grace configurable?
- Does `continue: false` beat exit 2 on TeammateIdle and TaskCompleted?
- Elicitation/ElicitationResult: the CLI suggests top-level `decision:
  block` declines on exit 1 or 3-255; the reference is silent. Does
  ElicitationResult `cancel` forward the user's values, and does an
  `accept` without `content` forward them or none?
- PostModelSwitch: which `source` does an `opusplan` plan-mode transition
  report? DirectoryAdded: does it fire when the 2.1.326 `/add-dir` trust
  prompt is declined?
- SubagentStop repeated firing (fixed 2.1.311): which releases had it?
- PostCompact/SessionEnd: does exit 1 with schema-valid JSON still show
  the stderr notice?
- PreCompact: does exit-0 JSON contribute instructions in documented terms
  (the CLI says yes, raw), and how do hook instructions combine with the
  user's `/compact` text?
- Monitor via a Bash matcher: is `tool_use_id` the Monitor call's id, and
  does any field reveal that allow and rewrite will be ignored?
- 2.1.326 "decision: block holds in more cases": which events?
- 2.1.300 `updatedInput` validation: is a rejected rewrite reported to
  Claude with a reason; is a dropped key observable?
- StopFailure: does 2.1.326+ ever send `overloaded`?
- `background_tasks[].command` for monitor tasks: SDK lag or reference
  error?
- Prompt/agent concurrency: reference (12 per event) vs changelog 2.1.312
  (24 per session plus 12 overdue, 1 s minimum).
- MCP skills: v2.1.331 (docs) or 2.1.332 (changelog)? Which
  `expansion_type` and `command_source` do they produce?
- Runner-provided variables (`CLAUDE_CODE_REMOTE_SESSION_UUID`, Slack
  variables, `CLAUDE_RUNNER_SESSION_UUID`): native hook environment or
  inherited state?
- PreSkillShellCommand: `tool_use_id` format; `command_source` vocabulary;
  exit-2 stderr role; whether `systemMessage` reaches the user (the CLI
  relays `<hook> says: …`); environment profiles.
- MessageDisplay single call and managed PostToolUse on failed calls: new,
  or snapshot-time omissions?
- Served remote calls send `session_id` `served:<id>` and an empty
  `transcript_path`: add an uncertainty on every event?

## Claims narrowed or rejected during verification

- "Under `FailClosed`, HookKit exits 2 for an unknown
  `PreSkillShellCommand`" (CC-SHARED-14, CC-CL-1): wrong. The runtime names
  the event through the shared discriminator and lowers it as `NotGating`
  (exit 1), so the guard fails open today.
- "PreSkillShellCommand matcher, handler kinds, and exit 2 are
  undocumented and unknowable" (CC-CL-1): the CLI bundle settles them;
  prompt and agent handlers refuse rather than being skipped (CC-SDK-3
  corrected).
- "Output negative for `hookEventName: "PreToolUse"` on
  PreSkillShellCommand": rejected; the CLI accepts it.
- "`invoked_by` should be an open string per the closed-common-enums
  precedent": the precedent does not transfer (the SDK closes the union);
  either modeling is defensible.
- "The missing-file rule has no changelog entry" (CC-SHARED-3, CC-A-3):
  correct after all. The interim reading that 2.1.327 `:786` was its source
  ("probably 2.1.327", also CC-B-5) is refuted, because the 2.1.285 CLI
  already contains the rule; `:786` only changed the display. CC-B-5's
  "TeammateIdle is not included" and its proposed "stays `continue`"
  TeammateIdle control are refuted by the same builds. "Fix the
  FailClosed lowering for Stop, SubagentStop, and TaskCompleted": moot;
  they are not gates.
- "TaskCompleted/TeammateIdle `decision: block` is most likely new,
  probably 2.1.326" (CC-B-3): refuted. The 2.1.285 generic JSON processor
  already applied it, and `:1192` names no event.
- "2.1.290 changed the hook `agent_id` from a `name@team` address"
  (CC-B-12): unsupported; `:3713` concerns Agent results.
- CC-B-11's open `failure_policy` metadata proposal is superseded by the
  decided sibling-binding model. CC-B-16's "attribute to v2.1.325" is a
  route floor for EnterWorktree, not a floor for the general path rule.
- "Elicitation nonzero JSON is a non-blocking error" (CC-SHARED-6): too
  strong; the run is a success whose `hookSpecificOutput` is ignored, so
  the dialog proceeds (`no-op`), notice undocumented.
- "PostCompact/SessionEnd terminalSequence is a correction" (CC-SHARED-8):
  narrowed to documentation that resolves an uncertainty, by example only.
- "The snapshot lacks the JSON-lines parse-failure half" (CC-SHARED-10,
  CC-A-18): wrong for the text-context events, whose `TEXT_JSON_RULE`
  implies both halves; the real inaccuracy is on the 27 other events.
- "1xx should be modeled in HTTP `non-success`" (CC-SHARED-12): rejected.
- "The 10,000-character cap is drift" (CC-SHARED-15, CC-A-17): it dates
  from 2.1.89, so it is an omission; the escaping (2.1.292) is behavioral,
  not documentation-only; the 2.1.332 50,000-character shortening is
  ambiguous and above `stable`.
- "`continue: false` interacts the same way on PermissionRequest"
  (CC-SHARED-16): unsupported; undocumented there. "Non-allowlisted
  `terminalSequence` is ignored" is not new.
- "SessionStart `onFailure` docs become false" (CC-A-1): wrong; its exit-2
  row is the same user notice.
- "The `ask-with-reason` fixture is new" (CC-A-8): the snapshot already
  has it.
- "Audit the spawn sites for stdio inheritance" (CC-A-10): already done;
  every site sets stdio explicitly.
- "`PostToolUseOutput::block` rustdoc needs fixing" (CC-A-12): it is
  already correct; the rewrite pairing was in SDK 0.3.285.
- "PostToolBatch refused calls are a 2.1.328 behavior change" (CC-A-13):
  narrowed to a changelog clarification, version unknown.
- "SessionStart semantics are new" (CC-A-15): mostly pre-pin
  documentation; only the 2.1.323 `agent_id` fix is in-window.
- "Six independent `updatedInput` changes" (CC-CL-9): two are a
  conformance fix and a duplicate of the Monitor item.
- "Native impact: none" for handler kinds (CC-CL-20): wrong for
  `onFailure`, which changes the command binding.
- "The PermissionRequest `onFailure` wording is a docs-vs-SDK
  contradiction" (CC-CL-2, CC-SDK-4): loose SDK wording; the CLI denies.
  "The SDK mcp_tool ignore list conflicts with the reference": the CLI
  agrees with the SDK; the reference omits it.
- "`CLAUDE_CODE_REMOTE_SESSION_UUID` is new for the supplement" (CC-CL-18):
  already recorded; only the Slack variables are new, and the 2.1.328
  entry is ambiguous.
- "The PostToolUse scheduled-task rule covers both tools" (CC-CL-14):
  only ExitPlanMode, only user-settings hooks, changelog-only.
- "UserPromptSubmit block prevents UserPromptExpansion and
  PreSkillShellCommand" (CC-CL-13): inferred, not documented.
- "WorktreeCreate `.`/`..` screening since v2.1.216" (CC-CL-19): the
  reference no longer states a version.
- "MCP skill names put matchers on the regex path for the first time"
  (CC-CL-23): plugin-scoped names already contained `:`.
- "PreCompact instructions should reuse `text_context`" (CC-CL-8,
  CC-SDK-6): rejected; PreCompact has no `additionalContext` fallback.
- Citation fixes: the Agent SDK callback timeout is `cc-hooks.md:1801`, not
  `:908`; the Elicitation lines are `:3815`, `:3828`, `:3830`; the
  `onFailure` changelog line is `:3250`; `failure.rs` closure and arms are
  at `:373-378` and `:390-395` on `HEAD`; the hookkit-shell and
  tool-access Codex impls were cited for Claude in CC-CL-14 (the Claude
  ones are `adapters.rs:86-98` and `call.rs:229-245`).

## Post-review corrections

Two reviews of the draft (2026-11-25) changed the following; the draft
snapshot and `xtask/src/bin/generate-claude-contracts.rs` carry them.

- The hooks reference changed upstream after it was staged. From about
  06:16 UTC, `curl --compressed` returns 301,732 bytes (SHA-256
  `d16003851f60562eaa793e6270316b17550f8e24691214d7d798e5a503537c79`),
  which `sources.yaml` now pins; it cites the staged `194272ed…3099` body
  as reviewed. The new text documents `PreSkillShellCommand` (its own
  section plus rows in the event, matcher, `if`, Windows-without-Git-Bash,
  exit-2, decision-control, and handler-kind lists) and adds a PreToolUse
  note that injected commands don't fire it. The "0 matches" claim above
  and the CLI-only status of the matcher, `if`, prompt/agent blocking,
  PreToolUse-format answers, top-level `decision: "block"`, and the
  `continue: false` message are superseded. A successor command-environment
  supplement that cites `hooks.md` must use the new hash.
- `PreSkillShellCommand` now cites the reference for input, output, and
  outcomes; accepts `hookSpecificOutput` named `PreToolUse` (the reference
  prescribes that format); uses the documented `command_source` values;
  records exit-2 stderr as `user-message-and-agent-context`; and adds the
  documented frontmatter, permission-check, auto-mode, Code Mode timeout,
  Remote Control, and Windows rules as uncertainties.
- `onFailure: "block"` on the 15 generated notice events: `nonzero-structured`
  does change (schema-valid JSON with exit 1 or 3-255 counts as a failure,
  `cc-hooks.md:813`, `:982`), so the "no modeled outcome changes" text in
  the `onFailure` section above is narrowed to events without that outcome
  (Setup, WorktreeRemove), and WorktreeRemove gets its own text.
- Elicitation and ElicitationResult: a top-level `decision: "block"` with
  exit 1 or 3-255 declines (`nonzero-decision-block`), because the
  exit-code exception covers only `hookSpecificOutput` and the generic
  other-exit-code rule lets the fields decide. The Elicitation effect id
  stays `deny`, as in `docs-2026-09-30-r1`, with an uncertainty.
- HTTP `non-success` covers 300-599 ("Non-2xx status" is a failure); the
  output-discarding events gain `invalid-json` and the JSON-lines plain-text
  rule; the on-failure-block bindings gain an `exit-2-structured` case; the
  mcp_tool non-empty-answer rule under block is recorded (with a PreCompact
  note); PreToolUse `command-exit-2-continue-false` maps to
  `exit-2-structured`; PermissionRequest records the 2.1.300 and 2.1.326
  rewrite rules; the MCP-skill ask is marked as CLI inspection.

## Final status

`docs-2026-11-25-r1` is frozen with the post-review corrections, recorded in
`FROZEN_LEDGER`, and selected in `contracts/registry.yaml`, together with
supplement `command-environments-2026-11-25-r1`. The supplement pins
`hooks.md` at `d16003851f60562eaa793e6270316b17550f8e24691214d7d798e5a503537c79`,
maps `PreSkillShellCommand` to the `PreToolUse` profiles without
`environment-file`, and records the PowerShell `CLAUDE_ENV_FILE` exclusion.
`hookkit-claude` implements the snapshot, and all 34 events,
`PreSkillShellCommand` included, are `command-runtime-beta` targets for the
`command` binding with `source-reviewed` verification.

The native changes follow this note, with these decisions:

- `PreCompactOutput::no_op()` prints nothing, and
  `PreCompactOutput::compaction_instructions` writes instructions.
- `StopOutput` and `SubagentStopOutput` `into_blocking_error` reject a
  response that sets `continue: false`.
- `TaskCompletedOutput` and `TeammateIdleOutput` gain `block` and
  `with_block`.
- `UserPromptSubmit` rejects a non-string `session_title`;
  `PreSkillShellCommand` does not check the optional `command_source`, and
  its context carries no tool-call id.
- Under `FailurePolicy::FailClosed`, `PreSkillShellCommand` exits 2, and
  `UserPromptSubmit` exits 2 with a JSON `block` decision on stdout, as the
  aligned `UserPromptSubmitOutput::block` now does on Claude Code.
- `hookkit-shell` matches `Monitor` calls that run a command
  (`CLAUDE_MONITOR_PROFILE`), and `hookkit-tool-access` reports a
  `PostToolUse` with a string `tool_response` as `Conditional` evidence.

Deferred:

- `command-on-failure-block` is `catalog-only`: no native descriptor
  declares it and conformance does not execute its cases, so
  `contracts/status/implementation-gaps.yaml` records a
  `binding-implemented` gap on each of its 14 events. Implementing it also
  needs a binding name in `HandlerKind` and in the conformance registry
  writer, and xtask's report and support checks, which treat only `command`
  as a command runtime. `http-on-failure-block` is `unsupported`, like every
  HTTP binding.
- `PreSkillShellCommand` is not a `hookkit-shell` shell source, there is no
  PowerShell profile (PowerShell calls stay `NotShell`), and
  `forbidden-file-guard` does not handle `PreSkillShellCommand`.
- On `PreCompact`, structured JSON at exit 0 that does not block, such as
  `no_op().with_terminal_sequence(..)`, is still appended to the compaction
  instructions, according to inspection of the CLI; this is documented on
  the type but not prevented.
- The runner variables `CLAUDE_CODE_REMOTE_SESSION_UUID` and
  `CLAUDE_CODE_REMOTE_SLACK_*` are not modeled.
- Every question under "Open questions and uncertainties" is still open.
