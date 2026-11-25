# Forbidden file guard

`forbidden-file-guard` is one aligned executable with three native pre-tool
modes:

```bash
forbidden-file-guard --harness=claude
forbidden-file-guard --harness=codex
forbidden-file-guard --harness=antigravity
```

`--harness=claude-code`, HookKit's canonical identity, is accepted as an alias
for `claude`.

On Claude Code the same command also guards `PreSkillShellCommand` (Claude
Code v2.1.332 or later): the injected commands, written as `` !`<command>` ``
or in a ` ```! ` block, that a skill or a command such as `/security-review`
runs while it loads. `PreToolUse` does not fire for them, so register the
guard on both events; the payload's `hook_event_name` tells them apart.

## Responses

A call that touches a forbidden path gets the harness's native deny, with a
reason Claude or Codex reads. Every other call gets a *pass-through*
(`hookkit_common::PreToolUseOutput::pass_through`, or
`PreSkillShellCommandOutput::no_op` for an injected command), never an
explicit allow:

| Harness and event | Forbidden path | No objection |
| :- | :- | :- |
| Claude Code `PreToolUse` | `permissionDecision: "deny"` | `{}`: the normal permission flow decides |
| Claude Code `PreSkillShellCommand` | `permissionDecision: "deny"` | `{}`: the command runs unless another hook blocks it |
| Codex | `permissionDecision: "deny"` | empty stdout: the normal approval flow decides |
| Antigravity | `{"decision":"deny"}` | `{"decision":"ask"}` |

An explicit Claude Code `allow` would skip the permission prompt for every
call the guard does not deny, turning a deny-list into an auto-approver. A
`PreSkillShellCommand` hook cannot approve an injected command at all, so its
deny (`PreSkillShellCommandOutput::deny`) names that event and its no-op is
the only other answer. A blocked injected command does not run; the skill
still loads, and Claude reads the reason in place of the command's output.

Antigravity requires a decision on every response. Its current reference
documents `allow` as a pass-through to the next hook or the default policy
and `auto_approve` as the approval, but it does not date that change, and
builds that predate it auto-approve every call answered `allow`. The guard
therefore answers `ask`, which approves nothing on any build: it respects
"Always Allow" settings and cached grants, and can prompt for a call
Antigravity would otherwise have run silently, which is the least-privilege
trade-off.

The guard runs with `RunOptions::fail_closed()`. If it cannot decide (stdin
is unreadable, the payload or hook environment is invalid, the handler fails
or panics, or the response cannot be emitted), it blocks the call: exit 2 with
the diagnostic on stderr for Claude Code and Codex, which on
`PreSkillShellCommand` keeps the injected command from running, and a `deny`
decision for Antigravity. A policy file that fails to load is also a deny. A
Claude Code payload for an event the guard does not guard is a handler
failure, which blocks only where that event's exit 2 blocks.

Argument errors, such as `--harness=claud` or `--confg`, block the call too:
a guard that cannot read its own command line cannot decide. The usage error
becomes one `hookkit:` stderr line and the guard exits 2, which Claude Code
and Codex treat as a block, or answers `deny` when `--harness` names
Antigravity. A non-blocking exit 1 would let every call through while the hook
is misconfigured. `--help` and `--version` exit 0.

Only the harness can block when the guard binary is missing, cannot start,
times out, or aborts. On Claude Code (v2.1.295 or later), register the guard
in exec form (`args` set, so no shell runs it) with `"onFailure": "block"`
on both events, which makes such a failure do what exit 2 does:

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "/absolute/path/to/forbidden-file-guard",
            "args": ["--harness=claude"],
            "onFailure": "block"
          }
        ]
      }
    ],
    "PreSkillShellCommand": [
      {
        "matcher": "Bash|PowerShell",
        "hooks": [
          {
            "type": "command",
            "command": "/absolute/path/to/forbidden-file-guard",
            "args": ["--harness=claude"],
            "onFailure": "block"
          }
        ]
      }
    ]
  }
}
```

The `PreToolUse` group has no matcher, so it sees every tool call. Define the
`PreSkillShellCommand` hook in a settings file or a plugin: a skill's own
frontmatter hooks do not run for its first load in a session, for loads in a
subagent, or for a forked skill. Claude Code v2.1.331 and earlier ignore the
event and run injected commands unguarded, and they open each interactive
session with an `Unknown hook event` warning when a settings file names it.
In auto mode, an injected command that would need approval does not run while
the skill loads; Claude can run it afterward as a tool call, which the
`PreToolUse` registration guards. Codex and Antigravity have no such event:
register the guard on their `PreToolUse` only, and do not set `onFailure` on
a Codex hook.

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

The policy roots are the stable project roots, then the native working
directory, then the checkout enclosing it: Claude Code's `CLAUDE_PROJECT_DIR`,
its current `cwd`, and the nearest ancestor of `cwd` with a `.git` directory
or file; Codex's `cwd` and the checkout enclosing it, since from Codex 0.166.0
a Bash command reports its `workdir` as `cwd`; and Antigravity's workspace
paths. Claude Code's `cwd` follows `cd` and worktree switches, so after
`cd src` or `cd /tmp` the guard still discovers the project policy and matches
`secrets/**` relative to the project root. `CLAUDE_PROJECT_DIR` stays at the main checkout when Claude Code
enters a git worktree, so the worktree's root (its `.git` file) is a root as
well, and `secrets/**` still covers the worktree's `secrets/` after a `cd src`
inside it.

`access_policy` selects one of three postures:

- `inspect_known` matches recovered candidates and allows analysis or resolver gaps;
- `deny_unresolved` also denies any incomplete analysis or materialization; and
- `deny_all_shell` denies native shell calls (Claude Code `Bash`,
  `PowerShell`, and `Monitor`, and every injected command, Codex `Bash`, and
  Antigravity `run_command`) while continuing to inspect non-shell calls.

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

An injected command (`PreSkillShellCommand`) is a shell command run by the
`Bash` tool or the `PowerShell` tool its `tool_name` names, in the session
shell's current directory that the hook `cwd` reports. The guard inspects it
exactly as it inspects a `PreToolUse` call of that tool, through the
`hookkit-shell` Claude `Bash` profile, so one policy gives one answer on both
events. Its `tool_use_id` names no tool call in the transcript, so the
analyzed call carries no tool-call id.

For example, after building the binary, the checked-in Antigravity fixture
produces a native deny response:

```bash
target/debug/forbidden-file-guard \
  --harness=antigravity \
  --config examples/forbidden-file-guard/forbidden-files.example.yaml \
  < examples/forbidden-file-guard/fixtures/antigravity_pre_tool_use.json
```

The checked-in Claude Code fixture, an injected `cat .env`, is denied the
same way. Claude Code payloads need the hook environment, and its session ID
must match the payload's `session_id`:

```bash
CLAUDECODE=1 CLAUDE_CODE_CHILD_SESSION=1 \
  CLAUDE_CODE_SESSION_ID=claude-guard-example CLAUDE_PROJECT_DIR=/repo \
  target/debug/forbidden-file-guard \
  --harness=claude \
  --config examples/forbidden-file-guard/forbidden-files.example.yaml \
  < examples/forbidden-file-guard/fixtures/claude_pre_skill_shell_command.json
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

Two shells are only partly visible. Claude Code's `PowerShell` commands,
`PowerShell` injected commands included, are not parsed at all:
`deny_all_shell` denies them and `deny_unresolved` denies them as unresolved,
but under `inspect_known` a command such as `Get-Content .env` goes through.
A `Monitor` command is parsed as Bash, so the same `Get-Content` there is an
unknown command that only `deny_unresolved` and `deny_all_shell` deny. Codex runs a shell command in its `workdir` argument, which the
`Bash` hook payload omits, so a relative operand such as `token.txt` may name
`secrets/token.txt`. The guard resolves it against the hook's `cwd` anyway,
also matches it against every root-relative pattern anchored anywhere (so
`app/secrets/token.txt` matches `secrets/**`), and counts it as unresolved:
`deny_unresolved` denies such a call, while `inspect_known` cannot see a
workdir inside the forbidden directory.
