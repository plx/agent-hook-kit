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

## Item 3 — Implement selective discharge and handled baselines

Outcome:

- Turn completion now commits its run summary, appends retry evidence for only manual, operationally incomplete, and unresolved work, records handled fingerprints for discharged clean/auto-fixed/not-applicable paths, and then acknowledges the sealed source generations.
- Retry ids are deterministic and deduplicate in the pending projection. A fresh retry record is still appended outside every sealed window so a crash or acknowledgement cannot consume the only remaining copy.
- Mtime and opt-in Git-dirty reconciliation suppress unchanged handled fingerprints. Direct observations bypass the baseline, and equal-length content changes are detected by SHA-256.
- Exact deleted/non-file targets become not applicable, while partially materialized and unresolved scoped targets remain durable coverage gaps.
- Added explicit `best-effort` (default) and `strict` Pkl coverage-gap policies. Both retain gaps; strict additionally blocks Stop.

Public API/config/state changes:

- `hookkit-file-activity` adds `FileActivityRetry`, handled fingerprint/baseline types, handled-baseline inspection/recording, exact/scoped/gap requeue APIs, suppression counts, and not-applicable resolution output.
- `FileActivitySettings.coverageGapPolicy` accepts `best-effort` or `strict`.
- The `pending-files` entity is version 2 because its event and aggregate schema now includes retries. `handled-baselines` is a new version-1 monotonic entity; the reconciliation cursor and family remain version 1.
- Batch summaries replace the misleading completed-state `acknowledged` boolean with `plannedSourceAcknowledgement` because the summary is durably committed before state disposition.

Compatibility decision:

- There is no migration from pending entity v1: session coordination state is transient, old readers cannot safely interpret the new retry variant, and fallback reconciliation can recover best-effort candidates after upgrade. Restarting the agent session is the documented choice when exact continuity is required.
- Handled keys canonicalize the nearest existing ancestor, preserving missing suffixes and file type while collapsing platform aliases such as macOS `/var` and `/private/var`.
- Not-applicable exact paths also receive missing-state baselines so Git-dirty deletion evidence does not immediately resurrect a discharged deletion.
- Best-effort coverage gaps are already retained and exposed in `summary.json`; user-facing gap rendering is completed with the bucket templates and native lowering in Items 5 and 6.

Focused validation:

- `cargo test -p hookkit-pkl-config --all-targets` (29 passed with Pkl available)
- `cargo test -p hookkit-file-activity --all-targets` (17 passed)
- `cargo test -p hookkit-tool-runner --all-targets` (27 unit tests passed; real-tool lane ignored by design)
- `cargo test -p hookkit-runtime --test integration` (39 passed)
- `cargo clippy -p hookkit-file-activity -p hookkit-pkl-config -p hookkit-tool-runner -p hookkit-runtime --all-targets --all-features -- -D warnings`
- `cargo fmt --all -- --check`
- `git diff --check`

Known gaps:

- Durable command output is still combined per tool. Item 4 creates one artifact per command and enriches the summary with the completed state-disposition details.
- Best-effort coverage-gap user warnings await the configurable reporting templates and exact native lowering in Items 5 and 6; gaps are already structured, summarized, and retained.

Commit/PR:

- This item is committed as the selective-discharge and handled-baseline review unit.

Next item readiness:

- Ready for Item 4. The transaction boundary and per-file disposition sets are now stable inputs to richer artifact and summary records.

## Item 4 — Build durable per-tool artifacts and a rich run summary

Outcome:

- Replaced combined per-tool logs with one complete artifact for every executed initial check, remedy, and final check, including successful commands and operational failures.
- Stable artifact ids and run-relative paths encode deterministic tool, workflow, job, and command-phase identity. User-controlled ids are sanitized as path components and `RunBundle` continues to reject absolute or parent-traversal paths.
- Each artifact records structured program/arguments, working directory, exit code, classification, candidate files, changed files, full contents, and its exact report id. Report references update in place, so batched files reuse one artifact while overlapping tools retain distinct artifacts.
- Expanded `summary.json` with run identity, counts, normal buckets, groups, artifact paths, a separate path-to-contents map, the complete result/report/problem/gap model, planned state disposition, and current rendered-message metadata.

Public API/config/state changes:

- Exported `ArtifactClassification` and enriched `RunArtifact` with report, command, classification, candidate/change, and working-directory metadata.
- `BatchToolSummary.log` is replaced by an ordered artifact-path list.
- `BatchRunSummary` is versioned as schema 1 and no longer exposes a completed-state acknowledgement boolean; the nested state disposition explicitly says `acknowledge-sealed-window` is planned.
- No Pkl or persisted-state schema changes.

Compatibility decision:

- Immediate PostToolUse diagnostics retain their existing combined formatting and behavior. The per-command artifact layout is confined to deferred turn completion.
- Human-readable command lines remain for compatibility, while every deferred artifact adds JSON-encoded argv so spaces, empty values, quoting, and control characters are unambiguous.
- A post-transition receipt is not written: exact-generation acknowledgement is already recoverable, while the crash-safe summary truthfully records only the planned transition.

Focused validation:

- `cargo test -p hookkit-session-state --all-targets` (18 passed)
- `cargo test -p hookkit-tool-runner --all-targets` (28 unit tests passed; real-tool lane ignored by design)
- `cargo test -p hookkit-runtime --test integration` (41 passed)
- `cargo clippy -p hookkit-session-state -p hookkit-tool-runner -p hookkit-runtime --all-targets --all-features -- -D warnings`
- `cargo fmt --all -- --check`
- `git diff --check`

Known gaps:

- All current file results still use the built-in `other` group and rendered messages still reflect the legacy fixed Stop text. Item 5 adds configured groups and bucket/master templates over the now-complete context.

Commit/PR:

- This item is committed as the per-command-artifact and rich-summary review unit.

Next item readiness:

- Ready for Item 5. Templates can consume typed artifacts, artifact paths, and artifact contents without reopening files or reverse-engineering numeric tool indices.

## Item 5 — Add groups and configurable bucket/master templates to Pkl

Outcome:

- Added ordered first-match file groups with built-in C/C++, Rust, Python, JavaScript/TypeScript, documentation, and final `other` coverage. Default output uses project-relative display paths where possible.
- Added independently configurable user/agent templates for clean, auto-fixed, manual, and operational buckets plus master templates over raw and rendered bucket views. Empty audiences suppress cleanly and empty buckets render only when explicitly enabled.
- Compiled all group globs and MiniJinja templates before tool execution. Runtime rendering failures preserve completed command artifacts, add a durable reporting configuration artifact, block completion, and retry affected files.
- Exposed typed run, count, file, bucket, group, report, artifact, problem, and gap context, including independent artifact path and path-to-content views.

Public API/config/state changes:

- `settings.deferredReporting` adds `groups`, four template pairs, `masterUser`, `masterAgent`, and `renderEmptyBuckets`.
- Nested reporting patches are field-preserving across discovered configuration layers. `merge.resetDeferredReporting` restores the built-in reporting block before the current file's overrides.
- `summary.json.renderedMessages` now contains the configured bucket and master renderings; its lowering marker is `pending-item-6` until native lowering consumes these values.
- No persisted coordination-state versions changed.

Compatibility decision:

- Existing `ToolSpec.messages` continue to control only the immediate PostToolUse runner. Deferred reporting has a separate namespace and does not reinterpret per-tool message fields.
- Configuration discovery order and explicit `--config` bypass behavior are unchanged.
- Syntax failures prevent every external command, while failures that genuinely arise only during rendering are represented as operational configuration failures without discarding earlier results.

Focused validation:

- `cargo test -p hookkit-pkl-config --all-targets` (31 passed with Pkl available)
- `cargo test -p hookkit-tool-runner --all-targets` (34 unit tests passed; real-tool lane ignored by design)
- `cargo test -p hookkit-runtime --test integration` (43 passed)
- `cargo clippy -p hookkit-pkl-config -p hookkit-tool-runner -p hookkit-runtime --all-targets --all-features -- -D warnings`
- `cargo fmt --all -- --check`
- `git diff --check`

Known gaps:

- Native Stop responses still use the legacy fixed blocking text and do not yet emit allowed clean/auto-fixed reporting. Item 6 applies the rendered master audiences with explicit harness capability policy and removes those hard-coded strings.

Commit/PR:

- This item is ready to commit as the groups, templates, and merge-behavior review unit.

Next item readiness:

- Ready for Item 6. Configured master messages, bucket metadata, and durable reporting errors are available to exact native lowering.

## Item 6 — Lower rendered results through exact native Stop outputs

Outcome:

- Added an explicit capability matrix and runner-local lowering plan for Claude Stop, Codex Stop, Gemini AfterAgent, and Antigravity Stop. Every output remains an exact native arm.
- Configured user and agent master messages now drive both allowed and blocked completion. Clean/auto-fixed runs allow Stop; manual, operational, and strict coverage failures use each harness's native block/deny/continue signal.
- `strict` records the unrepresentable audience, commits the summary, fails the hook, and leaves pending state untouched. `best-effort` omits unavailable audiences; `best-effort-with-warnings` emits a native-channel warning without converting allowed completion into a block.
- Removed the legacy fixed blocking strings. Only configuration/reporting failure fallback text remains for cases where configured templates cannot be loaded or rendered.

Public API/config/state changes:

- `summary.json.renderedMessages.lowering` now records policy, blocked state, per-audience emitted/omitted/empty/unrepresentable status, native channel, warnings, and any strict error.
- No native, aligned, Pkl, or persisted-state public schema changed. The existing `settings.loweringPolicy` now applies to deferred Stop audiences as documented.

Compatibility decision:

- Claude uses `systemMessage` plus `hookSpecificOutput.additionalContext` while allowed, and `systemMessage` plus blocking `reason`/context while blocked.
- Codex and Gemini use `systemMessage` while allowed and cannot faithfully deliver an allowed agent message; blocked agent messages use `reason`.
- Antigravity exposes only `decision` and one optional `reason`. It cannot preserve a separate user audience; warning fallback may use `reason`, and the loss remains explicit in the summary.
- Lowering is planned before summary construction, but strict failure is returned only after summary commit and before state disposition. This preserves diagnostics without falsely discharging work.

Focused validation:

- `cargo test -p hookkit-tool-runner --all-targets` (38 unit tests passed; real-tool lane ignored by design)
- `cargo test -p hookkit-runtime --test integration` (47 passed)
- `cargo clippy -p hookkit-tool-runner -p hookkit-runtime --all-targets --all-features -- -D warnings`
- `cargo fmt --all -- --check`
- `git diff --check`

Known gaps:

- The file-activity producer is still the standalone example and setup/docs do not yet bundle the complete observer/start/Stop suite. Item 7 consolidates that installation surface.

Commit/PR:

- This item is ready to commit as the exact native Stop lowering review unit.

Next item readiness:

- Ready for Item 7. Deferred execution and reporting now produce final native behavior for every supported turn-completion harness.

## Item 7 — Promote and consolidate the file-activity observer

Outcome:

- Added the shipped `file-activity-agent-hook` binary to `hookkit-tool-runner` beside session start and turn completion. It is a quiet aligned PostToolUse observer for Claude, Codex, and Gemini.
- The binary delegates all structured writer, patch, shell-write, read-only, and gap analysis to `hookkit-file-activity::observe_post_tool`; no extraction logic was copied.
- Converted `session-modified-file-tracker` into a thin compatibility wrapper around the shipped library entry point.
- Removed the immediate runner's recursive open-payload discovery implementation. Immediate exact candidates now come from the same shared file-activity/tool-access analysis path.

Public API/config/state changes:

- `hookkit-tool-runner` exports `FileActivityCli`, `parse_file_activity_args`, and `run_file_activity_observer`, and installs the new binary target.
- Observer, session-start, and turn-completion CLIs accept consistent `--claude|--codex|--gemini` and `--state-dir` forms plus `--harness=...` and `--state-dir=...` compatibility aliases. Turn completion continues to accept `--config` and now also accepts `--config=...`.
- No file-activity or session-state schema versions changed.

Compatibility decision:

- The old example executable and its `--harness=claude|codex|gemini` invocation remain available, but new installations bind `file-activity-agent-hook`.
- Exact native no-op responses remain `{}` for all three supported post-tool harnesses; evidence and gaps are persisted without stdout/stderr chatter.
- Antigravity remains Stop-only because its PostToolUse payload lacks the tool call and arguments. Documentation explicitly describes its mtime-only default observation path and optional Git-dirty fallback.

Focused validation:

- `cargo test -p hookkit-file-activity --all-targets` (17 passed)
- `cargo test -p hookkit-tool-runner --all-targets` (35 unit tests passed; real-tool lane ignored by design)
- `cargo test -p session-modified-file-tracker --all-targets` (build and zero-test compatibility target passed)
- `cargo test -p hookkit-runtime --test integration` (49 passed)
- `cargo clippy -p hookkit-tool-runner -p hookkit-runtime -p hookkit-file-activity -p hookkit-session-state -p session-modified-file-tracker --all-targets --all-features -- -D warnings`
- `cargo fmt --all -- --check`
- `git diff --check`

Known gaps:

- The embedded catalog still contains legacy phase-only entries, including mutating-only tools whose deferred compatibility translation cannot prove a clean final state. Item 8 migrates and audits the full catalog.

Commit/PR:

- This item is ready to commit as the bundled observer and shared-analysis migration review unit.

Next item readiness:

- Ready for Item 8. The complete deferred executable suite is now shipped and its producer/consumer state contract is exercised end to end.

## Item 8 — Migrate and audit the complete built-in catalog

Outcome:

- Validated all 134 embedded tool specs: 122 enabled entries now comprise seven explicit deferred-workflow entries and 115 structurally verified compatibility translations; twelve unfinished drafts remain disabled and explicitly claim no support.
- Added genuine read-only checks for `go-fmt`, `gofumpt`, `goimports`, and `golines` using stdout issue classification, for `gomod-tidy` using `go mod tidy -diff`, and for yq using a per-file POSIX shell comparator that preserves executable overrides and extra argv.
- Split Ruff into ordered lint and format workflows. Lint remedies precede format remedies when both are initially dirty; bounded final verification reports a format issue for manual follow-up if a lint fix dirties a workflow that was initially format-clean.
- Added a deterministic generated Markdown audit of every built-in's commands, scope, invocation granularity, and precision limitation. Its drift test evaluates the embedded Pkl catalog rather than duplicating hand-maintained metadata.

Public API/config/state changes:

- `WorkflowCommand.issuesOnStdout` upgrades a clean exit with non-whitespace stdout to source issues, supporting non-mutating list/dry-run modes that retain exit zero.
- The new `ToolExecutable` argv token passes the configured tool executable through structured command rendering; yq uses it inside its shell comparator.
- `ToolSpec.unverifiedRemedyFallback` is an explicit catalog escape hatch requiring a nonempty limitation for an unavoidable mutator-first compatibility entry. No enabled built-in uses it.
- `validate_builtin_catalog` checks tool identity, order references, authoritative checks, read/write scopes, and disjoint exit-code policies. `builtin_specs` applies it automatically.
- No persisted state or native wire schema changed.

Compatibility decision:

- Immediate PostToolUse continues to execute every built-in's legacy `phases`; explicit `workflows` affect only deferred turn completion.
- Existing formatter/fixer entries with a read-only final phase retain compatibility translation. The checked audit calls that translation and real-tool version dependence out rather than claiming cross-version semantic verification.
- Batch remains the conservative default. `gomod-tidy` is explicitly workspace-scoped, yq is per-file, and Ruff plus the stdout-list Go formatters are target-file batches.

Focused validation:

- `cargo test -p hookkit-pkl-config --all-targets` (34 passed with Pkl available)
- `cargo test -p hookkit-tool-runner --all-targets` (37 unit tests passed; controlled-version real-tool lane intentionally ignored)
- `cargo test -p hookkit-runtime --test integration` (49 passed)
- `cargo clippy -p hookkit-pkl-config -p hookkit-tool-runner --all-targets --all-features -- -D warnings`
- `cargo fmt --all -- --check`
- `git diff --check`

Known gaps:

- The controlled-version real-tool compatibility lane was not run; it remains opt-in and was reported as ignored rather than passing. The generated audit records version and classification ambiguities, notably `go mod tidy -diff` exit 1 and golines' archived upstream.
- Item 9 still owns the broader cross-harness regression matrix, final documentation sweep, and module cleanup assessment.

Commit/PR:

- This item is ready to commit as the built-in workflow migration and catalog-audit review unit.

Next item readiness:

- Ready for Item 9. Every enabled built-in has an authoritative deferred check or a validated compatibility pairing, and catalog regressions fail before execution.

## Item 9 — End-to-end regression matrix, documentation, and cleanup

Outcome:

- Expanded the hermetic turn-completion matrix to 56 integration tests while retaining the prior Stop cases. One sealed generation now proves clean, auto-fixed, and manual outcomes together; separate cases prove multi-tool artifacts, invalidation, selective retry/discharge, partial operational failure, and every exact native output arm.
- Added an actual post-seal concurrency probe: a fake check blocks after the pending view is sealed, the file-activity producer appends another observation, and a second Stop proves that observation survived the first acknowledgement.
- Added end-to-end handled-baseline coverage for an unchanged Git-dirty file, plus uncovered, deleted, unresolved-scope, and traversal-budget cases. The Git-dirty case proves no second summary/tool run occurs; scoped truncation remains summarized and requeued.
- Corrected the fake Ruff formatter to honor `format --check` without mutation. Added 128 KiB diagnostic output coverage proving native user/agent channels remain concise while complete bytes stay in artifacts.
- Added custom group, bucket-template, and master-template integration coverage. Existing pure reporting tests continue to cover every default bucket, first-match grouping, artifact views, and audience suppression.
- Updated the root setup/config/migration guide, runner design, file-activity/session-state docs, and compatibility example. Removed obsolete whole-window retention and quiet-success descriptions.

Public API/config/state changes:

- None. This item adds regression coverage and documentation around the APIs/config/state formats introduced in Items 1–8.

Compatibility decision:

- The deferred policy remains split into runner-owned model, execution, reporting, and lowering modules. The main module retains shared CLI, conversion, state-transaction, artifact, and immediate-runner orchestration; a further mechanical split would increase cross-module coupling without isolating another policy boundary.
- Legacy `phases` compatibility remains live for 115 validated builtin entries and is therefore not dead code. Immediate PostToolUse also still consumes it. Removal is intentionally deferred until catalog migration makes it genuinely unused.
- Pending entity v1 is not migrated in place: v2 uses a fresh transient subtree, with restart recommended when exact upgrade continuity matters. Existing phase-only configs retain immediate behavior and receive validated Stop compatibility translation when a verifier exists.

Focused validation:

- `cargo test -p hookkit-runtime --test integration` (56 passed)
- focused runner execution/reporting/lowering and file-activity/session-state concurrency/truncation tests from the prior items remain part of the workspace suite
- `cargo clippy -p hookkit-runtime -p hookkit-tool-runner -p hookkit-file-activity -p hookkit-session-state --all-targets --all-features -- -D warnings`
- `cargo fmt --all -- --check`
- `git diff --check`
- The final repository ladder also passed: workspace fmt, Clippy, and all-target tests; conformance check; contract check/report/vendor verification; and diff whitespace validation.

Known gaps:

- The controlled-version real-tool compatibility lane remains intentionally ignored and was not run. No performance latency guarantee is claimed; the release gate verifies bounded concurrency/walks and context size behavior, not timing targets.

Commit/PR:

- This item is ready to commit as the end-to-end release matrix, documentation, and cleanup review unit.

Next item readiness:

- All punchlist items and the full repository validation ladder are complete. Commit this item, push the branch, and open the PR against `main`.
