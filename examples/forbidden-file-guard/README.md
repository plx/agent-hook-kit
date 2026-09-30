# Forbidden file guard

`forbidden-file-guard` is one aligned executable with three native pre-tool
modes:

```bash
forbidden-file-guard --harness=claude
forbidden-file-guard --harness=codex
forbidden-file-guard --harness=antigravity
```

## Responses

A call that touches a forbidden path gets the harness's native deny, with a
reason Claude or Codex reads. Every other call gets a *pass-through*
(`hookkit_common::PreToolUseOutput::pass_through`), never an explicit allow:

| Harness | Forbidden path | No objection |
| :- | :- | :- |
| Claude Code | `permissionDecision: "deny"` | `{}`: the normal permission flow decides |
| Codex | `permissionDecision: "deny"` | empty stdout: the normal approval flow decides |
| Antigravity | `{"decision":"deny"}` | `{"decision":"ask"}` |

An explicit `allow` would skip Claude Code's permission prompt and bypass
Antigravity's Ask presets for every call the guard does not deny, turning a
deny-list into an auto-approver. Antigravity requires a decision and documents
no pass-through, so the guard answers `ask`, which respects "Always Allow"
settings and cached grants; it can prompt for a call Antigravity would
otherwise have run silently, which is the least-privilege trade-off.

The guard runs with `RunOptions::fail_closed()`. If it cannot decide (stdin
is unreadable, the payload or hook environment is invalid, the handler fails
or panics, or the response cannot be emitted), it blocks the call: exit 2 with
the diagnostic on stderr for Claude Code and Codex, and a `deny` decision for
Antigravity. A policy file that fails to load is also a deny.

Argument errors, such as `--harness=claud`, print the usage error and exit 1,
which Claude Code and Codex treat as a non-blocking hook error. Clap's default
status 2 would be read as a blocking decision. `--help` and `--version` exit 0.

## Configuration

With no `--config` arguments, it additively loads existing files from:

1. `~/.agent-hook-kit/forbidden-files.yaml`
2. each ancestor's `.agent-hook-kit/forbidden-files.yaml` for every policy
   root

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
relative to each policy root. Home `~/` patterns are expanded before glob
compilation.

The policy roots are the stable project roots followed by the native working
directory: Claude Code's `CLAUDE_PROJECT_DIR` and its current `cwd`, Codex's
`cwd`, and Antigravity's workspace paths. Claude Code's `cwd` follows `cd` and
worktree switches, so after `cd src` or `cd /tmp` the guard still discovers the
project policy and matches `secrets/**` relative to the project root. The
current directory stays a root so a worktree the agent entered is covered too.

`access_policy` selects one of three postures:

- `inspect_known` matches recovered candidates and allows analysis or resolver gaps;
- `deny_unresolved` also denies any incomplete analysis or materialization; and
- `deny_all_shell` denies exact native shell calls while continuing to inspect
  non-shell calls.

The legacy `block_shell_commands: true` remains accepted and maps to
`deny_all_shell`; `false` maps to the default `inspect_known` behavior.

`deny_unresolved` and `deny_all_shell` constrain different calls, so they are
independent strictness flags. Every setting in every loaded file adds its flag:
`access_policy: deny_unresolved` with `block_shell_commands: true`, or a home
file with `deny_unresolved` and a project file with `deny_all_shell`, denies
every shell call *and* every incomplete non-shell analysis. No layer can relax
a stricter setting from another layer.

Every native input is handled through aligned `PreToolUse` and
`ToolAccessAnalyzer`. Structured fields, patch operations, exact native shell
profiles, heuristic literal operands, and literal shell `apply_patch` heredocs
produce one provenance-bearing report. `resolve_targets` then materializes
descendant, glob, and workspace scopes within a 100,000-entry budget while
retaining nonexistent exact write targets. Thus `rm -rf secrets` can match a
`secrets/**` policy when descendants exist.

The inspected tools include Claude Code `Read`, `Write`, `Edit`,
`MultiEdit`, `NotebookEdit`, `Grep`, and `Glob` (a repository-wide `Grep` or
`Glob` covers every descendant), Codex `apply_patch` and shell calls, and the
documented Antigravity file tools such as `view_file`, `write_to_file`,
`replace_file_content`, and `grep_search`.

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
