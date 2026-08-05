# Antigravity hook contract refresh

- Audit date: 2026-08-04
- Previous selected snapshot: `docs-2026-07-12-r2`
- Successor snapshot: `docs-2026-08-04-r1`
- Official hook reference: <https://antigravity.google/docs/hooks>
- Companion IDE reference: <https://antigravity.google/docs/ide/hooks>
- Official CLI changelog revision: `bfab12dac5bd090015a89cf82e65093d13b567d9`

This audit re-read the complete current hook reference, compared its event and
field tables with the selected July snapshot, checked the IDE-specific copy of
the reference, and reviewed the Antigravity CLI changelog through 1.1.10.

## Result

The documented wire inventory remains five command events:
`PreInvocation`, `PostInvocation`, `PreToolUse`, `PostToolUse`, and `Stop`.
No new input field, output field, handler type, or Antigravity-provided process
environment variable was found. All inputs still carry `conversationId`,
`workspacePaths`, `transcriptPath`, and `artifactDirectoryPath`.

The refresh did find one implementation mismatch in an already documented
output contract. `PostInvocation.injectSteps` is explicitly the same schema as
`PreInvocation.injectSteps`, but the Rust API accepted arbitrary JSON values and
only rejected non-objects. The successor snapshot and implementation now use
the shared typed `InjectStep` enum and reject objects that combine step kinds.
The `TerminationBehavior` enum also now represents the documented explicit
empty-string default in addition to omission, `force_continue`, and `terminate`.

| Event | Current event-specific input | Current output | Change from selected contract |
| --- | --- | --- | --- |
| `PreToolUse` | `toolCall{name,args}`, `stepIdx` | required decision; optional reason and permission overrides | none |
| `PostToolUse` | `stepIdx`, optional `error` | `{}` | none |
| `PreInvocation` | `invocationNum`, `initialNumSteps` | optional typed injected steps | none |
| `PostInvocation` | same as `PreInvocation` | optional typed injected steps and termination behavior | implementation/schema tightening |
| `Stop` | `executionNum`, `terminationReason`, optional `error`, `fullyIdle` | required open-vocabulary decision and optional reason | none |

## Tool and runtime review

The current reference enumerates 20 built-in matcher tool names. HookKit keeps
Antigravity `toolCall.name` and `toolCall.args` lossless rather than closing the
protocol over that mutable catalog, so additions do not require enum changes.
The exact shell adapter remains current: `run_command` still places the command
at `CommandLine` and the optional working directory at `Cwd`.

The CLI 1.1.9 and 1.1.10 changelogs describe fixes to `PostToolUse` matcher
dispatch, stop-hook continuation limits, and hook ordering so final
`PostInvocation` and `Stop` hooks execute. Those are harness runtime fixes, not
new stdin/stdout fields, so they are recorded here without inventing protocol
members.

## Evidence and classification

The generic hook page returned SHA-256
`8425858481fb9fdc43613c1ddfbf7e851ba4b1a9f8981974ff094c4619702669`;
the IDE-specific page returned
`a53f89622bce86355b06d1681575b07b0b0be470a7b52d343de3f01de16365bb`.
Both are mutable rendered pages, so the frozen ledger retains source-reviewed,
low-confidence assurance and does not claim live-observed stability.

The successor is classified as a contract tightening plus documentation refresh:
the event and input inventory is unchanged, while one permissive output type is
narrowed to the schema the documentation has always described.
