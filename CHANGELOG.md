# Changelog

## Unreleased — contract-first reboot

- Added a frozen, source-provenance-aware contract catalog for 56 events across
  Claude Code, Codex, Gemini CLI, and Antigravity.
- Added event-associated typed command APIs and exact byte/process emission.
- Added selected-harness resolution, optional hints, and a non-executing detector.
- Added a distinct Antigravity crate and lossless four-harness PostToolUse arms.
- Split hermetic required tests from opt-in real-tool and live-harness lanes.
- Added generated Rust/catalog parity and support reporting.
- Removed the legacy generic native/common runners and protocol-invalid output
  envelopes; use `run_event`, `run_harness`/`dispatch_builtin_harness`, or
  `run_aligned_event`.

This is an intentional pre-1.0 protocol API reboot. Event-scoped output types are
the supported surface.
