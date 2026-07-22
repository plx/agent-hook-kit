# Deferred Lint/Format Hook Rebuild Punchlist

<!-- markdownlint-disable MD013 -->

- Status: implementation and final validation complete
- Prepared: 2026-07-21
- Reviewed baseline: `f5cc5de168326e742edfbe6beb3d819e45648d5c`
- Target branch: `origin/main`
- Primary crates: `hookkit-tool-runner`, `hookkit-pkl-config`, `hookkit-file-activity`, `hookkit-session-state`
- Primary binaries: `turn-completion-agent-hook`, the file-activity observer, and `session-start-state-agent-hook`

## 1. Purpose and authority

This document is the execution punchlist for rebuilding the Pkl-configured deferred formatter/linter hook around the shared HookKit APIs and the desired Stop-time semantics.

It is intended to be handed to an implementation agent in a fresh workspace. The agent should work through the numbered items in dependency order, run each item's focused validation before moving on, and finish with the full repository validation suite.

The behavioral requirements and safety properties in this document are controlling. Exact illustrative Rust and Pkl type or field names are not frozen. If a better spelling preserves the behavior, crate ownership, native fidelity, and compatibility requirements, use it and update this document or the implementation handoff.

Before editing, read:

- `AGENTS.md`;
- `crates/hookkit-tool-runner/RUNNER_DESIGN.md`;
- `crates/hookkit-file-activity/README.md`;
- `crates/hookkit-session-state/README.md`;
- `planning/tool-runner-pkl-format.md`;
- `planning/shared-api-enhancement-plan.md`; and
- the current implementations in the four primary crates listed above.

Compare the current branch with `origin/main` before starting. Do not assume the reviewed baseline is still current.

## 2. Target behavior

The hook family tracks files that an agent may have touched during a session on a best-effort basis. At the native turn-completion event—Claude/Codex `Stop`, Gemini `AfterAgent`, or the supported Antigravity stop path—the deferred runner processes every pending file covered by configured tools.

For each applicable file/tool check:

1. run a non-mutating check;
2. if the check reports source issues and an automatic remedy is supported, run the remedy once;
3. after all scheduled remedies have run, rerun every check invalidated by those writes; and
4. classify the final file result.

The three normal file categories are:

- **clean**: every applicable check is clean and the runner did not need to modify the file;
- **auto-fixed**: the runner ran an automatic remedy or changed the file, and every applicable final check is clean; and
- **manual-fixes-needed**: at least one applicable final check still reports source issues, including checks that have no automatic remedy.

For a file covered by multiple tools, aggregate with the ordered severity relation:

```text
clean < auto-fixed < manual-fixes-needed
```

Operational failures are not source issues and do not belong in that three-value relation. Missing executables, invalid configuration, spawn errors, tool crashes, and ambiguous failure exits must remain a separate operational-error outcome.

Normal disposition rules:

- clean files are discharged from pending tracking until a later modification;
- auto-fixed files are discharged and included in the current run's auto-fixed result list;
- manual-fix files remain pending for the next Stop attempt even if no new edit event arrives;
- files affected by an operational error remain pending unless explicit configuration chooses another policy;
- observations appended while a Stop run is executing remain outside its sealed window and must not be acknowledged by that run; and
- files not covered by any configured tool are recorded as uncovered/not-applicable, not falsely described as clean.

Deleted or no-longer-existing exact files are not lintable. Record them as not applicable and discharge the covered observation unless an unresolved scoped target or configured coverage policy requires retention.

## 3. User and agent reporting defaults

Reporting is configured through user-overridable MiniJinja templates. Each normal bucket has a user-facing and agent-facing template.

Built-in defaults:

- clean user message: a terse count and list of checked clean files;
- clean agent message: empty;
- auto-fixed user message: a terse count and list of auto-fixed files;
- auto-fixed agent message: a terse count only, without file details;
- manual user message: a terse file count, group/category count, and file list;
- manual agent message: files organized by configured group, with every associated tool-report artifact linked; and
- operational user/agent messages: concise environment/tool failure summaries with links to run artifacts, kept separate from manual source fixes.

Optional master user and master agent templates compose the rendered bucket messages. Master templates receive both the rendered submessages and all raw structured values.

Templates must be able to access artifact paths and artifact contents independently. They should also receive a typed artifact list that associates each report with its tool, files, command phase, and run.

Empty template output means “emit no message for this audience.” Empty buckets are not rendered by default.

## 4. Explicit design decisions

These decisions resolve ambiguities found during review and should not be reopened casually during implementation.

### 4.1 One remediation pass, then authoritative final verification

The runner performs one automatic-remediation pass per dirty workflow. It does not loop to a fixed point. After all remedies, it performs an authoritative verification sweep across every applicable check whose inputs may have been invalidated.

If final verification still reports issues—even issues another remediation pass might fix—the file is manual-fixes-needed for this run. This bounds runtime and avoids oscillation between tools.

### 4.2 Later tools may invalidate earlier clean checks

Tool order alone cannot make an earlier clean verdict authoritative when a later tool writes the same file or workspace. Track write scopes and actual snapshot changes, then rerun checks whose input files or workspace scopes intersect those changes.

Use conservative invalidation when exact affected files cannot be proven. It is better to rerun a check than to report stale cleanliness.

### 4.3 Batch tools may conservatively taint all job inputs

Diagnostic parsing is not mandatory. When a batched or workspace-scoped tool reports issues but cannot identify individual files, apply that tool result to every candidate file in the job. Preserve the job-level provenance so templates and summaries can make that conservatism visible.

Allow a tool to opt into per-file invocation or a future structured diagnostic adapter for greater precision, but do not make either a prerequisite for this rebuild.

Actual byte changes discovered by snapshots should be attributed to the exact changed files, including matching/workspace files that were not original session candidates. Add those changed files to the run result rather than hiding them.

### 4.4 Operational errors are a separate bucket

Do not relabel operational errors as manual lint failures. The default is to block/continue the agent, retain affected files, and render separate operational templates.

`failFast` may stop later tool execution after an operational error, but files skipped because of that stop remain pending and must not be classified as clean.

### 4.5 Selective discharge uses exact-window acknowledgement plus durable retry state

The current entity API acknowledges sealed generations, not arbitrary records inside a generation. Preserve that concurrency property.

The preferred low-risk transaction is:

1. process the exact sealed window;
2. commit all logs and `summary.json`, describing the intended state disposition rather than claiming it has already completed;
3. durably append idempotent retry evidence for manual and operationally unprocessed files into the new active generation;
4. durably record handled baselines for discharged clean and auto-fixed files;
5. acknowledge only the sealed source generations; and
6. emit the native response.

If the process crashes between steps, duplicate retry evidence is acceptable and must deduplicate in the projection; lost manual work is not acceptable. Event keys should make requeue idempotent.

An alternative state design is allowed only if it proves the same crash and concurrency properties with tests.

### 4.6 Handled baselines suppress fallbacks, not direct observations

Mtime reconciliation currently runs before auto-fix, and Git-dirty reconciliation repeatedly sees unchanged dirty files. Record a content-based handled baseline after a clean or auto-fixed result.

Filesystem-mtime and Git-dirty fallback evidence should be suppressed when the current file content/existence fingerprint matches its handled baseline. A direct post-tool activity observation must still create pending work even when content happens to match an old baseline.

Use a deterministic content digest plus existence/type information. Metadata-only fingerprints are not sufficient because timestamp granularity and equal-length rewrites can miss modifications. If a fingerprint cannot be read, do not suppress fallback evidence.

The state is session-scoped, so long-term cross-session invalidation is not required.

### 4.7 Harness lowering remains exact and loss-aware

Do not invent a universal output envelope. Continue constructing the exact native output arm for each harness.

Not every harness can represent separate user and agent messages while allowing a stop. Apply a documented Stop-time lowering policy:

- strict: error when a nonempty configured audience message cannot be represented faithfully;
- best-effort: omit or redirect unsupported audience output without an extra warning; and
- best-effort-with-warnings: omit or redirect and include an appropriate warning in a representable user/diagnostic channel.

Do not block an otherwise successful stop merely to deliver the auto-fixed agent note under a best-effort policy.

### 4.8 File groups are ordered glob groups

Use an ordered list of configurable groups. Each group has a stable id, display name, and include globs. First match wins; unmatched files enter a built-in `other` group.

This is flexible enough to group `.c`, `.h`, `.cc`, `.cpp`, `.hpp`, and related files as one C/C++ category without hard-coding extension equivalence in Rust.

## 5. Target domain model

Introduce a runner-owned domain model before changing native lowering. A representative shape is:

```rust
enum FileStatus {
    Clean,
    AutoFixed,
    ManualFixesNeeded,
}

struct FileResult {
    path: PathBuf,
    display_path: String,
    group_id: String,
    status: FileStatus,
    changed_by_runner: bool,
    reports: Vec<ToolReportRef>,
}

struct ToolReport {
    tool_id: String,
    tool_name: String,
    workflow_id: String,
    candidate_files: Vec<PathBuf>,
    changed_files: Vec<PathBuf>,
    initial_check: CheckOutcome,
    fix_attempted: bool,
    final_check: Option<CheckOutcome>,
    artifact: Option<RunArtifact>,
}

struct OperationalProblem {
    tool_id: Option<String>,
    phase: Option<String>,
    affected_files: Vec<PathBuf>,
    message: String,
    artifact: Option<RunArtifact>,
}

struct DeferredRunResult {
    files: BTreeMap<PathBuf, FileResult>,
    operational_problems: Vec<OperationalProblem>,
    uncovered_files: Vec<PathBuf>,
    not_applicable_files: Vec<PathBuf>,
    coverage_gaps: Vec<CoverageGap>,
    artifacts: Vec<RunArtifact>,
}
```

Keep this policy model in `hookkit-tool-runner`; do not promote formatter/linter semantics into core/common/runtime crates.

## 6. Work items

### Item 0 — Refresh the baseline and record the implementation boundary

Priority: P0 prerequisite

- [x] Fetch/inspect current `origin/main` and compare it with the reviewed baseline.
- [x] Confirm there are no overlapping uncommitted user changes before editing.
- [x] Re-read the current public APIs in `hookkit-tool-access`, `hookkit-file-activity`, `hookkit-session-state`, aligned runtime, and each native Stop output.
- [x] Inventory the current built-in catalog by phase modes, write scopes, workspace grouping, and whether a genuine non-mutating precheck exists.
- [x] Record any upstream changes that invalidate assumptions in this plan.
- [x] Keep the immediate post-tool runner behavior separate from the deferred runner unless an item explicitly migrates it.

Exit criteria:

- The agent can identify which existing code is retained, replaced, or compatibility-wrapped.
- Any material deviation from this plan is documented before implementation begins.

### Item 1 — Add the per-file deferred outcome model

Priority: P0 correctness foundation

Likely files:

- `crates/hookkit-tool-runner/src/lib.rs`, preferably split into focused modules;
- new runner unit-test modules.

Tasks:

- [x] Introduce `FileStatus` with the explicit severity order clean < auto-fixed < manual.
- [x] Add per-file results, per-tool/workflow reports, operational problems, uncovered files, not-applicable files, coverage gaps, and artifact metadata.
- [x] Implement a deterministic aggregation function that joins results from multiple tools and jobs.
- [x] Preserve every report reference when more than one tool reports on the same file.
- [x] Ensure operational failures do not participate in the normal status join.
- [x] Define conservative attribution for batched/workspace results.
- [x] Include exact snapshot-changed files even when they were not original candidates.
- [x] Replace batch-wide boolean-only decision logic in the deferred path.
- [x] Do not change native output lowering yet; test the pure domain layer first.

Required tests:

- [x] every pairwise and multi-value status join;
- [x] deterministic ordering independent of tool/job completion order;
- [x] one file with formatter auto-fix plus manual linter issue becomes manual and keeps both reports;
- [x] one tool report attached to multiple conservative batch candidates;
- [x] operational failure alongside successful file results remains separate;
- [x] changed non-candidate workspace file is added to the result;
- [x] uncovered and deleted files are not called clean.

Exit criteria:

- The complete desired three-bucket result can be represented before rendering or native lowering.
- No deferred decision depends only on batch-wide `issues`/`operational_failure` booleans.

### Item 2 — Redesign deferred tool workflows around check/fix/final-check

Priority: P0 semantic core

Dependencies: Item 1

Likely files:

- `crates/hookkit-pkl-config/src/schema.rs`;
- `crates/hookkit-pkl-config/src/builtins/Config.pkl`;
- `crates/hookkit-tool-runner/src/lib.rs` or new execution modules;
- Pkl configuration tests.

Tasks:

- [x] Define an explicit Pkl/Rust workflow shape in which non-mutating checks and optional remedies are distinguishable and repeatable.
- [x] Support more than one check/remedy workflow per tool; Ruff must be able to represent formatting and linting checks independently.
- [x] Preserve structured argv tokens, exit-code policy, write behavior, extra args, workspace indicators, and deterministic ordering.
- [x] Provide a compatibility interpretation for existing `phases` configs or a clear, tested migration. Do not silently reinterpret immediate post-tool behavior.
- [x] Run all initial checks without mutation.
- [x] Schedule a remedy only when its workflow's initial check reports source issues.
- [x] Never run a remedy after an operationally failed initial check.
- [x] Run each remedy at most once per Stop attempt.
- [x] Track actual changed files from snapshots of declared write scopes.
- [x] Invalidate prior checks when later remedies intersect their file/workspace inputs.
- [x] Perform the final verification sweep after all remedies.
- [x] Classify final check issues as manual, including workflows with no remedy.
- [x] Treat missing or ambiguous final verification for an auto-fixable workflow as an operational/configuration problem, not clean.
- [x] Retain bounded job parallelism and deterministic output ordering.

Compatibility guidance:

- Existing verifier/check-only phases can seed initial/final checks.
- Existing format/fix phases can seed remedies.
- A tool with multiple semantic capabilities may need more than one explicit check.
- A mutating-only tool must gain a real precheck or explicitly document an unavoidable compatibility fallback; built-in defaults should not rely on “run mutator and inspect diff” when the tool offers a read-only check.

Required hermetic scenarios:

- [x] initially clean: check runs, remedy does not run, result clean;
- [x] dirty and fixable: check reports issues, remedy changes file, final check clean, result auto-fixed;
- [x] dirty and partly fixable: remedy changes file, final check issues, result manual;
- [x] dirty with no remedy: result manual without a mutating command;
- [x] remedy reports clean but changes nothing: classification follows final checks and records the attempted remedy honestly;
- [x] initial check fails operationally: no remedy, affected files retained;
- [x] remedy fails after changing files: changed files recorded, operational problem emitted, affected files retained;
- [x] later tool changes a file previously checked clean: earlier check reruns;
- [x] unrelated later write does not rerun an independent per-file check;
- [x] workspace-scoped write conservatively invalidates workspace checks;
- [x] parallel jobs produce the same ordered result as serial jobs.

Exit criteria:

- The deferred engine implements the three-step procedure directly rather than inferring it from a mutator-first linear phase list.
- A clean file never invokes an available automatic remedy.
- Final cleanliness is authoritative across overlapping tools.

### Item 3 — Implement selective discharge and handled baselines

Priority: P0 persistence correctness

Dependencies: Items 1 and 2

Likely files:

- `crates/hookkit-file-activity/src/lib.rs`;
- `crates/hookkit-session-state/src/entity.rs` only if a missing generic primitive is proven;
- `crates/hookkit-tool-runner/src/lib.rs`;
- file-activity/session-state documentation and tests.

Tasks:

- [x] Design and version the handled-baseline state representation.
- [x] Store normalized path, existence/type, content digest, handled timestamp/run id, and enough metadata to diagnose suppression decisions.
- [x] Add a file-activity API for recording handled clean/auto-fixed baselines.
- [x] Suppress only fallback mtime/VCS observations whose current fingerprint matches the handled baseline.
- [x] Never suppress direct structured, patch, or shell post-tool evidence merely because a baseline matches.
- [x] Add an idempotent API for requeueing exact manual/operational files into the active generation.
- [x] Preserve unresolved scoped targets according to the selected coverage-gap policy; do not collapse them into silently discharged exact files.
- [x] Commit artifacts and summary before changing pending disposition.
- [x] Requeue manual and operationally unprocessed files durably before acknowledging sealed source generations.
- [x] Record handled baselines for clean and auto-fixed files before acknowledgement.
- [x] Acknowledge the sealed window even when some files remain pending via the new retry entries.
- [x] Confirm observations arriving during execution remain pending independently.
- [x] Keep state directories excluded from reconciliation scans.
- [x] Update persisted schema/family/entity versions when compatibility requires it; document why migration is or is not needed.

Crash/concurrency tests:

- [x] clean and manual files in one sealed generation: clean is discharged, manual remains;
- [x] clean and auto-fixed files remain absent on a second Stop with no new edits;
- [x] the runner's own auto-fix mtime does not resurrect the file;
- [x] Git-dirty fallback does not resurrect an unchanged handled dirty file;
- [x] content change with the same length invalidates the baseline and becomes pending;
- [x] a direct post-tool observation requeues a path even when its content matches a prior baseline;
- [x] a concurrent observation written during checking survives acknowledgement;
- [x] duplicate manual retry append is idempotent;
- [x] crash after retry append but before acknowledgement cannot lose the manual file;
- [x] crash after summary commit but before state disposition safely retries;
- [x] operational failure retains only affected/skipped files while already completed clean files discharge;
- [x] retained unresolved scoped target is not lost after partial materialization.

Exit criteria:

- Disposition is per file, not per batch.
- Clean and auto-fixed files remain handled until a later direct modification or changed fallback fingerprint.
- Manual files retry without requiring a fresh edit event.

### Item 4 — Build durable per-tool artifacts and a rich run summary

Priority: P1 reporting foundation

Dependencies: Items 1 and 2

Likely files:

- `crates/hookkit-tool-runner/src/lib.rs` or new report/artifact modules;
- `crates/hookkit-session-state/src/lib.rs` only if `RunBundle` needs a small generic extension.

Tasks:

- [x] Continue using a unique `RunBundle` and commit `summary.json` last.
- [x] Write complete output for every check/remedy/final-check workflow, not only failing tools.
- [x] Give artifacts stable run-relative names that include deterministic tool/workflow/job identity.
- [x] Record absolute path, run-relative path, media type, tool/workflow identity, candidate files, changed files, classification, and contents in the in-memory template context.
- [x] Make one artifact reference reusable by multiple file results.
- [x] Preserve multiple artifacts for a file reported by multiple tools.
- [x] Enrich `summary.json` with all file buckets, groups, reports, operational problems, uncovered/not-applicable files, coverage gaps, state disposition, and rendered message metadata.
- [x] Because `summary.json` is committed before acknowledgement for crash safety, record the planned disposition there; do not write a false `acknowledged: true`. If confirmed disposition must be inspectable, write a separate post-transition receipt.
- [x] Do not put unbounded tool output directly into a default agent message; keep full output in artifacts.
- [x] Make command rendering in artifacts unambiguous enough for debugging arguments with spaces or control characters.

Required tests:

- [x] multiple tools on one file produce multiple distinct artifact references;
- [x] one batched artifact can be linked from multiple conservative file results;
- [x] summary is absent until commit and complete afterward;
- [x] artifact contents exposed to templates match the bytes written to disk;
- [x] failed and successful phases both remain inspectable;
- [x] paths cannot escape the run bundle.

Exit criteria:

- The agent can navigate from a file result to every relevant report without manually reverse-engineering tool indices.
- Templates have both artifact listings and contents as separate values.

### Item 5 — Add groups and configurable bucket/master templates to Pkl

Priority: P1 user configuration

Dependencies: Items 1 and 4

Likely files:

- `crates/hookkit-pkl-config/src/schema.rs`;
- `crates/hookkit-pkl-config/src/builtins/Config.pkl`;
- `crates/hookkit-pkl-config/src/merge.rs`;
- Pkl discovery/merge/builtin tests;
- runner template-rendering modules.

Tasks:

- [x] Add an ordered file-group configuration with id, display name, and include globs.
- [x] Supply useful built-in groups, including one C/C++ group covering headers and implementation extensions, plus a final `other` fallback.
- [x] Add template pairs for clean, auto-fixed, manual, and operational results.
- [x] Add optional/default master user and master agent templates.
- [x] Provide the requested built-in default wording and plural handling.
- [x] Render bucket templates only for nonempty buckets unless the user explicitly asks otherwise.
- [x] Pass rendered bucket strings plus raw structured buckets into master templates.
- [x] Expose at least the template context described in Section 7.
- [x] Compile/validate configured templates before executing external tools when practical, so syntax errors do not occur after mutations.
- [x] Treat template rendering failures as operational configuration errors with a durable artifact.
- [x] Define field-preserving nested merge behavior so a local override of one audience/bucket does not reset unrelated templates to built-in defaults.
- [x] Add explicit reset semantics if Pkl's emitted defaults make omission indistinguishable from reset.
- [x] Retain the current config discovery order and explicit `--config` behavior.
- [x] Document old per-tool immediate-runner messages separately; do not overload them with deferred bucket meaning.

Suggested Pkl shape:

```pkl
class TemplatePair {
  user: String
  agent: String
}

class FileGroup {
  id: String
  displayName: String
  include: Listing<String>
}

class DeferredReporting {
  groups: Listing<FileGroup>
  clean: TemplatePair
  autoFixed: TemplatePair
  manualFixesNeeded: TemplatePair
  operationalError: TemplatePair
  masterUser: String
  masterAgent: String
}
```

Exact nesting under `settings` or another top-level field is an implementation choice, but layered overrides must remain ergonomic.

Required tests:

- [x] built-in defaults render the specified clean, auto-fixed, and manual messages;
- [x] clean agent output is empty;
- [x] C/C++ header and implementation files land in one group;
- [x] first matching custom group wins;
- [x] fallback group catches unmatched extensions;
- [x] manual agent output groups files and links every associated artifact;
- [x] multiple reports for one file all render;
- [x] master templates receive raw buckets and rendered submessages;
- [x] artifact path list and artifact contents can be used independently;
- [x] overriding only `manualFixesNeeded.agent` preserves every other inherited template;
- [x] empty user or agent template suppresses that audience cleanly;
- [x] invalid template syntax fails before any remedy command runs.

Exit criteria:

- All requested messages can be produced entirely from Pkl-configured MiniJinja templates.
- Defaults work without user configuration.

### Item 6 — Lower rendered results through exact native Stop outputs

Priority: P1 harness behavior

Dependencies: Items 1, 4, and 5

Likely files:

- `crates/hookkit-tool-runner/src/lib.rs` or a new lowering module;
- native/aligned fixture tests only when existing public output constructors are insufficient.

Tasks:

- [x] Create a harness-capability matrix for user-only and agent-facing output on allowed and blocked completion.
- [x] Apply the configured Stop-time lowering policy to every nonempty rendered audience message.
- [x] Clean/auto-fixed with no blocking condition must allow completion.
- [x] Manual and default operational outcomes must emit the native continue-working/block signal.
- [x] User messages should use the exact native user/system channel when available.
- [x] Manual agent instructions should use the exact blocking reason/context channel.
- [x] Do not block successful completion solely because an allowed-stop agent message is unrepresentable under a best-effort policy.
- [x] Keep protocol stdout pure; diagnostic logging must use contract-supported stderr/emission behavior.
- [x] Preserve Antigravity limitations explicitly instead of inventing fields.
- [x] Remove the current hard-coded Stop strings after defaults are supplied through templates.

Required per-harness cases:

- [x] no pending work;
- [x] clean-only;
- [x] auto-fixed-only;
- [x] mixed clean and auto-fixed;
- [x] manual result;
- [x] operational error;
- [x] strict failure for an unrepresentable audience;
- [x] best-effort omission/redirect;
- [x] best-effort-with-warnings behavior;
- [x] empty agent template;
- [x] multiple rendered buckets through a master template.

Exit criteria:

- Every supported harness emits schema-valid native output.
- Audience loss is explicit and policy-controlled.

### Item 7 — Promote and consolidate the file-activity observer

Priority: P1 product completeness

Dependencies: Item 3

Current issue:

The Stop runner is shipped by `hookkit-tool-runner`, but the producer it depends on is currently `examples/session-modified-file-tracker`. The deferred product should not require users to discover and install a separate example package.

Tasks:

- [x] Add a shipped quiet file-activity observer binary alongside the Stop and session-start binaries, or provide an equally simple bundled subcommand design.
- [x] Use aligned `PostToolUse` and `hookkit-file-activity::observe_post_tool`; do not copy structured/patch/shell extraction into the binary.
- [x] Preserve exact native no-op output for Claude, Codex, and Gemini.
- [x] Preserve current Antigravity limitations and document mtime-only fallback behavior where applicable.
- [x] Standardize CLI harness/config/state flags across the three binaries while retaining reasonable compatibility aliases for the example invocation.
- [x] Convert the existing example into a thin usage demonstration or retire it with a documented migration.
- [x] Document the three required hook bindings: session start when available, post-tool activity, and turn completion.
- [x] Ensure every component uses the same state-root resolution.
- [x] Remove or migrate the immediate runner's older private path-discovery duplication if it remains part of the supported product path.

Required tests:

- [x] stdin/stdout observer fixture for every supported post-tool harness;
- [x] direct structured writer, patch, shell write, read-only tool, malformed/dynamic gap;
- [x] all three binaries coordinate through one explicit `--state-dir`;
- [x] observer remains quiet while persisting evidence and gaps.

Exit criteria:

- A released installation contains every executable needed for deferred checking.
- The preferred tracking path uses the shared tool-access API end to end.

### Item 8 — Migrate and audit the complete built-in catalog

Priority: P1/P2 catalog correctness

Dependencies: Items 2 and 5

Scope: every Pkl file under `crates/hookkit-pkl-config/src/builtins/tools/`

Tasks:

- [x] Migrate all built-ins to the final workflow schema or verify their compatibility translation.
- [x] For every formatter/fixer, add a genuine non-mutating precheck whenever the external tool supports one.
- [x] Specifically resolve the six reviewed mutating-only built-ins: `go-fmt`, `gofumpt`, `goimports`, `golines`, `gomod-tidy`, and `yq`.
- [x] Give combined tools such as Ruff distinct checks for every mutation capability; a lint-only check must not stand in for format cleanliness.
- [x] Review whether remedy order should change when fixes can invalidate formatting.
- [x] Declare invocation granularity or conservative batch behavior for each tool.
- [x] Verify exit-code policies separately for checks and remedies.
- [x] Verify write scopes cover every file an automatic remedy may change without making snapshots unnecessarily workspace-wide.
- [x] Preserve workspace indicators and structured argv behavior.
- [x] Add a catalog validation test that rejects auto-fix workflows without authoritative final checks unless explicitly marked as an unavoidable compatibility fallback.
- [x] Add/refresh representative fake-executable tests by workflow family.
- [x] Keep real-tool compatibility tests opt-in and version-controlled as documented.

Catalog audit output:

- [x] Record a machine-readable or Markdown inventory of each built-in's checks, remedies, scopes, granularity, and known precision limitations.
- [x] Do not claim full support for a built-in whose precheck/final-check semantics are unverified.

Exit criteria:

- Every enabled built-in has honest check/fix/final-check semantics.
- The catalog-wide validator prevents regression to mutator-first-only behavior.

### Item 9 — End-to-end regression matrix, documentation, and cleanup

Priority: P0 release gate

Dependencies: Items 1 through 8

Tasks:

- [x] Replace the three narrow Stop integration tests with a broader hermetic matrix while retaining their original coverage.
- [x] Add multi-file mixed-category coverage in one sealed generation.
- [x] Add one file covered by multiple tools, including multiple report artifacts.
- [x] Add a later fixer invalidating an earlier clean checker.
- [x] Add selective discharge followed by a second Stop.
- [x] Add manual retry with no new activity event, followed by a user fix and discharge.
- [x] Add auto-fix self-write and Git-dirty handled-baseline regressions.
- [x] Add observations arriving concurrently during Stop.
- [x] Add operational failure affecting only part of a batch.
- [x] Add uncovered, deleted, unresolved, and traversal-truncated candidates.
- [x] Add group and all template/master-template defaults and overrides.
- [x] Exercise exact output for Claude, Codex, Gemini, and supported Antigravity behavior.
- [x] Update root README, runner design, Pkl format documentation, file-activity/session-state docs, and example/setup instructions.
- [x] Remove stale statements that clean/auto-corrected batches are always quiet or that manual findings retain the entire source window.
- [x] Document best-effort tracking limitations and conservative batch attribution.
- [x] Document persisted-state/config migration behavior.
- [x] Split the current 2,900-line runner module into coherent modules if the rebuild would otherwise make it harder to review and maintain.
- [x] Remove dead compatibility code only after migration tests prove it is no longer needed.

Performance/robustness checks:

- [x] clean files do not spawn remedy commands;
- [x] unchanged handled files do not repeatedly hash or run tools beyond the intended reconciliation/check boundary without justification;
- [x] final verification reruns only invalidated checks where exact invalidation is available;
- [x] job concurrency remains bounded;
- [x] large diagnostic output remains in artifacts rather than default agent context;
- [x] recursive walks remain bounded and report truncation.

Exit criteria:

- All target semantics in Sections 2 and 3 are covered through executable tests.
- Public documentation describes the implemented behavior rather than the previous batch-wide model.
- No known P0/P1 item in this punchlist remains unresolved.

## 7. Required template context contract

The final names may vary, but templates must receive equivalent information with stable documented types.

At minimum expose:

```text
run
  id
  project_root
  summary_path
  state_directory

counts
  clean
  auto_fixed
  manual_fixes_needed
  operational_errors
  groups

files
  all checked FileResult objects

clean_files
auto_fixed_files
manual_fix_files
uncovered_files
not_applicable_files

groups
  ordered group objects with id, display_name, files, and count

artifacts
  typed artifact objects with paths, contents, tool/workflow ids, and files

artifact_paths
  a path-only list suitable for terse rendering

artifact_contents
  a path-keyed mapping or equivalent contents-only structure

operational_problems
coverage_gaps

rendered_buckets
  clean.user / clean.agent
  auto_fixed.user / auto_fixed.agent
  manual_fixes_needed.user / manual_fixes_needed.agent
  operational_error.user / operational_error.agent
```

Each file object should include absolute path, project-relative/display path when available, group identity, final status, changed-by-runner state, and associated reports/artifacts.

Default templates should prefer project-relative paths but must safely represent files outside the project root.

## 8. Coverage-gap policy decision gate

Best-effort tracking does not mean silently claiming complete coverage. Before completing Item 3, choose and document the default behavior for unresolved scoped targets, traversal truncation, and recorded analyzer gaps.

Recommended default:

- process every resolved existing file;
- retain/requeue unresolved scoped targets that may still contain unchecked files;
- expose gaps and truncation in `summary.json` and template context;
- warn the user without labeling resolved clean files as manual; and
- block only when a configurable coverage policy requests strict completeness.

Whatever policy is selected must have explicit Pkl configuration and end-to-end tests.

## 9. Compatibility requirements

- Preserve Rust 1.85 compatibility.
- Preserve exact native wire formats and explicit harness selection.
- Preserve Pkl discovery order and embedded builtin imports.
- Preserve existing immediate post-tool behavior unless its migration is intentional, documented, and tested.
- Prefer additive Pkl schema evolution. When behavior must change, provide a clear error or migration rather than silently changing meaning.
- Version persisted state when an older reader could misinterpret new events or projections.
- Do not put runner policy in `hookkit-core`, native crates, `hookkit-common`, or `hookkit-runtime`.
- Keep `hookkit-tool-access` stateless and phase-agnostic.
- Keep post-tool evidence, fallback reconciliation, pending windows, and handled fallback baselines in `hookkit-file-activity` unless a documented crate-ownership review proves otherwise.
- Keep generic concurrency, journals, entities, and run bundles in `hookkit-session-state`.

## 10. Focused validation ladder

Run narrow tests after each item. Before declaring the entire punchlist complete, run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
cargo run -p hookkit-conformance -- --check
cargo xtask contracts check
cargo xtask contracts report --check
cargo xtask contracts verify-vendor
git diff --check
```

Also run the opt-in real-tool compatibility lane in a controlled environment when practical. Report it as skipped when tool versions are not controlled; do not describe an ignored lane as passing.

The highest-risk manual/executable probes are:

1. one sealed window with clean, auto-fixed, and manual files together;
2. one file covered by a formatter and two linters, with multiple artifact links;
3. a clean file proving no remedy command ran;
4. an auto-fixed file followed by a second Stop proving it was not rediscovered;
5. an unchanged Git-dirty handled file proving fallback suppression;
6. a manual file retrying without a new edit and discharging after a fix;
7. a concurrent post-tool observation surviving Stop acknowledgement; and
8. every harness's manual blocking output plus allowed clean/auto-fixed output.

## 11. Suggested implementation/commit sequence

Keep commits or PR review units coherent even if all work occurs on one branch:

1. domain model and pure aggregation tests;
2. workflow schema and staged execution engine;
3. selective discharge and handled-baseline state;
4. artifacts and enriched summary;
5. groups, templates, and config merge behavior;
6. exact native Stop lowering;
7. bundled observer and setup migration;
8. built-in catalog migration/audit; and
9. end-to-end matrix, documentation, and cleanup.

After each unit, record:

```text
Item:
Outcome:
Public API/config/state changes:
Compatibility decision:
Focused validation:
Known gaps:
Commit/PR:
Next item readiness:
```

Do not mark an item complete merely because it compiles. Check every task and exit criterion, and leave unchecked boxes for genuine remaining work.

## 12. Completion definition

The rebuild is complete when:

- direct/shared API-based hooks track session file activity on a documented best-effort basis;
- Stop uses check → conditional remedy → authoritative final check;
- each checked file lands in clean, auto-fixed, or manual-fixes-needed with worst-wins aggregation;
- operational failures remain separate;
- clean and auto-fixed files are discharged until later modification;
- manual and operationally unprocessed files remain pending without retaining unrelated clean work;
- mtime and Git-dirty fallbacks do not resurrect unchanged handled files;
- groups and bucket/master MiniJinja templates are user-overridable with useful defaults;
- templates can access both artifact paths and contents, including multiple tool reports per file;
- exact native lowering is tested for every supported harness;
- the complete required hook suite ships together and is documented; and
- the full validation suite is green, with environment-dependent skips reported accurately.
