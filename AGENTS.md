# agent-hook-kit

Rust 2024 libraries and a Pkl-configured CLI for writing fast, maintainable smart hooks for Claude Code, Codex, Gemini CLI, and Antigravity.

## Why

A hook for a coding agent is usually a thin wrapper around some other tool (a linter, formatter, etc.) with a very repetitive shape:

1. parse the harness's hook-input JSON, then decide whether the hook should actually run (e.g. match the modified file against a pattern);
2. invoke the underlying tool, perhaps with hook-specific configuration tweaks (e.g. disabling unused-import removal in a post-edit linter pass so it doesn't fight code that hasn't been written yet);
3. parse the tool's output to classify the result (clean / auto-fixed / needs manual intervention / etc.);
4. emit a harness-specific JSON response that carries not just a coarse allow-or-block decision, but also distinct user-facing and agent-facing messages — e.g. the user sees the full diagnostic while the agent only sees a pointer to a fix-up skill.

These wrappers are typically written in bash as a lowest-common-denominator, but bash is a poor fit for that much branching logic: hard to get right, harder to maintain, and harder still to read. `agent-hook-kit` lifts the reusable pieces into fast Rust so it's straightforward to:

- ship a stable of hook wrappers for common linters and formatters;
- write fast, clean telemetry and observability hooks;
- build coordinated hook suites that maintain per-session aggregates (files modified, tools invoked, etc.) without each hook reinventing the plumbing.

## Layout

- `crates/hookkit-core` — shared error and primitive types.
- `crates/hookkit-runtime` — exact typed (`run_event`), selected-harness (`run_harness`/`dispatch_builtin_harness`), and aligned (`run_aligned_event`) stdin/stdout/exit-code plumbing.
- `crates/hookkit-{claude,codex,gemini,antigravity}` — event-scoped native input/output contracts.
- `crates/hookkit-common` — lossless native-arm wrappers for genuinely aligned lifecycle events.
- `crates/hookkit-session-state` — automatic typed session metadata plus concurrent, versioned claims, content-addressed record journals, NDJSON-backed aggregate entities, run bundles, scopes, observations, locks, and cleanup.
- `crates/hookkit-pkl-config` — Pkl evaluation, embedded builtin tool catalog, multi-file config merge, and discovery for the post-tool-use runner.
- `crates/hookkit-tool-runner` — ships the immediate `post-tool-use-agent-hook`, session-batched `turn-completion-agent-hook`, and precise `session-start-state-agent-hook` metadata observer.
- `examples/` — runnable hook stubs paired with `fixtures/` JSON for each harness.
- `planning/` — design notes; not shipped.

`pkl` must be on `$PATH` at runtime for the post-tool-use runner; see `README.md` for prerequisites, build/test commands, and example invocations.
