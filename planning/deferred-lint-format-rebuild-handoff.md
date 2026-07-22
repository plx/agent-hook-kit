# Deferred lint/format rebuild implementation handoff

<!-- markdownlint-disable MD013 -->

This log records the review boundary after each item in
`deferred-lint-format-rebuild-punchlist.md`.

## Item 0 — Refresh the baseline and record the implementation boundary

Outcome:

- Fetched `origin/main` on 2026-07-21. Both `HEAD` and `origin/main` are the reviewed baseline `f5cc5de168326e742edfbe6beb3d819e45648d5c`; no upstream change invalidates the punchlist.
- The worktree contained no overlapping user changes. The approved punchlist itself was the only untracked file.
- Re-read the public access-analysis, file-activity, session-state entity/run-bundle, aligned turn-completion, and native Stop/AfterAgent APIs plus the required design documents.

Public API/config/state changes:

- None. This item records the boundary only.

Compatibility decision:

- Retain `hookkit-tool-access` target evidence/resolution, `hookkit-session-state` exact-generation acknowledgement, and `RunBundle` as generic lower-level mechanisms.
- Keep the immediate `post-tool-use-agent-hook` behavior separate. Its existing `phases`, per-tool messages, and private discovery are not reinterpreted by the deferred rebuild; Item 7 may migrate only the supported file-observation path.
- Compatibility-translate existing Pkl `phases` into explicit deferred workflows. The new deferred engine, reporting, and lowering replace only the current batch-wide turn-completion policy.
- Add handled baselines and retry evidence in `hookkit-file-activity`; do not move formatter/linter policy into session-state.
- Adopt the recommended default coverage-gap policy in Section 8: process resolved files, retain unresolved scopes, expose and warn about gaps, and block only under an explicit strict policy.

Catalog inventory:

- 134 embedded tool specs define 184 phases: 37 `format`, 31 `fix`, 116 `verify`, and no `check-only` phases.
- Write scopes comprise 58 `target-files`, 7 `matching-globs`, and 3 `workspace` phases. The workspace writers are `gomod-tidy`, `knip`, and `knip-strict`.
- 27 tools use a workspace indicator; every builtin has an explicit `phaseOrder`.
- 61 mutating tools already have at least one read-only verifier, while 67 tools are check-only in practice.
- Six mutating-only builtins have no non-mutating phase: `go-fmt`, `gofumpt`, `goimports`, `golines`, `gomod-tidy`, and `yq`.
- A legacy `verify` is not necessarily an authoritative precheck for every mutation capability. Ruff is the important example: its current lint verifier does not prove format cleanliness.
- Current deferred execution runs the legacy phase list mutator-first and aggregates only tool-batch `issues`/`operational_failure` booleans. It retains or acknowledges the whole sealed window and writes one combined log per tool.

Native Stop capability boundary:

- Claude Stop can represent a user system message and a distinct blocking reason/additional context; it can also emit an allowed user message.
- Codex Stop can represent a user system message and a blocking reason, but has no distinct allowed-stop agent-context field.
- Gemini AfterAgent can represent a user system message and a deny reason; allowed-stop agent feedback is not independently representable by the current exact constructor surface.
- Antigravity Stop has only `decision` plus one optional `reason`, so separate user and agent audiences cannot both be represented.

Focused validation:

- `git fetch origin main`
- baseline/worktree/API/catalog inspection commands documented above

Known gaps:

- None for Item 0. Exact capability-lowering behavior remains an Item 6 implementation decision within the approved strict/best-effort policies.

Commit/PR:

- This item is committed as the baseline/inventory review unit.

Next item readiness:

- Ready for Item 1. The runner-owned per-file model can be added without changing native lowering.

## Item 1 — Add the per-file deferred outcome model

Outcome:

- Added a runner-owned deferred domain module with the explicit `clean < auto-fixed < manual-fixes-needed` join, per-file results, stable report references, operational problems, uncovered/not-applicable files, coverage gaps, and typed artifact metadata.
- Conservative job attribution reuses one report across every candidate, preserves all reports when tools overlap, and adds exact snapshot-changed non-candidates.
- The legacy deferred executor now builds this model and bases its block/allow decision on per-file manual results plus separate operational problems. Native Stop lowering and legacy phase execution order are unchanged for this item.

Public API/config/state changes:

- Exported `FileStatus`, `FileResult`, `FileAssessment`, `CheckOutcome`, `ToolReport`, `ToolReportRef`, `OperationalProblem`, `CoverageGap`, `RunArtifact`, `CommandPhase`, and `DeferredRunResult` from `hookkit-tool-runner`.
- No config or persisted-state change.

Compatibility decision:

- Existing phase-list executions are represented as a temporary `legacy-phases` workflow report. This is an honest compatibility wrapper: it records that an initial check was unavailable and is replaced by the staged workflow engine in Item 2.
- Existing native output strings and wire formats remain unchanged.

Focused validation:

- `cargo test -p hookkit-tool-runner --lib deferred::model::tests`
- `cargo test -p hookkit-tool-runner --all-targets` (real-tool lane remained ignored by design)
- `cargo fmt --all`

Known gaps:

- Legacy deferred phases are still mutator-first. Item 2 replaces their execution semantics with explicit initial check, conditional remedy, and final verification.
- Artifact construction still produces one combined legacy log per tool; Item 4 makes every command phase independently durable.

Commit/PR:

- This item is committed as the domain-model review unit.

Next item readiness:

- Ready for Item 2. The staged engine can emit `ToolReport` values directly without changing native lowering.

## Item 2 — Redesign deferred tool workflows around check/fix/final-check

Outcome:

- Added additive Pkl/Rust `workflows` and `workflowOrder` fields. A workflow has an authoritative non-mutating check, optional remedy, target/workspace invalidation scope, and per-file/batch/workspace invocation granularity.
- Replaced the Stop-time mutator-first loop with a global staged engine: all initial checks run first, only dirty workflows receive one remedy, snapshot-discovered writes invalidate intersecting checks, and final checks run after all remedies.
- Check stages retain bounded parallelism and deterministic result/log order. Remedies remain ordered to avoid concurrent writers.
- Final issues become manual fixes; check/remedy spawn and exit failures remain operational; failed remedies retain exact changed files.

Public API/config/state changes:

- Added `Workflow`, `WorkflowCommand`, `CheckScope`, and `InvocationGranularity` to `hookkit-pkl-config`.
- Added matching `ToolWorkflow`, `CheckScope`, and `InvocationGranularity` runner types.
- No persisted-state change.

Compatibility decision:

- Immediate PostToolUse continues to execute `phases` without reinterpretation.
- Deferred configs with no explicit workflows are compatibility-translated: mutators pair with the last enabled verifier, while read-only tools become check-only workflows.
- A legacy mutating-only workflow is run at most once but reported as operationally unverifiable because it has no authoritative final check. Item 8 migrates all six affected builtins before release.
- Explicit workflow checks that declare writes, remedies with no declared write scope, missing checks, and unknown workflow-order entries fail validation before an external command runs.

Focused validation:

- `cargo test -p hookkit-tool-runner --all-targets` (25 unit tests passed; real-tool lane ignored by design)
- `cargo test -p hookkit-pkl-config --all-targets` (29 tests passed with Pkl available)
- `cargo test -p hookkit-runtime --test integration` (37 tests passed, including the existing three Stop scenarios)
- `cargo clippy -p hookkit-tool-runner -p hookkit-pkl-config --all-targets --all-features -- -D warnings`
- `cargo fmt --all -- --check`
- `git diff --check`

Known gaps:

- Builtins still use the compatibility translation until Item 8, including Ruff's currently insufficient lint-only verifier for formatting.
- Command artifacts are still combined per tool for compatibility; Item 4 writes every check/remedy/final-check independently.

Commit/PR:

- This item is committed as the staged-workflow review unit.

Next item readiness:

- Ready for Item 3. The per-file result now supplies the exact disposition sets needed for retry entries and handled baselines.
