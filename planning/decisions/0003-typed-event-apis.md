# ADR 0003: Exact events own typed execution contracts

- Status: accepted
- Date: 2026-07-12
- Phase: 0; API spelling finalized in Phase 2

## Context

Umbrella input/output enums make callers manually re-establish relationships the
type system already knows for a concrete hook.

## Decision

Native crates expose event-oriented modules and implement a core `EventSpec`
relationship associating exact identity, input, command output, catalog contract,
decode, and encode behavior. Exact execution is generic over that event. Separate
APIs cover selected-harness dynamic dispatch and aligned multi-harness events.
Dynamic enums remain checked conveniences, not the foundation of exact hooks.

## Consequences

Normal concrete hooks neither detect harness/event nor return unrelated output
types. Phase 2 adds compile-fail or equivalent coverage for invalid pairings.
