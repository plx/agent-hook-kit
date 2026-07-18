# Codex Claude Code rules handler

`codex-claude-rules` is a typed Codex `PreToolUse` example. It discovers
path-scoped markdown rules recursively from the same two locations used by
Claude Code:

1. `~/.claude/rules/**/*.md`
2. `<project>/.claude/rules/**/*.md`

User rules are considered before project rules. Only files with a YAML
frontmatter `paths` string or list are handled; Claude Code loads unscoped rules
at session start, which is outside this example's purpose.

For structured and patch tools, the hook resolves visible path fields and patch
headers against the native working directory. For `Bash`, HookKit's exact Codex
adapter extracts `/command`, its bounded Bash parser recovers commands and
redirections, and its file-access analyzer supplies semantically classified
path candidates. Commands outside the analyzer's built-in table fall back to
path-like operands from HookKit's recovered literal argv; the example no longer
maintains its own shell lexer.

Each candidate is matched relative to the project root. A newly matched rule is
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
arguments and patch headers are reliable enough to match. Shell analysis is
bounded and static: variables, command substitutions, sourced/generated code,
executables' internal behavior, and runtime filesystem effects can still hide
the eventual path. Unsupported commands use a deliberately broad fallback over
literal operands, which can also over-report path-looking arguments.

The file-access analyzer retains exact/descendant/glob/workspace target scope,
but this example can only reuse its resolved path and does not expand scoped
targets into the files a command will eventually touch. A broad directory read
can therefore miss a descendant-only rule until Codex exposes a more specific
path. The hook implements the observable subset and does not claim that every
Codex file action triggers a rule.

`ClaimSet` fits the immediate first-writer-wins decision here better than an
NDJSON entity journal: this hook does not consume event history or need a
materialized loaded-rule projection.

## Refactoring notes

- [`WHAT_CHANGED.md`](WHAT_CHANGED.md) records the completed refactor and its
  behavioral impact.
- [`FUTURE_REFINEMENTS.md`](FUTURE_REFINEMENTS.md) captures the HookKit API
  opportunities exposed by the example.
