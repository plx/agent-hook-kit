# Shared API Enhancement 02: Aligned Pre-Tool Execution

<!-- markdownlint-disable MD013 -->

- Status: approved
- Priority: P1 API foundation; required before consumer migration
- Dependencies: none; may proceed independently of item 01
- Primary crates: `hookkit-common`, `hookkit-runtime`
- Native crates involved: `hookkit-claude`, `hookkit-codex`, `hookkit-gemini`, `hookkit-antigravity`
- Controlling plan: [Shared API Enhancement Plan](../shared-api-enhancement-plan.md)

## Objective

Add a library-owned aligned pre-tool event so one handler can inspect and decide a pre-tool call across Claude Code, Codex, Gemini CLI, and Antigravity while retaining each harness's complete native input, output, command environment, and process semantics.

## Current state

`hookkit-runtime` provides aligned `PostToolUse` and `TurnCompletion` markers, but no pre-tool equivalent. `forbidden-file-guard` therefore has three nearly identical runtime arms and native output constructors.

`AlignedEventSpec` is intentionally sealed. This task should extend the library's sealed family; it should not make external event specifications possible.

All four required native command contracts already exist:

- Claude Code: `hookkit_claude::catalog::PreToolUse`, using `CatalogInput` and `catalog::PreToolUseOutput`;
- Codex: `hookkit_codex::protocol::PreToolUse`;
- Gemini CLI: `hookkit_gemini::protocol::BeforeTool`; and
- Antigravity: `hookkit_antigravity::PreToolUse`.

The generic Claude catalog input is still a lossless, contract-backed native input. A new event-specific Claude struct is not a prerequisite for this task.

## Required API shape

In `hookkit-common`, add non-exhaustive native-arm enums analogous to the existing post-tool types:

- `PreToolUseCommandEnvironment`;
- `PreToolUseInput`; and
- `PreToolUseOutput`.

Each input and output must expose `harness()` and `event_id()`. Input should expose workspace roots and may expose allocation-free convenience accessors for genuinely aligned facts such as tool name, tool input, and native cwd. If native tool-input shapes require a borrowed `Value`/`Map` enum, keep that wrapper lossless and non-exhaustive.

Add an aligned `PreToolUse` marker in `hookkit-runtime`, with:

- explicit harness selection;
- exact native parsing and command-environment validation;
- a shared runtime context;
- native-arm output validation; and
- exact native emission.

Convenience constructors on the aligned output may cover only the demonstrated common intersection. The initial requirement is:

- pass through or allow for a selected harness; and
- deny with a reason for a selected harness.

Those constructors must return concrete native enum arms before runtime emission. They must not introduce a lowest-common-denominator serialized envelope. Keep native-only capabilities reachable through the enum arms.

## Implementation outline

1. Add the common command-environment, input, and output enums.
2. Add harness/event/workspace accessors and any narrowly justified tool-call accessors.
3. Add the sealed runtime marker and implementation.
4. Parse the exact native event for each explicitly selected harness.
5. Validate that the handler returns an output arm matching the selected/input harness before emitting it.
6. Add convenience execution spellings only if they match the existing aligned-runtime conventions.
7. Document the mapping from the aligned name to each native event name.

## Guardrails and non-goals

- Do not unseal `AlignedEventSpec`.
- Do not infer the harness from input JSON.
- Do not flatten native inputs or outputs into a shared JSON schema.
- Do not claim that rewrite, extra context, permission overrides, or other richer decisions align unless every supported arm can represent the exact stated semantics.
- Do not substitute one harness's event or output type for another because their current JSON happens to resemble it.
- Use the contract catalog and native conformance fixtures, not protocol memory.

## Required tests

For all four harnesses, cover:

- successful parsing through `execute_aligned_event::<PreToolUse, _>`;
- exact command-environment validation;
- pass-through/allow emission;
- deny emission with the correct native shape and process status;
- rejection of a handler output arm for the wrong harness; and
- preservation of native-only input fields.

Also add at least one stdin/stdout integration test for the aligned runtime path.

Run at minimum:

```bash
cargo test -p hookkit-common -p hookkit-runtime
cargo test -p hookkit-claude -p hookkit-codex -p hookkit-gemini -p hookkit-antigravity
cargo clippy -p hookkit-common -p hookkit-runtime --all-targets --all-features -- -D warnings
cargo run -p hookkit-conformance -- --check
cargo xtask contracts check
cargo xtask contracts report --check
cargo fmt --all -- --check
git diff --check
```

## Exit criteria

- `run_aligned_event::<PreToolUse>` works for all four harnesses.
- Native input and output arms remain complete and reachable.
- Wrong-arm emission is rejected before output is written.
- The sealed-event and explicit-harness design remains intact.
- The support report and public documentation remain accurate.
