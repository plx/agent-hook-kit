# ADR 0001: Versioned contract catalog is authoritative

- Status: accepted
- Date: 2026-07-12
- Phase: 0; format details finalized in Phase 1A

## Context

Protocol models were implemented from incomplete or stale recollection. JSON
Schema alone cannot express process streams, zero bytes, exit status, or handler
binding differences.

## Decision

`contracts/` is the repository's presumptive source of truth for interpreted
upstream contracts. Draft 2020-12 schemas describe JSON values and validated YAML
describes transport, provenance, assurance, identification, and support. Metadata
format version, immutable snapshot ID, and crate semver are independent axes.
Snapshots freeze behind deterministic manifests/checksums; corrections create a
new revision. CI resolves all references offline and rejects frozen mutations.

Phase 1A may settle exact field spelling after four difficult vertical cases, but
must preserve these semantics.

## Consequences

Rust implementation cannot outrun a reviewed contract. Catalog tooling becomes a
required offline PR lane and must be maintained as production-quality protocol
infrastructure rather than handwritten documentation.
