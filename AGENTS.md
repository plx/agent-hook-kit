# agent-hook-kit

Rust 2024 library plus a CLI stub for writing fast, maintainable smart hooks for coding agents (Claude, Codex, Gemini).

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
- `crates/hookkit-runtime` — stdin/stdout/exit-code plumbing; entry points like `run_native` and `run_common`.
- `crates/hookkit-{claude,codex,gemini}` — per-harness input/output models.
- `crates/hookkit-common` — cross-harness wrapper layer for hooks that should behave the same everywhere.
- `crates/hookkit-pkl-config` — Pkl evaluation, embedded builtin tool catalog, multi-file config merge, and discovery for the post-tool-use runner.
- `crates/hookkit-tool-runner` — ships the `post-tool-use-agent-hook` binary that drives a Pkl-configured pipeline of formatter/linter/checker tools.
- `examples/` — runnable hook stubs paired with `fixtures/` JSON for each harness.
- `planning/` — design notes; not shipped.

`pkl` must be on `$PATH` at runtime for the post-tool-use runner; see `README.md` for prerequisites, build/test commands, and example invocations.
