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

For structured tool calls the handler recursively inspects path-bearing fields
and recognizes `apply_patch` headers. Shell tool calls are identified by
`hookkit-shell`'s native adapters (`ShellToolCallExt`) — Codex/Claude `Bash`,
Gemini `run_shell_command`, and Antigravity `run_command` (whose command lives in
`CommandLine`, with its own `Cwd`) — and their commands are inspected with a
bounded Bash parse (`BashAnalyzer` + `FileAccessAnalyzer`). That parse resolves
each read, write, delete, move, and redirection target against the call's working
directory, so `cat .env`, `echo x > sub/.env`, and `rm -rf .env` are all caught,
while a write to an unlisted file passes. A match is lowered to Codex `deny`,
Gemini `deny`, or Antigravity `ToolDecision::Deny`.

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
open. The Bash parse resolves literal and glob targets, but parameter expansion,
command substitution, `eval`/`source`, generated scripts, and subprocess behavior
remain outside static syntax — the analyzer reports those as *unresolved* file
accesses rather than silently missing them (as the old lexical scan did). This
guard still denies only on a concrete pattern match, so with
`block_shell_commands: false` a command like `cat "$SECRET"` can pass. Set
`block_shell_commands: true` to fail closed; the tradeoff is that every shell tool
call is denied while that policy is active. For a real secret boundary, combine
this hook with an OS sandbox or harness-native filesystem restrictions rather than
treating static analysis as isolation.

Claude Code mode is intentionally unavailable. The selected `hookkit-claude`
snapshot catalogs Claude `PreToolUse` but does not yet implement its native
parser/output type, so the runtime cannot safely decode or deny that event.

`hookkit-shell` now unifies the *input* side across harnesses, but the three arms
still build their native outputs by hand. Unlike post-tool hooks — which run
through `hookkit_runtime::aligned::run_aligned_event::<PostToolUse>` and match on
one `PostToolUseInput`/`PostToolUseOutput` enum — there is no aligned `PreToolUse`
event, and its `AlignedEventSpec` trait is sealed, so the allow/deny lowering
cannot yet be shared.
