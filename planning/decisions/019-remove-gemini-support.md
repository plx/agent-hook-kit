# ADR 019: Remove Gemini CLI support

- Status: accepted
- Date: 2026-08-04
- Supersedes: ADR 0008

## Context

HookKit now supports Claude Code, Codex, and Antigravity. Maintaining a fourth
native protocol, contract catalog, fixture matrix, examples, and runner lowering
path for Gemini CLI no longer matches the project's supported-harness scope.

## Decision

Remove Gemini CLI as a built-in harness. Its public identity, native crate,
contract snapshots, command-environment supplement entries, runtime and aligned
arms, shell adapter, examples, fixtures, CLI flags, conformance cases, and
support-report rows are removed.

Historical planning documents remain unchanged as records of the earlier
four-harness design. They do not define the current supported surface.

## Consequences

- The built-in harness set is Claude Code, Codex, and Antigravity.
- Generated contract and conformance totals cover 45 selected events.
- Gemini CLI inputs and flags are rejected as unsupported instead of being
  parsed or lowered through a compatibility path.
- Reintroducing Gemini CLI requires a new scope decision and a fresh,
  source-reviewed native contract implementation.
