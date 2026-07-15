# ADR 018: HookKit runtime scope is command bindings

- Status: accepted
- Date: 2026-07-15
- Supersedes: ADR 0010

## Context

The contract ledger records bindings exposed by upstream harnesses so protocol
history and drift remain reviewable. Claude Code's selected snapshot therefore
includes HTTP bindings alongside command bindings. Catalog presence can be
misread as an implementation commitment, however, especially when auditing
which catalogued events still need native vertical slices.

HookKit's current consumers and runtime model are command processes: native
input plus declared environment in, followed by exact stdout, stderr, and exit
status out. HTTP and other handler transports require different invocation,
configuration, response, and operational abstractions.

## Decision

HookKit implementation work in this iteration is limited to the `command`
binding across every harness. HTTP, prompt, agent, MCP, and any other
non-command bindings are out of scope.

The contract ledger continues to inventory non-command bindings when upstream
evidence supports them. The active stabilization target marks those bindings
`unsupported`, rather than `catalog-only`, so they do not appear to be pending
vertical-slice work. A future non-command adapter requires a new explicit scope
decision and a corresponding status-target change.

## Consequences

- Missing-vertical-slice audits and bulk implementation work consider command
  bindings only.
- Generated reports retain inventoried non-command rows but label their release
  target `unsupported`.
- Native event, output, runtime, conformance, and live-verification work does not
  need HTTP or other non-command arms in this iteration.
- Frozen upstream snapshots remain unchanged; implementation policy continues
  to live in the mutable status layer.
