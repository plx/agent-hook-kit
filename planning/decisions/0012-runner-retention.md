# ADR 0012: Retain the runner through stabilization

- Status: accepted
- Date: 2026-07-12
- Phase: 0; extraction decision deadline is Phase 7

## Context

The post-tool-use runner and Pkl catalog dominate repository file count, but they
are the most demanding real consumer of shared hook behavior.

## Decision

Keep the runner in this workspace through Phase 6 as the north-star acceptance
consumer. Every protocol change preserves compilation and a small hermetic smoke
path. Tool policy, Pkl configuration, artifact strategy, and path discovery stay
downstream. Temporary adapters name an owner and removal phase. Phase 7 evaluates
extraction only after the runner consumes released public APIs and a pinned
downstream compatibility job can preserve coverage.

## Consequences

Unrelated runner feature work is frozen during remediation. Repository size alone
does not justify premature extraction or upstreaming runner policy into common
crates.
