# agent-hook-kit

Contract-first Rust plumbing for command hooks across Claude Code, Codex, and
Antigravity.

## Scope

HookKit implements command-handler hooks: a hook process receives native input
and a declared process environment, then emits the harness-specific stdout,
stderr, and exit status. HTTP, prompt, agent, MCP, and other non-command hook
bindings are outside the implementation scope for this iteration.

The contract catalog can still record non-command bindings when an upstream
harness documents them. Those records preserve protocol evidence; they are not
HookKit implementation targets or missing vertical slices. Adding a
non-command runtime later requires an explicit scope decision and support-target
change.

## What This Repository Provides

- Native input/output models for implemented command-hook events per harness:
  - `hookkit-claude`
  - `hookkit-codex`
  - `hookkit-antigravity`
- First-class command-hook environment models, including deterministic map-based
  parsing and automatic capture in stdin/stdout runners:
  - [`docs/command-environments.md`](docs/command-environments.md)
- Lossless cross-harness aligned event wrappers:
  - `hookkit-common`
- Opt-in, bounded Bash syntax analysis and file-access inference for native shell tool calls:
  - [`hookkit-shell`](crates/hookkit-shell/README.md)
- Phase-agnostic structured, patch, and shell file-access evidence plus bounded
  scoped-target materialization:
  - [`hookkit-tool-access`](crates/hookkit-tool-access/README.md)
- Loss-aware file activity evidence, pending windows, and reconciliation:
  - [`hookkit-file-activity`](crates/hookkit-file-activity/README.md)
- Concurrent, versioned session-scoped state primitives:
  - [`hookkit-session-state`](crates/hookkit-session-state/README.md)
- Runtime stdin/stdout/exit-code plumbing:
  - `hookkit-runtime`
- Pkl-driven post-tool and batched turn-completion runners with embedded tool catalog:
  - `hookkit-pkl-config` (Pkl evaluation, builtin specs, multi-file merge)
  - `hookkit-tool-runner` (ships `post-tool-use-agent-hook`, the quiet
    `file-activity-agent-hook`, `turn-completion-agent-hook`, and the precise
    `session-start-state-agent-hook` metadata observer)
- Shared core errors, types, and deterministic lexical path operations:
  - `hookkit-core` path helpers normalize, resolve, expand an explicitly
    supplied home, and render UTF-8 slash paths without filesystem access;
    canonicalization and symlink resolution remain explicit caller concerns.
- Versioned upstream protocol ledger and generated support matrix:
  - [`contracts/`](contracts/README.md)
  - [`contracts/status/support.md`](contracts/status/support.md)
- Runnable examples:
  - `examples/claude-sessionstart-context`
  - `examples/codex-bash-guard`
  - `examples/shared-posttool-autofix`
  - `examples/antigravity-pre-invocation`
  - [`examples/codex-claude-rules`](examples/codex-claude-rules/README.md)
  - [`examples/forbidden-file-guard`](examples/forbidden-file-guard/README.md)
  - [`examples/session-modified-file-tracker`](examples/session-modified-file-tracker/README.md)

## Workspace Layout

```text
crates/
  hookkit-core/
  hookkit-runtime/
  hookkit-claude/
  hookkit-codex/
  hookkit-antigravity/
  hookkit-common/
  hookkit-shell/
  hookkit-tool-access/
  hookkit-file-activity/
  hookkit-pkl-config/
  hookkit-tool-runner/
  hookkit-session-state/
examples/
fixtures/
planning/
```

## Prerequisites

- Rust 2024 edition toolchain (`stable` works).
- [`pkl`](https://pkl-lang.org/main/current/pkl-cli/index.html) on `$PATH` —
  needed at runtime by the post-tool-use hook so it can evaluate embedded and
  user Pkl configs. Install with `brew install pkl` (macOS) or follow the
  upstream instructions for your platform.

## Build And Test

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
cargo run -p hookkit-conformance -- --check
cargo xtask contracts check
cargo xtask contracts report --check
cargo xtask contracts verify-vendor
```

The `hookkit-pkl-config` and `hookkit-tool-runner` integration tests skip
themselves when `pkl` is not on `$PATH`, so the test suite still passes in
build environments without Pkl installed.

## Quick Start: Exact Typed Runtime

Use `run_event` when a hook executable handles one exact event. The event type
fixes the harness, selected contract snapshot, wire event, and only valid output
type:

```rust
use hookkit_claude::protocol::{SessionStart, SessionStartOutput};

fn main() -> std::process::ExitCode {
    hookkit_runtime::run_event::<SessionStart, _>(
        |input, environment, context| {
            assert_eq!(context.event().name(), "SessionStart");
            assert_eq!(
                environment.session_id.as_str(),
                input.session_id.as_str()
            );
            Ok(SessionStartOutput::with_context(format!(
                "Session {} loaded",
                input.session_id
            )))
        },
    )
}
```

The handler returns `SessionStartOutput`, not raw bytes or a generic envelope, so
it cannot emit another event's discriminator. `WorktreeCreate` demonstrates the
same typed contract with a non-JSON result: its output emits a worktree path as
exact plain text. Claude Code accepts absolute paths and resolves relative paths
against the hook's working directory.

## Quick Start: Aligned `PreToolUse`

Aligned events keep native inputs and outputs intact while allowing one handler
to cover several harnesses. All three supported harnesses expose the aligned
pre-tool event as `PreToolUse`:

```rust
use hookkit_common::PreToolUseOutput;
use hookkit_core::HarnessId;
use hookkit_runtime::aligned::{PreToolUse, run_aligned_event};

fn main() -> std::process::ExitCode {
    let harness = HarnessId::CODEX; // select from trusted configuration or CLI input
    run_aligned_event::<PreToolUse, _>(harness, |input, environment, _context| {
        assert_eq!(input.harness(), environment.harness());
        if input.tool_name() == Some("dangerous_tool") {
            PreToolUseOutput::deny(&input.harness(), "blocked by policy")
        } else {
            PreToolUseOutput::allow(&input.harness())
        }
    })
}
```

The convenience constructors return a concrete native enum arm. Callers can
instead match `PreToolUseInput` and construct any native-only output capability
available to that arm.

## Quick Start: Aligned `PostToolUse`

Use `run_aligned_event` for genuinely shared lifecycle logic. Its enums retain
the complete native values, and the handler must return the matching native
output arm:

```rust
use hookkit_common::{PostToolUseInput, PostToolUseOutput};
use hookkit_core::HarnessId;
use hookkit_runtime::aligned::{PostToolUse, run_aligned_event};

fn main() -> std::process::ExitCode {
    run_aligned_event::<PostToolUse, _>(
        HarnessId::CLAUDE_CODE,
        |input, environment, _context| {
            assert_eq!(input.harness(), environment.harness());
            match input {
                PostToolUseInput::Claude(_) => Ok(PostToolUseOutput::Claude(
                    hookkit_claude::protocol::PostToolUseOutput::no_op(),
                )),
                _ => Err(std::io::Error::other("selected harness changed").into()),
            }
        },
    )
}
```

For a multi-harness executable, select a `HarnessId` from trusted CLI or
configuration input and match every supported `PostToolUseInput` arm. There is
no universal serialized output envelope: each arm constructs the native
response its harness actually supports. See `examples/shared-posttool-autofix`
for the complete pattern.

## Exact, Selected, and Aligned Execution

Choose the narrowest execution mode that fits the executable:

<!-- markdownlint-disable MD013 -->

| Need | Entry point | Identity, environment, and output safety |
| --- | --- | --- |
| One exact event | `run_event::<E>` / `execute_typed::<E>` | `E: EventSpec` fixes harness, snapshot, event, contract, input, command environment, and output. |
| One compile-time-selected harness, event chosen from input | `run_harness::<H>` / `execute_harness::<H>` | `H: HarnessSpec` resolves only its declared events, parses `H::CommandEnvironment`, and rejects a different output event arm. |
| A runtime-selected built-in harness | `dispatch_builtin_harness` / `execute_builtin_harness` | `BuiltinHarness` selects matching input, command-environment, and output arms; the runtime rejects cross-harness and cross-event results. |
| One aligned lifecycle concept | `run_aligned_event::<K>` / `execute_aligned_event::<K>` | Lossless native input, command-environment, and output arms are preserved and checked against the explicitly selected `HarnessId`. |

<!-- markdownlint-enable MD013 -->

Compile-time selected harnesses use native selectors such as
`hookkit_claude::protocol::Event`. Runtime-selected built-ins use a
harness-scoped `EventId` hint. A hint is validated against the selected harness
and payload; it does not force a mismatched parser. Candidate detection is an
inspection API and never authorizes execution of a guessed contract.

The main identity types are deliberately distinct:

- `HarnessId` is an open harness identifier; `BuiltinHarness` is the convenience
  selector for the three bundled adapters.
- `SnapshotId` identifies the immutable catalog snapshot used to parse input.
- `EventId` is always scoped by `HarnessId`; an event name alone is not an exact
  identity.
- `ContractId` identifies the exact event/binding emission contract.
- `AlignedEventKind` names only a shared lifecycle concept and never substitutes
  for an `EventId`.

Every command handler receives its parsed command environment as the second
argument and an exact `RuntimeContext` as the third. The context exposes the
identities and resolution provenance above, the original `RawInvocation`,
workspace roots, and only those typed session/conversation/turn/tool-call paths
or identifiers that the native event supplied. It does not probe alternate key
casing, recursively mine arbitrary JSON, or invent a current directory.

The `run_*` adapters capture only the environment names declared by the selected
harness type. In-memory `execute_*` APIs instead accept an explicit
`EnvironmentVariables` map, keeping tests deterministic and free of
process-global environment mutation. See the
[command-hook environment reference](docs/command-environments.md) for the full
46-event matrix, handler-binding boundary, and migration notes.

### Diagnostics and process streams

Protocol emission and application diagnostics are separate:

- `stdout` contains only the exact command response bytes produced by the event
  contract. Empty bytes, `{}`, JSON `null`, and text are intentionally distinct,
  and the runtime does not append a newline.
- Protocol-defined `stderr` and exit status are constructed by event-specific
  output APIs, such as `with_protocol_stderr` or a blocking-error constructor.
- Runtime and handler diagnostics go to a `DiagnosticsSink`. The convenience
  runners use `DISABLED_DIAGNOSTICS`; pass a sink to
  `run_event_with_diagnostics` or `execute_harness_with_diagnostics` when the
  application needs out-of-band records.
- Parse, resolution, or handler errors return a nonzero adapter exit status; the
  runtime does not turn them into protocol decisions or automatically print
  them on protocol streams.

For example, a handler can record an application diagnostic without changing
its wire response:

```rust
use hookkit_core::{Diagnostic, DiagnosticLevel};

context.diagnostics().record(Diagnostic::new(
    DiagnosticLevel::Info,
    "policy cache hit",
));
```

Native event output types remain the authority for any user- or agent-visible
message. This keeps diagnostic logging from contaminating hook JSON.
Handler code must likewise avoid uncontrolled `println!` or `eprintln!` calls:
either can corrupt a harness protocol stream even when the returned output is
otherwise valid.

## Run The Examples With Fixtures

Claude session-start context:

```bash
cat fixtures/claude/session_start.json \
  | cargo run -q -p claude-sessionstart-context
```

Codex bash guard (JSON deny on blocked commands):

```bash
cat fixtures/codex/pre_tool_use.json \
  | cargo run -q -p codex-bash-guard
```

Antigravity pre-invocation reminder:

```bash
cat fixtures/antigravity/pre_invocation.json \
  | cargo run -q -p antigravity-pre-invocation
```

Shared post-tool autofix (aligned runtime):

```bash
cat fixtures/claude/post_tool_use.json \
  | cargo run -q -p shared-posttool-autofix -- --claude
```

The three stateful/policy examples have their own setup, state-layout, and
limitation notes:

- [`codex-claude-rules`](examples/codex-claude-rules/README.md) lazily injects
  path-scoped files from Claude Code's user and project rules directories.
- [`forbidden-file-guard`](examples/forbidden-file-guard/README.md) selects a
  native pre-tool contract with `--harness=claude|codex|antigravity` and merges
  home/project YAML policy.
- [`session-modified-file-tracker`](examples/session-modified-file-tracker/README.md)
  demonstrates the shipped file-activity observer while retaining the former
  example command and flags as compatibility aliases.

## `post-tool-use-agent-hook`

The reusable post-tool-use formatter/linter runner ships as a single binary
that selects tools and policy from Pkl configuration:

```bash
cat fixtures/claude/post_tool_use.json \
  | cargo run -q -p hookkit-tool-runner --bin post-tool-use-agent-hook -- --claude
```

The CLI accepts `--claude` or `--codex` to choose the harness, and
`--config PATH` to load a single Pkl file directly (bypassing discovery).

The companion `turn-completion-agent-hook` reconciles and consumes the
NDJSON-backed pending file-activity window at each supported harness's `Stop`
event. For each matching deferred workflow it runs a read-only check,
runs one remedy only when that check reports source issues, and reruns every
check invalidated by observed writes. It allows completion after clean or
fully auto-fixed results while emitting the configured deferred report through
each harness's native channels:

```bash
cargo run -q -p hookkit-tool-runner --bin turn-completion-agent-hook -- \
  --claude --state-dir .context/hookkit-state
```

Use the same `--state-dir` for `file-activity-agent-hook`. Before sealing
the window, the runner scans workspace mtimes since the durable reconciliation
cursor (or the current session start on its first pass). It commits artifacts
and `summary.json`, requeues only manual, operationally incomplete, and
unresolved work into the active generation, records handled fingerprints for
clean/auto-fixed/deleted files, and then acknowledges the sealed source
generations. Manual issues block the stop attempt and point to those committed
logs. See the
[file-activity crate](crates/hookkit-file-activity/README.md) and
[session-state walkthrough](crates/hookkit-session-state/README.md).

For precise automatic start metadata even before another stateful hook runs,
bind the no-op observer to the harness's native `SessionStart` event:

```bash
cargo run -q -p hookkit-tool-runner --bin session-start-state-agent-hook -- \
  --claude --state-dir .context/hookkit-state
```

Codex uses `--codex`. Every later
`SessionState::ensure` still refreshes typed project metadata automatically;
without a start binding, the timestamp is explicitly marked as a
first-observed fallback.

### Deferred hook suite

A complete deferred installation binds these shipped executables to one shared
state root:

| Purpose | Claude | Codex |
| --- | --- | --- |
| Precise session lower bound | `SessionStart` → `session-start-state-agent-hook` | `SessionStart` → `session-start-state-agent-hook` |
| File-activity producer | `PostToolUse` → `file-activity-agent-hook` | `PostToolUse` → `file-activity-agent-hook` |
| Deferred consumer | `Stop` → `turn-completion-agent-hook` | `Stop` → `turn-completion-agent-hook` |

For example, every command below must use the same path:

```bash
session-start-state-agent-hook --claude --state-dir .context/hookkit-state
file-activity-agent-hook --claude --state-dir .context/hookkit-state
turn-completion-agent-hook --claude --state-dir .context/hookkit-state
```

Use `--codex` consistently for Codex. Both harnesses also accept the
compatibility form `--harness=claude|codex` and
`--state-dir=PATH`; turn completion alone accepts `--config PATH`.

Antigravity can bind `Stop` to `turn-completion-agent-hook --antigravity`, but
its PostToolUse payload has no tool call or path arguments and it has no
supported precise start/activity producer in this suite. Antigravity therefore
uses best-effort filesystem-mtime reconciliation only (plus optional Git-dirty
fallback) and may miss changes outside that observable window.

### Configuration discovery

When `--config` is not used the runner loads, in order:

1. `~/.agent-hook-kit/post-tool-use.pkl` (home/global)
2. each `<ancestor>/.agent-hook-kit/post-tool-use.pkl` walking up from `cwd`
   (root → leaf)
3. each `<ancestor>/.agent-hook-kit/post-tool-use.local.pkl` walking up from
   `cwd` (root → leaf, intended to be `.gitignore`d)

Later files override earlier ones. A file can opt out of earlier state with
`merge { resetAll = true }`, `merge { reset = new Listing { "tools"; "run" } }`,
or `merge { resetTools = new Listing { "ruff" } }`.

### Bundled builtins

`hookkit-pkl-config` embeds `Builtins.pkl` with 134 reusable specs. Every
enabled entry either declares explicit deferred workflows or passes catalog
validation for the legacy-phase compatibility translation. The complete
command, scope, granularity, and limitation inventory is
[`planning/builtin-deferred-workflow-audit.md`](planning/builtin-deferred-workflow-audit.md).
Representative entries include:

- `Builtins.ruff` — Python format/fix/verify with ruff
- `Builtins.prettier` — JS/TS/CSS/HTML/JSON/Markdown formatter
- `Builtins.eslint` — JS/TS fix + verify
- `Builtins.biome` — JS/TS/JSON fix + verify
- `Builtins.cargoFmt` — Rust workspace formatter
- `Builtins.cargoClippy` — Rust workspace fix + verify

Ruff has distinct lint and format workflows. `go-fmt`, `gofumpt`, `goimports`,
and `golines` use non-mutating stdout-aware checks; `gomod-tidy` uses
`go mod tidy -diff`; and yq uses a per-file comparator. No enabled builtin
relies on an unchecked mutator-first fallback.

### Example project config

`.agent-hook-kit/post-tool-use.pkl`:

```pkl
amends "Config.pkl"
import "Builtins.pkl"

settings {
  diagnosticsDirectory = ".agent-hook-kit/post-tool-use"
  missingToolPolicy = "user-notice"
}

tools {
  ["ruff"] = (Builtins.ruff) {
    phases {
      ["fix"] {
        // keep unused-import removal out of the post-edit pass
        extraArgs = new Listing<String> { "--unfixable"; "F401" }
      }
    }
    workflows {
      ["lint"] {
        remedy {
          // apply the same choice to deferred turn completion
          extraArgs = new Listing<String> { "--unfixable"; "F401" }
        }
      }
    }
  }
  ["prettier"] = Builtins.prettier
}

run = new Listing<String> { "ruff"; "prettier" }
```

### Settings reference

| Field | Default | Purpose |
| --- | --- | --- |
| `settings.jobs` | `0` (auto) | Max independent per-workspace jobs to run concurrently. `1` (or `0`/auto, currently serial) runs jobs sequentially; `>= 2` runs up to that many at once, capped at the job count. |
| `settings.failFast` | `true` | Stop after operational failures. |
| `settings.continueAfterIssues` | `true` | Keep running later tools after source issues. |
| `settings.exclude` | `[".git/**", "node_modules/**"]` | Global file exclusions applied before per-tool filters. |
| `settings.loweringPolicy` | `"best-effort-with-warnings"` | How to handle a nonempty user/agent message that the selected native event cannot represent faithfully: fail, omit, or omit with a native-channel warning. |
| `settings.diagnosticsDirectory` | `".agent-hook-kit/post-tool-use"` | Where to write diagnostic artifacts. |
| `settings.missingToolPolicy` | `"user-notice"` | What to do when a configured tool executable is missing. Options: `"user-notice"`, `"hard-failure"`, `"harness-block"`. |
| `settings.fileActivity.filesystemMtime` | `true` | Reconcile mtime evidence through a durable cutoff before each Stop. |
| `settings.fileActivity.vcs` | `"disabled"` | Optional `"git-dirty"` fallback; broad because it cannot identify which dirty changes came from the agent. |
| `settings.fileActivity.maxEntries` | `100000` | Bound scoped/workspace traversal; truncation is retained as a coverage gap. |
| `settings.fileActivity.coverageGapPolicy` | `"best-effort"` | Process resolved files and warn while retaining gaps, or use `"strict"` to block until coverage is complete. |

### Deferred reporting templates

`settings.deferredReporting` controls only the session-batched
`turn-completion-agent-hook` report. Its ordered `groups` assign the first
matching file group, then fall back to `other`. The `clean`, `autoFixed`,
`manualFixesNeeded`, and `operationalError` fields each contain `user` and
`agent` MiniJinja templates; `masterUser` and `masterAgent` combine the
rendered nonempty buckets. Set `renderEmptyBuckets = true` to render empty
buckets too, or use an empty template to suppress one audience.

Templates receive run paths, counts, typed file/report/artifact records,
ordered groups, operational problems and coverage gaps. They also receive
`artifact_paths`, `artifact_contents`, raw `buckets`, and
`rendered_buckets` as independent views. Paths stored in file `displayPath`
are project-relative when possible. Reporting syntax is validated before any
configured tool runs; a later rendering error is retained as a durable
operational artifact.

Layered Pkl files merge this block field by field, including nested template
pairs, so overriding only `manualFixesNeeded.agent` preserves inherited
siblings. `merge { resetDeferredReporting = true }` restores the built-in
block before applying that file's local overrides.

The `messages` block inside an individual `ToolSpec` is separate: it remains
the per-tool message policy for the immediate `post-tool-use-agent-hook` and
does not define deferred bucket meaning.

Deferred batch and workspace checks conservatively attach a finding to every
candidate in that invocation unless exact changed-file snapshots or a future
diagnostic adapter provide narrower evidence. Tracking is best effort: dynamic
commands, changes outside supplied workspaces, timestamp limitations, and
Antigravity's missing PostToolUse arguments can create retained coverage gaps.
The summary distinguishes uncovered, not-applicable, unresolved, truncated,
manual, and operational outcomes rather than calling them clean.

Deferred Stop lowering uses the exact native fields below:

| Harness/event | Allowed user | Allowed agent | Blocked user | Blocked agent |
| --- | --- | --- | --- | --- |
| Claude `Stop` | `systemMessage` | `hookSpecificOutput.additionalContext` | `systemMessage` | `reason` plus `additionalContext` |
| Codex `Stop` | `systemMessage` | unavailable | `systemMessage` | `reason` |
| Antigravity `Stop` | unavailable | unavailable | unavailable | `reason` |

`strict` fails before pending-state acknowledgement if a configured audience
is unavailable. `best-effort` omits it. `best-effort-with-warnings` adds an
omission warning to a representable native user channel, or to Antigravity's
single `reason` fallback. The summary records each audience disposition and
any warning. An unrepresentable allowed-stop agent note never turns a
successful result into a block under either best-effort policy.

### Migration from per-tool binaries

Prior versions shipped six per-tool binaries (`ruff-agent-hook`,
`prettier-agent-hook`, etc.) configured by TOML. Those binaries have been
removed; replace them with the single `post-tool-use-agent-hook` binary and a
Pkl config that selects the same tool. For example:

```diff
- ruff-agent-hook --claude
+ post-tool-use-agent-hook --claude
```

…with `.agent-hook-kit/post-tool-use.pkl` referencing `Builtins.ruff` and any
overrides previously set in `.agent-hook-kit/ruff-agent-hook.toml`.

For deferred config migration, existing `phases` still drive immediate
PostToolUse and are compatibility-translated at Stop when they have a read-only
verifier. Prefer explicit `workflows` for new or combined tools. The pending
file-activity entity is version 2 and handled baselines are a separate version-1
entity; an upgrade from the former pending v1 creates a fresh transient subtree,
so restart the agent session when exact continuity matters. Mtime and optional
Git-dirty reconciliation recover only best-effort candidates.

## Example Behavior Summary

- `claude-sessionstart-context`:
  - injects additional model context on `SessionStart`.
- `codex-bash-guard`:
  - inspects `PreToolUse` Bash commands and emits deny JSON for blocked patterns.
- `shared-posttool-autofix`:
  - runs a formatter/linter-autofix pipeline when applicable,
  - emits each harness's exact native no-op response on clean success,
  - prints concise user status to `stderr` when autofix/manual work occurs,
  - writes verbose manual diagnostics to a temp artifact and gives concise agent guidance when supported.
- `codex-claude-rules`:
  - discovers Claude Code rule files recursively in user-before-project order,
  - evaluates `paths` frontmatter against materialized structured, patch, and
    shell access targets from `hookkit-tool-access`,
  - atomically claims each matched rule in session state before injecting its
    body as additional context.
- `forbidden-file-guard`:
  - uses one aligned handler for Claude, Codex, and Antigravity pre-tool events,
  - merges additive YAML glob policy from home and workspace configuration,
  - applies inspect-known, deny-unresolved, or deny-all-shell posture to bounded
    structured, patch, and shell access evidence.
- `file-activity-agent-hook`:
  - uses the aligned post-tool API for Claude Code and Codex,
  - delegates structured writers, patches, and shell inference to the shared tool-access analyzer and never shells out to Git,
  - appends detailed observations to rotated NDJSON generations whose projection is a versioned per-session path set.
- `post-tool-use-agent-hook`:
  - loads merged Pkl config plus embedded builtin tool catalog,
  - discovers exact candidate paths through the shared file-activity/tool-access analyzer,
  - runs each tool's phases for format/fix/verify commands,
  - classifies clean versus issues and changed versus unchanged from exit policies plus file snapshots,
  - reports missing tools and operational failures per `missingToolPolicy`,
  - writes remaining diagnostics to artifacts,
  - lowers every result through an explicit Claude Code or Codex native output arm.
- `turn-completion-agent-hook`:
  - seals the current modified-file generations under an exclusive entity consumer lock,
  - dispatches Pkl-configured check/conditional-remedy/final-check workflows across the accumulated file set,
  - commits detailed per-tool logs and a summary before producing its decision,
  - acknowledges the sealed window after requeueing only unfinished work and recording handled baselines for discharged files,
  - emits configured clean/auto reports without blocking and uses each harness's native continue-working signal for manual or operational results.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.

Third-party crate licenses bundled with binary distributions are aggregated in
[`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md). That file is generated by
`cargo about` and validated in CI; regenerate it locally with
`scripts/regen-licenses.sh` after changing dependencies.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual-licensed as above, without any additional terms or conditions.
