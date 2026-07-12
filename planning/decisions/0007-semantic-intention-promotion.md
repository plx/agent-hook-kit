# ADR 0007: Semantic intentions are runner-local first

- Status: accepted
- Date: 2026-07-12
- Phase: 0; promotion decision deadline is Phase 5

## Context

The existing universal intention layer was designed before native contracts were
fully known and already needs best-effort dropping/redirection policies.

## Decision

No stable common intention model is required for stabilization. The runner first
owns its domain outcome and lowers explicitly to native outputs. Promotion needs
evidence from at least two materially aligned harnesses, total or typed-failing
lowering, no silently discarded native capability, and another real consumer.
Phase 5 records an evidence-backed follow-up ADR for any candidate; promotion is
optional.

## Consequences

Legacy helpers are retained, experimentalized, moved downstream, or removed in
Phase 4. Harness-specific final lowering is expected and acceptable.
