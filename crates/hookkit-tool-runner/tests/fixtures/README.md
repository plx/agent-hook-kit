# Tool fixture format

Each fixture exercises one builtin tool against a small on-disk example and
verifies the runner's harness-specific output. Fixtures are auto-discovered by
the `tool_fixtures.rs` integration test.

## Directory layout

```
tests/fixtures/<tool-id>/<example-name>/
  example.<ext>                # the input file (required)
  <supporting files>           # optional sibling files copied as-is
  expected/<rel-path>          # optional: expected post-run content of <rel-path>
  claude.json                  # optional: expected stdout JSON for --claude
  claude.stderr.txt            # optional: expected stderr (literal, normalized)
  claude.exit                  # optional: expected exit code (default 0)
  codex.json codex.stderr.txt codex.exit
```

- `<tool-id>` matches the tool's `id` field in `crates/hookkit-pkl-config/src/builtins/tools/<tool>.pkl` (e.g. `ruff`, `cargo-fmt`).
- The harness picks the entry file (the one cited in the synthesized
  `PostToolUse` event) by looking for a top-level file whose name starts with
  `example.`. If none exists, the first non-golden, non-`expected/` file at
  the fixture root is used.
- Every non-golden, non-`expected/` file (including subdirectory contents like
  `src/main.rs` or `Cargo.toml`) is copied into the test's temp workspace at
  the same relative path.

## Golden output files

`<harness>.json` is the expected normalized stdout. The harness parses both
golden and actual as JSON for structural comparison so whitespace and key order
don't matter.

`<harness>.stderr.txt` is the expected normalized stderr (literal trimmed
comparison). Absent means stderr is expected to be empty.

`<harness>.exit` is a single line with the expected exit code. Absent means 0.

If `<harness>.json` is missing, the harness asserts stdout is empty for that
harness — useful when a harness can't represent a scenario (e.g. Codex has no
post-tool additional-context surface).

## `expected/` post-state mirror

Any files inside `expected/<rel-path>` are compared against the post-run
content of `<rel-path>` in the temp workspace. Use this when the tool rewrites
a file (autofix, formatting) and you want to assert the result.

## Normalization placeholders

When comparing outputs, the harness substitutes the test's temp workspace
path with the literal `<workspace>` in both actual and golden, so paths in
golden files should use `<workspace>` for anything under the test project.

The session id is fixed at `test-session`, so golden files can reference it
directly without normalization.

## Skip behavior

- If `pkl` isn't on PATH, the whole test prints "skipping" and returns.
- If a tool's executable isn't on PATH, all that tool's fixtures are skipped.
- Fixtures referencing a tool with no builtin spec are skipped with a notice.
