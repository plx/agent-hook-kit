# Forbidden file guard

`forbidden-file-guard` is one aligned executable with four native pre-tool
modes:

```bash
forbidden-file-guard --harness=claude
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
access_policy: deny_unresolved
```

See [`forbidden-files.example.yaml`](forbidden-files.example.yaml) for a
commented version. Patterns are matched against both absolute paths and paths
relative to each workspace root. Home `~/` patterns are expanded before glob
compilation.

`access_policy` has three postures:

- `inspect_known` matches recovered candidates and allows analysis or resolver gaps;
- `deny_unresolved` also denies any incomplete analysis or materialization; and
- `deny_all_shell` denies exact native shell calls while continuing to inspect
  non-shell calls.

The legacy `block_shell_commands: true` remains accepted and maps to
`deny_all_shell`; `false` maps to the default `inspect_known` behavior.

Every native input is handled through aligned `PreToolUse` and
`ToolAccessAnalyzer`. Structured fields, patch operations, exact native shell
profiles, heuristic literal operands, and literal shell `apply_patch` heredocs
produce one provenance-bearing report. `resolve_targets` then materializes
descendant, glob, and workspace scopes within a 100,000-entry budget while
retaining nonexistent exact write targets. Thus `rm -rf secrets` can match a
`secrets/**` policy when descendants exist.

For example, after building the binary, the checked-in Antigravity fixture
produces a native deny response:

```bash
target/debug/forbidden-file-guard \
  --harness=antigravity \
  --config examples/forbidden-file-guard/forbidden-files.example.yaml \
  < examples/forbidden-file-guard/fixtures/antigravity_pre_tool_use.json
```

Raw and native-cwd-resolved exact forms are checked before materialization.
Existing concrete candidates are also canonicalized explicitly before matching,
so a symlink does not hide an otherwise forbidden target. Canonicalization is a
filesystem-aware guard choice, not part of HookKit's lexical path API.

## Security boundary

A hook cannot reliably determine which files an arbitrary shell program will
open. The Bash parse resolves literal and glob targets, but parameter expansion,
command substitution, `eval`/`source`, generated scripts, and subprocess behavior
remain outside static syntax — the analyzer reports those as *unresolved* file
accesses rather than silently missing them. Under `inspect_known`, a command
like `cat "$SECRET"` can pass; `deny_unresolved` rejects it without rejecting
every inspectable shell call. Resolver truncation follows the same explicit
posture. For a real secret boundary, combine
this hook with an OS sandbox or harness-native filesystem restrictions rather than
treating static analysis as isolation.

Native cwd determines relative operand meaning. Workspace roots determine
policy discovery and project-relative matching; they never rewrite operands.
