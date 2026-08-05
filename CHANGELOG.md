# Changelog

## Unreleased — contract-first reboot

- Added a frozen, source-provenance-aware contract catalog for 57 events across
  Claude Code, Codex, Gemini CLI, and Antigravity.
- Added event-associated typed command APIs and exact byte/process emission.
- Added harness-native command-environment types for all 57 catalog events,
  deterministic map-based parsing, selective process capture, redundant input
  validation, and explicit environment parameters on every command handler.
- Added selected-harness resolution, optional hints, and a non-executing detector.
- Added a distinct Antigravity crate and lossless four-harness PostToolUse arms.
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

This is an intentional pre-1.0 protocol API reboot. Event-scoped output types are
the supported surface.
