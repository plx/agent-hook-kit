# Antigravity hook contract refresh

- Audit date: 2026-09-29
- Previous selected snapshot: `docs-2026-08-04-r1`
- Successor snapshot: `docs-2026-09-29-r1`
- Official hook reference: <https://antigravity.google/docs/hooks> (unified
  Antigravity 2.0, CLI, and IDE page) and its Markdown source
  <https://antigravity.google/docs/hooks.md>
- Former IDE reference: <https://antigravity.google/docs/ide/hooks>, now a
  redirect to `/docs/hooks?tab=ide`
- Release notes: <https://antigravity.google/docs/changelog.md> (2.0 through
  v2.18.1, CLI through v1.2.11, IDE through v2.5.5) and the CLI repository
  changelog at `eaf9e06660d2ca8f20f1474ed03f10e3dbfd35e5` (CLI 1.2.14)

This audit re-read the complete unified hook reference, compared its field
tables and examples with the selected August snapshot, and checked the Internet
Archive captures of the generic and IDE-specific pages from 2026-07-21 onward.
It also reviewed every Antigravity 2.0 release note, and every CLI release note
after 1.1.10, the last version covered by the previous audit.

## Result

The documented wire inventory is still five command events: `PreToolUse`,
`PostToolUse`, `PreInvocation`, `PostInvocation`, and `Stop`. The events have
the same output fields, decision values, inject-step kinds, and termination
behaviors. The successor records one new input field and loosens three input
constraints that the evidence does not support:

- **New common input field `modelName` (string).** The reference now lists
  `modelName` among the fields "all hooks receive", and every official example
  carries `"modelName": "gemini-3.6-flash-medium"`. It first appears between the
  2026-07-21 and 2026-08-16 archive captures. It is absent from the August
  snapshot's official examples and from every archived IDE-specific copy, and no
  release note dates it. The successor therefore makes it optional in all five
  input schemas and records that choice as an uncertainty.
- **`workspacePaths` may be empty.** No capture of the reference ever required a
  non-empty list. Release notes describe conversations launched without a
  workspace (CLI 1.2.2) and standalone conversations (2.0 v2.14.0), and global
  hooks load from `~/.gemini/config/hooks.json`. The successor drops the
  `minItems: 1` constraint from all five input schemas. The field stays required
  because no source shows it being omitted.
- **`PostToolUse.toolCall` is optional.** The unified reference documents it for
  all three surfaces. The IDE-specific reference, however, documented
  `PostToolUse` input and its example without `toolCall` in every archived copy
  through 2026-09-16, a month after the latest IDE release (v2.5.5,
  2026-08-13), which has not been re-released since the page became a
  redirect. The generic reference also omitted the field on 2026-07-21, and CLI
  builds before 1.1.9 fired `PostToolUse` on non-tool steps. Because surfaces
  and builds disagree, the successor adopts the tolerant reading and adds the
  IDE example as an `official` positive fixture. A malformed `toolCall` is still
  rejected. `PreToolUse.toolCall` has been documented on every surface since the
  first capture and stays required.
- **Empty `error` means success.** `PostToolUse.error` is documented as "Empty
  if successful", and the official `Stop` example sends `"error": ""` with
  `terminationReason: "model_stop"`. The schemas always allowed the empty
  string; the successor now documents its meaning and adds an empty-error
  success fixture.

| Event | Current event-specific input | Current output | Change from selected contract |
| --- | --- | --- | --- |
| `PreToolUse` | `toolCall{name,args}`, `stepIdx` (trajectory-wide) | required decision (5 values); optional reason and permission overrides | common input changes; the official `ask` example now matches the reference text exactly |
| `PostToolUse` | optional `toolCall{name,args}`, `stepIdx` (trajectory-wide), optional `error` (`""` = success) | exactly `{}` | common input changes; `toolCall` no longer required |
| `PreInvocation` | `invocationNum`, `initialNumSteps` | optional typed injected steps | common input changes |
| `PostInvocation` | same as `PreInvocation` | optional typed injected steps and termination behavior | common input changes; firing time reworded upstream |
| `Stop` | `executionNum`, open `terminationReason`, optional `error` (`""` = none), `fullyIdle` | required open-vocabulary decision (`continue` is capped) and optional reason | common input changes; the `continue` cap is recorded |

The common input changes for every event are: optional `modelName`, and
`workspacePaths` may be empty.

Every contract also inventories `handler_kinds: [command]`, because the
reference says "Currently only `"command"` is supported". It also carries new
uncertainties for the points above, plus for `stepIdx` scope, exit behavior,
the `~/` prefix in official path examples, the open tool catalog, and the
`action(target)` vocabulary of `permissionOverrides`. `PreInvocation` and
`PostInvocation` previously had no uncertainties.

## Behavioral notes recorded without new protocol members

- **Failing hooks are non-fatal on Antigravity 2.0 since v2.12.0.** The release
  note says "A failing custom hook no longer ends the session; it is now reported
  as an error and the conversation continues." It does not define "failing"
  (nonzero exit, timeout, or invalid stdout), and neither the CLI nor the IDE
  notes have an equivalent. The hook reference still documents no exit codes,
  so no nonzero-exit outcome is modelled; each contract records this as an
  uncertainty instead. The fate of a pending tool call after a failing
  `PreToolUse` hook is undocumented, and CLI 1.1.7's note about a broken plugin
  hook breaking file-editing tools argues against presuming fail-open.
- **`Stop` `continue` is capped.** CLI 1.1.9 ends the turn "after a configurable
  number of consecutive continuations", and Antigravity 2.0 v2.6.0 finishes the
  turn "after repeated blocks". Neither the limit nor its configuration key is
  documented, so `continue` is recorded as a bounded rather than a hard gate.
- **`stepIdx` is trajectory-wide.** The reference defines it as "the 0-based
  index of the current step in the trajectory" (`PreToolUse`) and "of the
  completed step" (`PostToolUse`). It is not a per-invocation index. This
  wording predates the August snapshot; the Rust documentation still says
  "within the invocation".
- **`PostInvocation` timing wording.** The reference now says the event "Fires
  immediately after each model invocation completes". Both the 2026-07-21 copy
  and the IDE copy said "Fires after tool calls finish". This aligns with the
  CLI 1.1.10 hook-ordering fix and the 2.0 v2.6.0 note that custom hooks now run
  at the end of a turn.
- **`Stop.reason` with non-`continue` decisions.** `reason` is defined only
  when `decision` is `"continue"`, when it is injected as a system message. The
  schema still permits it with other decisions, but its effect there is
  undocumented.
- **`PreToolUse` pass-through.** `allow` is defined as "Automatically allows the
  tool execution", and the reference has no no-objection value. CLI 1.0.16
  handles empty decision strings without error, but their effect is
  undocumented. The output schema therefore still rejects `""`, and a new
  negative output fixture records this.

## Tool, handler, and configuration changes recorded as documentation only

- The unified page adds hook locations for each surface. All three read the
  workspace `.agents/hooks.json` and the global `~/.gemini/config/hooks.json`.
  The CLI also reads `~/.gemini/antigravity-cli/settings.json` and plugin
  `hooks.json` files, and 2.0 v2.17.0 lets custom agents declare hook files
  under a `hooks:` front-matter key. `<app_data_dir>` is
  `~/.gemini/antigravity`, `~/.gemini/antigravity-cli`, or
  `~/.gemini/antigravity-ide`, depending on the surface. HookKit does not
  generate Antigravity configuration.
- The reference still lists the same 20 built-in matcher tools with the same
  argument lists. The release notes add changes it does not list:
  - 2.0 v2.11.0 adds `StartPage`, `EndPage`, and `MediaResolution` to
    `view_file`;
  - CLI 1.2.7 retires `find_by_name`, `grep_search`, and `list_dir` from the
    default toolset;
  - tools named elsewhere include `manage_inbox`, `read_resource`, and
    `browser_*`.

  `toolCall.name` and `toolCall.args` remain open and lossless. `run_command`
  still places the command at `CommandLine` and the working directory at `Cwd`.
- `permissionOverrides` strings use the permissions page's `action(target)`
  vocabulary: `read_file`, `write_file`, `read_url`, `execute_url`, `command`
  (prefix, `regex:`, or `*` targets), and `mcp`. The schema keeps them open
  strings. CLI 1.1.28 shows a hook's `reason` as a `Reason:` line in approval
  prompts.
- The reference still supports only `command` handlers. Release notes mention
  "prompt hooks" (CLI 1.1.23) and "hooks that call a model" (2.0 v2.6.0), and
  the Python SDK has in-process lifecycle hooks. None of these is documented as
  a `hooks.json` handler type, so none is inventoried.
- Other hook runtime fixes do not change stdin or stdout:
  - 2.0 v2.6.0 rejects unrunnable configurations at load time;
  - CLI 1.1.12 adds print-mode `/hooks` output;
  - CLI 1.1.17 consolidates onto one execution path;
  - CLI 1.2.3 fixes `/hooks` omitting plugin hooks, and CLI 1.2.4 fixes
    `hooks.json` files dropped under customization truncation and terminal
    `ERROR` steps missing from `transcript.jsonl`;
  - CLI 1.2.5 records signal-killed commands as canceled, which can change
    `PostToolUse.error`.

  CLI 1.2.12 through 1.2.14 contain no hook items.
- The reference still defines no Antigravity-provided hook environment
  variables. The CLI variables in the release notes configure the CLI itself.

## Corrections to the 2026-08-04 audit

That audit's table reported no change for `PreToolUse` and `PostToolUse`, and
it listed `PostToolUse` input as "`stepIdx`, optional `error`". Compared with
`docs-2026-07-12-r2`, however, `docs-2026-08-04-r1` added
`deny_unless_prior_grant` and made `PostToolUse.toolCall` required. The
2026-07-21 capture of the reference documented neither. The audit also cited an
IDE-specific page that omitted both. The successor keeps
`deny_unless_prior_grant`, because the unified reference documents it for every
surface, and relaxes `toolCall` as described above. The audit's "all inputs
still carry" list of four common fields is superseded by `modelName`.

The August `PreToolUse` `ask` output and process fixture were labelled
`official`, but their reason read "Requires confirmation." Every capture of the
reference reviewed here, from 2026-07-21 onward, reads "Requires confirmation
for test execution." The successor uses the verbatim example.

## Evidence and classification

SHA-256 values were computed from bodies fetched on 2026-09-29 (US Pacific;
2026-09-30T04:41Z) with `curl -sL --compressed`. The documentation server
sometimes applies gzip content encoding even without an `Accept-Encoding`
request header, so decoded bodies are the reproducible pins. Two fetches of each
URL matched.

| Source | SHA-256 | Reproducibility |
| --- | --- | --- |
| `/docs/hooks` (HTML, 141741 bytes; identical for `?tab=ide` and `?tab=cli`) | `051967919edfb9904511c9a47e65115cb3def61c34f6f7e14924710df215f180` | content hash |
| `/docs/hooks.md` (16795 bytes) | `c1ce644786c46a666379af40e3983081887fa29245ead461be4235a92ffc80c3` | content hash |
| `/docs/ide/hooks` (redirect, 371 bytes decoded) | `9570816c39c7cef1314bcbac2beca0e9c604634bc3dffafb6cc953f3ea714288` | content hash |
| `/docs/changelog.md` (268410 bytes) | `404ef735cb652cefd51649855e83809b0f17d50d91926d5b0bf3d27acb681b8a` | content hash |
| CLI `CHANGELOG.md` at `eaf9e066` | `09edf2e8af87f1788348e4ec62e9d2ab26f9f90cfc48fca54dc0e14fe9c31d2b` | pinned revision |
| `/docs/permissions.md` (26356 bytes) | `f2dc8d81a16b037fb1493eb8976fa0b55fa9453cb156bbb872f7850ce9aad4bd` | content hash |
| Archive `20260721131156` `/docs/hooks` | `f80ee1810455129ddb7033e8efe8799b9a5cd2a758b22daab02109339c18f0d4` | content hash |
| Archive `20260816124320` `/docs/hooks/` | `b66c78e47c9378716f0b220de2996af380c6c2fd008ab4e10646d623859f8c62` | content hash |
| Archive `20260916032838` `/docs/ide/hooks` | `52752e77383aa12d06bd70e875deff15e2c562833cf31322946677d3be709dce` | content hash |

All documentation-site responses carried the site-wide ETag `"_KFTNQ"`. The
August hash of `/docs/hooks` (`8425858…`) came from the pre-migration site
renderer and cannot be compared with this one. The retired Markdown URL
`/assets/docs/antigravity-2-0/hooks.md`, cited by the July snapshots, now
returns HTTP 404. The ledger retains source-reviewed, low-confidence assurance
and does not claim live-observed stability.

The successor is classified as **additive** (optional `modelName`, new
fixtures, and documentation) plus a **tolerance relaxation** of three input
constraints (`workspacePaths` minimum, `PostToolUse.toolCall` requiredness,
and documented empty `error`). No output schema, exit selector, or event
inventory changed. Wire payloads that validated against `docs-2026-08-04-r1`
still validate against the successor. The native Rust API change is
nevertheless **breaking**: new public fields, an optional `tool_call`, and
acceptance of empty workspace lists. The integration step owns that change.

## Open questions for live observation

- Does the Antigravity IDE (v2.5.5) send `PostToolUse.toolCall` and
  `modelName`, and does it accept `deny_unless_prior_grant`?
- For workspace-less conversations, is `workspacePaths` sent as `[]` or
  omitted?
- After 2.0 v2.12.0, does a nonzero-exit `PreToolUse` hook let the tool run,
  deny it, or prompt? What do the CLI and IDE do?
- What is the consecutive-`continue` cap for `Stop`, and which setting controls
  it?
- What does an empty `PreToolUse` decision mean?
- Are `transcriptPath` and `artifactDirectoryPath` ever sent with a literal
  `~/` prefix, as in every official example?
