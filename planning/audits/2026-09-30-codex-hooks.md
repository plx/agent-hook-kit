# Codex hook contract correction: `commit-ff6aec9-r2`

- Audit date: 2026-09-30
- Previous snapshot: `commit-ff6aec9-r1` (selected)
- Successor snapshot: `commit-ff6aec9-r2` (frozen, not yet selected)
- Pinned upstream revision: unchanged,
  `ff6aec96948b70d94983af2641a6b67c94faeff5` (tag `rust-v0.159.2`)
- Official hook reference: <https://learn.chatgpt.com/docs/hooks.md>,
  unchanged (sha256 `724881d8…c630`, 46,965 bytes, re-fetched 2026-09-30)

This revision reinterprets the same pinned source. No upstream input changed:
the vendored schemas, the hook reference, and the tag are the ones
`commit-ff6aec9-r1` cites. The final review of `commit-ff6aec9-r1` found
uncertainty text that misstates what the pinned source does. A frozen snapshot
cannot be corrected in place, so the text is fixed in `-r2`.

`cargo run -p xtask --bin generate-codex-contracts` now generates both
revisions, from one seed list in which `-r2` overrides only the corrected
texts. Regenerating still reproduces `commit-ff6aec9-r1` byte for byte.
`PreToolUse` stays hand-authored and is identical apart from its snapshot id.

`cargo xtask contracts diff codex/commit-ff6aec9-r1 codex/commit-ff6aec9-r2`:

```text
old: codex/commit-ff6aec9-r1 (12 events)
new: codex/commit-ff6aec9-r2 (12 events)
~ Interrupt: contract.yaml
~ PermissionRequest: contract.yaml
~ PostCompact: contract.yaml
~ PostToolUse: contract.yaml
~ PreCompact: contract.yaml
= PreToolUse
= SessionEnd
= SessionStart
= Stop
= SubagentStart
= SubagentStop
= UserPromptSubmit
~ snapshot files: sources.yaml
events: 0 added, 0 removed, 5 changed, 7 unchanged
content: changed
```

Only uncertainties changed. The outcomes, schemas, and fixtures are the same
apart from snapshot ids.

## Corrections

| Finding | Event | `-r1` text | Pinned source at `rust-v0.159.2` | `-r2` text |
| --- | --- | --- | --- | --- |
| LCAE-4 | `PermissionRequest` | `updatedInput`, `updatedPermissions`, and `interrupt: true` "fail closed" (the reference's wording) | `engine/output_parser.rs`: `unsupported_permission_request_hook_specific_output` sets `invalid_reason`, and `parse_permission_request` then drops the decision whatever its behavior. `events/permission_request.rs`: `parse_completed` marks the run failed with no decision. `core/src/tools/approvals.rs`: a `None` hook decision goes to `request_reviewer_approval`, which is Guardian or the user. | These fields, like `continue: false`, `stopReason`, and `suppressOutput`, fail the run and discard the handler's whole decision. Unless another hook decides, an allow is not auto-approved, and a deny, for example a Claude Code style `interrupt: true` deny, is not enforced. |
| LCAE-5 | `PostCompact` | only "Plain text on stdout is ignored at exit 0" | `events/compact.rs` sets `should_stop` for a synchronous `continue: false` on both compaction events. `compact.rs`, `compact_remote_v2.rs`, and `compact_token_budget.rs` return `CodexErr::TurnAborted` on `PostCompactHookOutcome::Stopped` after compaction succeeded. | A synchronous `continue: false` keeps the compacted history and aborts the active turn for manual and automatic compaction. The turn is reported as interrupted, which on the main thread dispatches `Interrupt`. |
| LCAE-5 (related) | `PreCompact` | the abort applies "on a manual compact" | Pre-turn, mid-turn, and post-turn automatic compaction in `session/turn.rs` also propagate `TurnAborted`. `tasks/mod.rs` reports it as `TurnAbortReason::Interrupted` and runs `run_turn_interrupt_hooks`, which returns early for subagents. | The stop aborts the active turn whether compaction is manual or automatic, and Codex reports the turn as interrupted. |
| LCAE-5 (related) | `Interrupt` | self-aborts "such as a manual compact stopped by a PreCompact `continue: false`" | as above | "such as a turn aborted by a PreCompact or PostCompact `continue: false`" |
| LCAE-M1 | `PostToolUse` | no statement of the `continue: false` effect | `events/post_tool_use.rs` checks `continue: false` first. It sets the run to `Stopped` and pushes the trimmed `reason` (or `stopReason`, or `PostToolUse hook stopped execution`) as model feedback, but leaves `should_block` false. `core/src/tools/registry.rs` then replaces only the model-visible tool result (`PostToolUseFeedbackOutput`), and the turn goes on. | A synchronous `continue: false` does not end the turn. It takes precedence over `decision: block`, the reason rules, and the unsupported fields (`suppressOutput`, `updatedMCPToolOutput`). It replaces the model-visible result, and the model continues from that text. |

`PostToolUse` also narrows an `-r1` statement. `suppressOutput` and
`updatedMCPToolOutput` fail the run only when `continue` is not false,
because the `continue: false` arm of `parse_completed` is checked first.

The hook reference's PermissionRequest "fail closed" wording is recorded as a
limitation of `codex-hooks-reference` in `sources.yaml`. The source
limitations name the files re-reviewed for this revision.

## Other checks

The remaining `-r1` claims were re-read against the tag and still hold:

- the `Interrupt` output wire is `deny_unknown_fields`;
- the `SessionStart` `continue: false` ends the turn without a model request
  (`run_pending_session_start_hooks` makes the turn return `Ok(None)`);
- `SubagentStart` ignores `continue: false`;
- `Stop` and `SubagentStop` fail on plain text;
- compaction and `SessionEnd` report visible stderr on failures;
- exit 2 blocks only with non-blank stderr;
- the `PreToolUse` unsupported-output rules.

The `-r1` outcome sets still model a JSON-looking exit-0 stdout that fails to
parse, and plain text on events that ignore it, only in uncertainties. That
is incomplete but not wrong, so `-r2` keeps the outcome sets unchanged.

## Follow-up

- `hookkit-codex` needs its snapshot literal and contract ids moved to `-r2`,
  plus rustdoc fixes:
  - `PostToolUseOutput::with_continue` says `continue: false` "stops the
    turn", which contradicts LCAE-M1;
  - `PostCompactOutput` needs the LCAE-5 turn abort.
- `hookkit-conformance` `OPEN_VALUE_SET_NEGATIVES` names
  `codex/commit-ff6aec9-r1/SessionEnd` and `/SessionStart`; they become
  `-r2`. Other literals in `hookkit-runtime` and `hookkit-tool-access` tests
  change the same way.
- A successor command-environment supplement must target `-r2`.

Until then `contracts/registry.yaml` keeps `commit-ff6aec9-r1` selected.
