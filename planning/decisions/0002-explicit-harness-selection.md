# ADR 0002: Execution requires explicit harness selection

- Status: accepted
- Date: 2026-07-12
- Phase: 0

## Context

Compatible-looking JSON is not reliable evidence of harness identity. Automatic
detection can execute the wrong protocol and emit a harmful response.

## Decision

A concrete typed event implies its harness. Selected-harness and aligned dynamic
execution require the caller to supply a typed harness identifier. Optional event
hints constrain ambiguous resolution and contradictions are errors. Best-effort
cross-harness detection is a separate inspection API and never automatically
executes its result.

## Consequences

Legacy execution entry points that guess or over-infer identity are replaced.
Applications may parse `--harness`, while the library accepts protocol identity
types and provides deterministic ambiguity diagnostics.
