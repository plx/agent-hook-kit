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
- Shared core errors, types, and deterministic lexical path operations:
  - `hookkit-core` path helpers normalize, resolve, expand an explicitly
    supplied home, and render UTF-8 slash paths without filesystem access;
    canonicalization and symlink resolution remain explicit caller concerns.
- Versioned upstream protocol ledger and generated support matrix:
  - [`contracts/`](contracts/README.md)
  - [`contracts/status/support.md`](contracts/status/support.md)
- Copier 9.17.1 project generator for complete workspaces or namespaced CLI
  crates, with native and aligned hook selection, state capabilities,
  archetypes, and optional package-specific GitHub Actions:
  - [`copier.yml`](copier.yml)
  - [`planning/copier-hook-project-template-design.md`](planning/copier-hook-project-template-design.md)
- Runnable examples:
  - `examples/claude-sessionstart-context`
  - `examples/codex-bash-guard`
  - `examples/shared-posttool-autofix`
  - `examples/antigravity-pre-invocation`
  - [`examples/codex-claude-rules`](examples/codex-claude-rules/README.md)
  - [`examples/forbidden-file-guard`](examples/forbidden-file-guard/README.md)

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
  hookkit-session-state/
examples/
fixtures/
planning/
templates/hook-project/
```

## Prerequisites

- Rust 2024 edition toolchain (`stable` works).
- [`uv`](https://docs.astral.sh/uv/) for the repository's pinned Copier 9.17.1
  wrapper and template acceptance tests. Generated hook projects do not need
  Copier or uv at runtime.

## Generate A Hook Project

From a local checkout, run the pinned Copier version and answer the interactive
questions:

```bash
uvx --from copier==9.17.1 copier copy --vcs-ref HEAD . ../my-hooks
```

Choose a full Rust project or a CLI crate for an existing repository, then
choose single-harness native events or a supported cross-harness family set.
The questionnaire can also add state facilities, a starter archetype, and a
package-namespaced GitHub Actions workflow. Crate mode deliberately leaves the
host `Cargo.toml` unchanged; add the generated crate as a workspace member
manually when needed.

The template has no tasks or migrations, so generation does not require
`--trust`. Before using crate mode, confirm that its computed crate, answers,
and workflow paths do not already exist. Generated per-event and per-archetype
handler seams in the `src/hooks/` subdirectories are preserved across updates;
their top-level export modules and scaffold wiring remain template-owned.

After a template release is tagged, generate from its immutable public source:

```bash
uvx --from copier==9.17.1 copier copy \
  --vcs-ref <release-tag> \
  https://github.com/plx/agent-hook-kit.git ./my-hooks
```

Full-project updates use `copier update`. For a crate-mode instance, select its
namespaced answers file explicitly:

```bash
uvx --from copier==9.17.1 copier update \
  --answers-file .copier-answers.<package-name>.yml .
uvx --from copier==9.17.1 copier check-update \
  --answers-file .copier-answers.<package-name>.yml --quiet .
uvx --from copier==9.17.1 copier recopy \
  --answers-file .copier-answers.<package-name>.yml .
```

Use `update` for normal smart diffing. `recopy` deliberately regenerates the
managed tree from recorded answers and is intended for explicit answer/shape
changes; protected handler seams remain untouched. After the first successful
generated-project check, commit the resulting `Cargo.lock` so its Git or
crates.io dependency resolution is reproducible.

The current preview defaults to one immutable HookKit Git revision. The default
will switch to crates.io only after all 10 template-facing crates are published
on one compatible version train.

## Build And Test

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
cargo run -p hookkit-conformance -- --check
cargo xtask contracts check
cargo xtask contracts report --check
cargo xtask contracts verify-vendor
cargo xtask template-catalog check
templates/hook-project/tests/run.sh --validation render
```

The template acceptance wrapper stages the working-tree source outside Git,
pins Copier 9.17.1, and derives its native/aligned matrix from the canonical
catalogs. Its default lane compiles every matrix cell; the full release lane
also runs generated-project formatting, Rust 1.85 and stable checks, Clippy,
and tests:

```bash
templates/hook-project/tests/run.sh
templates/hook-project/tests/run.sh --validation full
```

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

The sealed aligned-runtime catalog is intentionally capability-based:

<!-- markdownlint-disable MD013 -->

| Supported harness set | Aligned marker families |
| --- | --- |
| Claude, Codex, Antigravity | `PreToolUse`, `PostToolUse`, `TurnCompletion` |
| Claude and Codex | `PermissionRequest`, `PreCompact`, `PostCompact`, `SessionStart`, `SessionEnd`, `SubagentStart`, `SubagentStop`, `UserPromptSubmit` |

<!-- markdownlint-enable MD013 -->

The Claude/Codex output helpers expose only the portable semantic floor. For
example, aligned `PreCompact` is observer-only even though each native arm
retains its harness-specific controls. A pair-only marker rejects Antigravity
before invoking its handler.

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
47-event matrix, handler-binding boundary, and migration notes.

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

The stateful/policy examples have their own setup, state-layout, and
limitation notes:

- [`codex-claude-rules`](examples/codex-claude-rules/README.md) lazily injects
  path-scoped files from Claude Code's user and project rules directories.
- [`forbidden-file-guard`](examples/forbidden-file-guard/README.md) selects a
  native pre-tool contract with `--harness=claude|codex|antigravity` and merges
  home/project YAML policy.

## Deferred quality tooling

The Pkl-driven immediate/deferred linting and formatting runner formerly hosted
in this repository now lives in
[Velvet Glove](https://github.com/plx/velvet-glove). HookKit retains the
lower-level protocol, runtime, session-state, tool-access, and file-activity
libraries used to build coordinated hook products.

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

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.

Third-party crate licenses used by the publishable library crates are aggregated in
[`THIRD_PARTY_LICENSES.md`](THIRD_PARTY_LICENSES.md). That file is generated by
`cargo about` and validated in CI; regenerate it locally with
`scripts/regen-licenses.sh` after changing dependencies.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual-licensed as above, without any additional terms or conditions.
