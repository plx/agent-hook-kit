# Copier-based Hook Project Templates

Status: implemented and locally path-validated; public Git preview and
crates.io publication gates remain

Date: 2026-08-08

Target: `agent-hook-kit` after `5ae45c2`

Repository-boundary amendment: the turnkey Pkl-driven linting and formatting
product was extracted to [Velvet Glove](https://github.com/plx/velvet-glove)
under [ADR 020](decisions/020-extract-quality-runner-to-velvet-glove.md). This
template now generates applications directly from HookKit's reusable protocol,
runtime, tool-access, file-activity, and state libraries. Product-specific
quality profiles, runner configuration, and managed deferred workflows are not
template concerns.

## 1. Recommendation

Maintain one independently updateable, task-free Copier template with two
output modes:

1. **Full project** — create a Rust workspace containing one hook CLI crate.
2. **CLI crate** — add one namespaced hook CLI crate and optional namespaced
   GitHub Actions workflow to an existing repository.

Both modes render the same application structure. They differ only in the
repository-level files the template owns. The generated application is one
branded binary with explicit event subcommands. Cross-harness projects also
take an explicit harness argument; the CLI never infers an event or harness
from input JSON.

The template has no tasks or migrations. Copy and update do not require
`--trust`; formatting, compilation, and tests remain explicit commands.

## 2. Protocol-derived choices

Single-harness mode exposes every implemented command event for Claude Code,
Codex, or Antigravity. Cross-harness mode exposes only explicit alignment
records whose supported-harness set contains every selected harness.

The three universal aligned families are:

| Stable ID | Native events |
| --- | --- |
| `pre_tool` | Claude, Codex, and Antigravity `PreToolUse` |
| `post_tool` | Claude, Codex, and Antigravity `PostToolUse` |
| `turn_completion` | Claude, Codex, and Antigravity `Stop` |

Claude and Codex also share the reviewed pair-only families in the canonical
alignment catalog. Antigravity-containing selections see only the compatible
intersection. Availability comes from the catalog and sealed runtime markers,
never from matching wire-event names.

Only command bindings are scaffoldable. HTTP, prompt, agent, and MCP contract
records remain evidence, not template implementation targets.

## 3. Questionnaire

Questions are resolved in this order:

1. output topology and identity;
2. starter archetype;
3. harness mode and harness selection;
4. generic state capabilities;
5. hook selection;
6. dependency source;
7. optional CI.

Stable machine IDs are persisted in the answers file. Display labels may
evolve independently. Validation enforces path-safe names, at least two
harnesses in cross mode, a nonempty compatible hook selection, archetype
requirements, state dependencies, and immutable Git revisions.

The generic state capabilities are:

- `session_metadata`;
- `claim_once`;
- `inspectable_set`;
- `record_queue`;
- `run_artifacts`; and
- `custom_aggregate`.

Capabilities other than metadata require `session_metadata`. Each editable
handler receives an optional state-directory argument so a project can add
state later without changing the handler seam.

## 4. Archetypes

An archetype is a named set of concrete defaults, not a second implementation
path. The user still sees and persists the selected harnesses, hooks, and state
capabilities.

| ID | Default shape | Generated seam |
| --- | --- | --- |
| `custom` | Configurable native or aligned hook set | One protected handler per event |
| `policy_guard` | Cross-harness pre-tool access analysis | Pure allow/deny evaluator |
| `scoped_context_once` | Single-harness pre-tool plus metadata and claims | Discover content separately from atomic claim |
| `session_bootstrap` | Claude/Codex aligned session start plus metadata | Build portable context; retain native lowering |
| `pre_tool_rewrite` | Claude/Codex aligned pre-tool rewrite | Pure input transform |

Projects seeking an immediate or deferred linting-and-formatting workflow
should generate or configure that product through
[Velvet Glove](https://github.com/plx/velvet-glove).

## 5. Generated layout and ownership

The full-project layout is:

```text
.copier-answers.yml
.github/workflows/<package>-hook-ci.yml   # optional
Cargo.toml
<crate_path>/
  Cargo.toml
  README.md
  hookkit-template.manifest.yml
  src/
    hooks/                                # protected user seams
    scaffold/                             # Copier-owned adapters
    lib.rs
    main.rs
  tests/
```

Crate mode uses `.copier-answers.<package>.yml`, leaves the host
`Cargo.toml` unchanged, and renders the exact workspace member the user may add
manually. Multiple instances must own disjoint crate, answers, and workflow
paths.

Ownership rules are:

- `src/scaffold/**`, top-level hook exports, manifests, README, fixtures, and
  CI are template-owned;
- per-event and per-archetype handlers beneath `src/hooks/**` are protected by
  `_skip_if_exists` after first creation;
- deselected protected seams remain harmless and become active if reselected;
- changing package, binary, or crate-path identity is an explicit migration,
  not an ordinary update.

## 6. Generated Rust design

Every selected event has an explicit subcommand. A native command calls
`run_event::<ExactEvent>`. An aligned command calls
`run_aligned_event::<AlignedMarker>(selected_harness, ...)`. Generic
`post_tool` and `turn_completion` selections use the same protected handler
flow as other aligned events; HookKit does not substitute a product runner.

The event scaffold catalog records a safe initial strategy:

- `no_op` emits the native no-op response;
- `allow` emits an explicit native allow decision;
- `observe` parses and acknowledges without inventing a decision; and
- `must_implement` fails explicitly when the contract requires a domain value
  that cannot be safely guessed.

Hook stdout, stderr, and exit status are protocol outputs. Generated domain
handlers return typed decisions through adapters rather than printing directly.

## 7. Dependency resolution and publication

Render only direct dependencies referenced by generated Rust source. Typical
additions are:

| Selection | Direct dependency |
| --- | --- |
| Base runtime | `hookkit-core`, `hookkit-runtime`, `clap` |
| Single native harness | selected native harness crate |
| Cross-harness event | `hookkit-common` |
| Tool access archetype | `hookkit-tool-access` |
| State capability | `hookkit-session-state` plus serialization crates as needed |

Before publication, every HookKit dependency uses the same public repository
and exact 40-character Git revision. Path mode exists for local development and
is explicitly nonportable. Do not default to a branch.

The 10 publishable crates use one compatibility train:

1. `hookkit-core`;
2. `hookkit-claude`, `hookkit-codex`, `hookkit-antigravity`, and
   `hookkit-session-state`;
3. `hookkit-common` and `hookkit-shell`;
4. `hookkit-tool-access`;
5. `hookkit-file-activity`; and
6. `hookkit-runtime`.

The default may switch to crates.io only after all required crates are
published at a compatible version and a clean locked render resolves entirely
from the registry.

## 8. Optional GitHub Actions workflow

The package-namespaced workflow uses read-only contents permission, a
PR-aware concurrency group, Rust 1.85 for MSRV checking, stable Rust with
rustfmt and Clippy, and package-scoped format/check/test commands. Full projects
test Ubuntu and macOS; crate mode defaults to Ubuntu. It does not edit live
harness registration or install product-specific tooling.

## 9. Template validation

The acceptance suite covers:

- full-project, non-workspace crate, and existing-workspace crate topologies;
- every native event and every compatible aligned family;
- every harness subset and negative selected-set assertions;
- every generic state capability and archetype;
- coexistence of multiple crate-mode instances;
- invalid names, paths, sources, archetype modes, and state combinations;
- preservation of protected handler seams through recopy and synthetic
  versioned update; and
- local path, public Git, and eventual crates.io dependency modes.

The canonical catalogs are checked against the implementation registry and
rendered into the hidden Copier question catalog by `cargo xtask
template-catalog sync`. CI fails when generated catalog data is stale.

## 10. Release gates

The Git-pinned preview is ready when the public compatibility revision is
reachable, all direct HookKit dependencies resolve to it, every generated
matrix cell compiles on Rust 1.85 and stable, protected handler edits survive
an update, and package-specific CI passes.

Public v1 additionally requires the compatible HookKit crate train on
crates.io and a clean locked render with no path or Git dependency escape.
