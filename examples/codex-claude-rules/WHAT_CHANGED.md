# What changed

This example originally predated HookKit's bounded Bash analysis, semantic
file-access inference, and higher-level session-state primitives. The refactor
keeps the hook's purpose the same—inject a path-scoped Claude Code rule the
first time Codex exposes a matching path—but delegates more of the generic hook
plumbing to the library.

## Shared analyzer/resolver migration

The follow-up shared API pass removed the remaining example-local structured
field walk, patch parser, shell candidate flattening, heuristic argv scan,
deduplication, and slash/path normalization. `ToolAccessAnalyzer` now supplies
loss-aware candidates and `resolve_targets` materializes scoped targets within
an explicit budget. This closes the directory-search/descendant-rule gap while
preserving native cwd as the operand basis. The `ClaimSet` family and version
remain unchanged.

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

HookKit intentionally has a bounded command-semantics table. The example opts
into HookKit's `LiteralPathOperands` fallback, which preserves argument
provenance, heuristic certainty, cwd uncertainty, and the original unresolved
record. `sed` is now covered by the shared built-in command table.

## Structured and patch paths

Structured fields and `apply_patch` add, update, delete, move, and unified-diff
headers are now collected by `hookkit-tool-access`.

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
