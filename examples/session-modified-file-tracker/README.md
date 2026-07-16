# Session-modified file tracker

`session-modified-file-tracker` is a cross-harness `PostToolUse` example built
on the lossless aligned runtime:

```bash
session-modified-file-tracker --harness=claude
session-modified-file-tracker --harness=codex
session-modified-file-tracker --harness=gemini
```

The hook classifies known structured writer names, parses `apply_patch` file
headers, and recognizes a small set of direct shell mutations and redirections.
It normalizes every candidate relative to the native workspace root and records
it without invoking `git`, diffing the workspace, or snapshotting the tree.

By default, state is stored as a windowed `ModifiedFiles` entity whose event
log uses rotated NDJSON generations:

```text
$TMPDIR/agent-hook-kit/session-state/v1/
  <harness>/session/<identity-hash>/families/
    agent-hook-kit.modified-files/v1/scopes/session/
      entities/dirty-files/v1/
        active-generation.json
        generations/<generation-id>.ndjson
        projection-cache.json
```

Each NDJSON line contains the normalized path plus its hook event and optional
tool-call identity. The entity projection supplies the consumer with a set of
paths while retaining detailed source events. A short append lock makes
concurrent writes safe. Pass `--state-dir PATH` to choose another common state
root.

The shipped `turn-completion-agent-hook` seals an exact generation window and runs all
matching Pkl-configured formatter/linter phases at turn completion. A clean or
fully auto-corrected batch is acknowledged quietly. Manual findings retain the
exact snapshot for a retry and point the agent and user at committed detailed
logs. Entries appended while a batch is running go to a new generation and are
not part of its acknowledgement. Retained retries load a cached set and fold
only newly arrived generations.

Checked-in Codex and Gemini fixtures make the state layout easy to inspect. For
example:

```bash
target/debug/session-modified-file-tracker \
  --harness=codex \
  --state-dir=.context/session-modified-files \
  < examples/session-modified-file-tracker/fixtures/codex_post_apply_patch.json
```

The tracker is deliberately best effort. It misses files changed inside scripts
or tools whose open payloads do not expose a recognizable path. A failed tool
can also partially modify a file, so observing post-tool input is more useful
than assuming a nonzero result means no change.

## API findings

- The aligned `PostToolUse` API makes the Claude, Codex, and Gemini executable
  genuinely shared while preserving native input and output arms.
- Antigravity is not offered as a mode: its `PostToolUse` payload has no tool
  call or arguments, while observing `PreToolUse` would require returning a
  permission decision and would record attempts rather than completed calls.
- Aligned `TurnCompletion` maps Claude/Codex `Stop`, Gemini `AfterAgent`, and
  Antigravity `Stop` without erasing their native contracts. Antigravity still
  cannot contribute modified paths because its `PostToolUse` omits the tool
  call.
- The tracker and consumer coordinate only through the versioned
  `agent-hook-kit.modified-files` family; unrelated hook families remain
  isolated.
