# ADR 0010: Stabilization implements command runtime first

- Status: superseded by [ADR 018](018-command-only-runtime-scope.md)
- Date: 2026-07-12
- Phase: 0; target entries finalized with Phase 1 inventory

## Context

Harnesses document command, HTTP, prompt, agent, and other bindings. Inventoried
body schemas do not by themselves define a safe runtime adapter.

## Decision

The catalog records every documented binding and complete transport facts where
evidence permits. Stabilization implements native input and command/process output
and runtime support only where `stabilization-v1.yaml` selects it. Other bindings
remain catalog-only unless a concrete consumer and complete transport contract
justify an adapter.

## Consequences

Reports separate inventoried, modeled, command-runtime, other-runtime, and live
verification status. Similar JSON bodies never imply adapter support.
