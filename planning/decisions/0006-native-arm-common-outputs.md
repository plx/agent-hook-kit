# ADR 0006: Common wrappers retain native arms

- Status: accepted
- Date: 2026-07-12
- Phase: 0

## Context

The legacy common output model normalizes early and can drop audience, context,
rewrite, stop, and permission semantics.

## Decision

Each justified aligned event has non-exhaustive input and output enums whose arms
contain complete native values. Common accessors expose only facts that genuinely
align and preserve absence/multiple workspace roots. Runtime verifies the output
arm matches the triggering input arm before emission. Native-only capabilities
remain reachable through matching or accessors.

## Consequences

There is no generic lowering envelope. Unsupported effects are typed errors, and
recursive path/tool inference remains downstream in concrete consumers.
