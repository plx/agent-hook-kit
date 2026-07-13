# ADR 0008: Gemini CLI and Antigravity are distinct harnesses

- Status: accepted
- Date: 2026-07-12
- Phase: 0

## Context

Product transition messaging does not establish interchangeable hook wire
contracts. Antigravity has a different event inventory and ambiguity behavior.

## Decision

Gemini CLI retains its own public identity and crate. Antigravity receives a
separate catalog snapshot, `HarnessId`, native crate, events, fixtures, examples,
and support status. Internal components may be shared only for reviewed
wire-identical structures.

## Consequences

Phase 2 creates the minimal Antigravity crate and Phase 3 completes it. Public
values never report Gemini identity for Antigravity invocations.
