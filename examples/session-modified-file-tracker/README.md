# Session-modified file tracker

`session-modified-file-tracker` is now a compatibility demonstration for the
shipped `file-activity-agent-hook`. Both commands call the same library-owned
aligned observer; new installations should bind the shipped binary:

```bash
file-activity-agent-hook --claude
file-activity-agent-hook --codex
```

The example keeps `--harness=claude|codex` and `--state-dir=PATH`
compatible, while the shipped binary accepts those aliases as well.

The hook uses `hookkit-file-activity` to classify known structured writer
names, parse `apply_patch` headers, and run the bounded `hookkit-shell`
file-access analyzer over native shell calls. Every candidate retains its
effect, source, certainty, timestamp, event, tool-call ID, and turn ID. Known
blind spots (such as a dynamic shell path) are recorded as coverage gaps rather
than silently discarded. The post-tool observer does not invoke Git, diff the
workspace, or snapshot the tree.

By default, state is stored as a windowed `PendingFileActivity` entity whose
event log uses rotated NDJSON generations. A separate monotonic entity stores
the stop-time reconciliation cursor:

```text
$TMPDIR/agent-hook-kit/session-state/v1/
  <harness>/session/<identity-hash>/families/
    agent-hook-kit.file-activity/v1/scopes/session/
      entities/pending-files/v2/
        active-generation.json
        generations/<generation-id>.ndjson
        projection-cache.json
      entities/reconciliation-cursor/v1/
        checkpoint.json
```

The entity projection supplies the consumer with a set of exact, descendant,
glob, or workspace targets while retaining detailed evidence and gaps. A short
append lock makes concurrent writes safe. Pass `--state-dir PATH` to choose
another common state root.

The shipped `turn-completion-agent-hook` first reconciles workspace mtimes from
the previous durable cursor (using current-session start metadata to bootstrap),
then seals an exact generation window and runs all matching Pkl-configured
read-only checks, conditional remedies, and invalidated final checks. It emits
configured clean and auto-fixed reports, appends retry evidence only for manual,
operationally incomplete, and unresolved work, records handled baselines for
discharged files, then acknowledges the sealed generations. Entries appended
while a batch is running go to a new generation and are not part of its
acknowledgement. Retried work loads a cached set and folds only newly arrived
generations.

The library also exposes an opt-in `GitDirty` fallback. It is disabled by
default because a dirty working tree cannot distinguish agent edits from
changes that predated the session.

A checked-in Codex fixture makes the state layout easy to inspect. For example:

```bash
target/debug/session-modified-file-tracker \
  --harness=codex \
  --state-dir=.context/session-modified-files \
  < examples/session-modified-file-tracker/fixtures/codex_post_apply_patch.json
```

The tracker is deliberately best effort. Executed programs can still change
files that do not appear in their open payload or statically analyzable argv.
Timestamp reconciliation is noisy, while dirty VCS state is broader still.
Consumers can inspect provenance and gap counts instead of mistaking the
combined result for a complete audit log.

## API findings

- The aligned `PostToolUse` API makes the Claude Code and Codex executable
  genuinely shared while preserving native input and output arms.
- Antigravity is not offered as a mode: its `PostToolUse` payload has no tool
  call or arguments, while observing `PreToolUse` would require returning a
  permission decision and would record attempts rather than completed calls.
- Aligned `TurnCompletion` maps Claude Code, Codex, and Antigravity `Stop`
  without erasing their native contracts. Antigravity still
  cannot contribute modified paths because its `PostToolUse` omits the tool
  call.
- The tracker and consumer coordinate only through the versioned
  `agent-hook-kit.file-activity` family; unrelated hook families remain
  isolated.
