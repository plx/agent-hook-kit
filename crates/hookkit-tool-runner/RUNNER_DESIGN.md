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
The consumer seals NDJSON generations and obtains their cached set projection before executing tools, writes complete
per-tool logs to a unique run bundle, and commits summary.json last. Clean and
auto-corrected snapshots are acknowledged and lowered to a quiet native
response. Manual findings and operational failures retain their generations and
lower to the native “continue working” response with a summary path. New
observations written during execution are outside the snapshot and remain
pending even when the completed batch is acknowledged. Projection caching means
a retained retry folds only generations that arrived after the prior attempt.
