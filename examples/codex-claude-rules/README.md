# Codex Claude Code rules handler

`codex-claude-rules` is a typed Codex `PreToolUse` example. It discovers
path-scoped markdown rules recursively from the same two locations used by
Claude Code:

1. `~/.claude/rules/**/*.md`
2. `<project>/.claude/rules/**/*.md`

User rules are considered before project rules. Only files with a YAML
frontmatter `paths` string or list are handled; Claude Code loads unscoped rules
at session start, which is outside this example's purpose.

For every path visible in the Codex tool input, the hook matches the path
relative to the project root. A newly matched rule is inserted with the shared
monotonic `SetJournal`, then its markdown body is returned through
`PreToolUseOutput::with_context`. The set is reconstructed from its checkpoint,
projection cache, and NDJSON generations; its consumer lock means concurrent
hook processes cannot both decide one rule is new. By default, state lives in the versioned
`agent-hook-kit.codex-claude-rules` family below
`$TMPDIR/agent-hook-kit/session-state/`; `--state-dir` overrides the common
state root. Session identifiers are hashed rather than used as path components.

Build and inspect the CLI:

```bash
cargo build -p codex-claude-rules
target/debug/codex-claude-rules --help
```

A Codex `hooks.json` entry can invoke it for every supported pre-tool call:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "/absolute/path/to/codex-claude-rules"
          }
        ]
      }
    ]
  }
}
```

For isolated tests, `--project-root`, `--claude-home`, and `--state-dir` replace
all ambient path choices.

The checked-in fixture can be exercised from the repository root:

```bash
target/debug/codex-claude-rules \
  --project-root examples/codex-claude-rules/fixture-project \
  --claude-home examples/codex-claude-rules/fixture-project/empty-claude-home \
  --state-dir .context/codex-claude-rules-state \
  < examples/codex-claude-rules/fixtures/pre_tool_use_apply_patch.json
```

The first invocation emits Codex `additionalContext`; repeating it with the
same state directory and session emits the native empty no-op.

## Exactness boundary

An exact clone of Claude Code's behavior is not possible with the current Codex
hook surface. Codex `PreToolUse` currently observes Bash, `apply_patch`, and MCP
tool calls rather than every internal file open. Structured `path`/`file_path`
arguments and patch headers are reliable enough to match; shell commands are
only lexically inspected, so indirection through variables, command
substitution, or a script can hide the eventual path. This example therefore
implements the observable subset and does not claim that every Codex file
action triggers a rule.

The example uses `SetJournal::insert_once`, so it is also a concrete example of
building a monotonic entity from journal events and caching/compacting the
aggregate without sharing files with unrelated hook families.
