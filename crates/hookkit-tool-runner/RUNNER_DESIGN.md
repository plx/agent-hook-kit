# Post-tool-use runner design

The runner owns tool selection, recursive file discovery, Pkl policy, execution,
classification, diagnostics artifacts, and the `RunnerDomainOutcome` semantic
result. Core, native, common, and runtime crates do not depend on those policies.

Per-harness lowering is intentionally explicit. The aligned runtime preserves a
lossless Claude, Codex, or Gemini CLI input arm; runner-local policy discovers
paths from that exact value; and the runner constructs the corresponding native
output arm. No generic output envelope or universal lowering layer participates.
Antigravity `PostToolUse` contains a step index and optional error but no tool call,
tool result, or changed-file list, so this runner does not claim Antigravity
support and must not infer those fields.

The runtime context supplies exact harness, snapshot, event, and contract identity
plus only context fields declared by the native event. Operational diagnostics are
kept out of protocol stdout; user-visible stderr is emitted only through an exact
native output, while verbose remaining-tool output belongs in runner artifacts.

To add a tool, extend the Pkl catalog and its fake-executable orchestration cases;
real-tool fixtures belong to the opt-in compatibility lane. To add a harness,
first prove how its native event yields changed files, then add an explicit final
lowering arm and exact native fixture tests.

Performance budgets are not yet release guarantees. The hermetic smoke lane checks
behavior but does not currently record startup, clean no-op, or subprocess timing;
add a dedicated benchmark/measurement lane before publishing performance claims.

## Turn-completion batching

turn-completion-agent-hook reuses the same Pkl catalog and execution engine,
but obtains candidate files from the `hookkit-file-activity` pending entity
maintained by session-modified-file-tracker. Before taking the entity view, it
reconciles workspace mtimes from the prior durable cursor, using current-session
start metadata only as the first lower bound. The aligned lifecycle is Claude/Codex Stop,
Gemini AfterAgent, or Antigravity Stop; Antigravity normally has no batch
because its post-tool payload cannot feed the tracker.

One runner-family advisory lock serializes stop attempts for a native session.
The consumer seals NDJSON generations and obtains their cached set projection
before executing tools. Stop-time `workflows` are distinct from the immediate
runner's legacy `phases`: all non-mutating initial checks run first, only dirty
workflows receive one ordered remedy, and snapshot-discovered writes invalidate
intersecting target-file or workspace checks for one authoritative final sweep.
Check stages retain bounded job parallelism and deterministic result ordering.

Every executed deferred command writes its own artifact under a deterministic
tool/workflow/job/phase path in a unique run bundle. Artifact metadata includes
structured argv, working directory, candidate and changed files, exit code,
classification, full output, and its report identity. One report/artifact can
therefore be linked by every conservatively attributed file, while a file
covered by several tools retains all distinct links.

The runner commits `summary.json` only after every command artifact is durable
and before changing pending state. The summary contains run identity, counts,
normal buckets, current groups, artifact paths and a separate path-to-contents
map, the complete result model, rendered-message metadata, and the planned
source disposition. The runner then appends stable retry evidence for only
manual, operationally incomplete, and unresolved work, records content-based
handled baselines for discharged work, and acknowledges the sealed source
generations. New observations written during execution are outside the snapshot
and remain pending independently. Mtime and opt-in Git-dirty reconciliation
suppress only fingerprints that still match a handled baseline; direct
observations always requeue the path.

Coverage gaps use the Pkl `fileActivity.coverageGapPolicy`. The default
`best-effort` policy retains and summarizes incomplete targets without treating
resolved clean files as manual. `strict` also blocks Stop until the gap clears.
