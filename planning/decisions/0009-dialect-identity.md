# ADR 0009: Dialects preserve exact public identity

- Status: accepted
- Date: 2026-07-12
- Phase: 0

## Context

Future harnesses may describe themselves as compatible with an existing protocol
while adding deviations. Silent parser fallback would erase identity and import
unreviewed future changes.

## Decision

Every dialect has a distinct open `HarnessId`, native module/crate, event roots,
and catalog snapshot. Optional `DIALECT_OF`/`wire_heritage` metadata documents
lineage and exact reuse. Deviations are enumerated; parsing never silently falls
back to the parent when dialect validation fails.

## Consequences

Built-in convenience enums coexist with open identifiers. Reuse is an internal
implementation detail and never a support or semantic-compatibility claim.
