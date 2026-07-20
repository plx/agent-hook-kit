# Agent Hook Kit: Shared API Enhancement Plan

<!-- markdownlint-disable MD013 MD024 -->

- Status: items 01 through 05 complete; item 06 deferred pending demand
- Prepared: 2026-07-19
- Repository baseline: `0f97b3f14416301120d0218c83a77e88ed1e59a6`
- Target branch: `origin/main`
- Scope: shared APIs exposed by the `codex-claude-rules` and `forbidden-file-guard` refactors

## 1. Authority and intended use

This document is the controlling plan for implementing the shared HookKit API work exposed by pull requests #34 and #35. It is designed to be given to an agent in a fresh session as a goal: work through the required items one by one, in the order below, using each linked task document as the task-specific specification.

Where older planning documents conflict with this plan on these APIs, the contract-first principles and accepted architecture in this document control. Existing ADRs and the contract catalog remain authoritative for native protocol facts.

The baseline identifies the code reviewed when this plan was prepared. At the start of execution, compare current `origin/main` with that baseline. If later work has already implemented part of an item, verify it against that item's exit criteria rather than redoing it.

Exact illustrative Rust names in the task documents are not frozen. The required semantics, crate boundaries, safety properties, native fidelity, and exit gates are controlling. Material deviations should be recorded in the implementation handoff and, when architectural or user-visible, in an ADR or an update to this plan.

## 2. Outcome

The target architecture is:

```text
hookkit-core
  lexical path primitives
          |
          +--------------------+
          |                    |
hookkit-common/runtime     hookkit-shell
  aligned PreToolUse         Bash + shell access
          |                    |
          +---------+----------+
                    |
          hookkit-tool-access
  phase-agnostic structured/patch/shell evidence
  bounded general target resolution
                    |
          +---------+------------------+
          |                            |
hookkit-file-activity          pre-tool policy examples
  post-tool write filter       rules injector + file guard

hookkit-session-state
  existing ClaimSet; optional observable metadata only if justified
```

The central architectural decision is to add `hookkit-tool-access` as a stateless layer. `hookkit-file-activity` remains responsible for post-tool modification evidence, persistence, reconciliation, and pending windows. `hookkit-shell` remains responsible for exact shell-call extraction, bounded Bash analysis, and shell-specific inference.

## 3. Source findings

The work is grounded in:

- `examples/codex-claude-rules/FUTURE_REFINEMENTS.md`;
- `examples/forbidden-file-guard/LIBRARY-REFINEMENT-NOTES.md`;
- the implementations of those two examples;
- private structured/patch extraction in `hookkit-file-activity`;
- scoped shell targets in `hookkit-shell`; and
- follow-up PR review cases for native-cwd fixtures, shell-invoked patches, and descendant-scope policy matching.

The notes converge on one missing vertical slice: HookKit can infer rich shell file access and can track post-tool modifications, but it cannot yet provide all file-access candidates for an arbitrary observable tool call independently of phase and persistence.

Two findings are deliberately reframed:

1. `AlignedEventSpec` remains sealed; the library adds the missing aligned `PreToolUse` family itself.
2. Unresolved access remains evidence for a consumer policy. The library should make uncertainty easy to inspect but should not choose allow-versus-deny on the application's behalf.

## 4. Execution rules

An implementation agent should:

1. read the repository `AGENTS.md`, this controlling plan, and the current item's full task document before editing;
2. inspect current `origin/main` and the relevant public APIs instead of assuming the baseline is unchanged;
3. implement only one numbered item at a time;
4. keep each item reviewable as one coherent API change, preferably one commit or one PR;
5. run the item's focused tests before starting the next item;
6. update public documentation and examples in the same item that changes their API;
7. record a short handoff after each item using section 9;
8. continue automatically to the next required item only when the assigned goal asks for the full sequence; otherwise stop at the review boundary; and
9. treat item 06 as gated rather than automatically authorized implementation.

If separate PRs are used, rebase each item on the latest `origin/main` after its dependencies merge. If all items are developed on one branch, preserve a coherent commit boundary per item and re-run the common suite after integration.

Do not rename the workspace branch as part of this plan.

## 5. Work items in execution priority order

Required items should be completed in this dependency-aware order:

1. [x] [Core lexical path primitives](shared-api-enhancements/01-core-path-primitives.md) — P0 foundation. Establish deterministic path normalization, resolution, explicit-home expansion, and UTF-8 slash rendering.
2. [x] [Aligned pre-tool execution](shared-api-enhancements/02-aligned-pre-tool-use.md) — P1 API foundation. Add lossless, explicit-harness aligned `PreToolUse` for all four native harnesses while keeping the event-spec trait sealed.
3. [x] [Phase-agnostic tool access analysis](shared-api-enhancements/03-phase-agnostic-tool-access.md) — P0 central abstraction. Add `hookkit-tool-access` and make `hookkit-file-activity` consume its write-filtered report.
4. [x] [Target resolution and shell completeness](shared-api-enhancements/04-target-resolution-and-shell-completeness.md) — P0 correctness/P1 ergonomics. Generalize scoped-target resolution, add opt-in heuristic shell candidates, publish exact profiles, and parse literal shell patch payloads.
5. [x] [Consumer migrations and regression closure](shared-api-enhancements/05-consumer-migrations-and-regressions.md) — P0 acceptance proof. Migrate both examples, close known correctness cases, and refresh documentation.
6. [x] [Optional observable atomic claims](shared-api-enhancements/06-observable-atomic-claims.md) — Deferred pending demand. No concrete metadata reader or schema currently justifies an implementation.

Items 01 and 02 are independent and may be developed in parallel workspaces if explicitly assigned that way, but the controlling sequential goal should still validate them before item 03. Items 03 through 05 are dependency ordered.

Mark an item's checkbox complete only after its exit criteria and required validation are satisfied and its handoff is recorded.

## 6. Common design requirements

### 6.1 Native fidelity and contracts

- Use the repository contract catalog and native fixtures for protocol facts.
- Keep aligned inputs and outputs as non-exhaustive native-arm enums.
- Require explicit harness selection for aligned/dynamic execution.
- Reject output arms that do not match the selected/input harness.
- Do not introduce a universal serialized output envelope.
- Do not infer aliases or harness identity from coincidental JSON shapes.
- Update support reports only when executable support actually changes.

### 6.2 Analysis honesty

- Static tool and shell analysis is evidence, not a sandbox or audit log.
- Preserve raw values, resolution basis, scope, provenance, certainty, and typed gaps.
- A recovered path does not imply that an operation executes successfully.
- A complete parse does not imply exhaustive runtime file access.
- Unknown, dynamic, truncated, malformed, or unavailable analysis must remain visible.
- Consumer code chooses whether uncertainty allows, denies, retries, or warns.

### 6.3 Path and filesystem boundaries

- Lexical path utilities do not touch the filesystem.
- Canonicalization, symlink resolution, directory walking, and glob materialization are separate explicit operations.
- Native tool-call cwd determines relative operand meaning.
- Project/policy roots determine matching and discovery; they do not rewrite native operands.
- Filesystem walks must be bounded and expose truncation and unresolved targets.

### 6.4 Crate ownership

- `hookkit-core` owns only low-level shared types and lexical path utilities.
- `hookkit-common` owns lossless aligned native arms.
- `hookkit-runtime` owns parsing, environment validation, runtime context, arm checking, and process emission.
- `hookkit-shell` owns native shell profiles, Bash syntax, and shell-specific access inference.
- `hookkit-tool-access` owns phase-agnostic structured/patch/shell target evidence and general target resolution.
- `hookkit-file-activity` owns post-tool modification filtering, persistence, reconciliation, and pending windows.
- `hookkit-session-state` owns generic concurrency/state primitives, not loaded-rule policy.

Avoid dependency cycles. If a small lower-level borrowed JSON or tool-call view must move to preserve this ownership, document the move and keep it protocol-neutral.

### 6.5 Compatibility

- Preserve Rust 1.85 compatibility.
- This is a pre-release `0.1` API, so justified public cleanup is allowed, but avoid gratuitous churn.
- Preserve native wire output and process behavior exactly.
- Preserve existing persisted state schemas unless an explicit version bump and migration decision are part of the item.
- Preserve old configuration behavior or provide a documented, tested migration.
- Keep unknown enum arms and non-exhaustive matching forward-compatible.

## 7. Common validation requirements

Use a risk-based ladder rather than waiting until the end.

### 7.1 During each item

Run:

- the changed crate's unit/property tests;
- directly affected consumer tests;
- strict Clippy for changed crates and all their targets/features;
- `cargo fmt --all -- --check`; and
- `git diff --check`.

Each task document lists its minimum focused commands. Add targeted regression or executable probes for the riskiest behavior; a green existing suite is not enough when it lacks the newly identified case.

### 7.2 Before declaring any item complete

Review:

- `git diff origin/main...` for scope and accidental changes;
- public API documentation and examples;
- new `unwrap`, wildcard matches, lossy path conversions, or debug-string persistence;
- dependency direction and feature flags;
- contract/support claims; and
- persisted-state/config compatibility.

If the branch contains earlier completed items, compare the current item against its preceding item commit as well as reviewing the integrated diff.

### 7.3 Integrated suite after items 03, 05, and any implemented item 06

Run the repository's full documented suite:

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

The Pkl and tool-runner integration tests may skip themselves when `pkl` is unavailable, as documented by the repository. Report skips; do not describe them as executed passes.

### 7.4 Highest-risk end-to-end probes

Before completing item 05, exercise:

- aligned pre-tool allow/pass and deny for every harness;
- raw, resolved, descendant, glob, and workspace targets;
- a truncated resolver result;
- an unknown shell command under both fallback settings;
- a shell-invoked literal patch;
- deny-on-unresolved policy;
- a first and repeated Claude-rule claim; and
- preservation of post-tool write filtering in file activity.

## 8. Documentation and planning hygiene

- Public APIs need rustdoc explaining semantics and incompleteness boundaries.
- New crates need a README and root workspace orientation.
- Examples should demonstrate the preferred public path, not private reimplementations.
- Historical refactor notes should be retained and marked resolved or deferred with links.
- Do not leave duplicated temporary APIs without a removal note.
- If exact public names differ from these documents, update task docs or the handoff so later agents do not follow stale spellings.

Use ADRs for decisions that materially alter crate ownership, native-contract interpretation, persistence compatibility, or the lossless aligned-event model. Local method naming does not require an ADR.

## 9. Per-item handoff template

At each review boundary, record:

```text
Item:
Outcome:
Public API added/changed:
Consumers migrated:
Compatibility decisions:
Focused validation:
Full validation (if required):
Known gaps or deferred work:
Commit/PR:
Next item readiness:
```

Do not mark an item complete merely because it compiles. Every exit criterion in its task document must be satisfied or explicitly reported as blocked.

## 10. Completion definition

The required plan is complete when items 01 through 05 satisfy their exit criteria and the integrated suite is green, with any environment-dependent skips reported accurately.

Item 06 is complete either when its decision gate records continued deferral or when a justified generic metadata-claim API satisfies its stronger concurrency, crash, privacy, and compatibility requirements.

The final handoff should summarize the resulting crate/API architecture, link each implementation commit or PR, report the full validation results, identify any compatibility migrations, and list only genuinely remaining work.
