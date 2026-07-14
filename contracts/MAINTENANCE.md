# Contract maintenance

Upstream drift discovery is non-mutating. The scheduled workflow reports current
source revisions and documentation hashes; it never edits a frozen snapshot.

When drift is confirmed:

1. create a new dated/revisioned draft snapshot directory;
2. refresh source provenance and permitted vendor evidence;
3. run `cargo xtask contracts diff <old> <new>`;
4. classify changes as additive, behavioral, breaking, or documentation-only;
5. update schemas, transport metadata, and provenance-aware fixtures;
6. review and freeze the candidate with `cargo xtask contracts freeze`;
7. update native types, static descriptors, and exact conformance cases;
8. regenerate the implementation registry and support report; and
9. run `scripts/release-check.sh` plus any safe live-harness matrix.

Live observations belong under `contracts/status/observations/`. They create a new
snapshot only when they change the interpreted protocol contract.

For command-environment-only drift, create a new draft under
`contracts/supplements/command-environments/`, update its source evidence and
complete event/profile matrix, freeze it with `cargo xtask contracts
freeze-command-environments <supplement-id>`, then select it in `registry.yaml`.
Do not edit a frozen supplement. If the same drift changes JSON payload or
process-output semantics, follow the full successor snapshot workflow above as
well.
