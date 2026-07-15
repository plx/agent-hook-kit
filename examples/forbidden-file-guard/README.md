# Forbidden file guard

`forbidden-file-guard` is one clap-based executable with three native pre-tool
modes:

```bash
forbidden-file-guard --harness=codex
forbidden-file-guard --harness=gemini
forbidden-file-guard --harness=antigravity
```

With no `--config` arguments, it additively loads existing files from:

1. `~/.agent-hook-kit/forbidden-files.yaml`
2. each ancestor's `.agent-hook-kit/forbidden-files.yaml` for every native
   workspace root

One or more `--config PATH` arguments replace discovery and are also merged
additively. An explicitly requested file that is absent, unreadable, or invalid
causes a native deny rather than a handler error.

The format is:

```yaml
patterns:
  - ".env"
  - "**/.env"
  - "**/.env.*"
  - "~/.ssh/**"
block_shell_commands: true
```

See [`forbidden-files.example.yaml`](forbidden-files.example.yaml) for a
commented version. Patterns are matched against both absolute paths and paths
relative to each workspace root. Home `~/` patterns are expanded before glob
compilation.

The handler recursively inspects structured path fields, recognizes
`apply_patch` headers, and lexically checks obvious shell path tokens. A match is
lowered to Codex `deny`, Gemini `deny`, or Antigravity `ToolDecision::Deny`.

For example, after building the binary, the checked-in Antigravity fixture
produces a native deny response:

```bash
target/debug/forbidden-file-guard \
  --harness=antigravity \
  --config examples/forbidden-file-guard/forbidden-files.example.yaml \
  < examples/forbidden-file-guard/fixtures/antigravity_pre_tool_use.json
```

Existing candidates are also canonicalized before matching so a symlink does not
hide an otherwise forbidden target.

## Security boundary

A hook cannot reliably determine which files an arbitrary shell program will
open. With `block_shell_commands: false`, constructs such as environment
variables, command substitution, generated scripts, and subprocess behavior can
bypass lexical inspection. Set `block_shell_commands: true` to fail closed; the
tradeoff is that every shell tool call is denied while that policy is active.
For a real secret boundary, combine this hook with OS sandbox or harness-native
filesystem restrictions rather than treating string inspection as isolation.

Claude Code mode is intentionally unavailable. The selected `hookkit-claude`
snapshot catalogs Claude `PreToolUse` but does not yet implement its native
parser/output type, so the runtime cannot safely decode or deny that event. The
three implemented arms also expose a missing aligned pre-tool abstraction: each
must currently extract the same policy inputs and build a different native
output by hand.
