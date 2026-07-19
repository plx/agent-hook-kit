# Codex Claude Code rules handler

`codex-claude-rules` is a typed Codex `PreToolUse` example. It discovers
path-scoped markdown rules recursively from the same two locations used by
Claude Code:

1. `~/.claude/rules/**/*.md`
2. `<project>/.claude/rules/**/*.md`

User rules are considered before project rules. Only files with a YAML
frontmatter `paths` string or list are handled; Claude Code loads unscoped rules
at session start, which is outside this example's purpose.

All structured, patch, and shell calls go through
`hookkit_tool_access::ToolAccessAnalyzer`. Its shell analyzer enables the
opt-in literal-operand fallback for unknown commands, and `resolve_targets`
materializes exact, descendant, exact-or-descendant, glob, and workspace
scopes within a 100,000-entry budget. Nonexistent exact paths are retained so
a rule can activate before a write creates its target.

Materialized paths are matched relative to the project root. A newly matched rule is
claimed through the shared session-scoped `ClaimSet`, then its markdown body is
returned through `PreToolUseOutput::with_context`. Atomic claims mean concurrent
hook processes cannot both decide one rule is new. By default, state lives in
the versioned `agent-hook-kit.codex-claude-rules` family below
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

The checked-in fixture uses a placeholder `/workspace` cwd. Set its native cwd
to the same absolute fixture project used by the command before exercising it:

```bash
fixture_project="$(pwd)/examples/codex-claude-rules/fixture-project"
jq --arg cwd "$fixture_project" '.cwd = $cwd' \
  examples/codex-claude-rules/fixtures/pre_tool_use_apply_patch.json \
  | target/debug/codex-claude-rules \
  --project-root "$fixture_project" \
  --claude-home "$fixture_project/empty-claude-home" \
  --state-dir .context/codex-claude-rules-state
```

The first invocation emits Codex `additionalContext`; repeating it with the
same state directory and session emits the native empty no-op.

## Exactness boundary

An exact clone of Claude Code's behavior is not possible with the current Codex
hook surface. Codex `PreToolUse` currently observes Bash, `apply_patch`, and MCP
tool calls rather than every internal file open. Structured `path`/`file_path`
arguments and patch headers are reliable enough to match. Analysis and target
materialization are bounded and static: variables, command substitutions,
sourced/generated code, executables' internal behavior, filesystem races, and
runtime effects can still hide the eventual path. Heuristic literal operands
can also over-report path-looking arguments. Recovered concrete paths may
activate rules; unresolved or truncated targets are never converted into a
match.

Relative tool operands always resolve against the native hook cwd.
`--project-root` controls rule discovery, materialization roots for unrooted
scopes, and project-relative pattern matching; it does not rewrite tool
operands. A resolved target outside that project root does not match a
project-relative rule.

`ClaimSet` fits the immediate first-writer-wins decision here better than an
NDJSON entity journal: this hook does not consume event history or need a
materialized loaded-rule projection.

## Refactoring notes

- [`WHAT_CHANGED.md`](WHAT_CHANGED.md) records the completed refactor and its
  behavioral impact.
- [`FUTURE_REFINEMENTS.md`](FUTURE_REFINEMENTS.md) records which exposed
  HookKit API opportunities are now addressed and which remain deferred.
