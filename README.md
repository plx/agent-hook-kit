# agent-hook-kit

Contract-first Rust plumbing for command hooks across Claude Code, Codex,
Gemini CLI, and Antigravity.

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
  - `hookkit-gemini`
  - `hookkit-antigravity`
- First-class command-hook environment models, including deterministic map-based
  parsing and automatic capture in stdin/stdout runners:
  - [`docs/command-environments.md`](docs/command-environments.md)
- Lossless cross-harness aligned event wrappers:
  - `hookkit-common`
- Opt-in, bounded Bash syntax analysis and file-access inference for native shell tool calls:
  - [`hookkit-shell`](crates/hookkit-shell/README.md)
- Loss-aware file activity evidence, pending windows, and reconciliation:
  - [`hookkit-file-activity`](crates/hookkit-file-activity/README.md)
- Concurrent, versioned session-scoped state primitives:
  - [`hookkit-session-state`](crates/hookkit-session-state/README.md)
- Runtime stdin/stdout/exit-code plumbing:
  - `hookkit-runtime`
- Pkl-driven post-tool and batched turn-completion runners with embedded tool catalog:
  - `hookkit-pkl-config` (Pkl evaluation, builtin specs, multi-file merge)
  - `hookkit-tool-runner` (ships `post-tool-use-agent-hook`,
    `turn-completion-agent-hook`, and the precise
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
  - `examples/gemini-beforetool-policy`
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
  hookkit-gemini/
  hookkit-antigravity/
  hookkit-common/
  hookkit-shell/
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
same typed contract with a non-JSON result: its output emits an absolute path as
exact plain text.

## Quick Start: Aligned `PreToolUse`

Aligned events keep native inputs and outputs intact while allowing one handler
to cover several harnesses. Pre-tool execution maps the shared name to Claude
Code `PreToolUse`, Codex `PreToolUse`, Gemini CLI `BeforeTool`, and Antigravity
`PreToolUse`:

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
  selector for the four bundled adapters.
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
56-event matrix, handler-binding boundary, and migration notes.

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

Gemini before-tool policy:

```bash
cat fixtures/gemini/before_tool.json \
  | cargo run -q -p gemini-beforetool-policy
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
  native pre-tool contract with `--harness=codex|gemini|antigravity` and merges
  home/project YAML policy.
- [`session-modified-file-tracker`](examples/session-modified-file-tracker/README.md)
  selects `--harness=claude|codex|gemini` and appends provenance-bearing,
  per-session file-activity evidence without consulting Git.

## `post-tool-use-agent-hook`

The reusable post-tool-use formatter/linter runner ships as a single binary
that selects tools and policy from Pkl configuration:

```bash
cat fixtures/claude/post_tool_use.json \
  | cargo run -q -p hookkit-tool-runner --bin post-tool-use-agent-hook -- --claude
```

The CLI accepts `--claude`, `--codex`, or `--gemini` to choose the harness,
and `--config PATH` to load a single Pkl file directly (bypassing discovery).

The companion `turn-completion-agent-hook` reconciles and consumes the
NDJSON-backed pending file-activity window at Claude/Codex `Stop` or Gemini
`AfterAgent`, runs the same configured tools over the candidate batch, and
stays quiet when everything is clean or auto-corrected:

```bash
cargo run -q -p hookkit-tool-runner --bin turn-completion-agent-hook -- \
  --claude --state-dir .context/hookkit-state
```

Use the same `--state-dir` for `session-modified-file-tracker`. Before sealing
the window, the runner scans workspace mtimes since the durable reconciliation
cursor (or the current session start on its first pass). Manual issues block
the stop attempt, retain the sealed generations and cached set for retry, and
point to detailed logs committed below the versioned session state. See the
[file-activity crate](crates/hookkit-file-activity/README.md) and
[session-state walkthrough](crates/hookkit-session-state/README.md).

For precise automatic start metadata even before another stateful hook runs,
bind the no-op observer to the harness's native `SessionStart` event:

```bash
cargo run -q -p hookkit-tool-runner --bin session-start-state-agent-hook -- \
  --claude --state-dir .context/hookkit-state
```

Codex and Gemini use `--codex` and `--gemini`. Every later
`SessionState::ensure` still refreshes typed project metadata automatically;
without a start binding, the timestamp is explicitly marked as a
first-observed fallback.

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

`hookkit-pkl-config` embeds `Builtins.pkl` with reusable specs for:

- `Builtins.ruff` — Python format/fix/verify with ruff
- `Builtins.prettier` — JS/TS/CSS/HTML/JSON/Markdown formatter
- `Builtins.eslint` — JS/TS fix + verify
- `Builtins.biome` — JS/TS/JSON fix + verify
- `Builtins.cargoFmt` — Rust workspace formatter
- `Builtins.cargoClippy` — Rust workspace fix + verify

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
| `settings.loweringPolicy` | `"best-effort-with-warnings"` | How to handle harness intents not natively expressible. |
| `settings.diagnosticsDirectory` | `".agent-hook-kit/post-tool-use"` | Where to write diagnostic artifacts. |
| `settings.missingToolPolicy` | `"user-notice"` | What to do when a configured tool executable is missing. Options: `"user-notice"`, `"hard-failure"`, `"harness-block"`. |

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

## Example Behavior Summary

- `claude-sessionstart-context`:
  - injects additional model context on `SessionStart`.
- `codex-bash-guard`:
  - inspects `PreToolUse` Bash commands and emits deny JSON for blocked patterns.
- `gemini-beforetool-policy`:
  - denies or rewrites risky tool invocations in `BeforeTool`.
- `shared-posttool-autofix`:
  - runs a formatter/linter-autofix pipeline when applicable,
  - emits each harness's exact native no-op response on clean success,
  - prints concise user status to `stderr` when autofix/manual work occurs,
  - writes verbose manual diagnostics to a temp artifact and gives concise agent guidance when supported.
- `codex-claude-rules`:
  - discovers Claude Code rule files recursively in user-before-project order,
  - evaluates `paths` frontmatter against structured paths, patch headers, and
    HookKit's parsed/inferred Bash file targets,
  - atomically claims each matched rule in session state before injecting its
    body as additional context.
- `forbidden-file-guard`:
  - uses clap to select Codex, Gemini, or Antigravity native pre-tool handling,
  - merges additive YAML glob policy from home and workspace configuration,
  - emits the selected harness's native deny output for matching structured paths, patch paths, or obvious shell path tokens.
- `session-modified-file-tracker`:
  - uses the aligned post-tool API for Claude, Codex, and Gemini,
  - infers direct modifications from native open tool payloads and never shells out to Git,
  - appends detailed observations to rotated NDJSON generations whose projection is a versioned per-session path set.
- `post-tool-use-agent-hook`:
  - loads merged Pkl config plus embedded builtin tool catalog,
  - discovers candidate paths from exact native input arms using runner-local tool policy,
  - runs each tool's phases for format/fix/verify commands,
  - classifies clean versus issues and changed versus unchanged from exit policies plus file snapshots,
  - reports missing tools and operational failures per `missingToolPolicy`,
  - writes remaining diagnostics to artifacts,
  - lowers every result through an explicit Claude, Codex, or Gemini native output arm.
- `turn-completion-agent-hook`:
  - seals the current modified-file generations under an exclusive entity consumer lock,
  - dispatches the same Pkl-configured phases across the accumulated file set,
  - commits detailed per-tool logs and a summary before producing its decision,
  - acknowledges only clean or fully auto-corrected snapshots,
  - retains manual findings for retry and emits each harness's native continue-working signal.

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
