# Contract maintenance

Upstream drift discovery is non-mutating. The scheduled workflow runs
`scripts/check-upstream-contract-drift.sh`, which reads every source it checks
from the registry-selected snapshots through `cargo xtask contracts
upstream-sources`. It compares vendored schemas with upstream at their pinned
revision and at upstream HEAD, lists pinned source trees that changed at HEAD
for review, and re-hashes documentation URLs. It exits 1 on drift or a
vendored-evidence mismatch and 2 when discovery itself fails, so a broken check
never looks clean, and it never edits a frozen snapshot.

Record a documentation `content_sha256` as the hash of the decoded body that
`curl --fail --silent --show-error --location --compressed <url>` returns,
because that is what the drift script hashes; some servers content-encode
responses that did not ask for it. For a file in a GitHub repository, record
its `https://raw.githubusercontent.com/<owner>/<repo>/<rev>/<path>` URL: a
`github.com/.../blob/...` page is HTML with per-request content, so its hash
never reproduces. `cargo xtask contracts check` rejects a content hash on any
`github.com` URL in a draft, or in a snapshot or supplement retrieved on or
after 2026-09-30; older frozen evidence keeps its URLs, and the drift script
hashes a frozen blob URL through its raw URL.

The selected snapshots and the selected supplement must agree on the hash of
a URL they share. When a later retrieval legitimately records a newer hash,
it must cite the earlier source's full hash in its `limitations` after
reviewing the difference; `contracts check` rejects an uncited or ambiguous
disagreement. The drift script then reports the earlier source as
acknowledged, not drifted, when upstream matches the newer hash, until a
successor snapshot pins it.

Every frozen snapshot and supplement that the registry does not select must
be recorded in `FROZEN_LEDGER` in `xtask/src/main.rs` with the SHA-256 of its
`snapshot.yaml` or `supplement.yaml`. That lifecycle file carries the frozen
marker outside the content manifest, so `contracts check` pins it there to
keep superseded evidence from being demoted to a draft and edited. `contracts
freeze` and `contracts freeze-command-environments` print the entry; add it
when freezing (the check requires it once the registry stops selecting that
snapshot or supplement), and never edit an existing entry.

When drift is confirmed:

1. create a new dated/revisioned draft snapshot directory;
2. refresh source provenance and permitted vendor evidence;
3. run `cargo xtask contracts diff <old> <new>`, which reports the event
   inventory and per-event content changes;
4. classify changes as additive, behavioral, breaking, or documentation-only;
5. update schemas, transport metadata, and provenance-aware fixtures;
6. review and freeze the candidate with `cargo xtask contracts freeze`; only a
   frozen snapshot can be selected. Add the `FROZEN_LEDGER` entry it prints;
7. create a successor command-environment supplement that targets the new
   snapshot (see below), because the selected supplement must target every
   registry-selected snapshot, and select both in `registry.yaml` together;
8. update native types, static descriptors, and exact conformance cases;
9. regenerate the implementation registry and support report; and
10. run `scripts/release-check.sh` plus any safe live-harness matrix.

Live observations belong under `contracts/status/observations/`. They create a new
snapshot only when they change the interpreted protocol contract.

For command-environment-only drift, and for every successor event snapshot,
create a new draft under `contracts/supplements/command-environments/`, update
its source evidence and complete event/profile matrix, freeze it with `cargo
xtask contracts freeze-command-environments <supplement-id>`, then select it in
`registry.yaml`. Do not edit a frozen supplement. If the same drift changes JSON
payload or process-output semantics, follow the full successor snapshot workflow
above as well.
