# Antigravity hook contract refresh: `docs-2026-11-25-r1`

- Audit date: 2026-11-25
- Previous selected snapshot: `docs-2026-09-30-r1` (selected in `620d5f1`)
- Proposed successor: `docs-2026-11-25-r1`, **draft (not yet frozen or
  selected)** when this note was written; since frozen and selected (see
  "Final status")
- Selected supplement: `command-environments-2026-09-30-r2`; it needs a
  successor that targets `docs-2026-11-25-r1`
- Native crate: `crates/hookkit-antigravity` (implements
  `docs-2026-09-30-r1`)
- Official hook reference: <https://antigravity.google/docs/hooks> and its
  Markdown source <https://antigravity.google/docs/hooks.md>
- Former IDE reference: <https://antigravity.google/docs/ide/hooks>, still a
  redirect to `/docs/hooks?tab=ide`
- Release notes: <https://antigravity.google/docs/changelog.md> (App through
  v2.35.1, CLI through v1.5.16, IDE Extensions through VS Code v1.13.0, SDK
  through v0.1.24, IDE through v2.8.0) and the CLI repository changelog at
  `bb5c8da497ef2eb2e2b98e6a092baed90c567938` (CLI 1.5.16)
- Newly consulted official pages:
  <https://antigravity.google/docs/build-plugins.md>,
  <https://antigravity.google/docs/plugins.md>,
  <https://antigravity.google/docs/mcp.md>, and the refreshed
  <https://antigravity.google/docs/permissions.md>

The upstream bodies retrieved on 2026-09-30 were hashed but never captured,
and `web.archive.org` is unreachable from this environment. Drift was
therefore found by comparing today's text with what `docs-2026-09-30-r1`
encodes (schemas, fixtures, outcomes, uncertainties, and `sources.yaml`
limitations) and with the 2026-09-29 and 2026-09-30 audit notes. Nothing
documented only in the reference can be dated more precisely than "after
2026-09-30". Every release note after App v2.18.1, CLI 1.2.14, SDK v0.1.18,
and IDE v2.5.5 was reviewed, together with the IDE Extensions section. Line
numbers below refer to the staged copies of each source, whose hashes match
the table under "Evidence and classification".

Two audit slices (reference text and release notes) produced 37 findings.
Each was adversarially verified; none was refuted outright, but many were
narrowed. This note uses the verified wording, resolves the conflicts
between the slices, and records the narrowed claims at the end.

## Result

The documented inventory grows from five to **six** command events:
`SessionStart` is new. No event was removed or renamed. The headline
changes are:

- **New event `SessionStart`.** It fires once when a new conversation
  session starts, receives the common fields only, and may return
  `injectSteps`.
- **`PreToolUse` `allow` changed meaning.** It is now a pass-through
  ("delegating to the next hook or default policy"); the old auto-approval
  meaning moved to the new decision `auto_approve`. No release note dates
  the redefinition.
- **New `PreToolUse` output `overwrite`**, a shallow top-level merge into
  `toolCall.args`.
- **The inject-step vocabulary changed** for `PreInvocation`,
  `PostInvocation`, and `SessionStart`: new object kinds `systemMessage`
  and `errorMessage`, `ephemeralMessage` deprecated, and the `toolCall`
  step no longer documented. The official `PreInvocation` reminder example
  now uses `systemMessage`, so the snapshot's `official` fixture is stale.
- **`PreInvocation` gains `terminationBehavior`** (`"terminate"` or
  `""`), which with an `errorMessage` step blocks a prompt.
- **New common input field `lastUserInput`.**
- **Failure handling is now documented as fail-open**, on the plugin
  authoring page: a nonzero exit is logged and ignored. This reverses the
  snapshot's "do not presume fail-open" guidance.
- **Plugin hooks receive `PLUGIN_ROOT` and `PLUGIN_DATA`** (CLI 1.5.3),
  so the supplement's empty Antigravity environment is incomplete.
- **Release notes now lead the reference** on several input fields:
  `hookEventName`, `cwd`, and `toolCall.id` (CLI 1.5.4) and the `Stop`
  field `continuation_active` (CLI 1.5.6) are not in the reference.

| Event | Current event-specific input | Current output | Change from selected contract |
| --- | --- | --- | --- |
| `SessionStart` (new) | common fields only | optional `injectSteps` | new event |
| `PreToolUse` | `toolCall{name,args}`, `stepIdx` | required decision (6 values); optional `reason`, `overwrite`, `permissionOverrides` | `allow` redefined; `auto_approve` and `overwrite` added; failure outcome |
| `PostToolUse` | optional `toolCall{name,args}`, `stepIdx`, optional `error` | exactly `{}` | common input; failure outcome; uncertainties |
| `PreInvocation` | `invocationNum`, `initialNumSteps` | optional `injectSteps` (new step vocabulary); optional `terminationBehavior` (`terminate`, `""`) | step vocabulary; termination; official fixtures; failure outcome |
| `PostInvocation` | same as `PreInvocation` | optional `injectSteps` (new step vocabulary); optional `terminationBehavior` (3 values) | step vocabulary; failure outcome |
| `Stop` | `executionNum`, open `terminationReason`, optional `error`, `fullyIdle` | required open decision; optional `reason` | common input; `continuation_active` (release notes only); failure outcome |

The common input changes for every event are an optional `lastUserInput`
(documented) and optional `hookEventName` and `cwd` (release notes only).
`handler_kinds` stays `[command]` for all six events.

## Event inventory change

`docs-2026-09-30-r1/snapshot.yaml:8-23` lists `PreToolUse`, `PostToolUse`,
`PreInvocation`, `PostInvocation`, and `Stop`. The successor adds
`events/session-start` (`wire_name: SessionStart`, `rust_key:
session_start`, `category: session`). The expected
`cargo xtask contracts diff antigravity/docs-2026-09-30-r1
antigravity/docs-2026-11-25-r1` result is 1 event added, 5 changed, and
`sources.yaml` changed. `Stop` and `PostToolUse` change for the common
input fields, the failure outcome, and new uncertainties.

## Confirmed drift by event

### `SessionStart` (new event)

Evidence:

- `ag-hooks.md:109`: "| `SessionStart` | array | Handlers that run once
  when a new conversation session starts, before the first model
  invocation. |"
- `ag-hooks.md:122`: "| `SessionStart` | Fires once when a new conversation
  session starts, before the first model invocation. | N/A (matcher
  ignored) |"
- `ag-hooks.md:140`: "For `SessionStart`, `PreInvocation`,
  `PostInvocation`, and `Stop`, the structure is simpler (a list of
  handlers directly under the event key) and the matcher is ignored."
- `ag-hooks.md:254` (input): "| _(Common fields)_ |  | Includes
  `conversationId`, `workspacePaths`, `transcriptPath`,
  `artifactDirectoryPath`, `modelName`, and `lastUserInput`. |"
- `ag-hooks.md:260` (output): "| `injectSteps` | array of objects |
  **Optional.** List of steps to inject into the conversation trajectory
  at the start of the session (same schema as `PreInvocation` inject
  steps). |" No `terminationBehavior` is documented for this event.
- Official example input `ag-hooks.md:266-274` (common fields with
  `modelName`, no `lastUserInput`) and output `ag-hooks.md:278-288`
  (`{"injectSteps":[{"systemMessage":{"systemMessage":"Environment
  initialized for workspace /workspace/project"}}]}`); configuration
  example `ag-hooks.md:91-98`.
- `build-plugins.md:492`: "| `SessionStart` | A new conversation is
  initialized | _(none — flat handler array)_ | Return `{"injectSteps":
  [{"userMessage": "..."}]}` to inject startup context. |";
  `plugins.md:66` lists bundled `SessionStart` hooks.
- No App, CLI, IDE, or IDE-extension release note introduces the event.
  The only mention is CLI 1.5.5 (2026-11-10, `ag-cli-changelog.md:247` =
  `ag-changelog.md:2197`), about `agy plugin import`: "`PreToolUse`,
  `PostToolUse`, `Stop`, and startup `SessionStart` hooks are now
  converted". That suggests, but does not prove, that only Claude Code's
  `startup` source corresponds to Antigravity's `SessionStart`. The SDK's
  `OnSessionStartHook` (`ag-changelog.md:6886`, SDK v0.1.5) is an
  in-process SDK hook, not a `hooks.json` event.

Classification: **additive** (new event).

Proposed contract (`events/session-start`):

- Input schema: required `conversationId`, `workspacePaths` (no
  `minItems`), `transcriptPath`, `artifactDirectoryPath`; optional
  `modelName`, `lastUserInput`, `hookEventName`, and `cwd`;
  `additionalProperties: true`.
- Output schema: `{injectSteps?: InjectStep[]}` with
  `additionalProperties: false` and the shared step definition below. No
  `terminationBehavior`.
- One outcome `inject` (effect `rewrite`, exit `{exact: 0}`, stdout
  required JSON, stderr diagnostics), plus the `failure` outcome described
  under "Process outcomes". `handler_kinds: [command]`.
- Identification: `ambiguous`, overlaps `[PreToolUse, PostToolUse,
  PreInvocation, PostInvocation, Stop]` (see "Identification" below).
- Uncertainties: the first build and surfaces are undated; whether it
  fires on resume, fork, compaction, subagent conversations, or app-restart
  auto-resume is undocumented (CLI 1.5.5's "startup" wording points to new
  sessions only, and CLI 1.3.3, `ag-cli-changelog.md:563`, stopped `/btw`
  side questions running user and workspace hooks on the CLI); whether
  `lastUserInput` is present or `""` before the first prompt; whether an
  `errorMessage` step or an undocumented `terminationBehavior` has any
  effect; its ordering relative to `PreInvocation` with `invocationNum`
  0.

Native impact. `crates/hookkit-antigravity/src/lib.rs` has no
`SessionStart`: `events()` (lib.rs:51-60), `identification_descriptors()`
(lib.rs:66-74), `Event` (lib.rs:1101), `AnyInput` (lib.rs:1130),
`AnyCommandOutput` (lib.rs:1146), and `HarnessSpec::decode`
(lib.rs:1175-1186), which returns `UnrecognizedEvent` for it. The three
enums are `#[non_exhaustive]`, so the new arm is additive. The crate needs
`SessionStartInput` (common fields, `last_user_input`, `extra`),
`SessionStartOutput { inject_steps }` (no-op serializes as `{}`), and a
`SessionStart` `EventSpec` (`CATEGORY = Session`) whose `context()` emits
`SessionBoundaryContext::observed(SessionBoundaryKind::Startup)` keyed by
`conversationId`. Keep the `invocationNum == 0` `InvocationStart` fallback
in `invocation_context` (lib.rs:1077-1096) for builds without the event;
`crates/hookkit-session-state/src/metadata.rs:466-472` already records a
`Startup` boundary as `LifecycleHookObservation`.

Repository statements that `SessionStart` does not exist on Antigravity,
all of which must change:

- `crates/hookkit-tool-runner/src/cli.rs:178` ("Antigravity has no
  SessionStart event") and the test
  `session_start_rejects_antigravity_at_parse_time` (cli.rs:397-410);
  `crates/hookkit-tool-runner/src/lib.rs:191-222`
  (`run_session_start_observer` rejects Antigravity);
- `crates/hookkit-session-state/README.md:110-111` and `:152-154`;
- `README.md:848-851` ("the session-start observer only `claude|codex`");
- `docs/command-environments.md:16`, `:62`, and `:155`, and
  `contracts/README.md:28` and `:34` (event counts);
- `crates/hookkit-antigravity/src/lib.rs:3` ("five command events").

### Common input fields (all events)

#### `lastUserInput` (documented)

- `ag-hooks.md:227`: "All hooks receive the following system metadata
  fields in their input payload on `stdin`:"; `ag-hooks.md:242`: "|
  `lastUserInput` | string | The most recent user prompt in the
  conversation. |". It is also named in every event's common-field row
  (`:254`, `:304`, `:367`, `:409`, `:521`).
- Only the `PreInvocation` example carries it (`ag-hooks.md:444`,
  `"lastUserInput": "Fix the failing unit test"`). The `SessionStart`,
  `PreToolUse`, `PostToolUse`, and `Stop` examples omit it.
- CLI 1.4.7 (2026-10-24, `ag-cli-changelog.md:397`): "Added
  `lastUserInput` to the JSON that hook commands receive on stdin". App
  v2.28.0 (2026-10-27, `ag-changelog.md:561`) describes the feature
  without naming the key: "Hooks you define now receive the current user
  message". No IDE or IDE-extension note mentions it.

Classification: **additive**. No input schema declares it today; it is
accepted untyped through `additionalProperties: true`.

Proposed contract: optional `lastUserInput: {type: string}` in all six
input schemas, with an uncertainty that builds before CLI 1.4.7 and App
v2.28.0, the IDE, and IDE extensions may omit it, that four of five
official examples omit it, and that its value before the first prompt is
undocumented. Refresh the `PreInvocation` `representative` fixture
(`pre-invocation/fixtures.yaml:8-11`, origin `official`) verbatim from
`ag-hooks.md:436-445`. Refresh the `PostInvocation` `representative`
(`post-invocation/fixtures.yaml:8-11`) to the same bytes and keep it
`official`, because `ag-hooks.md:483` and `:497` say its input is "Same
as `PreInvocation`". (Fixture origins are only `official`,
`sanitized-live`, `synthesized`, and `regression`; `contracts/README.md:85-86`.)

Native impact: the field lands in the flattened `extra` maps
(lib.rs:113-115, :462-464, :681-683, :874-876). Add
`#[serde(default, skip_serializing_if = "Option::is_none")] pub
last_user_input: Option<String>` to every input struct (all are
`#[non_exhaustive]`), and update the crate docs at lib.rs:10-12.

#### `hookEventName`, `cwd`, and `toolCall.id` (release notes only)

- CLI 1.5.4 (2026-11-10, `ag-cli-changelog.md:257` =
  `ag-changelog.md:2217`): "Added `hookEventName`, `cwd`, and
  `toolCall.id` to the JSON input that hooks receive, so a single hook
  command can handle several events, knows the agent's working directory,
  and can match a `PostToolUse` call to its `PreToolUse` call."
- The reference omits all three: the common-field table
  (`ag-hooks.md:229-242`) and the `toolCall` rows (`:300-302`, `:364`).
  The only `Cwd` in the reference is the `run_command` argument. The
  reference lags the release notes here; it does not contradict them.
- No App, IDE, or IDE-extension note mentions them. Their value sets and
  types are undocumented: whether `hookEventName` uses the PascalCase
  event names, and whether `toolCall.id` is a string or a number.

Classification: **additive** (release-note sourced, low assurance).

Proposed contract: optional `hookEventName: {type: string}` and `cwd:
{type: string}` in every input schema (no enum), and an optional
`toolCall.id` in both tool events **without a type constraint** (a
description only), sourced to `antigravity-cli-changelog`. A
string-only `id` would reject a numeric id that the evidence does not
rule out. Uncertainties: the doc lag; surfaces other than CLI 1.5.4 and
later; the value set and type; and that the JSON `cwd` (the agent's
working directory) can differ from the hook process's working directory,
which for plugin hooks is the plugin root. Identification stays
`ambiguous` for every event that is ambiguous today: the catalog cannot
express a discriminator that is only sometimes present (xtask allows a
discriminator only on `definitive` events whose `const` equals the wire
name; `xtask/src/main.rs:2883-2898`).

Native impact: the values survive in `extra` and `ToolCall.extra`
(lib.rs:362-363, through the `ToolCallWire` flatten). Add typed
`hook_event_name: Option<String>`, `cwd: Option<Utf8PathBuf>`, and
`ToolCall::id: Option<serde_json::Value>` with a string accessor (and
update `ToolCallWire`). Do not reject a `hookEventName` that differs from
the hint at parse time; the value set is unobserved. Return the new
values from the aligned accessors that now return `None` for Antigravity:
`cwd()` and `tool_call_id()` in `crates/hookkit-common/src/aligned.rs`
(:258-268, :290-299, :585-590, :620-628) and `Stop` `cwd()` (:762-768).
The comments that Antigravity has no discriminator
(`crates/hookkit-runtime/src/resolution.rs:831`,
`crates/hookkit-runtime/tests/stdin_runners.rs:392`, lib.rs:64-65 and
:1042) should say "no documented discriminator".

#### Blank `modelName` before CLI 1.5.3

CLI 1.5.3 (2026-11-07, `ag-cli-changelog.md:282` =
`ag-changelog.md:2258`): "Fixed hooks receiving a placeholder or empty
model name when the session uses a custom model". The schema already
admits `""` (no `minLength`), but no contract or crate doc says that a
blank or placeholder value means unknown. Classification:
**documentation-only**. Extend the `modelName` uncertainty and add a
synthesized positive with `modelName: ""`. The native `model_name`
fields have no blank filter; a `model_name()` accessor using the existing
`non_blank` helper (lib.rs:1058-1060) would keep callers from treating
`""` as a model id.

#### Surfaces and `<app_data_dir>`

- `ag-hooks.md:3`: "Available on: Antigravity App, Antigravity CLI, IDE
  extensions (VS Code, Visual Studio, Eclipse, JetBrains, Android Studio,
  Zed, Xcode), Antigravity IDE". "Antigravity 2.0" is now "Antigravity
  App" here and in the changelog (`ag-changelog.md:3`, `:7`), with
  unchanged version numbering.
- `<app_data_dir>` is given only for the App, CLI, and IDE (`:17`, `:37`,
  `:48`, `:234-237`). IDE extensions have no location section and no
  `<app_data_dir>`. The ACP-based JetBrains, Zed, and Xcode extensions
  v1.0.0 (2026-08-20) support `hooks.json` lifecycle hooks
  (`ag-changelog.md:5714`), gate repository-local hooks behind Workspace
  Trust (`:5715`), and store conversation artifacts under
  `<GEMINI_HOME>/antigravity-acp` (`:5719`). Their v1.5.0 (2026-11-10)
  adds "secondary project folders in multi-root workspaces across ...
  session hooks" (`:5500`), so `workspacePaths` may carry several roots.
  Whether `antigravity-acp` is the hook-payload `<app_data_dir>` is
  inferred, not stated.
- A new cross-page conflict: `mcp.md:197` places the App's **and the
  IDE's** MCP cache under `~/.gemini/antigravity/mcp/`, while
  `ag-hooks.md:48` says the IDE's `<app_data_dir>` is
  `~/.gemini/antigravity-ide`.

Classification: **documentation-only**. The schemas do not constrain path
prefixes. Update the `sources.yaml` limitation that names three surfaces
(`docs-2026-09-30-r1/sources.yaml:14`), rename "Antigravity 2.0" to
"Antigravity App" in successor prose ("2.0 v2.12.0" becomes "App
v2.12.0"), and add an input uncertainty for the IDE-extension prefixes,
the `mcp.md` conflict, and multi-root `workspacePaths`. Native docs at
lib.rs:3-6 and :86-89 use the old names.

### `PreToolUse`

#### `allow` is now a pass-through

- `ag-hooks.md:311`: "`"allow"`: Allows the tool call to proceed,
  delegating to the next hook or default policy."
- The 2026-09-29 audit (line 106-107) recorded the reference's previous
  wording, "Automatically allows the tool execution", and the selected
  schema still says "allow auto-approves"
  (`pre-tool-use/output.command.schema.json:9`). The selected uncertainty
  (`pre-tool-use/contract.yaml:46`) says the reference "documents no
  pass-through or no-objection decision".
- No App, CLI, or IDE release note announces the redefinition.

Classification: **breaking** for meaning (the wire enum member is
unchanged, its semantics are not). The snapshot's "no pass-through"
uncertainty is resolved for current builds.

Proposed contract: describe `allow` as a pass-through to the next hook or
the default policy. Replace uncertainty `:46` with: the redefinition is
undated; builds documented by `docs-2026-09-30-r1` and earlier may still
auto-approve on `allow`; multi-hook precedence is undocumented beyond
"next hook"; an empty decision remains undocumented (keep the
`empty-decision` output negative). Add a synthesized output fixture
`allow-pass-through` (`{"decision":"allow"}`).

Native impact:

- `crates/hookkit-antigravity/src/lib.rs:467-497` (`ToolDecision` docs:
  `Allow` "is an auto-approval, not a pass-through"; `Ask` "the closest
  documented equivalent of deferring to normal policy"), :512-528
  (`PreToolUseOutput` "# No pass-through"), and :556-560 (`allow()`) are
  stale.
- `crates/hookkit-common/src/aligned.rs:346-375`: `pass_through` emits
  `{"decision":"ask"}` "because allow would auto-approve".
  `aligned.rs:377-405`: `allow` emits `{"decision":"allow"}`, which is now
  a pass-through, not an explicit approval. `explicit_allow_supported`
  (:407-418) explains Antigravity on the old basis.
- The mappings are mis-aligned under current docs, not inverted:
  `pass_through` over-prompts and `allow` under-approves.

Recommendation: keep `pass_through` at `{"decision":"ask"}` until a
release note or live observation dates the redefinition. Lowering it to
`allow` would auto-approve every call a guard does not object to on any
build that still has the old meaning, and that change is undated. Rewrite
the docs to say so. Map aligned `allow()` to `auto_approve`; on a build
that predates that value the explicit approval degrades (see below)
rather than escalating. The tests that encode the current mapping are
`crates/hookkit-common/src/aligned/tests.rs:99-118` (pass-through bytes,
and that no harness's pass-through contains "allow") and :120-147
(`allow` bytes, `explicit_allow_supported`).

#### New decision `auto_approve`

- `ag-hooks.md:312`: "`"auto_approve"`: Automatically approves the tool
  call when the default policy would otherwise prompt, while honoring
  explicit user-configured rules."
- CLI 1.5.7 (2026-11-12, `ag-cli-changelog.md:215` =
  `ag-changelog.md:2129`): "Fixed a `PreToolUse` hook returning
  `auto_approve`, or a wildcard `command(*)` allow rule, bypassing an
  organization's admin-enforced review ...". The value therefore existed
  before 1.5.7; its introduction is undated. App v2.33.0 (2026-11-17,
  `ag-changelog.md:198`) fixes "hooks or wildcard rules" bypassing
  enforced review without naming the value.
- CLI 1.0.16 (`ag-cli-changelog.md:1287`) shows that builds before 1.0.16
  failed an unrecognized decision with "unknown pre-tool hook decision".
  Emitting `auto_approve` to a build that predates it plausibly produces a
  hook error.
- `deny`, `ask`, `force_ask`, and `deny_unless_prior_grant` keep their
  definitions (`ag-hooks.md:313-316`).

Classification: **additive**. The selected enum
(`pre-tool-use/output.command.schema.json:8`) rejects it.

Proposed contract: add `auto_approve` to the enum and description, with
uncertainties: it is subordinate to explicit deny and ask rules, and from
CLI 1.5.7 and App v2.33.0 to admin-enforced review (earlier builds could
bypass it); IDE and IDE-extension support is undocumented; builds that
predate the value may treat it as an unknown decision.

Native impact: add `ToolDecision::AutoApprove` (`"auto_approve"`,
lib.rs:473-510) and `PreToolUseOutput::auto_approve()` (constructors at
:546-581). Extend lib.rs:1430-1446 and
`tests/snapshot_contract.rs:348-360`.

#### New output `overwrite`

- `ag-hooks.md:318`: "| `overwrite` | object | **Optional.** Replacement
  arguments merged (shallow top-level merge) into `toolCall.args` before
  execution. When arguments are modified, a notice naming the changed keys
  is prepended to the tool result. |"
- `build-plugins.md:495`: "... or `"overwrite": {...}` to rewrite
  arguments."
- No App, CLI, or IDE release note dates it. SDK v0.1.13
  (`ag-changelog.md:6665`) describes argument rewriting for in-process SDK
  hooks only.

Classification: **additive**. The selected output schema has
`additionalProperties: false` (`output.command.schema.json:19`), so any
response using it fails validation.

Proposed contract: `overwrite: {type: object}` with open values and no
`minProperties`. Uncertainties: a shallow merge cannot delete a key, and
the effect of a `null` value is undocumented; nested objects are replaced
whole; whether it applies with `deny`, `ask`, or `force_ask`, and whether
permission evaluation sees pre- or post-merge arguments, is undocumented;
whether `PostToolUse` `toolCall.args` echoes the merged arguments is
undocumented; the change notice is model-visible; surface and version
support is undated. Keep the outcome effect `control`.

Native impact: `PreToolUseOutput` (lib.rs:529-544) needs
`overwrite: Option<serde_json::Map<String, Value>>` (skipped when `None`,
declared between `reason` and `permission_overrides` to match the
reference's field order) and `with_overwrite()`. The struct is not
`#[non_exhaustive]`, so this is a breaking native API change. In the
aligned layer, `PreToolUseOutput::rewrite` (aligned.rs:444-485) receives
only the new input, so it cannot detect a replacement that drops a key;
keep its Antigravity arm as `UnsupportedHarness` and add a method that
also receives the original arguments (for example
`rewrite_for(input: &PreToolUseInput, updated, approval)`). Its
Antigravity arm emits only the changed keys, maps
`RewriteApproval::AutoApprove` to `auto_approve` and `Ask(reason)` to
`ask` with `reason`, and fails when `updated` drops an original key.

#### `permissionOverrides` vocabulary (correction)

`permissions.md` (35698 bytes, previously 26356) uses a broader
`action(target)` vocabulary than the selected uncertainty
(`pre-tool-use/contract.yaml:48`: read_file, write_file, read_url,
execute_url, command, mcp):

- `download_url(domain|*)` appears only in the App tables (`:79` updated
  settings, `:212` previous settings), configurable from App v2.30.0
  (2026-11-09) under Browser Actuation Rules (`:89`).
- `unsandboxed(prefix|regex:|*)` appears in the App "previous permission
  settings" table (`:214`, `:225`) and the CLI table (`:351`). The CLI
  section (`:361`) says "Business and enterprise accounts only. Other
  accounts ignore `unsandboxed` rules and show a warning". CLI 1.2.2
  (`ag-cli-changelog.md:798`) only "Improved the startup warning for
  deprecated `unsandboxed` permission rules", so the deprecation predates
  1.2.2. The pinned CLI changelog already named it, so omitting it was a
  snapshot error.
- The App sections spell out `read_file(dir)`, `mcp(server/tool|
  server/*)`, and per-token anchored `regex:` matching `^(?:pattern)$`
  (`:90`). The hooks reference example at `ag-hooks.md:319` is
  unchanged.

Classification: **correction** (schema unchanged; open strings). Rewrite
the uncertainty to list all eight actions with these caveats and to say
the docs do not state which actions `permissionOverrides` honors, on which
surfaces, or how overrides combine with the decision. Native docs at
lib.rs:536-539 may list the full vocabulary.

#### Tool catalog and matcher (documentation only)

- The reference now groups the same 20 tools under "Supported tools"
  (`ag-hooks.md:142-209`). `view_file` lists `IsSkillFile` (`:151`) but
  still not `StartPage`, `EndPage`, or `MediaResolution`; `find_by_name`,
  `grep_search`, and `list_dir` are still listed although CLI 1.2.7
  retired them from the default toolset; argument casing is mixed
  (`search_web` `query`, `:170`; `define_subagent`, `:196`;
  `ask_question`, `:207`).
- Tools named only elsewhere: `run_workflow` (CLI 1.5.9,
  `ag-cli-changelog.md:157`), `update_steps` (CLI 1.5.16, `:10`),
  `manage_inbox` (CLI 1.1.13, `:1049`), `read_resource` (CLI 1.1.16,
  `:998`), and `call_mcp_tool`.
- `mcp.md:195-205`: MCP tools are lazy-loaded by default and invoked
  "through the built-in `call_mcp_tool` tool (passing `ServerName`,
  `ToolName`, and `Arguments`)"; only `"eager": true` tools are called as
  `mcp_<serverName>_<toolName>`. The `"mcp_.*"` matcher example at
  `build-plugins.md:495` would therefore probably miss default MCP calls.
  `edit_file` (`build-plugins.md:469`) is not a reference tool.
- Matcher (`ag-hooks.md:131-136`): a regular expression over the tool
  name; `""` or `"*"` matches all; `"run_command"` matches "exactly";
  alternation and prefixes work. An anchored full match and a special
  case for `*` are inferred, not stated. CLI 1.4.3
  (`ag-cli-changelog.md:465`) warns about matchers "that aren't valid
  regular expressions and so never match"; App v2.6.0
  (`ag-changelog.md:1440`) instead rejects never-runnable configurations
  at load. CLI 1.5.5 maps Claude Code `Bash` to `run_command` on import.

Classification: **documentation-only**. `toolCall.name` and `args` stay
open (lib.rs:345-367). Extend the open-catalog uncertainty with
`run_workflow`, `update_steps`, `call_mcp_tool`, eager `mcp_*` names, and
`IsSkillFile`, and add the matcher semantics to the `sources.yaml`
limitations. Out of slice: `crates/hookkit-tool-access/src/builtin.rs`
(`ANTIGRAVITY_FILE_FREE` at :70-81, `analyze_antigravity` at :146-209)
treats the new names as unrecognized, which is conservative; `run_workflow`
fans out to subagents whose own tool calls may touch files.

### `PostToolUse`

- CLI 1.4.8 (2026-10-27, `ag-cli-changelog.md:388` =
  `ag-changelog.md:2468`): failed tool calls, "including calls to a tool
  that doesn't exist and canceled or interrupted calls", are no longer
  "passed back to the agent as ordinary results". The note concerns what
  the model sees and never mentions hooks.
- The `error` description is unchanged (`ag-hooks.md:366`), and
  `build-plugins.md:496` says the hook "Receives the tool call and any
  error (not the tool output)."

Classification: **behavioral, uncertainty only**. Extend the `error`
uncertainty as an open question: whether `PostToolUse` fires for an
unknown tool name, canceled, or interrupted call, and what `error` then
contains, is undocumented. Do not claim `error` is non-empty for them.
No schema change. Native: none (`error_message()` and `failed()`,
lib.rs:690-700, treat any non-blank `error` as failure). The CLI 1.4.3
item about failing `PostToolUse` hooks belongs under "Process outcomes".

### Inject steps (`PreInvocation`, `PostInvocation`, `SessionStart`)

- `ag-hooks.md:420-425` list exactly four kinds:
  - "**`systemMessage`** (object): An out-of-band system instruction for
    the agent. The value is an object with its own `systemMessage` string
    and optional `metadata` (for example, `{"systemMessage":
    {"systemMessage": "Remember to lint", "metadata": {"source":
    "policy"}}}`)."
  - "**`userMessage`** (string): A message from you."
  - "**`ephemeralMessage`** (string): **Deprecated.** Use `systemMessage`
    instead."
  - "**`errorMessage`** (object): Displays an error message without
    sending it to the model (for example, `{"userMessage": "Prompt
    blocked by policy."}`). Marks the turn as errored with a non-zero exit
    code in non-interactive `-p` mode, and does not stop execution on its
    own unless paired with `"terminationBehavior": "terminate"`."
- `toolCall` is no longer listed.
- `ag-hooks.md:429`: injected `systemMessage` and `userMessage` steps
  "remain in the conversation history for later turns".
- The official reminder output is now
  `{"injectSteps":[{"systemMessage":{"systemMessage":"Remember to
  lint"}}]}` (`ag-hooks.md:448-460`).
- `PostInvocation` (`:489`) and `SessionStart` (`:260`) use "same schema
  as `PreInvocation` inject steps".
- `errorMessage` is dated by CLI 1.4.11 (2026-10-30,
  `ag-cli-changelog.md:348`). No release note dates `systemMessage`, the
  deprecation, or the removal of `toolCall`.

The selected step schema (`pre-invocation/output.command.schema.json:11-21`,
identical in `post-invocation`) is a `oneOf` over `toolCall`,
`userMessage`, and `ephemeralMessage`. The selected `official`
`inject-reminder` output (`pre-invocation/fixtures.yaml:37-41`) and its
process case (`:56-63`) use `ephemeralMessage`; the synthesized
`post-invocation` `terminate-after-reminder` (`:42-46`) does too.

Classification: **breaking** for the catalog (a documented output member
is withdrawn and an official example changed) and **additive** for the two
new kinds.

Proposed contract: one step definition, copied under `$defs` into each of
the three output schemas (xtask resolves only fragment `$ref`s;
`xtask/src/main.rs:3950-3974`). Keep the current pattern of member types
in `properties` and an item-level `oneOf` of `required`:

- `systemMessage: {type: object, required: [systemMessage], properties:
  {systemMessage: {type: string}, metadata: {type: object}}}`
- `userMessage: {type: string}`
- `ephemeralMessage: {type: string, deprecated: true}`
- `errorMessage: {type: object, required: [userMessage], properties:
  {userMessage: {type: string}}}`
- `additionalProperties: false` on the step; no `toolCall`.

Uncertainties: the runtime may still accept the withdrawn `toolCall`
step; `metadata` value types are undocumented; `errorMessage` alone does
not stop execution and makes a `-p` run exit nonzero (the agent's exit,
not the hook's); whether `errorMessage` has an effect in `PostInvocation`
and `SessionStart` is unconfirmed; injected messages persist.

Fixtures (pointers and keywords checked with Draft 2020-12 against this
definition):

- `inject-reminder` output and process case become the new official bytes
  (stdout base64
  `eyJpbmplY3RTdGVwcyI6W3sic3lzdGVtTWVzc2FnZSI6eyJzeXN0ZW1NZXNzYWdlIjoiUmVtZW1iZXIgdG8gbGludCJ9fV19`,
  sha256 `a3a9179b992016d196a84a6c186a91f9064cbc128907d09370e23818eb7de213`).
- Synthesized positives: `inject-system-message-with-metadata`,
  `inject-deprecated-ephemeral`.
- Output negatives: `system-message-not-object` (pointer
  `/injectSteps/0/systemMessage`, keyword `type`);
  `error-message-missing-user-message` (`/injectSteps/0/errorMessage`,
  `required`); `two-step-kinds` (`/injectSteps/0`, `oneOf`);
  `inject-tool-call-undocumented` (`/injectSteps/0`, `oneOf`; it also
  fails `additionalProperties` at the same pointer). The existing
  `inject-step-mutually-exclusive` negative still fails with `oneOf`.

Native impact: `InjectStep` (lib.rs:118-166, `#[non_exhaustive]`) needs
`SystemMessage { system_message: { system_message, metadata:
Option<Map> } }` and `ErrorMessage { error_message: { user_message } }`
plus constructors; mark `ephemeral_message()` and `tool_call()`
`#[deprecated]`. Code that pins the old vocabulary or bytes:

- conformance case `crates/hookkit-conformance/src/lib.rs:973-977`;
- `crates/hookkit-antigravity/src/lib.rs:1331-1340`
  (`inject_output_matches_exact_catalog_bytes`);
- `crates/hookkit-antigravity/tests/snapshot_contract.rs:196-198` and
  :306-316 (`tool_call` at :310 fails against the new schema);
- `crates/hookkit-antigravity/tests/properties.rs:22`;
- `crates/hookkit-runtime/tests/integration.rs:1247` and
  `examples/antigravity-pre-invocation/src/main.rs:6-8`
  (`InjectStep::EphemeralMessage`).

### `PreInvocation` `terminationBehavior`

- `ag-hooks.md:416-418`: "`"terminate"`: Stops execution immediately
  before calling the model (pair with an injected `errorMessage` to
  display a block message). `""` (or omitted): Proceeds to the model call
  (default)." Official example `ag-hooks.md:462-475`.
- CLI 1.4.11 (2026-10-30, `ag-cli-changelog.md:348`): "Added
  `terminationBehavior` to `PreInvocation` hooks: returning
  `"terminate"` stops the turn before the model is called ...". No App or
  IDE counterpart. CLI 1.5.8 (`:188`) calls a `PreInvocation` hook one
  "that should block the agent".
- `force_continue` is documented only for `PostInvocation`
  (`:490-493`, unchanged).

Classification: **additive** (output) and **behavioral** (the event is now
a gate). The selected schema (`additionalProperties: false` at `:25`)
rejects the official block example.

Proposed contract: `terminationBehavior: {enum: ["", "terminate"]}`.
Rename the single outcome to `control` with effect `control`, mirroring
`PostInvocation`. Uncertainties: surfaces other than CLI 1.4.11 and later
are undated; `terminate` without `errorMessage` displays nothing
documented. Fixtures: official `block-prompt` output and process case
(stdout base64
`eyJpbmplY3RTdGVwcyI6W3siZXJyb3JNZXNzYWdlIjp7InVzZXJNZXNzYWdlIjoiUHJvbXB0IGJsb2NrZWQgYnkgcG9saWN5LiJ9fV0sInRlcm1pbmF0aW9uQmVoYXZpb3IiOiJ0ZXJtaW5hdGUifQ==`,
sha256 `f0bc3f404a4c0dadc86627a201f063d089423ddc4975268f17578cca1c464a89`);
synthesized `explicit-default` (`{"terminationBehavior":""}`); negatives
`force-continue-undocumented` and `unknown-termination` (pointer
`/terminationBehavior`, keyword `enum`).

Native impact: `PreInvocationOutput` (lib.rs:168-203, not
`#[non_exhaustive]`) needs `termination_behavior:
Option<PreInvocationTermination>` with a separate enum `{Default,
Terminate}`; do not reuse `TerminationBehavior` (lib.rs:239-262), which
admits `ForceContinue`. Add `PreInvocationOutput::block(message)` that
emits the `errorMessage` step plus `terminate`. The no-op must stay `{}`
(lib.rs:1343).

### `PostInvocation`

Only the shared step vocabulary changed. Input ("Same as `PreInvocation`",
`ag-hooks.md:483`), `terminationBehavior` (`:490-493`), the official
`default` example (`:500-505`), and the firing description ("Fires
immediately after each model invocation completes", `:481`) are
unchanged. `build-plugins.md:493-494` describes the invocation events per
turn ("Before the model processes a user turn", "After the model finishes
its response turn"), and `ag-hooks.md:429` itself says "each turn"; keep
the per-invocation model and add an uncertainty citing both.

### `Stop`

#### `continuation_active` (release notes only)

- CLI 1.5.6 (2026-11-11, `ag-cli-changelog.md:223` =
  `ag-changelog.md:2147`; summary `:2142`): "Added a
  `continuation_active` field to the JSON input of Stop hooks, which is
  `true` while the agent is already working on a continuation an earlier
  Stop hook requested ...".
- The reference's `Stop` table (`ag-hooks.md:513-521`) omits it, and
  `ag-hooks.md:223` says "Field names use camelCase." All three
  release-note mentions spell it in snake_case, so that spelling has the
  direct evidence, but the wire key is unobserved.

Classification: **additive** (release-note sourced, low assurance).
Proposed contract: optional `continuation_active: {type: boolean}` in
`stop/input.schema.json`, with an uncertainty that the wire key may be
`continuationActive`, that other surfaces are unknown, and that absence
means unknown, not false. A camelCase key stays accepted through
`additionalProperties: true`.

Native impact: `StopInput` (lib.rs:843-877) keeps it in `extra`. Add a
typed `continuation_active: Option<bool>` and an accessor that falls back
to `extra["continuationActive"]` (a serde alias would break lossless
round trips). `aligned.rs:770-783` `stop_hook_active()` returns `None` for
Antigravity ("Antigravity sends no loop guard") and should return the
accessor's value. `crates/hookkit-tool-runner/src/stop_guard.rs:15` and
:58 ("reports no continuation signal") should use it when present and
keep `Unavailable` otherwise. Pre-existing error in the same file:
`stop_guard.rs:20-21` says Antigravity has no continuation cap, but the
snapshot records the CLI 1.1.9 and App v2.6.0 caps.

#### `{}` described as allowing the stop

`build-plugins.md:497`: "Return `{"decision": "continue", "reason":
"..."}` to prevent completion ..., or `{}` to allow the stop."
`ag-hooks.md:527`: "`decision` | string | **Required.** ... Any other
value allows the stop." The runtime effect of `{}` is plausibly the same,
but the stated requirement differs. Classification:
**documentation-only**. Keep `decision` required as a HookKit emission
rule; add an uncertainty citing both pages and an `output_negative`
`empty-object` (`{}`, pointer `""`, keyword `required`, origin
`regression`), following the `PreToolUse` `empty-decision` precedent.
Native: none; `StopOutput` always serializes a decision (lib.rs:964-985)
and `Stop::emit` rejects an empty one.

### Process outcomes (all events)

- The hooks reference still defines no exit codes and no handling of
  stderr or invalid stdout (`ag-hooks.md:223`). Its handler-configuration
  table documents an optional `timeout` field in seconds that defaults to
  `30` (`:219`). That is a configuration default, not a process outcome,
  and `docs-2026-09-30-r1` recorded no hook configuration details, so
  whether the default is new since then is unknown.
- `build-plugins.md:461`: "Command hooks run with their working directory
  (`cwd`) set to `<plugin-root>`, receive event context as JSON on
  `stdin`, and must exit with code **`0`** and write a JSON response
  object to `stdout` ... Any non-zero exit code is logged as a hook
  execution error and ignored (failing open):"
- CLI 1.4.3 (2026-10-20, `ag-cli-changelog.md:471`): a `PostToolUse`
  hook that "fails to run, for example because its command is missing,
  exits with an error, times out, or prints invalid JSON" no longer marks
  the tool call failed; "the failure is now logged and the tool keeps its
  result."
- CLI 1.5.8 (2026-11-13, `:188`): Windows hook commands containing double
  quotes were "failing and being silently skipped, which let a
  `PreInvocation` or `PreToolUse` hook that should block the agent go
  unenforced". The fix changed quoting, not the skip policy.
- CLI 1.5.14 (2026-11-21, `:68`): on Linux and macOS, "stopping a hook
  now terminates its entire process group" after a timeout or cancel.
- App v2.12.0 (`ag-changelog.md:1191`, already encoded): a failing custom
  hook is reported and the conversation continues.

The selected contracts model only exit-0 outcomes. Their shared
uncertainty ("the CLI and IDE notes have no counterpart") is contradicted
by CLI 1.4.3 and 1.5.8, and the `PreToolUse`-only "Do not presume
fail-open" (`pre-tool-use/contract.yaml:41`) is contradicted by the
official fail-open statement. The CLI 1.1.7 note behind it is about a
disabled plugin's hooks still running.

Classification: **behavioral** (newly documented failure semantics).

Proposed contract: add to every event an outcome `failure` with exit
`{range: {min: 1, max: 255}}`, effect `nonblocking-error`, stdout and
stderr `{presence: optional, role: ignored, content_kind: opaque}`,
sources `antigravity-build-plugins`, `antigravity-cli-changelog`, and
`antigravity-changelog`, assurance low (the Codex `Stop` `failure` outcome
is the precedent). Rewrite the failure uncertainties: fail-open is
documented for plugin command hooks and inferred for workspace and global
hooks and for the App and IDE; exit 2 has no special meaning; invalid
stdout at exit 0 is documented as ignored only for `PostToolUse` (CLI
1.4.3); the 30-second default and the process-group kill (CLI 1.5.14,
Linux and macOS); the `-p` nonzero exit from `errorMessage` belongs to the
agent. Add a process case `failure` per event (exit 1, empty stdout,
stderr `hook failed\n`, base64 `aG9vayBmYWlsZWQK`, sha256
`339e9a9a2a85233d852616e657d6b262da208ce3ef1e7c8c5fa32659916f0257`).

Native and runtime impact. Emission is unaffected; every `emit` exits 0.
Documentation that is now inaccurate:

- `crates/hookkit-antigravity/src/lib.rs:15-19` ("Do not assume that a
  failing `PreToolUse` hook lets the pending tool run").
- `crates/hookkit-runtime/src/failure.rs:94-95` ("Antigravity documents
  no exit-code semantics"). HookKit's fail-closed lowering, a JSON
  `deny` at exit 0 (failure.rs:307-309, :405), is consistent with the
  documented fail-open and is now better justified.
- `crates/hookkit-tool-runner/src/post_tool.rs:1337` and
  `crates/hookkit-runtime/tests/integration.rs:3965` should say that
  Antigravity logs and ignores the nonzero exit, so the diagnostic never
  reaches the agent and the tool result is unaffected (CLI 1.4.3 and
  later). The runtime test at `integration.rs:3964-3985` already asserts
  the nonzero exit, empty stdout, and `hookkit: ` stderr.

A new consequence: `PreInvocation` is now a gate, but
`fail_closed_lowering` (failure.rs:306-309) treats every Antigravity event
except `PreToolUse` and unknown as `NotGating`, so a fail-closed
prompt-screening guard fails open. Add an
`AntigravityBlockPrompt` lowering for `PreInvocation` that emits the
`errorMessage` step plus `terminate` at exit 0. With `SessionStart` in the
inventory, a hint-less common-fields-only payload becomes
`AmbiguousEvent([SessionStart])` rather than `UnrecognizedEvent`; the
unknown-event `{"decision":"deny"}` is invalid output for `SessionStart`
and the invocation events, whose handling of invalid output is
undocumented.

### Command environment

- CLI 1.5.3 (2026-11-07, `ag-cli-changelog.md:271` =
  `ag-changelog.md:2244`): "Added the `PLUGIN_ROOT` and `PLUGIN_DATA`
  environment variables to hook commands that come from a plugin,
  pointing to the plugin's folder and its persistent data folder".
- CLI 1.5.6 (`ag-cli-changelog.md:226`): `agy plugin validate` warns when
  a plugin hook uses `${PLUGIN_ROOT}`, "which older CLI versions do not
  set", and suggests relative paths "since hooks already run in the
  plugin's folder".
- `build-plugins.md:248-249` and `:257` document both variables only for
  plugin MCP server processes. `build-plugins.md:255`: hook commands
  "automatically execute with `<plugin-root>` as their working directory
  (`cwd`) and should use relative paths ... rather than
  `${PLUGIN_ROOT}`"; `:461` repeats the working directory.
- The hooks reference has no environment-variable text; plugin-level
  hooks are listed for the App and CLI (`ag-hooks.md:15`, `:29`) but not
  the IDE (`:43-46`), although `plugins.md:246-254` documents plugin
  loading in the standalone IDE.

The selected supplement (`supplement.yaml:8-22`) gives Antigravity no
profiles and empty `event_profiles`, and its source limitation
(`sources.yaml:158`) and `docs/command-environments.md:153-160` say no
Antigravity variable reaches hook processes. Classification:
**additive**.

Proposed supplement: an Antigravity profile `plugin-paths` modelled on the
Codex one (`supplement.yaml:600-611`): activation conditional ("command
hook was loaded from an Antigravity plugin; Antigravity CLI 1.5.3 or
later"), completeness `all-or-none`, `PLUGIN_ROOT` and `PLUGIN_DATA` of
value kind `path`, low confidence, sources `antigravity-cli-changelog`
and `antigravity-build-plugins`. Map it to all six events. Limitations:
the hooks reference documents no variables; `build-plugins.md`
documents them only for MCP servers and steers hooks to relative paths;
App and IDE behavior is undocumented and CLI before 1.5.3 does not set
them; plugin hooks run in the plugin root (documented), while the working
directory of workspace and global hooks is undocumented.

Native impact: `AntigravityCommandEnvironment`
(`crates/hookkit-antigravity/src/environment.rs:12-45`) is a `Copy` unit
struct with `VARIABLE_NAMES = &[]`; its module doc (:1-6) and test
(:52-68) assert an empty environment for five events. Mirror
`CodexCommandEnvironment` (`crates/hookkit-codex/src/environment.rs`):
`plugin: Option<AntigravityPluginEnvironment { root, data }>`, set only
when both variables are present and non-empty, with
`LENIENT_VARIABLE_NAMES` for non-UTF-8 values. This is a breaking API
change (the type loses `Copy`). Unlike Codex, Antigravity sets no
`CLAUDE_*` aliases, so ambient `PLUGIN_*` values inherited from an outer
Codex or Claude Code plugin hook are indistinguishable; document that.
Update `crates/hookkit-conformance/tests/environment_parity.rs:43-44` and
`docs/command-environments.md:153-160`.

### Handler kinds

`ag-hooks.md:217`: "| `type` | string | Optional. Currently only
`"command"` is supported. Defaults to `"command"`. |" The example at
`:75-79` omits `type`. `plugins.md:23` and `build-plugins.md:240`
describe lifecycle hooks as "Synchronous shell commands or prompt
evaluations"; `build-plugins.md:461` speaks of "Command hooks". No
release note after the snapshot mentions prompt hooks; the only ones are
CLI 1.1.23 and App v2.6.0. Classification: **documentation-only**. Keep
`handler_kinds: [command]` for all six events and extend the shared
uncertainty to cite the plugin pages.

### Configuration and loading (documentation only)

None of these changes stdin or stdout, and HookKit generates no
Antigravity configuration:

- `hooks.json` maps hook names to definitions (`ag-hooks.md:52`); a
  definition may set `enabled: false` (`:108`); handler `command` is
  required and `timeout` defaults to 30 seconds (`:218-219`).
- `ag-hooks.md:28`: "Hooks in `~/.gemini/antigravity-cli/settings.json`
  aren’t loaded." The 2026-09-29 audit (lines 116-117) said "The CLI also
  reads `~/.gemini/antigravity-cli/settings.json`". No release note
  records a removal and the earlier body was not captured, so this is a
  correction of the record, not a dated behavior change.
- Plugin `hooks/hooks.json` is not read (CLI 1.4.3, `ag-changelog.md:2607`;
  `build-plugins.md:542`); a Claude Code-style top-level `"hooks"` key
  loads as a hook named `hooks`, except, since CLI 1.5.14
  (`ag-changelog.md:1864`), in plugins whose `plugin.json` declares an
  Agent Plugins `$schema`.
- `agy plugin import` gives converted hooks a 60-second default timeout
  (CLI 1.5.5), against 30 seconds in `hooks.json`.
  `crates/hookkit-pkl-config/src/builtins/Config.pkl:65` ("Antigravity
  defaults to 30 seconds") holds for native hooks.
- CLI 1.3.3 (2026-10-10, `ag-cli-changelog.md:563`): `/btw` side
  questions no longer run user and workspace hooks.
- Other loading fixes: ADC hook loading (CLI 1.4.7, `ag-changelog.md:2497`);
  UTF-8 BOM `hooks.json` (CLI 1.5.8 `:2080`, CLI 1.5.9 `:2043`, App v2.33.0
  `:185`); `pluginControls` (CLI 1.5.9 `:2033`, CLI 1.5.16 `:1790`); Auto
  mode still asks before editing `hooks.json` (CLI 1.5.16 `:1789`);
  custom-agent relative hook paths (App v2.23.0 `:786`, CLI 1.3.1
  `:2865`); App v2.17.0 custom-agent `hooks:` front matter (`:929`), still
  absent from the reference; Remote Control honoring a hook that requires
  confirmation each time (CLI 1.5.10, `ag-cli-changelog.md:152`).

Record the timeout defaults, the process-group kill, the plugin-hook
working directory, the IDE plugin-hook ambiguity, and the `/btw` scope in
the successor's `sources.yaml` limitations or uncertainties.

## Identification

Every other event's payload validates against a common-fields-only
`SessionStart` schema with `additionalProperties: true` (checked: the
official `Stop` example validates). The successor therefore records:

| Event | Inferability | Overlaps |
| --- | --- | --- |
| `SessionStart` | `ambiguous` | `PreToolUse`, `PostToolUse`, `PreInvocation`, `PostInvocation`, `Stop` |
| `PreToolUse` | `ambiguous` | `PostToolUse`, `SessionStart` |
| `PostToolUse` | `ambiguous` | `PreToolUse`, `SessionStart` |
| `PreInvocation` | `ambiguous` | `PostInvocation`, `SessionStart` |
| `PostInvocation` | `ambiguous` | `PreInvocation`, `SessionStart` |
| `Stop` | `shape-based` | `SessionStart` |

One verified finding proposed making `Stop` `ambiguous`. This note keeps
it `shape-based`: no documented `SessionStart` payload carries
`executionNum`, `terminationReason`, and `fullyIdle`, so a payload that
satisfies `Stop` is still a `Stop`. `resolve_event_with`
(`crates/hookkit-runtime/src/resolution.rs:249-257`) resolves a payload
when exactly one sound-shape descriptor validates, so hint-less `Stop`
resolution, which `crates/hookkit-runtime/tests/stdin_runners.rs:391-412`
relies on, keeps working. `detect_candidates` will report a `Stop`
payload as `ambiguous` (resolution.rs:380-387), which is accurate.
`crates/hookkit-conformance/tests/identification_parity.rs:79-88`
requires each descriptor's overlaps to equal the contract's exactly, so
`identification_descriptors()` must mirror this table, including
`sound_shape::<Stop>(&["SessionStart"])`.

## Corrections to the selected snapshot and earlier audits

- **The IDE was re-released.** Uncertainties at
  `pre-tool-use/contract.yaml:47`, `post-tool-use/contract.yaml:41`, and
  every `modelName` uncertainty ("possibly the IDE v2.5.5 build"), plus
  `sources.yaml:45` and `:94`, assume the IDE has not shipped since v2.5.5.
  IDE v2.6.0 (2026-10-26, `ag-changelog.md:7063-7074`) "Updated the agent
  interface and architecture powered by the built-in Antigravity
  extension, delivering ... seamless feature parity"; v2.7.0
  (2026-11-12) and v2.8.0 followed. No IDE note mentions hooks, so
  current IDE payloads are inferred, not observed. Keep `PostToolUse`
  `toolCall` and `modelName` optional, keep the `ide-reference-example`
  fixture (`post-tool-use/fixtures.yaml:16-19`) as a regression for IDE
  v2.5.5 and earlier, and update lib.rs:494-495 and :662-667.
- **`unsandboxed`** was missing from the `permissionOverrides` vocabulary
  although the pinned CLI changelog named it (see `PreToolUse`).
- **"Do not presume fail-open"** is contradicted by the official
  plugin-authoring page (see "Process outcomes").
- **CLI `settings.json` hooks**: the 2026-09-29 audit's location list is
  superseded by `ag-hooks.md:28`.
- **Release-note limitation**: `sources.yaml:46` says release notes
  "define neither hook exit codes nor payload schemas". They still give
  no types, but they now name input fields (`lastUserInput`,
  `hookEventName`, `cwd`, `toolCall.id`, `continuation_active`), plugin
  hook variables, and failure handling, some ahead of the reference.

## Evidence and classification

SHA-256 values are of decoded bodies fetched on 2026-11-25 with
`curl --fail --silent --show-error --location --compressed`; the audit
re-hashed the staged copies. Every source cited by `docs-2026-09-30-r1`
changed, except the Internet Archive captures, which cannot be refetched
from this environment and stay as historical evidence.

| Source | Bytes | SHA-256 (2026-11-25) | Recorded by `docs-2026-09-30-r1` |
| --- | --- | --- | --- |
| `/docs/hooks` (HTML) | 244063 | `95015b077211200defd915604f75a6cb7dc1cbadda482c6cd6a5c37569ec8680` | 141741, `051967919edfb9904511c9a47e65115cb3def61c34f6f7e14924710df215f180` |
| `/docs/hooks.md` | 21303 | `c500f806f5ccb285b0866d3fbbbfb4e5aa5c50b45f788446ac6ca76bd5b44f7a` | 16795, `c1ce644786c46a666379af40e3983081887fa29245ead461be4235a92ffc80c3` |
| `/docs/ide/hooks` (redirect to `/docs/hooks?tab=ide`) | 3956 | `9f497b7bebc851f1e93c3f4aebfe312e7a8b19d6f2ca5f30ffe52b9c7ef05c39` | 371, `9570816c39c7cef1314bcbac2beca0e9c604634bc3dffafb6cc953f3ea714288` |
| `/docs/changelog.md` | 670457 | `f0bfcac6085100aebee39c3a1dbb8dca0bdb2f5d1642dfbd5b6db662f81ec99a` | 268410, `404ef735cb652cefd51649855e83809b0f17d50d91926d5b0bf3d27acb681b8a` |
| CLI `CHANGELOG.md` raw at `bb5c8da4` | 268726 | `cbbe47863d9e029d0ba8d4314c772be4fc5581a3214191a18e79d427a9725815` | at `eaf9e066`: `09edf2e8af87f1788348e4ec62e9d2ab26f9f90cfc48fca54dc0e14fe9c31d2b` |
| `/docs/permissions.md` | 35698 | `ca9048c14ca975a7ab19133d267bdd0999e11e30b213d0ba4f07dbcd98997742` | 26356, `f2dc8d81a16b037fb1493eb8976fa0b55fa9453cb156bbb872f7850ce9aad4bd` |
| `/docs/build-plugins.md` (new) | 35989 | `587b736b48409a193d27ccb8b2ef177f56c36e5dd62c6bbede0dc86b976d32cb` | not cited |
| `/docs/plugins.md` (new) | 14915 | `a6ce5caa3bdaae7627b10221f7b464a0f81f3e1cdd0ecc283878afe715298d96` | not cited |
| `/docs/mcp.md` (new) | 23408 | `78027dc5fe70bb42a2ecd698ff8003703ad842f31ef81d1b70238e384f3da168` | not cited |

The CLI repository's `main` is `bb5c8da4` (CLI 1.5.16). Its changelog diff
from `eaf9e066` is the single insertion hunk `6a7,651` (1.2.15 through
1.5.16), so no earlier release note was edited. The changelog's IDE
Extensions section has entries from 2026-08-18, before the snapshot;
whether the 2026-09-30 body included it is unknown. The selected
supplement shares the hooks HTML and Markdown URLs
(`supplement sources.yaml:138-158`), so the successor supplement must
record the same new hashes and, per `contracts/MAINTENANCE.md:23-29`, cite
the earlier full hashes in its limitations.

The successor is classified as **breaking** for the catalog: the inject
`toolCall` step is withdrawn, an official example's bytes changed, and
`allow` changed meaning. The rest is **additive** (one event, one input
field, three release-note input fields, two output fields, one decision,
two step kinds, one termination value, one environment profile) plus
**behavioral** failure outcomes. Every input payload that validated
against `docs-2026-09-30-r1` still validates. Every output that validated
still validates, except one with a `toolCall` inject step. The native Rust
API change is **breaking**: new fields on output structs that are not
`#[non_exhaustive]` (`PreToolUseOutput`, `PreInvocationOutput`), and
`AntigravityCommandEnvironment` gains fields and loses `Copy`.

## Items checked and unchanged

- The five existing event names (`ag-hooks.md:120-127`); no event removed.
- Transport: stdin JSON in, stdout JSON out, camelCase names (`:223`); one
  document per invocation.
- Common fields `conversationId`, `workspacePaths` (strings, no minimum),
  `transcriptPath`, `artifactDirectoryPath`, and `modelName`, with the
  same types and descriptions (`:229-241`); the
  `<app_data_dir>/brain/<conversationId>/.system_generated/logs/transcript.jsonl`
  layout and the three `<app_data_dir>` values (`:234-237`); official
  examples still use a literal `~/` prefix and `gemini-3.6-flash-medium`.
- `PreToolUse` input: `toolCall`, `toolCall.name`, `toolCall.args`
  (object), trajectory-wide `stepIdx` (`:300-303`); `list_permissions`
  still "Arguments: None" (`:185`), so the null or absent `args`
  tolerance stays. The official input (`:325-341`) equals the snapshot
  `representative` fixture.
- `PreToolUse` `deny`, `ask`, `force_ask`, and `deny_unless_prior_grant`
  definitions (`:313-316`); `decision` required; `reason` (`:317`);
  `permissionOverrides` type and example (`:319`). The official `ask`
  output (`:346-352`) is byte-identical to the snapshot fixture.
- `PostToolUse` input (`:364-366`), output exactly `{}` (`:369`, `:395`,
  consistent with `build-plugins.md:496`), and the official example
  (`:375-392`).
- `PreInvocation` input `invocationNum` and `initialNumSteps`
  (`:407-408`).
- `PostInvocation` input, `terminationBehavior` values, official `default`
  output (`:500-505`), and firing description (`:481`).
- `userMessage` step is still a string (`:423`).
- `Stop` input `executionNum`, open `terminationReason`, optional
  `error`, required `fullyIdle` (`:517-520`); output `decision` required,
  only `"continue"` meaningful, `reason` injected only with `continue`
  (`:527-528`); the official input and output match the snapshot
  fixtures. No release note documents the consecutive-continuation cap's
  size or key.
- `handler_kinds`: only `command` (`:217`).
- The permissions engine semantics hooks rely on (Deny > Ask > Allow,
  `action(target)` form, `ask` and `force_ask` versus "Always Allow";
  `permissions.md:11-21`).
- The `/docs/hooks` HTML has the same hook content as `hooks.md` (App,
  CLI, and IDE tabs only).
- No `AGY_*`, `GEMINI_*`, or `ANTIGRAVITY_*` variable is provided to hook
  processes; the CLI variables in new notes configure the CLI itself.
- App v2.19.1 to v2.35.1 contain exactly four hook items (v2.23.0
  `:786`, v2.28.0 `:561`, v2.33.0 `:185` and `:198`); CLI 1.2.15 to
  1.3.0 contain no payload or output items; IDE v2.6.0 to v2.8.0 and the
  VS Code, Visual Studio, and Eclipse extension notes contain no payload,
  output, or exit-code items; SDK v0.1.21 to v0.1.24 hook items concern
  in-process SDK hooks.

## Open questions for live observation

- Does `allow` still auto-approve on any current build, and when did the
  redefinition and `auto_approve` ship on each surface? What does an older
  build do with `auto_approve`?
- Which surfaces fire `SessionStart`, and on resume (`agy -r`), fork,
  compaction, subagents, or app-restart auto-resume? Is `lastUserInput`
  sent there?
- Is the `Stop` loop guard sent as `continuation_active` or
  `continuationActive`, and by which surfaces?
- What are the types of `hookEventName`, `cwd`, and `toolCall.id`; does
  `hookEventName` use the PascalCase event names (including for
  `SessionStart`); is `toolCall.id` identical between `PreToolUse` and
  `PostToolUse`; do the App and IDE send them?
- `overwrite`: does `null` delete a key; is it applied with `ask`,
  `force_ask`, or `deny`; are permissions and matchers evaluated on the
  merged arguments; does `PostToolUse` see the merged arguments?
- Is the withdrawn `toolCall` inject step still accepted? Does
  `errorMessage` accept other members, and does it act in `PostInvocation`
  and `SessionStart`? Are `metadata` values strings only?
- Does fail-open apply to workspace and global hooks and to the App and
  IDE? Is invalid stdout at exit 0 ignored for `PreToolUse`,
  `PreInvocation`, and `Stop`? What happens to a pending tool call when a
  `PreToolUse` hook times out?
- Do App and IDE plugin hooks receive `PLUGIN_ROOT` and `PLUGIN_DATA`?
  What is the working directory of workspace and global hooks?
- What `<app_data_dir>` do IDE extensions use, and is the IDE's
  `~/.gemini/antigravity-ide` (`ag-hooks.md:48`) or `~/.gemini/antigravity`
  (`mcp.md:197`)?
- Does a `Stop` hook returning `{}` allow the stop silently or count as a
  hook error?
- Is the plugin pages' "prompt evaluations" wording a planned handler type
  or residual copy?

## Claims narrowed or rejected during verification

No finding was refuted outright. These claims did not survive as stated:

- **Identification was one-sided.** Both `SessionStart` findings listed
  `SessionStart` as overlapping all five events but changed at most
  `Stop`. The parity test requires reciprocal overlaps on all five, and
  this note keeps `Stop` `shape-based` rather than `ambiguous`.
- **"`hookEventName` is an unknown field."** It is release-noted (CLI
  1.5.4).
- **Inject-step negative pointers.** Under the snapshot's pattern,
  `{systemMessage: "..."}` fails at `/injectSteps/0/systemMessage`
  (`type`) and `{errorMessage: {}}` at `/injectSteps/0/errorMessage`
  (`required`), not at `/injectSteps/0` with `oneOf`.
- **A "derived" fixture origin** for the `PostInvocation` representative
  is not a valid origin.
- **"`allow` and pass-through are inverted."** `pass_through` emits
  `ask`, not `allow`; the mappings are mis-aligned, not inverted. The
  conformance citation `hookkit-conformance/src/lib.rs:514-530` points at
  Claude Code and Codex cases; the affected tests are in
  `crates/hookkit-common/src/aligned/tests.rs`.
- **App v2.33.0 names `auto_approve`.** It does not.
- **"`unsandboxed` deprecated in the CLI since 1.2.2."** CLI 1.2.2 only
  improved an existing warning, and the enterprise-only caveat is
  CLI-section text; `download_url` is App-only.
- **"Keep only exit-0 outcomes and retain the warning not to rely on
  either direction."** `build-plugins.md:461` documents fail-open.
- **"Typed `toolCall.id` string with a type negative"** and **"reject a
  mismatched `hookEventName` at parse time"** go beyond the evidence.
- **"Since CLI 1.4.8 unknown-tool, canceled, and interrupted calls carry
  a non-empty `error`."** The note concerns what the model sees.
- **"MCP tools are matched as `mcp_.*`."** Default MCP calls go through
  `call_mcp_tool` (`mcp.md:199`); `mcp_<server>_<tool>` is only for eager
  tools. **"An invalid matcher silently never matches"** is CLI
  behavior; the App rejects never-runnable configurations at load.
- **"CLI `settings.json` hooks are no longer loaded"** asserts an
  undated behavior change; **"plugin-level hooks exist for the App and
  CLI only"** conflicts with `plugins.md:246-254`.
- **"Only IDE ≤ 2.5.5 may omit `toolCall` or `modelName`"** overstates
  "feature parity", which no IDE note ties to hooks.
- **"IDE extensions v1.0.0 introduced `hooks.json` hooks"** holds only
  for JetBrains, Zed, and Xcode; **"the changelog adds a new IDE
  Extensions section"** cannot be shown new.
- **"The working directory of plugin hooks comes from CLI 1.5.6."** That
  note restates documented behavior (`build-plugins.md:255`, `:461`).
- Several line citations were corrected: the 2026-09-29 settings quote
  is at lines 116-117; the snapshot fixtures are at
  `pre-invocation/fixtures.yaml:8-11`, `:37-41`, `:56-63`,
  `post-invocation/fixtures.yaml:42-46`, and
  `post-tool-use/fixtures.yaml:16-19`; `resolution.rs:831` is test code.

## Final status

`docs-2026-11-25-r1` is frozen, recorded in `FROZEN_LEDGER`, and selected
in `contracts/registry.yaml`, together with supplement
`command-environments-2026-11-25-r1`. The supplement adds the
`plugin-paths` profile (`PLUGIN_ROOT` and `PLUGIN_DATA`) and maps it to all
six events. `hookkit-antigravity` implements the snapshot, and all six
events, `SessionStart` included, are `command-runtime-beta` targets with
`source-reviewed` verification.

The native changes follow this note, with these decisions:

- The aligned `PreToolUseOutput::allow` emits `auto_approve`; the aligned
  `pass_through` keeps emitting `ask`, which never approves on any build.
- Under `FailurePolicy::FailClosed` a failing `PreInvocation` hook blocks
  the prompt at exit 0 (an `errorMessage` step plus `"terminate"`).
- The invocation-zero `InvocationStart` boundary is recorded only while no
  other epoch exists for the session, so it never replaces a `SessionStart`
  epoch.
- `NativeContext.tool_call_id` carries `toolCall.id` when it is a non-blank
  string.
- `InjectStep::ephemeral_message` and `InjectStep::tool_call`, with their
  variants, are `#[deprecated]`.
- `session-start-state-agent-hook` accepts `--antigravity`, and the Stop
  runner uses `continuation_active` as its loop signal when it is present.

Deferred:

- An aligned rewrite for `overwrite` (`rewrite_for`): what `overwrite` does
  with `ask` and `force_ask` is undocumented, so the aligned `rewrite`
  still fails for Antigravity.
- Antigravity is not part of the aligned `SessionStart` family, which stays
  Claude Code and Codex only.
- Nothing pairs a `PreToolUse` with its `PostToolUse` by `toolCall.id`.
- The hint-less fail-closed `{"decision":"deny"}` is still invalid output
  for `SessionStart` and the invocation events; `failure_response`
  documents this.
- Every question under "Open questions for live observation" is still
  open.
