# ADR 014: Common semantic helpers remain legacy during stabilization

- Status: accepted
- Date: 2026-07-12
- Phase: 4

The existing `CommonHookOutput` intention/lowering API is not promoted as the
stable cross-harness model. Its best-effort branches can redirect or drop effects,
and Antigravity exposes materially different capabilities. It remains a temporary
compatibility surface for the in-repository runner.

New shared execution uses lossless native-arm enums. Input and output arms must
match before emission, and native-only outputs remain directly available. Phase 5
keeps tool classification and its semantic result runner-local; a later ADR may
promote only intentions proven total and faithful by that migration.
