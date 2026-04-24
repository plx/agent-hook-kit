# Hookkit Remediation Report

Audit date: 2026-04-11  
Scope: implementation in `crates/*`, `examples/*`, tests, and docs compared against `planning/hookkit-implementation-plan.md` and `planning/hookkit-spec.md`.

Severity rubric:
- `P0`: protocol/correctness bug that can produce invalid harness behavior.
- `P1`: major spec deviation or missing capability needed for phase/acceptance criteria.
- `P2`: important completeness/usability gap.
- `P3`: lower-priority polish/alignment item.

## Remediations

### 1. P0 — Runtime validation is not event-aware, so invalid outputs can be emitted
- Severity: `P0`
- Title: Event-unaware validation allows protocol-invalid output
- Issue:
  - Emission validation only sees output envelopes, not the triggering event (`crates/hookkit-runtime/src/lib.rs:62-120`, `crates/hookkit-runtime/src/validate.rs:8-70`).
  - This misses spec-required checks like Codex `Stop` JSON requirements and event-specific field restrictions (`planning/hookkit-spec.md:654-669`, `688-695`).
  - Current Claude validation only rejects simultaneous top-level `decision` and `permissionDecision`, but still allows top-level `decision` alone for `PreToolUse` (`crates/hookkit-runtime/src/validate.rs:19-30`).
- Fix:
  - Thread event identity (`HookEventKey` or concrete event enum tag) through runtime emission.
  - Replace `validate_{harness}(envelope)` with `validate_output(harness, event, output)`.
  - Add event-specific rules:
    - Codex `Stop`: enforce required JSON shape on success path.
    - Claude `PreToolUse`: reject top-level `decision`/`reason`; require hook-specific permission payload when blocking/asking.
    - Gemini `BeforeToolSelection`: reject unsupported top-level decision controls.
    - Worktree/file/watch-path outputs: validate absolute-path constraints.
  - Fail before writing stdout/stderr payload bytes.
- Validation:
  - Add unit tests for each invalid combination listed above and verify non-zero exit with no stdout JSON emission.
  - Add golden tests for valid event-specific outputs.
  - Add integration tests that feed known invalid handler outputs and assert failure mode.
- Additional notes:
  - This is the highest-risk area because it can silently produce harness-invalid protocol output even when tests are green.

### 2. P1 — `run_common` is still a placeholder with native signatures
- Severity: `P1`
- Title: Cross-harness runtime API not implemented
- Issue:
  - `run_common` still takes/returns native types and delegates to `run_native` (`crates/hookkit-runtime/src/lib.rs:177-186`).
  - This does not satisfy the common-runtime contract (`planning/hookkit-spec.md:637-640`) and blocks phase-2 ergonomics.
- Fix:
  - Implement `run_common(harness, handler: FnOnce(CommonHookInput, &RuntimeContext) -> Result<CommonHookOutput>)`.
  - Add conversion pipeline:
    - native parsed input -> `CommonHookInput` (for mapped events),
    - `CommonHookOutput` -> harness-native output with explicit unsupported-capability errors.
  - Keep unmapped events explicit (e.g., conversion error type or dedicated passthrough strategy).
- Validation:
  - Add runtime tests proving one shared prompt-submit and one shared post-tool handler runs on Claude/Codex/Gemini (`planning/hookkit-implementation-plan.md:186-188`).
  - Update one example to use `run_common` end-to-end.
- Additional notes:
  - Without this, consumers still have to hand-roll native-to-common conversion logic per binary.

### 3. P1 — Common input wrapper mapping is incomplete for declared aligned events
- Severity: `P1`
- Title: Missing wrapper coverage/conversions for aligned lifecycle events
- Issue:
  - `CommonPreCompressInput` only contains Gemini and does not model Claude `PreCompact` (`crates/hookkit-common/src/input.rs:371-374`; expected mapping in `planning/hookkit-spec.md:523`).
  - `From<T> for CommonHookInput` conversions stop at `Stop` and do not include `Notification`, `SessionEnd`, or pre-compress/pre-compact (`crates/hookkit-common/src/input.rs:388-476`).
- Fix:
  - Expand `CommonPreCompressInput` to include `Claude(claude::PreCompact)` and add accessors.
  - Add missing conversion impls for:
    - Claude/Gemini `Notification`,
    - Claude/Gemini `SessionEnd`,
    - Claude `PreCompact` + Gemini `PreCompress`.
  - Add mapping tests for every aligned event pair listed in phase 2.
- Validation:
  - Unit tests: each native aligned event converts into expected `CommonHookInput` variant.
  - Compile-time coverage check (e.g., exhaustive match test helper) to ensure no aligned event is omitted.
- Additional notes:
  - This currently weakens the “shared handler” goal because only a subset is directly bridgeable.

### 4. P1 — Common output layer is partial and currently lossy
- Severity: `P1`
- Title: Common output API drops intent and omits required event families
- Issue:
  - `CommonHookOutput` only includes `Empty`, `PostToolUse`, `PreToolUse`, `PromptSubmit`, `Stop` (`crates/hookkit-common/src/output.rs:14-25`), missing `SessionStart`, `Notification`, `SessionEnd`, `PreCompress` required by spec shape (`planning/hookkit-spec.md:565-575`).
  - `CommonPostToolUseOutput` exposes `user_notice`, `continue_session`, `stop_reason` but conversions silently ignore parts of this intent on some harnesses (`crates/hookkit-common/src/output.rs:37-109`).
- Fix:
  - Add missing common output families and conversion paths.
  - Make conversions explicit and non-lossy:
    - map supported fields,
    - return `UnsupportedCapability` for unsupported fields,
    - never silently drop `user_notice`/control semantics.
  - Add per-harness conversion matrix tests documenting support.
- Validation:
  - Tests asserting unsupported fields produce structured errors (not silent no-op).
  - Tests asserting all supported fields survive conversion for each harness.
- Additional notes:
  - This directly impacts message-audience separation and predictable behavior.

### 5. P1 — `shared-posttool-autofix` example does not implement planned behavior
- Severity: `P1`
- Title: Autofix example is a stub, not an autofix/lint workflow
- Issue:
  - Current implementation only checks tool name and injects one context line (`examples/shared-posttool-autofix/src/main.rs:51-75`).
  - Missing planned behavior: run formatter, run linter/autofix, quiet success, user summary, agent guidance only when needed, diagnostics artifact spill (`planning/hookkit-implementation-plan.md:198-207`).
- Fix:
  - Implement command execution pipeline:
    - detect changed files/tool outputs,
    - run formatter and linter/autofix,
    - classify outcomes (`clean`, `autofixed`, `manual action`).
  - Use `hookkit-runtime::artifacts` for verbose diagnostics.
  - Route messages separately for user vs agent channels.
- Validation:
  - Integration tests for at least:
    - clean success (quiet),
    - autofix performed (concise user summary),
    - manual action required (user summary + artifact path + concise agent guidance).
- Additional notes:
  - This is currently the most visible mismatch between code and plan narrative.

### 6. P1 — Codex deny behavior is inconsistent across library surfaces
- Severity: `P1`
- Title: Codex denial path is split between JSON deny and stderr/exit-2 deny
- Issue:
  - `CodexEnvelope::deny(...)` exists and is golden-tested as JSON (`crates/hookkit-codex/src/output.rs:40-47`; `fixtures/golden/codex_pre_tool_deny.json`).
  - But common conversion and example prefer `BlockingDeny` stderr/exit-2 (`crates/hookkit-common/src/output.rs:145-148`, `examples/codex-bash-guard/src/main.rs:33-36`).
  - Plan explicitly calls for emitting documented Codex block format (`planning/hookkit-implementation-plan.md:210-213`).
- Fix:
  - Choose one canonical deny protocol per Codex docs and enforce it everywhere.
  - If JSON deny is canonical:
    - migrate example/common conversion to `CodexHookOutput::Json(OutputEnvelope::deny(...))`.
  - If stderr/exit-2 is canonical:
    - remove/mark JSON deny builder as unsupported for that event and adjust goldens.
- Validation:
  - Integration tests assert the exact wire format for denial.
  - Golden fixtures align with the selected canonical behavior.
- Additional notes:
  - Mixed semantics create consumer confusion and compatibility risk.

### 7. P2 — Runtime polish items from phase 3 are not complete
- Severity: `P2`
- Title: Missing `--dump-parsed` and runtime key helpers
- Issue:
  - Artifact manager and logging exist, but planned runtime additions for session/turn/tool key helpers and `--dump-parsed` are absent (`planning/hookkit-implementation-plan.md:225-230`).
  - No runtime API or CLI flag currently exposes parsed-event dumping.
- Fix:
  - Add helper accessors on `RuntimeContext` (or companion module) for stable session/turn/tool identifiers.
  - Add optional debug mode that prints parsed native/common structures to stderr in deterministic JSON.
- Validation:
  - Unit tests for key extraction behavior across representative fixtures.
  - Integration test proving `--dump-parsed` emits debug output to stderr only and does not contaminate stdout channel.
- Additional notes:
  - This improves developer ergonomics and troubleshooting during hook development.

### 8. P2 — Docs and quick-start are missing
- Severity: `P2`
- Title: README does not meet release-candidate documentation criteria
- Issue:
  - README currently contains only a one-line description (`README.md:1-2`).
  - Plan/DoD expects documented examples and quick-start guidance for Claude, Codex, Gemini (`planning/hookkit-implementation-plan.md:234-236`, `463-473`).
- Fix:
  - Expand README with:
    - workspace overview,
    - minimal `run_native` quick starts for each harness,
    - `run_common` quick start after remediation #2,
    - fixture and integration-test workflow,
    - example behavior matrix.
- Validation:
  - “Fresh clone” smoke test: follow README commands to run one example per harness successfully.
  - Add docs CI check for command snippets where practical.
- Additional notes:
  - Lack of docs currently hides available capabilities and expected behavior from adopters.

### 9. P2 — Naming/typing alignment drift from spec contract
- Severity: `P2`
- Title: Field naming and path typing diverge from spec examples
- Issue:
  - Parsers read `hookEventName` (`crates/hookkit-claude/src/input.rs:399-401`, `crates/hookkit-codex/src/input.rs:221-223`, `crates/hookkit-gemini/src/input.rs:252-254`) while spec contract text references `hook_event_name` (`planning/hookkit-spec.md:298-309`).
  - `RuntimeContext.cwd` uses `String` (`crates/hookkit-runtime/src/lib.rs:19-24`) while spec shows `Utf8PathBuf` (`planning/hookkit-spec.md:646-651`).
- Fix:
  - Clarify canonical field names in docs and align code accordingly.
  - Prefer backward-compatible parsing aliases where feasible (accept both naming styles, emit canonical style).
  - Move path-bearing fields/context to `Utf8PathBuf` where practical.
- Validation:
  - Add fixture variants for naming aliases and verify successful parse.
  - Add type-level tests ensuring path APIs accept/return UTF-8 path types.
- Additional notes:
  - This is partly a spec/code contract drift issue; resolve in one pass to prevent downstream confusion.

### 10. P3 — Claude output model remains a single generic envelope
- Severity: `P3`
- Title: Claude output typing not yet event-precise
- Issue:
  - Claude output is currently one broad `OutputEnvelope` with optional fields (`crates/hookkit-claude/src/output.rs:20-35`).
  - Spec guidance recommends event-precise modeling and explicitly warns against a flattened internal struct (`planning/hookkit-spec.md:357-368`).
  - This broad envelope contributes to validation complexity and silent invalid combinations.
- Fix:
  - Introduce event-scoped output structs/enums (e.g., `ClaudePreToolUseOutput`, `ClaudeStopOutput`, `ClaudePermissionDeniedOutput`, etc.).
  - Keep ergonomic builders, but route through event-specific types before serialization.
- Validation:
  - Compile-time guarantees for invalid field combinations where possible.
  - Reduced runtime validation surface + targeted unit tests per event output type.
- Additional notes:
  - Lower priority than remediation #1/#2, but improves long-term maintainability and correctness.

## Suggested execution order
1. Remediation 1 (`P0`)  
2. Remediation 2 (`P1`)  
3. Remediations 3 + 4 (`P1`)  
4. Remediation 6 (`P1`)  
5. Remediation 5 (`P1`)  
6. Remediations 7 + 8 + 9 (`P2`)  
7. Remediation 10 (`P3`)
