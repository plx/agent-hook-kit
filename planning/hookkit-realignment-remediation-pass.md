# Agent Hook Kit: Contract-First Realignment — Remediation Pass

<!-- markdownlint-disable MD013 MD024 -->

- Status: **RESOLVED** — the remediation (commit `5890c54`) closed all R1–R9 items; the realignment
  is now ready to merge. See §0 for the re-review resolution and the small residual punch-list.
- Prepared: 2026-07-12 (original review); **re-reviewed 2026-07-12** after remediation commit `5890c54`.
- Reviews branch: `plx/contract-first-realignment` (git log shows Phases 0–7 all merged)
- Controlling plan under review: `planning/hookkit-contract-first-realignment-plan.md`
- Review method: whole-tree read + execution probes by the integrator, plus a 15-dimension
  skeptical multi-agent review with adversarial per-finding verification (65 agents; 2 findings
  refuted at the verify stage; 44 confirmed actionable findings, ~20 of them blockers).

---

## 0. Re-review resolution (2026-07-12, commit `5890c54`)

The remediation landed as a single 328-file commit (`5890c54 refactor: complete contract-first
remediation`, +23.5k/−10.3k). A re-review — integrator execution probes plus a 9-dimension
multi-agent pass with adversarial per-finding verification — finds **every R1–R9 item resolved** and
recommends **merge**. Ground truth: `fmt`/`clippy` clean, `contracts check` passes ("validated 56
selected event contracts and all catalog snapshots"), `cargo test` 135 passed / 0 failed / 1 ignored
(the 259→135 drop is deletion of legacy code + its now-obsolete tests, not lost coverage of the new
surface).

Verdict per item (✔ = verified against the new code, several by execution):

| Item | Resolution |
| --- | --- |
| **R1** legacy removal + SessionStart + migration | ✔ `OutputEnvelope`/`run_native`/`run_common`/`HookEventKey`/`RawPayload`/`CommonHook*`/old `Harness` **deleted** (0 live refs, no shim needed). `hookkit-claude/src/output.rs` gone; discriminators are private `&'static str` stamped inside builders. **Ran** `claude-sessionstart-context` → emits `hookEventName:"SessionStart"`. `validate.rs` behavior relocated into `EventSpec::parse` + typed builders + `validate_command_emission` + aligned harness-arm guard. §8.8 `RuntimeContext` rebuilt in `core/context.rs` (workspace_roots collection, no `.`-default, no casing probes, diagnostics sink). All 5 examples + runner on the new API. |
| **R2/R3** report honesty | ✔ Descriptors derive from real `EventSpec` (`NativeEventDescriptor::command::<E>()`); conformance is **execution-gated** (`conformance/src/lib.rs` errors on "declared/executed conformance mismatch"). `support.md` now 13 command-runtime-beta / 71 catalog-only (was 56/28). Target↔registry cross-check added. |
| **R4** native wire bugs | ✔ Codex parses snake_case (**ran** bash-guard → schema-valid `permissionDecision:"deny"`); Codex `block`/`deny` shapes correct; Gemini fictional `shell` contract removed (open `run_shell_command`, string command); Gemini stamps required `hookEventName`; native fixtures + Codex snake_case tests added. |
| **R5** heuristics out of common | ✔ `hookkit-common/src/semantic.rs` deleted; discovery moved into the runner (`discover_modified_files`/`discover_path_candidates`); aligned enums now `#[non_exhaustive]`. |
| **R6** negative-output fixtures | ✔ New superseding `docs-2026-07-12-r2` snapshot adds `output_negative` with `inject-step-mutually-exclusive` + `expected_keyword: oneOf`; the fixture format now expresses negative-output fixtures and `contracts check` proves rejection. |
| **R7** resolution/detector | ✔ `run_harness` (generic) + `dispatch_builtin_harness` (runtime) built; production `identification_descriptors()` + `identification_parity.rs` parity test; 9-rule hint logic with real tests. No `run_auto`. |
| **R8** gap + checker enforcement | ✔ Gap stale-detection ("assertion … is stale because it now passes"); `contracts check` step 9 both directions; §7.10 10-step checklist intact after the +1005-line xtask rewrite. |
| **R9** provenance/packaging | ✔ (mostly) examples now `publish = false`; `RUNNER_DESIGN.md` perf claims corrected. Two residual nits below. |

### Residual punch-list (non-blocking follow-ups)

None of these block merge; log them as small follow-ups:

1. **(major, test-coverage)** The runner's rewritten `lower_report` lowering surface — user/agent
   channel split, Codex/Gemini native arms, `LoweringPolicy` + `harness_block` branches
   (`hookkit-tool-runner/src/lib.rs:846–926`) — has no hermetic test. It is load-bearing R1.6/R5 code;
   add a hermetic test that asserts the emitted per-harness bytes for each branch.
2. **(minor, correctness)** `lower_report` `harness_block` path (`lib.rs:850–865`) returns early with
   only `blocking_error(message)`, **silently dropping** `notices`/`diagnostics`/`agent_feedback`
   accumulated from earlier tools when `continue_after_issue` is set. Fold the accumulated context into
   the block emission (or document the drop).
3. **(minor, coverage)** `hookkit-core` and `hookkit-common` have **0 direct unit tests**; the
   `ProcessEmission` validating-constructor error branches and `RuntimeContext::new` harness-mismatch
   guard are only exercised on happy paths. Add focused negative-branch unit tests.
4. **(minor, docs)** Mode B (`run_harness`/`dispatch_builtin_harness`) has no runnable example, only a
   README snippet — add one so all three execution modes ship an example.
5. **(nit, test)** The SessionStart E2E integration test (`integration.rs:420–433`) asserts context is
   present but not `hookEventName == "SessionStart"` — assert the exact discriminator at the
   shipped-binary boundary (it is asserted in the `protocol.rs` unit test).
6. **(observation, packaging)** `hookkit-pkl-config` and `hookkit-tool-runner` lack `publish = false`
   despite `RELEASE.md` listing only the seven library crates for publication.

The original findings below (§1–§6) are retained as the historical record of what was fixed.

---

## 1. Executive verdict

**Do not merge.** The workspace is fully green on the surface — `cargo fmt --check`, `cargo clippy
--workspace --all-targets`, and `cargo test --workspace` (259 passed, 0 failed, 1 ignored) all pass,
and `cargo run -p xtask -- contracts check` validates 56 event contracts — but the green masks that
**the realignment was implemented as an additive new layer beside the old architecture, not as the
replacement the plan mandates.** The old, protocol-invalid API remains the primary, public,
undeprecated surface; the flagship correctness bug the plan was chartered to eliminate still ships;
the north-star runner and 4 of 5 examples were never migrated; and the "support" apparatus that the
plan designates as *the source of public support claims* reports capabilities that the code does not
implement.

Concretely, the single most damning fact: the plan (§2D, §8.6, and the Phase 2 exit gate) makes it a
controlling requirement that *"a Claude SessionStart handler must not be able to accidentally emit
`hookEventName: PostToolUse`."* Running the shipped `claude-sessionstart-context` example on a
`SessionStart` event today produces:

```
$ echo '{"hook_event_name":"SessionStart",...}' | ./target/debug/claude-sessionstart-context
{"systemMessage":"...","hookSpecificOutput":{"additionalContext":"...","hookEventName":"PostToolUse"}}
```

The exact bug the realignment existed to kill is still reproducible through the primary API, in the
example the plan specifically called out.

### What is genuinely good (and should be preserved)

The new contract-first machinery is real, well-built, and worth keeping. Credit where due:

- **Phase 0 is complete and solid** (the only "ready" dimension). The durable baseline artifact
  (`planning/audits/2026-07-12-baseline.md`) records every 0A item honestly; all 12 blocking ADRs
  (`planning/decisions/0001–0012`) exist and are topic-substantive with later decisions correctly
  deadline-scheduled (013–015); the six 0C test lanes are defined and CI runs lanes 1–3 hermetically.
- **Dependency direction is exactly correct** per §6/§8: `hookkit-core` has no internal deps; native
  crates depend only on core; common on native; runtime on native/common; **no** core/native/common/
  runtime crate depends on `hookkit-pkl-config` or `hookkit-tool-runner`. Tooling crates are
  `publish = false`.
- **The contract catalog is substantial and real**: 56 frozen event contracts across 4 harnesses,
  with recomputable `MANIFEST.sha256` files whose mutation `contracts check` actually rejects
  (verified by tampering), no mutable `current` symlinks, and byte-for-byte Codex vendoring.
- **The registry is machine-generated** from Rust static descriptors with a `--check` staleness gate
  (the *mechanism* satisfies §7.10 even though the descriptor *inputs* are wrong — see R2).
- **The new typed layer is correctly designed**: `typed.rs::run_typed`, event-scoped `protocol.rs`
  types that stamp their own discriminator internally, `ProcessEmission` with private fields and
  real exact-byte conformance tests, `resolution.rs` with genuine hint-semantics tests, and
  `aligned.rs` with real input/output arm-match verification before emission.
- **The Codex fictional version selector is fully removed** (0 repo-wide hits for `CodexVersion`/
  `CodexFeatureSet`/`feature_set`/`protocol_version`) — a Phase 3B key ask, done.
- **Support-level *labels* are honest about maturity** — everything is `command-runtime-beta` /
  `source-reviewed` / live-verified: no; HTTP bindings are `catalog-only`. Nothing over-claims
  "stable" or "live-verified." (The *rows* over-claim; the *labels* do not — see R2.)

The problem is not that the new design is wrong. It is that **the migration to it was never
finished**, and the reporting layer papers over how much remains.

---

## 2. The core structural problem

Six independent review dimensions and the integrator converged on one root cause:

> The realignment **added** `contracts/`, the typed/aligned/resolution modules, the conformance
> crate, and per-event descriptors **on top of** the pre-existing architecture, and then marked
> Phases 3–7 complete — **without removing the old surface, without migrating the consumers, and
> while auto-generating a support matrix from hardcoded "true" literals** that makes the incomplete
> work look finished.

Everything below is downstream of that. The remediation is therefore **finish the replacement**, not
"add more features."

Migration-map §12.1 status (every row was supposed to be Removed/Replaced):

| §12.1 surface | Plan disposition | Actual state |
| --- | --- | --- |
| `hookkit_core::Harness` | Replace with open `HarnessId`+builtin enum | **Still `{Claude,Codex,Gemini}`, no Antigravity arm, primary runtime type** |
| `HookEventKey` | Replace with harness-scoped `EventId` + `AlignedEventKind` | **Still present (~170 refs); `EventId` is not harness-scoped** |
| `RawPayload` | Evolve into `RawInvocation` | **Both exist** |
| Native `OutputEnvelope` | Remove | **Present in all 4 native crates, public, load-bearing emit type** |
| `ClaudeEventOutput` permissive enum | Retain only as checked dynamic enum matching input event | **Present but arm is not checked against input event** |
| `CommonHookOutput`, `LoweringPolicy`, universal lowering | Replace with native-arm enums | **Present, crate-root public, the live runner path** |
| `run_native` | Split into `run_event`/dispatcher | **Still present, undeprecated, primary** |
| `run_common` | Replace with `run_aligned_event::<K>` | **Still present, docstring literally "Placeholder", primary** |
| Runtime `RuntimeContext` casing/path probes | Replace with §8.8 exact context | **Still present with casing probes + `cwd unwrap_or(".")`** |
| Runtime logging to process streams | Replace with diagnostics sink | Partial |

None of the retained surfaces is `#[deprecated]`, none is private, none has a removal milestone — so
none qualifies for the plan's §5.9/§12 "temporary shim" exception.

---

## 3. Remediation work items

Ordered by priority. **P0 = merge-blocking**, **P1 = required before release**, **P2 = quality/debt.**
Each item lists the confirmed findings it resolves, the controlling spec, and concrete evidence.

### R1 (P0) — Finish the migration: remove/replace the legacy protocol-invalid surface

Resolves the SessionStart bug and the §12.1 debt. Findings: SessionStart bug (Phase 2A/2B, Phase 3A),
§12.1 retention (Phase 2A/2B, Holistic), `run_common`/RuntimeContext (Phase 2C/2E, Phase 4, Phase 5),
examples on legacy API (Holistic). Spec: §2D, §5.9, §8.6, §8.8, §12/§12.1, Phase 2/3 exit gates.

Actions:

1. **Eliminate the free-form discriminator path.** `hookkit-claude/src/output.rs` `OutputEnvelope`
   exposes `pub hook_specific_output: Option<serde_json::Value>` plus event-agnostic builders
   (`with_context` → PostToolUse, `worktree_path` → WorktreeCreate, `stop_continue`, `permission_*`,
   …). A `SessionStart` handler can call any of them. Either remove `OutputEnvelope` from the public
   API (route all output through the event-scoped `Claude*Output` types that stamp their own
   discriminator) or make it a `pub(crate)`/`unchecked`-namespaced escape hatch excluded from
   guarantees per §8.6. Do the same for the Codex and Gemini `OutputEnvelope`s.
2. **Fix `examples/claude-sessionstart-context/src/main.rs:26`** to use
   `ClaudeSessionStartOutput` (verify the emitted `hookEventName` is `SessionStart`).
3. **Replace `run_native`/`run_common`** (`hookkit-runtime/src/lib.rs:741,796`) with the typed and
   aligned entry points. `run_common`'s docstring is literally *"Placeholder for the common
   (cross-harness) runtime"* and it lowers through the generic `CommonHookOutput` envelope, which §4B
   forbids. If a shim must survive one release, mark it `#[deprecated(note=…, since=…)]` with a
   removal milestone and stop using it in shipped examples/the runner.
4. **Delete the §8.8-violating legacy `RuntimeContext`** (`lib.rs:36–62,761–765,819–823`): remove the
   `cwd = get("cwd").unwrap_or(".")` default and the `session_id()`/`turn_id()`/`tool_use_id()`
   camelCase-or-snake_case probes. Provide the §8.8 exact context contract (workspace **roots** as a
   collection; typed identifiers only when the native contract supplies them; diagnostics sink).
5. **Retire `hookkit_core::Harness`, `HookEventKey`, `RawPayload`, `CommonHookOutput`,
   `LoweringPolicy`** per §12.1 (or deprecate-with-milestone as above), and give `HarnessId` an
   Antigravity identity.
6. **Migrate the 4 legacy examples and the runner** to `run_typed`/`execute_post_tool_use`.

Exit check: no shipped path can emit a wrong-event discriminator; `grep -R "#\[deprecated\]"` covers
every retained legacy surface; no example/consumer imports `OutputEnvelope`/`run_common`/`CommonHookOutput`.

### R2 (P0) — Make the support report/registry reflect real implementation

The report is designated by §7.10 as *"the source of public support claims."* Today it is a closed,
self-referential loop. Findings: report/registry green-theater (Holistic, Phase 2F, Phase 3A, Phase 3B,
Phase 6), missing §7.10 cross-check (xtask, Phase 2F). Spec: §7.10, §17 DoD bullet 4, Phase 1E, Phase 3
exit gate.

Root mechanism: the `descriptor!` macro (`hookkit-claude/src/protocol.rs:8–20` and peers) hardcodes
`native_input: true, native_output: true, bindings:[Command]`, non-empty `conformance_cases` for
**every** event; `hookkit-conformance/src/main.rs` serializes those literals verbatim into
`registry.json`; `xtask render_report` copies them into `support.md`; and
`validate_implementation_registry` only *requires* those booleans be true — it never checks that any
Rust adapter or executed test backs them.

Actions:

1. **Descriptors must reflect reality.** Derive `native_input`/`native_output`/`command-runtime` from
   the actual presence of an `EventSpec` impl (and its `encode_command`), not from a literal. An event
   with no `EventSpec` must report `native_output: false` / catalog-only.
2. **"Hermetic conformance" must mean executed.** Today the column is
   `binding_implemented && !conformance_cases.is_empty()` (`xtask/src/main.rs:1242`), and only ~5
   events are actually run through `emit`/parse in `hookkit-conformance/tests/vertical_slices.rs`.
   Either wire the conformance runner to execute **every** declared `conformance_case` against the Rust
   type (preferred), or compute the column from cases that a test harness proves it ran. ~51 of 56
   rows currently claim conformance that no test exercises.
3. **Implement the missing §7.10 CI guard (direction b):** reject a stabilization target that claims a
   runtime implementation level not present in the machine-enumerable registry
   (`xtask/src/main.rs:662–709`). Direction (a) is implemented; (b) is absent.
4. **Regenerate `support.md`.** After R1–R3 the honest matrix should show ~9 events with native
   input+output and the remainder as catalog-only / native-input-only.

### R3 (P0) — Deliver, or stop promising, the native events

Findings: ~16% remediation (9/56 events have an `EventSpec`), stabilization target promises
command-runtime for 30 Claude events but only 2 are implemented. Spec: §17 DoD bullet 4, §8.2–8.3,
Phase 3/6 exit gates.

Action: for each event `stabilization-v1.yaml` promises at `command-runtime-*`, either implement the
typed `EventSpec` (input + event-specific output + `encode_command`) and a conformance case, **or**
downgrade it in the target to `catalog-only`/`native-source-reviewed` and encode the catalog-only vs
native split explicitly. A fresh consumer must be able to `run_typed::<E>` for every event the matrix
labels command-runtime (Phase 6 exit gate).

### R4 (P0) — Fix native protocol-correctness bugs that ship in the live builders

These are wire-format defects in the currently-primary output/input path. Findings: Phase 3B (Codex
casing, Codex deny), Phase 3C/3D (Gemini shell contract, Gemini hookEventName, Gemini example), Codex
test gap. Spec: Phase 3B/3C, §12 casing rule, Phase 3 exit gate.

1. **Codex input casing** (`hookkit-codex/src/input.rs:76–84`): event structs use
   `rename_all="camelCase"` with no snake_case aliases, but the cataloged wire is snake_case → real
   payloads yield `tool_name=None` and silently disable the `codex-bash-guard` example. Fix casing and
   **add snake_case parse tests** (current tests only feed camelCase `fixtures/codex/*.json`, masking
   the defect).
2. **Codex deny output** (`hookkit-codex/src/output.rs:41–47`): `OutputEnvelope::deny` emits
   `{"decision":"deny"}`, but the cataloged PreToolUse output schema requires top-level
   `decision: const "block"` with deny expressed via `hookSpecificOutput.permissionDecision:"deny"`.
   The schema-faithful `protocol.rs::PreToolUseOutput::Deny` already exists — remove/redirect the
   invalid convenience builder.
3. **Gemini fictional `shell` contract** (`hookkit-gemini/src/input.rs:42–70`): §3C explicitly forbids
   this. The real tool is `run_shell_command` and `tool_input.command` is a **string**, not
   `Vec<String>`. Keep tool payloads open.
4. **Gemini structured output missing `hookEventName`** (`hookkit-gemini/src/output.rs:49–79`): the
   before-tool/after-tool schemas require the event-specific `hookEventName`; the convenience builders
   omit it → schema-invalid output.
5. **Fix `examples/gemini-beforetool-policy`** accordingly (its policy is gated on the fictional
   `Shell` variant and never fires on real input).

### R5 (P0) — Move the recursive path/tool heuristics out of `hookkit-common`

Findings: Phase 4, Phase 5. Spec: §8.9 (line 990), §5A (line 1516), §8.8, ADR 0006 Consequences
("recursive path/tool inference remains downstream").

`hookkit-common/src/semantic.rs` implements exactly what the plan repeatedly bans from the common
layer: `collect_path_candidates` (313–337) recursively mines arbitrary nested tool JSON for path
fields, `candidate_from_field` probes camelCase **and** snake_case names, and `is_known_file_writing_tool`
bakes tool policy into the shared crate — and this is the **live path the runner consumes**
(`hookkit-tool-runner/src/lib.rs:395`). Move this discovery/policy into the runner. Related: make the
aligned `PostToolUseInput`/`PostToolUseOutput` enums `#[non_exhaustive]` (`aligned.rs:4–10,31–37`,
required by §8.9/ADR 0006).

### R6 (P1) — Fixtures: express and add the mandated negative-output/exclusivity fixtures

Finding: Phase 1 fixtures (blocker). Spec: §7.6 ("Use `oneOf` … include invalid combination
fixtures"), §7.8 minimum set ("mutually exclusive output traps"), Phase 1 freeze gate.

The catalog has exactly one `oneOf` output shape (Antigravity `pre-invocation` injectStep), and its
`fixtures.yaml` has only positive output fixtures; the fixture format has **no way to express a
negative *output* fixture at all** (`xtask/src/main.rs:268–312` only models negative *input*). Extend
the format to carry negative/invalid-combination output fixtures, add the required exclusivity-trap
fixture, and have `contracts check` prove it is rejected at the expected pointer/keyword. This is a
Phase 1 freeze-gate item that was checked off without the artifact.

### R7 (P1) — Build the missing resolution/detector production surface

Findings: Phase 2C/2E (2 majors + a minor). Spec: §8.3 mode B, §8.4 rule 1, §8.5, Phase 2E/2F.

1. **Mode B is unbuilt.** §8.3 requires a compile-time-generic selected-harness executor and a runtime
   `dispatch_builtin_harness`; neither exists (grep for `run_harness`/`dispatch_builtin_harness`/
   `HarnessSpec`/`EventSelector` is empty). `resolve_event`/`detect_candidates` are `pub` but only
   their own unit tests call them, against fabricated in-test registries.
2. **No production identification descriptors + no parity test.** §8.5 requires lightweight native
   identification descriptors derived from the catalog and a parity test proving they match catalog
   identification metadata; the descriptors are constructed only in `#[cfg(test)]`. Build the real
   ones and the parity test (also satisfies the Phase 2F "resolver/detector matches catalog" bullet).
3. **Implement hint rule 1** (`HintHarnessMismatch` before payload analysis) and make `EventId`
   harness-scoped per §8.1/§8.2.

### R8 (P1) — Make the gap mechanism and `contracts check` enforce what the plan requires

Findings: Phase 2F (gap mechanism), xtask (checks 9 and target-cross-check). Spec: §7.10 steps 9 &
637–638, §2F, §1E.

1. **Gap entries are inert.** `validate_implementation_gaps` (`xtask/src/main.rs:1358–1414`) only
   shape-checks entries; the `assertion` field is free-form text linked to nothing. §2F requires "a
   gap whose assertion starts passing must also fail" and §1E requires rejecting "stale entries whose
   expected failure no longer occurs." Link each gap to a real conformance assertion and add
   stale-gap detection.
2. **`contracts check` step 9 half-missing** (`xtask/src/main.rs:868–883`): it errors when a declared
   output schema has non-JSON content, but never errors when a JSON response outcome has **no** output
   schema. Enforce the response direction too.
3. **Implement the target→registry cross-check** (same as R2.3).

### R9 (P2) — Catalog provenance, fixture quality, identity completeness, packaging hygiene

Confirmed minors, batchable:

- **Rubber-stamped provenance**: all 259 assurance blocks are identical `confidence: high /
  verification: source-reviewed`, including the docs-only Antigravity harness whose remote-doc hash is
  unverifiable offline and events with acknowledged ambiguity. Differentiate confidence/verification so
  the field carries signal (§7.7).
- **Codex PreToolUse control schema silently narrows its cited vendored schema** (drops valid
  `permissionDecision:"ask"`/`decision:"approve"`, omits `continue`/`suppressOutput`/`stopReason`
  under `additionalProperties:false`) with no documented deviation — one of the four flagship Phase-1A
  cases. Either widen to match the source or record the deviation + conflict per §7.6/§7.1.
- **8 Gemini events have byte-identical `minimal` and `representative` positive fixtures**, gaming the
  count-only check while adding zero coverage. Supply genuinely-representative fixtures with optional
  fields (§7.8).
- **Negative fixtures assert the JSON pointer but not the validation keyword** (§7.8 asks for both to
  avoid validator-order dependence).
- **§8.1 identity model incomplete**: `BuiltinHarness`, `SnapshotId`, `ContractId`, `AlignedEventKind`
  absent; `EventId` not harness-scoped; contract/snapshot are bare `&str`.
- **`ProcessEmission` omits the mandated contract/binding field**; its public constructors validate
  nothing and are not marked unchecked (§8.7).
- **Four of five example crates lack `publish = false`** (contradicts `RELEASE.md:18`).
- **`RUNNER_DESIGN.md:18–20` claims performance budgets "are measured"/"recorded by the hermetic smoke
  lane," but no timing/benchmark code exists** — Phase 5D was not done. Either add the measurements or
  correct the doc.

---

## 4. Findings the verifier refuted (do NOT action)

The adversarial verify stage overturned two finder claims; they are recorded so a future pass does not
re-raise them:

- "Per-harness runtime behavior is only tested behind `#[ignore]`" — **REFUTED**. The `#[ignore]` is
  the opt-in real-tool lane; hermetic per-harness behavior is tested unconditionally.
- "6B deliverable unmet: examples are not run against contract fixtures in CI" — **REFUTED/invalid** as
  stated (the specific mechanism claimed was inaccurate). Note R2/R3 still cover the substantive
  "examples under-exercise the new API" concern separately.

---

## 5. Suggested remediation sequencing

The items are largely parallelizable, but there is a natural order because R2's honesty depends on
R1/R3/R4 landing first (otherwise the regenerated matrix is still wrong):

1. **R1** (remove/replace legacy surface + fix SessionStart + migrate examples/runner) and **R4**
   (native wire-format bugs) — these are the correctness core and can proceed in parallel per harness.
2. **R3** (implement or downgrade promised events) and **R5** (move heuristics downstream).
3. **R2** + **R8** (make descriptors/report/registry/gaps/checks reflect reality) — do these *after*
   R1/R3/R4 so the regenerated `support.md` is truthful.
4. **R6** (fixture format + exclusivity trap) and **R7** (resolution/detector production surface).
5. **R9** (provenance/quality/packaging cleanup).

Keep each harness's protocol changes in a coherent PR (per §12/§15), and update the runner adapter and
examples in the same PR as the public API they exercise so `main` stays buildable and honest.

## 6. Re-review exit criteria

The realignment is mergeable when:

- No shipped path (typed, aligned, or otherwise public) can emit an output whose discriminator
  contradicts its event; the `claude-sessionstart-context` example emits `hookEventName:"SessionStart"`.
- Every §12.1 legacy surface is removed, or `#[deprecated]` with a removal milestone AND unused by
  examples/the runner.
- `support.md` is regenerated and every "native output / command-runtime / hermetic conformance = yes"
  cell is backed by an `EventSpec` and an *executed* conformance case; the target↔registry cross-check
  and gap stale-detection are enforced in CI.
- Codex parses snake_case and emits schema-valid deny; Gemini drops the fictional `shell` contract and
  emits the required `hookEventName`; both examples work on real input.
- `hookkit-common` contains no recursive path-mining/tool policy; aligned enums are `#[non_exhaustive]`.
- The mandated negative-output/exclusivity fixture exists and `contracts check` proves it rejected.
- The north-star runner runs clean/auto-fixed/manual-action/failure paths end-to-end through the
  **new** public API.

Phase 0's baseline, ADRs, dependency direction, catalog freezing, and the new typed/aligned/ProcessEmission
design do not need rework — they are the foundation the remediation builds on.
