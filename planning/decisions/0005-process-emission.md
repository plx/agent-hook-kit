# ADR 0005: Native outputs lower to checked process emission

- Status: accepted
- Date: 2026-07-12
- Phase: 0; representation finalized in Phase 2

## Context

JSON values cannot represent exact stdout/stderr framing and exit behavior.
Runtime diagnostics can also contaminate protocol streams.

## Decision

Core owns an encoded `ProcessEmission` representation with private fields for an
exact contract, stdout payload kind, stderr bytes, and exit code. Native protocol
authors create it through checked contract-aware constructors; typed application
handlers cannot return it directly. Runtime reads once, invokes once, validates
dynamic arm identity, writes each protocol stream once, and exits exactly.
Diagnostics use a separate configured sink.

## Consequences

Exact-byte tests cover empty/text/binary streams and framing. JSON is structurally
validated unless canonical bytes are part of the upstream protocol.
