# What changed

This example originally predated HookKit's bounded Bash analysis, semantic
file-access inference, and higher-level session-state primitives. The refactor
keeps the hook's purpose the same—inject a path-scoped Claude Code rule the
first time Codex exposes a matching path—but delegates more of the generic hook
plumbing to the library.

## Shell inspection

The hand-written shell lexer and guessed shell aliases were removed. Codex
`PreToolUse` input now goes through HookKit's exact `Bash` adapter, which reads
the contract-backed `/command` field. `BashAnalyzer` recovers bounded syntax
facts and literal argv, while `FileAccessAnalyzer` classifies paths associated
with known reads, writes, searches, listings, moves, deletions, and
redirections.

This changes shell inspection from "any token that resembles a path" to
command-aware inference. For example, a path-looking `rg` query is no longer
treated as the search root. Quoted paths containing spaces and paths appearing
in separate commands or redirections are also recovered without example-local
shell parsing.

HookKit intentionally has a bounded command-semantics table. When it reports an
unknown command, ambiguous arguments, or indirect evaluation but still
recovers a fully literal argv, the example applies a local path-looking operand
fallback. This preserves useful coverage for commands such as `sed` while
retaining HookKit's parser as the source of shell structure.

## Structured and patch paths

Structured `path`, `paths`, `file_path`, and related fields are still collected
recursively from non-shell tool input. `apply_patch` headers continue to supply
add, update, delete, move, and unified-diff paths.

The structured-field walk was simplified so a string below a recognized path
key is not visited and added twice. All relative structured, patch, and shell
paths are now based on the native tool-call working directory. The
`--project-root` override affects rule discovery and project-relative glob
matching only; it no longer changes the meaning of a relative tool operand.

## Session state

The monotonic `SetJournal<String>` was replaced with a session-scoped
`ClaimSet`. The hook only needs an immediate, atomic first-writer-wins answer
for each canonical rule path; it never consumes loaded-rule event history or a
materialized aggregate. `ClaimSet::try_claim` expresses that requirement with
less machinery and still prevents concurrent hook processes from injecting the
same rule twice.

Changing the state primitive bumped the example's state-family version from 1
to 2, keeping the new claim layout explicitly separate from earlier entity
state.

## Runtime and tests

The executable now uses the current `run_event` spelling instead of the
backward-compatible `run_typed` alias. The example enables HookKit's Codex shell
adapter through its `hookkit-shell` dependency.

Tests now cover:

- structured fields and patch headers;
- quoted shell paths containing spaces;
- multiple commands and output redirection;
- the literal-argv fallback for an unsupported command;
- exact Codex `Bash` recognition rather than guessed aliases;
- semantic separation of a search query from its path operand; and
- one-time session claims.

The refactor was verified with the example unit tests, its runtime integration
test, focused Clippy with warnings denied, formatting and diff checks, and the
full workspace test suite.
