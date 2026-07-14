# Hook contract catalog

This tree is the versioned protocol ledger for `agent-hook-kit`. JSON Schema
Draft 2020-12 describes JSON values; strict YAML metadata describes identity,
provenance, assurance, handler bindings, process/HTTP channels, and exact cases.

The version axes are independent:

- `format_version` versions this metadata language;
- each immutable `snapshot` versions one interpretation of an upstream contract;
- an immutable command-environment supplement versions the process environment
  that augments the selected event snapshots; and
- Rust crate semver versions the public implementation.

`registry.yaml` selects one snapshot per harness and one command-environment
supplement. Frozen snapshot and supplement evidence is immutable. Implementation
targets and later observations live under `status/` and can evolve without
rewriting protocol history.

Run:

```sh
cargo xtask contracts check
cargo xtask contracts report --check
cargo xtask contracts verify-vendor
```

Checks are offline. Remote URIs in provenance are never fetched during normal
validation. JSON schemas are self-contained or vendored under approved catalog
roots; a network resolver is not enabled in the validator.

Every catalog snapshot is validated on each check, including deterministic
manifest recomputation for historical frozen snapshots. The current v1 metadata
schema remains backward-compatible with the first frozen snapshots; successor
`-r2` snapshots additionally require reproducibility classifications, distinct
representative fixtures, and negative-fixture validation keywords.

Every indexed event has a `contract.yaml`, event-root input schema, each distinct
JSON output schema, and `fixtures.yaml`. Fixtures include distinct `minimal` and
`representative` positive inputs, focused negative inputs that name both the
expected JSON pointer and validation keyword, JSON output examples, optional
`output_negative` exclusivity/regression traps, and exact base64/checksummed
stream cases. Origins are explicit: `official`, `sanitized-live`, `synthesized`,
or `regression`.

Command-process environment contracts live under
`supplements/command-environments/`. Each supplement links to one frozen snapshot
per harness, defines reusable sourced variable profiles, and maps every event in
the linked snapshot to its applicable profiles. An empty event profile list is
an explicit claim that the harness defines no native environment variables for
that hook; inherited ambient process variables are outside this contract.

## Snapshot lifecycle

Populate and review a snapshot while its state is `draft`, then freeze it once
all catalog checks pass:

```sh
cargo xtask contracts freeze <harness> <snapshot-id>
```

Freezing writes a sorted `MANIFEST.sha256` covering all evidence and interpreted
contracts (excluding the snapshot index and manifest itself). Ordinary checks
recompute it and reject any mutation. Upstream changes therefore require a new
snapshot ID; refresh tooling must never overwrite a frozen snapshot.

Command-environment supplements follow the same draft/frozen lifecycle:

```sh
cargo xtask contracts freeze-command-environments <supplement-id>
```

The selected supplement must be frozen and must target every registry-selected
event snapshot. Environment-only corrections create a new supplement revision;
changes to JSON or process-output contracts still create successor event
snapshots.

`status/implementation-gaps.yaml` is the machine-readable exception interface
for Rust conformance. Each exception names one exact harness, snapshot, event,
binding, and stable assertion (`native-input`, `native-output`,
`binding-implemented`, or `conformance-case:<fixture-id>`), plus its rationale,
owner, and removal phase. Unknown assertions and stale entries whose assertion
now passes are rejected.

Until a machine-validated live-observation overlay is added, status validation
rejects `*-stable` and `live-observed` claims rather than deriving them from a
handwritten label.
