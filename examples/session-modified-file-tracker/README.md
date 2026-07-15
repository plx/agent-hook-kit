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

By default, state is stored as:

```text
$TMPDIR/agent-hook-kit/session-modified-files/
  <harness>/
    <session>/
      modified-files/
        <sha256-of-normalized-path>.path
```

Each marker contains the normalized path as one line. Creating markers with
`create_new` makes repeated and concurrent observations idempotent without a
read/modify/write race. Pass `--state-dir PATH` to choose another root. A future
stop-time linter can enumerate the marker contents to recover the accumulated
set.

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
- Stop-time linting cannot yet be added cross-harness. Antigravity has a typed
  `Stop`, but the selected Claude, Codex, and Gemini adapters only catalog their
  stop events; there is no aligned stop input/output API.
- There is no shared session-state/update primitive, so this example owns an
  atomic marker layout and leaves retention/cleanup to a future stop phase.
