# ADR 017: Command environments use immutable catalog supplements

- Status: accepted
- Date: 2026-07-13

## Context

The first frozen event snapshots describe JSON values and process output but do
not inventory the harness-provided environment of a command hook. Those values
are a cross-cutting part of the command binding: most profiles apply to every
event, while a few are event- or invocation-origin-specific. Editing the frozen
snapshots would destroy their evidentiary meaning. Copying all 56 event
directories into successor snapshots solely to add the same process metadata
would duplicate unrelated schemas and fixtures and would couple implementation
identity churn to an orthogonal contract dimension.

## Decision

Record command-hook environment contracts in an immutable, registry-selected
supplement under `contracts/supplements/command-environments/`. A supplement:

- identifies the exact frozen event snapshot it augments for each harness;
- defines sourced variable profiles, activation and group-completeness rules,
  exact names or prefixes, and basic value relationships;
- maps every event in every linked snapshot to its applicable profiles, using
  an explicit empty list when the official contract defines no native process
  variables; and
- is frozen behind a deterministic manifest. Corrections create a new
  supplement revision rather than modifying frozen evidence.

The contract checker rejects an incomplete harness or event inventory, stale
links from the selected supplement to non-current snapshots, unknown sources or
profiles, duplicate selectors, invalid value relationships, and manifest drift.
The environment supplement and event snapshot are separate version axes; a
change that also alters hook JSON or output semantics still requires a new event
snapshot.

## Consequences

Rust environment models can cite a reviewed, machine-validated contract without
claiming that older frozen event files contained evidence they did not contain.
The registry now selects both the current event snapshot per harness and one
current cross-harness command-environment supplement. Maintainers must update or
supersede the supplement whenever an event inventory or environment contract
changes.
