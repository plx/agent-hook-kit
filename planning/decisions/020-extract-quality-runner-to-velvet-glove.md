# ADR 020: Extract the quality runner to Velvet Glove

- Status: accepted
- Date: 2026-08-08
- Phase: post-7 repository boundary cleanup

## Context

The Pkl configuration catalog and immediate/deferred linting-and-formatting
runner were retained in this repository while HookKit's public contracts and
library APIs stabilized. They are a complete downstream product with their own
configuration language, release cadence, executable suite, fixtures, and
operator documentation. Keeping them beside the libraries now obscures the
boundary between reusable hook infrastructure and one opinionated consumer.

The Copier project template must likewise describe projects built directly on
HookKit's reusable libraries. Product-specific `immediate_quality`,
`deferred_quality`, managed file-activity, Pkl, and runner options would couple
new HookKit projects back to the extracted product.

## Decision

Move `hookkit-pkl-config`, `hookkit-tool-runner`, their executable and fixture
corpus, and the session-modified-file-tracker compatibility example to
[Velvet Glove](https://github.com/plx/velvet-glove).

Remove their product-specific Copier archetypes and managed configuration.
Keep HookKit's generic native and aligned lifecycle scaffolds, including
PostToolUse and turn-completion events, and keep the reusable
`hookkit-file-activity` library.

Velvet Glove consumes HookKit from its public Git repository until the HookKit
crates are published. HookKit documentation points users seeking the turnkey
quality workflow to that project.

## Consequences

HookKit's workspace, CI, release process, and license attribution cover only
the reusable libraries and remaining examples. Runner and Pkl compatibility
testing moves with Velvet Glove. Changes to HookKit's public APIs can still be
validated downstream by pinning Velvet Glove to a reviewed HookKit revision.

This decision supersedes ADR 0012 and ADR 015.
