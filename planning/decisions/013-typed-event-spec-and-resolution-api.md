# ADR 013: Typed event specifications and resolution API

- Status: accepted
- Date: 2026-07-12
- Phase: 2

## Decision

An exact command event implements `hookkit_core::EventSpec`, which associates one
input and one command-output type with exact harness, event, category, and catalog
contract identities. The typed runtime accepts the event specification as a type
parameter, parses without harness or event detection, and materializes one exact
`ProcessEmission` before writing either stream.

Dynamic resolution is separate. It requires an explicit open `HarnessId`, accepts
an optional `EventId` hint, records resolution provenance, and rejects an
authoritative discriminator/hint contradiction. A cross-harness detector returns
inspection candidates only and has no execution API.

Native crates expose static `NativeEventDescriptor` values. The non-published
conformance crate is the source of the generated implementation registry consumed
by catalog parity and support reporting.

## Consequences

- Returning one event's output from another typed event is a compile error.
- JSON, plain text, empty streams, stderr protocol messages, and exit status remain
  distinct through emission.
- Antigravity events with identical shapes need external event knowledge or a hint.
- Dialect lineage is metadata and never enables implicit parser fallback.
- Existing dynamic APIs remain temporary compatibility paths until Phase 4.
