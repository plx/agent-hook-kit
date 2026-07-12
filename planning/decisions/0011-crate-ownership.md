# ADR 0011: Protocol crates remain composable and acyclic

- Status: accepted
- Date: 2026-07-12
- Phase: 0; optional facade reconsidered in Phase 6

## Context

Exact identity and encoded emission must be shared without making native crates
depend on runtime or creating a dependency cycle.

## Decision

`hookkit-core` owns identity, catalog descriptors, raw invocation/context,
event/harness specifications, errors, and encoded process emission. Native crates
depend only on core. `hookkit-common` depends on native crates. Runtime owns I/O
and dispatch adapters and may depend on core/native/common. Examples and the
runner are downstream. Pkl and runner code never become dependencies of protocol
or runtime crates. A non-published `xtask`/conformance crate is tooling, not API.

## Consequences

Existing runtime/native relationships are refactored toward this direction. An
umbrella package is optional Phase 6 packaging work, not an architecture owner.
