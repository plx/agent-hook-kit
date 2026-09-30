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
responses that did not ask for it.

When drift is confirmed:

1. create a new dated/revisioned draft snapshot directory;
2. refresh source provenance and permitted vendor evidence;
3. run `cargo xtask contracts diff <old> <new>`, which reports the event
   inventory and per-event content changes;
4. classify changes as additive, behavioral, breaking, or documentation-only;
5. update schemas, transport metadata, and provenance-aware fixtures;
6. review and freeze the candidate with `cargo xtask contracts freeze`; only a
   frozen snapshot can be selected;
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
