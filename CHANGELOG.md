# Changelog

## Unreleased — contract-first reboot

- Added a frozen, source-provenance-aware contract catalog for 46 events across
  Claude Code, Codex, and Antigravity.
- Added event-associated typed command APIs and exact byte/process emission.
- Added harness-native command-environment types for all 46 catalog events,
  deterministic map-based parsing, selective process capture, redundant input
  validation, and explicit environment parameters on every command handler.
- Added selected-harness resolution, optional hints, and a non-executing detector.
- Added a distinct Antigravity crate and lossless three-harness PostToolUse arms.
- Removed Gemini CLI support, including its native crate, contracts, fixtures,
  examples, runtime selection, and runner integrations.
- Split hermetic required tests from opt-in real-tool and live-harness lanes.
- Added generated Rust/catalog parity and support reporting.
- Defined HookKit's runtime scope as command bindings; catalogued HTTP and other
  non-command bindings are explicit unsupported implementation targets.
- Removed the legacy generic native/common runners and protocol-invalid output
  envelopes; use `run_event`, `run_harness`/`dispatch_builtin_harness`, or
  `run_aligned_event`.
- Refreshed Claude Code against the 2026-08-05 hook reference: added
  `DirectoryAdded`, forked session starts, current optional fields and output
  controls, deferred pre-tool decisions, dynamic watch paths, and relative
  worktree paths.
- Fixed `run_aligned_event`, `run_typed`/`run_event_with_diagnostics`,
  `run_harness`, and `dispatch_builtin_harness` so a failed stdin read, a bad
  hook environment, or a handler/parse error now write a concise one-line
  diagnostic (program, hook identity, full cause chain) to stderr before
  `exit 1`, instead of leaving both stdout and stderr byte-for-byte empty.
- Downgraded two Claude Code environment checks that could reject a
  legitimate real-world hook invocation from hard failures to silent
  fallbacks: a missing `CLAUDE_ENV_FILE` on `SessionStart`/`Setup`/
  `CwdChanged`/`FileChanged` (undocumented in current public docs) now
  yields `environment_file: None` instead of an error, and a lone
  `CLAUDE_PLUGIN_ROOT`/`CLAUDE_PLUGIN_DATA`/`CLAUDE_PLUGIN_OPTION_*` value
  without its pair (ambient state from an unrelated parent process) now
  yields `plugin: None` instead of an error, matching how
  `CodexCommandEnvironment` already treats a partial alias set.

This is an intentional pre-1.0 protocol API reboot. Event-scoped output types are
the supported surface.
