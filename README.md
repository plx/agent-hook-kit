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

- Native input/output models for every command-hook event in the selected
  contract snapshots, 50 in all (see [Supported harnesses](#supported-harnesses)):
  - `hookkit-claude` (33 Claude Code events)
  - `hookkit-codex` (12 Codex events)
  - `hookkit-antigravity` (5 Antigravity events)
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
- Versioned upstream protocol ledger, generated support matrix, and per-refresh
  audits:
  - [`contracts/`](contracts/README.md)
  - [`contracts/status/support.md`](contracts/status/support.md)
  - [`planning/audits/`](planning/audits/)
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
contracts/
docs/
examples/
fixtures/
planning/
templates/hook-project/
xtask/
```

## Supported harnesses

`contracts/registry.yaml` selects one frozen contract snapshot per harness and
one command-environment supplement. The native crates implement exactly those
snapshots, and conformance fails if they drift apart.

<!-- markdownlint-disable MD013 -->

| Harness | Selected snapshot | Events | Refresh audit |
| --- | --- | --- | --- |
| Claude Code | `claude-code/docs-2026-09-29-r1` (Claude Code 2.1.285) | 33, including `PreModelSwitch` and `PostModelSwitch` | [`2026-09-29-claude-code-hooks.md`](planning/audits/2026-09-29-claude-code-hooks.md) |
| Codex | `codex/commit-ff6aec9-r1` (`rust-v0.159.2`) | 12, including `Interrupt` | [`2026-09-29-codex-hooks.md`](planning/audits/2026-09-29-codex-hooks.md) |
| Antigravity | `antigravity/docs-2026-09-29-r1` (2.0, CLI, and IDE) | 5 | [`2026-09-29-antigravity-hooks.md`](planning/audits/2026-09-29-antigravity-hooks.md) |

<!-- markdownlint-enable MD013 -->

The command-process environment is recorded separately in supplement
`command-environments-2026-09-30-r1` and summarized in
[`docs/command-environments.md`](docs/command-environments.md).

### Claude Code notes

- `PreModelSwitch` and `PostModelSwitch` live in `hookkit_claude::model_switch`.
  `hookkit_claude::events` re-exports every event and output at one path, and
  `hookkit_claude::SNAPSHOT` names the implemented snapshot.
- Claude Code reads JSON stdout on every exit code. `into_blocking_error` and
  `into_feedback_error` keep a structured response on stdout while exiting 2.
  Plain-text context that starts with `{` and ends with `}` would be parsed as
  JSON and dropped, so `text_context` sends such text as structured
  `additionalContext`.
- `PreToolUseOutput::allow()` auto-approves the call and skips the permission
  prompt. A hook that only adds context or has no objection should return
  `no_op()` or `with_context(..)`.
- `PermissionRequest` ignores exit 2; only `PermissionRequestOutput::deny(..)`
  denies. `Setup`, `InstructionsLoaded`, `Notification`, `StopFailure`,
  `SessionEnd`, and `PostCompact` discard every JSON output field except
  `terminalSequence`; builders for discarded fields are deprecated.
- `WorktreeRemove` is decided by exit code alone, so a HookKit runtime error
  (exit 1) blocks the removal while the worktree still exists. A
  `UserPromptSubmit` block reason is shown only to the user, never to Claude.
- Exit-0 stderr reaches only Claude Code's debug log. User-visible notices
  belong in `systemMessage`.

### Codex notes

- `Interrupt` (`hookkit_codex::catalog::Interrupt`) fires on the main thread
  when a turn is interrupted. Its output is empty stdout or
  `{"systemMessage": ...}` (`InterruptOutput::no_op` or `system_message`); it
  cannot block. It has no Claude Code counterpart, so it is not an aligned
  family.
- `SessionStart` reports the `fork` source, which maps to a fork session
  boundary; an unknown source parses as `Unknown` and claims no boundary.
- `no_op()` emits empty stdout on every event, which is Codex's no-op. Codex
  ignores stderr from a hook that exits 0, so user notices belong in
  `with_system_message`. Other non-zero exits, and exit 2 with blank stderr,
  fail open; Codex shows their stderr only for `PreCompact`, `PostCompact`,
  and `SessionEnd`.
- A blank (after trimming) block reason or exit-2 stderr is an emission error,
  because Codex would treat it as a failed hook and not block. Text context
  that starts with `{` or `[` is sent as structured `additionalContext`.
- `apply_patch` hooks carry the patch text in `tool_input.command`, for example
  `{"command": "*** Begin Patch\n*** Update File: src/lib.rs\n...*** End Patch\n"}`.
  Tool events report the step's own working directory in `cwd`.
- Do not configure HookKit control hooks with `async: true` (Codex ignores the
  control output of background hooks), and note that command hooks do not run
  under Codex cloud orchestration.

### Antigravity notes

- Every input carries five common fields: `conversationId`, `workspacePaths`,
  `transcriptPath`, `artifactDirectoryPath`, and the optional `modelName`
  (`model_name`). `workspacePaths` may be empty, and `PostToolUse.toolCall`
  may be absent (`Option<ToolCall>`) on IDE builds. An empty `error` means
  success; use `error_message()` or `failed()` instead of `error.is_some()`.
- Built-in file tools take PascalCase arguments, such as
  `view_file {"AbsolutePath": ...}` and `write_to_file {"TargetFile": ...}`.
- `PreToolUseOutput::allow()` auto-approves and bypasses Ask presets. There is
  no pass-through decision; `PreToolUseOutput::ask()` defers to the user's
  normal policy. Duplicate `permissionOverrides` are removed on emission.
- Stop: use `StopOutput::continue_with(reason)` or `StopOutput::allow_stop()`.
  `continue` is capped by the harness, so it is not a hard gate, and `reason`
  has a documented effect only with `continue`. Near-misses such as
  `"Continue"` are rejected at emission.
- The reference documents no exit-code semantics. Antigravity 2.0 v2.12.0 and
  later report a failing hook as an error and continue; do not assume a failing
  `PreToolUse` hook lets the tool run.

## Prerequisites

- Rust 2024 edition toolchain (`stable` works).
- [`pkl`](https://pkl-lang.org/main/current/pkl-cli/index.html) on `$PATH` —
  needed at runtime by the post-tool-use and turn-completion runners so they
  can evaluate embedded and user Pkl configs. Install with `brew install pkl`
  (macOS) or follow the upstream instructions for your platform.
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
Generated Pkl configuration is user-owned policy, so changing runner or
configuration answers requires manually reconciling the existing config.

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
will switch to crates.io only after all 12 template-facing crates are published
on one compatible version train.

## Build And Test

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
HOOKKIT_REQUIRE_PKL=1 cargo test --workspace --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo run -p hookkit-conformance -- --check
cargo xtask contracts check
cargo xtask contracts report --check
cargo xtask contracts verify-vendor
cargo xtask template-catalog check
templates/hook-project/tests/run.sh --validation render
```

[`docs/testing.md`](docs/testing.md) describes every lane, including the
process-level runner tests, the opt-in real-tool and Bash-differential lanes,
and the upstream drift check.

The template acceptance wrapper stages the working-tree source outside Git,
pins Copier 9.17.1, and derives its native/aligned matrix from the canonical
catalogs. Its default lane compiles every matrix cell; the full release lane
also runs generated-project formatting, Rust 1.85 and stable checks, Clippy,
and tests:

```bash
templates/hook-project/tests/run.sh
templates/hook-project/tests/run.sh --validation full
```

Tests that need Pkl (in `hookkit-pkl-config`, `hookkit-tool-runner`, and the
runtime integration suite) skip themselves when `pkl` is not on `$PATH`, so
the suite still passes without Pkl installed. `HOOKKIT_REQUIRE_PKL=1` turns
every such skip into a failure; CI and `scripts/release-check.sh` set it. For
the release lane, run `templates/hook-project/tests/run.sh --validation full`
with `CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback`, as
`scripts/release-check.sh` does.

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
exact plain text. Claude Code reads the last non-empty stdout line, resolves a
relative path against the hook's working directory, and refuses an absolute
path with `.` or `..` segments, so `WorktreeCreateOutput::path` rejects those
paths and any line break or escape character up front.

## Quick Start: Aligned `PreToolUse`

Aligned events keep native inputs and outputs intact while allowing one handler
to cover several harnesses. All three supported harnesses expose the aligned
pre-tool event as `PreToolUse`:

```rust
use hookkit_common::PreToolUseOutput;
use hookkit_core::HarnessId;
use hookkit_runtime::RunOptions;
use hookkit_runtime::aligned::{PreToolUse, run_aligned_event_with_options};

fn main() -> std::process::ExitCode {
    let harness = HarnessId::CODEX; // select from trusted configuration or CLI input
    // A guard should deny, not fail open, when it cannot decide.
    let options = RunOptions::new().fail_closed();
    run_aligned_event_with_options::<PreToolUse, _>(
        harness,
        options,
        |input, environment, _context| {
            assert_eq!(input.harness(), environment.harness());
            if input.tool_name() == Some("dangerous_tool") {
                PreToolUseOutput::deny(&input.harness(), "blocked by policy")
            } else {
                // "No objection": leave the call to the harness's own policy.
                PreToolUseOutput::pass_through(&input.harness())
            }
        },
    )
}
```

The aligned helpers are not interchangeable, because the harnesses give
"allow" and "no answer" different meanings:

<!-- markdownlint-disable MD013 -->

| Helper | Claude Code | Codex | Antigravity |
| --- | --- | --- | --- |
| `pass_through` | `{}`: the normal permission flow decides | empty stdout: the normal approval flow decides | `{"decision":"ask"}`: prompts unless an Always Allow grant covers the call |
| `allow` | `permissionDecision: "allow"`: explicit auto-approval, no prompt | no equivalent; lowers to empty stdout (`explicit_allow_supported` returns `false`) | `{"decision":"allow"}`: auto-approves and bypasses Ask presets |
| `deny` | `permissionDecision: "deny"`, reason shown to Claude | `permissionDecision: "deny"` | `{"decision":"deny"}` with the reason |
| `rewrite` | `updatedInput` with the caller's `RewriteApproval` (`AutoApprove` or `Ask(reason)`) | `updatedInput`; approval policy still runs | unsupported |

<!-- markdownlint-enable MD013 -->

A guard that objects only to some calls must answer every other call with
`pass_through`; answering with `allow` turns it into an auto-approver on Claude
Code and Antigravity. Every deny and block helper rejects a reason that is
empty after trimming, on every harness, because Codex would otherwise ignore
the denial. The convenience constructors return a concrete native enum arm.
Callers can instead match `PreToolUseInput` and construct any native-only
output capability available to that arm.

The sealed aligned-runtime catalog is intentionally capability-based:

<!-- markdownlint-disable MD013 -->

| Supported harness set | Aligned marker families |
| --- | --- |
| Claude, Codex, Antigravity | `PreToolUse`, `PostToolUse`, `TurnCompletion` |
| Claude and Codex | `PermissionRequest`, `PreCompact`, `PostCompact`, `SessionStart`, `SessionEnd`, `SubagentStart`, `SubagentStop`, `UserPromptSubmit` |

<!-- markdownlint-enable MD013 -->

The output helpers cover what the harnesses genuinely share and document
where they still differ; the `hookkit_common::aligned` module docs are the
reference. For example, aligned `PreCompact` is observer-only even though each
native arm retains its harness-specific controls;
`PostCompactOutput::with_system_notice` is a documented no-op on Claude Code,
which discards `PostCompact` `systemMessage` (see `system_notice_delivered`);
and a `UserPromptSubmitOutput::block` reason is shown to the Claude Code user
only. A pair-only marker, or any harness without an aligned adapter, fails
with `HookkitError::UnsupportedHarness` before the payload is parsed.

Aligned inputs report locations the way the native payloads do.
`workspace_roots()` is `[cwd]` on Claude Code and Codex and `workspacePaths`
on Antigravity. Claude Code's `cwd` follows `cd` in the Bash tool, so use
`project_roots(environment)` (Claude Code's `CLAUDE_PROJECT_DIR`, Codex's
`cwd`, Antigravity's `workspacePaths`) for configuration discovery and
project-relative matching, and `cwd()` to resolve relative operands.

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
                _ => Err(hookkit_core::HookkitError::handler(
                    "selected harness changed",
                )),
            }
        },
    )
}
```

For a multi-harness executable, select a `HarnessId` from trusted CLI or
configuration input and match every supported `PostToolUseInput` arm. There is
no universal serialized output envelope: each arm constructs the native
response its harness actually supports. See `examples/shared-posttool-autofix`
for the complete pattern. Return application failures with
`HookkitError::handler(error)`, which keeps the error typed and downcastable,
rather than wrapping them in `std::io::Error`.

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

Each stdin runner has a `_with_options` variant (`run_event_with_options`,
`run_harness_with_options`, `dispatch_builtin_harness_with_options`, and
`run_aligned_event_with_options`) that takes `RunOptions`: a failure policy
and a diagnostics sink. See [Failure policy](#failure-policy).

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
casing, recursively mine arbitrary JSON, or invent a current directory. Its
workspace roots are the input `cwd` on Claude Code and Codex (Claude's moves
with `cd`; `CLAUDE_PROJECT_DIR` in the command environment is the stable root)
and `workspacePaths` on Antigravity, which may be empty.

The `run_*` adapters capture only the environment names declared by the selected
harness type. In-memory `execute_*` APIs instead accept an explicit
`EnvironmentVariables` map, keeping tests deterministic and free of
process-global environment mutation. See the
[command-hook environment reference](docs/command-environments.md) for the full
50-event matrix, handler-binding boundary, and migration notes.

### Diagnostics and process streams

Protocol emission and application diagnostics are separate:

- `stdout` contains only the exact command response bytes produced by the event
  contract. Empty bytes, `{}`, JSON `null`, and text are intentionally distinct,
  and the runtime does not append a newline.
- Protocol-defined `stderr` and exit status are constructed by event-specific
  output APIs, such as `with_protocol_stderr` or a blocking-error constructor.
- Runtime and handler diagnostics go to a `DiagnosticsSink`. The convenience
  runners use `DISABLED_DIAGNOSTICS`; pass a sink through
  `RunOptions::with_diagnostics` to a `_with_options` runner, or use
  `run_event_with_diagnostics`, `run_aligned_event_with_diagnostics`,
  `execute_harness_with_diagnostics`,
  `execute_builtin_harness_with_diagnostics`, or
  `execute_aligned_event_with_diagnostics`, when the application needs
  out-of-band records. Every runner failure, including a caught handler panic,
  is also recorded there.
- When a stdin runner cannot produce the handler's response (stdin is
  unreadable, the environment or payload is invalid, the handler returns an
  error or panics, or the output cannot be emitted), it writes exactly one
  stderr line, `hookkit: <program> <hook> failed: <cause chain>`, never writes
  partial stdout, and then responds according to its failure policy.

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

Diagnostic artifacts written through `hookkit_runtime::artifacts` default to
the per-user `$TMPDIR/hookkit-artifacts-<uid>` directory (owner-only; a
directory owned by another user, or a symlink, is refused). Artifacts are
replaced atomically with owner-only permissions, and their filenames have four
percent-encoded fields (session, turn, tool call, label).

### Failure policy

By default (`FailurePolicy::NonBlocking`) a failed runner exits 1 with empty
stdout. Claude Code and Codex treat exit 1 as a non-blocking hook error: the
pending action proceeds, and Claude Code shows the first stderr line to the
user. A policy hook therefore fails *open* by default. Guards should opt into
`FailurePolicy::FailClosed`:

```rust
use hookkit_codex::protocol::{PreToolUse, PreToolUseOutput};
use hookkit_runtime::{RunOptions, run_event_with_options};

fn main() -> std::process::ExitCode {
    run_event_with_options::<PreToolUse, _>(
        RunOptions::new().fail_closed(),
        |_input, _environment, _context| Ok(PreToolUseOutput::no_op()),
    )
}
```

Under `FailClosed`, `hookkit_runtime::failure::failure_response` lowers a
failure to the harness's native blocking response wherever the event gates a
pending action:

<!-- markdownlint-disable MD013 -->

| Harness | Events | Fail-closed response |
| --- | --- | --- |
| Claude Code | `PreToolUse`, `UserPromptSubmit`, `UserPromptExpansion`, `PostToolBatch`, `PreCompact`, `PreModelSwitch`, `ConfigChange`, `Elicitation`, `ElicitationResult`, `TaskCreated`, `WorktreeCreate`, `WorktreeRemove` | exit 2, diagnostic on stderr |
| Claude Code | `PermissionRequest` (ignores exit 2) | exit 0 with a JSON `behavior: "deny"` decision |
| Codex | `PreToolUse`, `PermissionRequest`, `UserPromptSubmit` | exit 2, diagnostic on stderr |
| Antigravity | `PreToolUse` | exit 0 with `{"decision":"deny","reason":...}` |
| any | observers and Stop-like events | exit 1, as for `NonBlocking` |

<!-- markdownlint-enable MD013 -->

When the payload cannot be identified at all, Claude Code and Codex exit 2 and
Antigravity receives a JSON deny. Runners outside the typed, selected, and
aligned families can reuse the same lowering through
`hookkit_runtime::failure::report_run_failure`.

## Run The Examples With Fixtures

Claude Code hooks require the baseline command environment that Claude Code
exports to every hook (see
[`docs/command-environments.md`](docs/command-environments.md)), and the
session ID must match the payload's `session_id`. Export it once for the
Claude examples below:

```bash
export CLAUDECODE=1 CLAUDE_CODE_CHILD_SESSION=1 \
  CLAUDE_CODE_SESSION_ID=abc-123-def CLAUDE_PROJECT_DIR=/home/user/project
```

Claude session-start context (agent instructions in `additionalContext`, a
user note in `systemMessage`):

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

Shared post-tool autofix (aligned runtime; prints Claude's `{}` no-op because
the fixture's file does not exist locally):

```bash
cat fixtures/claude/post_tool_use.json \
  | cargo run -q -p shared-posttool-autofix -- --claude
```

Codex and Antigravity payloads need no environment. Codex's no-op is empty
stdout, so a Codex hook with nothing to say prints nothing.

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
that selects tools and policy from Pkl configuration. With the Claude
environment exported as in the previous section:

```bash
cat fixtures/claude/post_tool_use.json \
  | cargo run -q -p hookkit-tool-runner --bin post-tool-use-agent-hook -- --claude
```

The CLI accepts `--claude`, `--codex`, `--antigravity`, or
`--harness=claude|codex|antigravity` to choose the harness, and `--config PATH`
to load a single Pkl file directly (bypassing discovery). A relative
`--config` resolves against the harness project root (`CLAUDE_PROJECT_DIR` on
Claude Code, otherwise the input's first workspace root), not the hook's
current directory. A usage error prints a diagnostic naming the binary and
exits 1, never the harness-blocking exit 2; `--help` exits 0.

A tool call that wrote no existing file returns the native no-op before any Pkl
is evaluated. Otherwise the runner lowers its results to the native channels:
user notices go to the top-level `systemMessage` on Claude Code and Codex
(truncated to Claude's 10,000-character cap with a pointer to the diagnostics
artifact), agent feedback to `hookSpecificOutput.additionalContext`, and
`missingToolPolicy = "harness-block"` to an exit-0 `decision: "block"` that
keeps earlier tools' notices and feedback. Neither harness shows stderr from a
hook that exits 0, so the runner never uses it for notices on those harnesses.
A failure of the run itself (`missingToolPolicy = "hard-failure"`, or a
configuration that cannot be loaded, including a missing `pkl`) stops the run
and is reported the same way, as an `error:` notice leading `systemMessage`
with earlier tools' feedback kept in `additionalContext`: a failed PostToolUse
hook's stderr never reaches the agent, Claude shows the user only its first
line, and Codex drops it. On Antigravity, which has no channel, it is a hook
error. Full tool output stays in the diagnostics artifacts.

Tool commands never outlive the hook. Each runs in a process group whose
watchdog kills it when the hook process dies, including when Codex SIGKILLs
the hook's own process group on a hook timeout or turn interrupt;
`settings.commandTimeoutSeconds` bounds each command and
`settings.runTimeoutSeconds` bounds all of them together.

The companion `turn-completion-agent-hook` reconciles and consumes the
NDJSON-backed pending file-activity window at each supported harness's `Stop`
event. For each matching deferred workflow it runs a read-only check,
runs one remedy only when that check reports source issues, and reruns every
check invalidated by observed writes. It allows completion after clean or
fully auto-fixed results while emitting the configured deferred report through
each harness's native channels:

```bash
cat fixtures/claude/stop.json \
  | cargo run -q -p hookkit-tool-runner --bin turn-completion-agent-hook -- \
      --claude --state-dir "${TMPDIR:-/tmp}/hookkit-demo-state"
```

(These fixture runs use the Claude environment exported above and an absolute
state directory, because the fixtures' placeholder project directory does not
exist.) Use the same `--state-dir` for `file-activity-agent-hook`. A relative
`--state-dir` resolves against the harness project root (`CLAUDE_PROJECT_DIR`
on Claude Code, otherwise the input's first workspace root), never the hook's
current directory, which Claude Code moves with `cd`. On Claude Code both
runners work in the session's working root: `CLAUDE_PROJECT_DIR`, or the
linked Git worktree of the same repository that the session entered (Claude
keeps `CLAUDE_PROJECT_DIR` at the main checkout while `cwd` follows the
session into the worktree). Linked worktrees below that root, such as other
sessions' `.claude/worktrees/*`, are excluded from mtime and Git-dirty
reconciliation. Before sealing the window, the runner scans workspace mtimes
since the durable reconciliation cursor (or the current session start on its
first pass). It commits artifacts and `summary.json` (schema version 2), requeues
only manual and operationally incomplete files and targets the traversal
budget never reached, records handled fingerprints for every discharged file
(clean, auto-fixed, deleted, uncovered, or left unchecked by a missing tool),
and then acknowledges the sealed source generations. Manual issues block the
stop attempt and point to those committed logs. The summary's `status` is
`operational-failure`, `issues`, `incomplete` (a tool was unavailable or
coverage has gaps, so some files were not checked), `not-applicable`, or
`clean`, most severe first.

Stop semantics are designed not to trap the agent:

- Coverage gaps and unresolvable targets are reported once, in the Stop that
  first sees them, and are not re-queued; `coverageGapPolicy = "strict"` blocks
  only that Stop. A reconciliation gap that reconciliation observes again at
  every Stop (an unreadable directory, a scan that hit `maxEntries`, a root
  that is not a Git repository) is reported once per session.
- Under the default `missingToolPolicy = "user-notice"`, a missing tool is
  reported to the user without blocking or re-queueing its files. A missing,
  unrunnable, or timed-out `pkl`, or an unreadable configuration file, does
  not block either; a configuration that fails to evaluate does.
- `failFast` skips only the failing tool's remaining remedies.
- A block that would repeat unchanged (same manual-fix file contents,
  operational problems, and strict gaps) is allowed to stop, with a note to the
  user, when `stop_hook_active` marks a continuation (Claude Code, Codex), and
  always on Antigravity, which has no such signal.
- After eight consecutive blocked attempts in one continuation chain, the next
  Stop is allowed with a note, matching Claude Code's own continuation cap on
  Codex and Antigravity, which have none.
- An Antigravity Stop whose `terminationReason` is `max_steps_exceeded` or
  `error` is allowed without running any tool, so `decision: "continue"` never
  re-enters a loop the harness is ending; the work stays queued.

See the
[file-activity crate](crates/hookkit-file-activity/README.md) and
[session-state walkthrough](crates/hookkit-session-state/README.md).

For precise automatic start metadata even before another stateful hook runs,
bind the no-op observer to the harness's native `SessionStart` event:

```bash
cat fixtures/claude/session_start.json \
  | cargo run -q -p hookkit-tool-runner \
      --bin session-start-state-agent-hook -- \
      --claude --state-dir "${TMPDIR:-/tmp}/hookkit-demo-state"
```

Codex uses `--codex`. Every later
`SessionState::ensure` still refreshes typed project metadata automatically;
without a start binding, the timestamp is explicitly marked as a
first-observed fallback.

### Deferred hook suite

A complete deferred installation binds these shipped executables to one shared
state root:

<!-- markdownlint-disable MD013 -->

| Purpose | Claude | Codex | Antigravity |
| --- | --- | --- | --- |
| Precise session lower bound | `SessionStart` → `session-start-state-agent-hook` | `SessionStart` → `session-start-state-agent-hook` | unavailable |
| File-activity producer | `PostToolUse` and `PostToolUseFailure` → `file-activity-agent-hook` | `PostToolUse` → `file-activity-agent-hook` | `PostToolUse` → `file-activity-agent-hook` |
| Deferred consumer | `Stop` → `turn-completion-agent-hook` | `Stop` → `turn-completion-agent-hook` | `Stop` → `turn-completion-agent-hook` |

<!-- markdownlint-enable MD013 -->

On Claude Code, bind `file-activity-agent-hook` to `PostToolUseFailure` as well
as `PostToolUse`: Claude reports a failed tool call only there, even when it
wrote files first (for example `sed -i ... && pytest` with failing tests).
The observer handles both events and treats any other Claude event as a
non-blocking hook error.

For example, every registered command below must use the same path (here
relative, so it resolves against each session's project root):

```bash
session-start-state-agent-hook --claude --state-dir .context/hookkit-state
file-activity-agent-hook --claude --state-dir .context/hookkit-state
turn-completion-agent-hook --claude --state-dir .context/hookkit-state
```

Use the matching selector consistently for each harness. Every runner also
accepts `--harness=claude|codex|antigravity` (the session-start observer only
`claude|codex`); the three stateful hooks accept `--state-dir=PATH`, and turn
completion accepts `--config PATH`. Antigravity has no precise session-start
binding, but its PostToolUse tool-call arguments (PascalCase, for example
`write_to_file {"TargetFile": ...}`) provide direct activity evidence; Codex
`apply_patch` evidence comes from the patch text in `tool_input.command`.
Filesystem-mtime and optional Git-dirty reconciliation remain fallbacks for
unresolved accesses.

### Configuration discovery

When `--config` is not used the runner loads, in order:

1. `~/.agent-hook-kit/post-tool-use.pkl` (home/global)
2. each `<ancestor>/.agent-hook-kit/post-tool-use.pkl` walking up from the
   start directory (root → leaf)
3. each `<ancestor>/.agent-hook-kit/post-tool-use.local.pkl` walking up from
   the start directory (root → leaf, intended to be `.gitignore`d)

The start directory is the input's first workspace root (Codex `cwd`,
Antigravity's first `workspacePaths` entry, or, for an Antigravity
conversation without a workspace, the deepest directory containing the written
files). On Claude Code both runners start from the session's working root,
`CLAUDE_PROJECT_DIR` or the linked worktree the session entered, never the
`cwd` that follows `cd`. The home directory's own file is only
the home layer, and the project root is the deepest directory holding a
project or local config (never `$HOME`). Later files override earlier ones;
`settings.fileActivity` and `settings.deferredReporting` merge field by field.
A file can opt out of earlier state with `merge { resetAll = true }`,
`merge { reset = new Listing { "tools"; "run" } }`, or
`merge { resetTools = new Listing { "ruff" } }`. All layers are evaluated by
one `pkl` process in a private staging directory, and Pkl errors name the real
configuration file (frames from the bundled schema name
`<hookkit embedded builtins>/...`). A `pkl eval` that runs longer than 60
seconds, for example on an import stalled on the network, is killed. Imports
that climb above a config's directory (`../`) are not supported; sibling
imports are.

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
`go mod tidy -diff`; yq uses a per-file comparator; and jq validates each file
in its own invocation, because `jq empty a.json b.json` parses its files as one
concatenated JSON stream. A phase's `invocation = "per-file"` runs it once per
file in both runners. No enabled builtin relies on an unchecked mutator-first
fallback.

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

<!-- markdownlint-disable MD013 -->

| Field | Default | Purpose |
| --- | --- | --- |
| `settings.jobs` | `0` (auto) | Max independent per-workspace jobs to run concurrently. `1` (or `0`/auto, currently serial) runs jobs sequentially; `>= 2` runs up to that many at once, capped at the job count. |
| `settings.failFast` | `true` | After an operational failure, the immediate runner stops running later tools; the Stop runner skips only the failing tool's remaining remedies. |
| `settings.continueAfterIssues` | `true` | Keep running later tools after source issues. |
| `settings.exclude` | `["**/.git/**", "**/node_modules/**"]` | Global file exclusions, matched against project-relative paths, applied before per-tool filters. Setting it replaces the default. |
| `settings.loweringPolicy` | `"best-effort-with-warnings"` | How to handle a nonempty user/agent message that the selected native event cannot represent faithfully: fail, omit, or omit with a native-channel warning. Deferred Stop text that comes only from the built-in templates is omitted silently under every policy. |
| `settings.diagnosticsDirectory` | `".agent-hook-kit/post-tool-use"` | Where the immediate runner writes diagnostic artifacts (relative to the project root). The Stop runner writes into session state instead. |
| `settings.missingToolPolicy` | `"user-notice"` | What to do when a configured tool executable is missing: `"user-notice"` reports it (at Stop without blocking or re-queueing its files), `"hard-failure"` stops the run and reports an error (through `systemMessage` on Claude Code and Codex, as a hook error on Antigravity; at Stop it blocks), and `"harness-block"` blocks through the native decision. |
| `settings.commandTimeoutSeconds` | `300` | Deadline for each external tool command; `0` disables it. A command that runs longer is killed with its whole process group and reported as an operational failure. It bounds one command; see `runTimeoutSeconds` for the total. |
| `settings.runTimeoutSeconds` | `540` | Budget for all external tool commands of one hook invocation together; `0` disables it. Each command gets the smaller of `commandTimeoutSeconds` and the budget left, and a command that would start after the budget is spent is reported as an operational failure instead of running. The default fits Claude Code's and Codex's 600-second default hook timeout; lower both deadlines below any shorter hook timeout you configure (Antigravity defaults to 30 seconds). |
| `settings.fileActivity.filesystemMtime` | `true` | Reconcile mtime evidence through a durable cutoff before each Stop. |
| `settings.fileActivity.vcs` | `"disabled"` | Optional `"git-dirty"` fallback; broad because it cannot identify which dirty changes came from the agent. |
| `settings.fileActivity.timestampToleranceMillis` | `2000` | Clock-resolution slack subtracted from every mtime scan's lower bound. |
| `settings.fileActivity.maxEntries` | `100000` | Bound scoped/workspace traversal; targets never reached are retried at the next Stop. |
| `settings.fileActivity.coverageGapPolicy` | `"best-effort"` | Gaps are reported once, in the Stop that first sees them; a gap reconciliation observes at every Stop is reported once per session. `"best-effort"` reports without blocking; `"strict"` also blocks that Stop. |
| `settings.fileActivity.ignoredDirectoryNames` | 23 names (`.agent-hook-kit`, `.context`, `.git`, `node_modules`, `target`, `.venv`, `__pycache__`, ...) | Directory basenames pruned from reconciliation, target resolution, and snapshot walks. Setting it replaces the whole default list; write-attribution snapshots always also prune `.agent-hook-kit`, `.git`, `.hg`, `.jj`, `.svn`, `node_modules`, and `target`. |

<!-- markdownlint-enable MD013 -->

Tool commands run with null stdin, and batch invocations are split below a
conservative argument-size budget so large candidate sets never fail with
`E2BIG`.

For immediate Antigravity `PostToolUse`, warning-mode losses are stored as
collision-safe JSON records under the input's exact `artifactDirectoryPath`
(a leading `~/`, as in the official examples, is expanded; any other relative
path is refused). The successful hook keeps stdout exactly `{}` and points to
the record on stderr; a persistence failure fails the hook. Plain
`best-effort` stays quiet.

### Deferred reporting templates

`settings.deferredReporting` controls only the session-batched
`turn-completion-agent-hook` report. Its ordered `groups` assign the first
matching file group, then fall back to `other`. The `clean`, `autoFixed`,
`manualFixesNeeded`, `operationalError`, and `unavailableTool` fields each
contain `user` and `agent` MiniJinja templates; `masterUser` and
`masterAgent` combine the rendered nonempty buckets, which they receive in
bucket order as `rendered_bucket_lists.user` and `rendered_bucket_lists.agent`.
`unavailableTool` renders when a configured tool's executable is missing under
`missingToolPolicy = "user-notice"`, so a custom master template that joins
the bucket lists still reports tools that never ran. Set
`renderEmptyBuckets = true` to render empty buckets too, or use an empty
template to suppress one audience.

Templates receive run paths, counts, typed file/report/artifact records,
ordered groups, operational problems, coverage gaps, and `unavailable_tools`
(each with `message`, `toolId`, `toolName`, `executable`, `installHint`, and
`affectedFiles`). They also receive `artifact_paths`, `artifact_contents`, raw
`buckets`, and `rendered_buckets` as independent views. Paths stored in file
`displayPath` are project-relative when possible. Reporting syntax is
validated before any configured tool runs; a later rendering error is retained
as a durable operational artifact.

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
commands, incomplete tool arguments, changes outside supplied workspaces, and
timestamp limitations can create coverage gaps, which are reported once.
The summary distinguishes uncovered, not-applicable, unresolved, truncated,
manual, and operational outcomes rather than calling them clean.

Deferred Stop lowering uses the exact native fields below:

<!-- markdownlint-disable MD013 -->

| Harness/event | Allowed user | Allowed agent | Blocked user | Blocked agent |
| --- | --- | --- | --- | --- |
| Claude `Stop` | `systemMessage` | unavailable | `systemMessage` | `reason` |
| Codex `Stop` | `systemMessage` | unavailable | `systemMessage` | `reason` |
| Antigravity `Stop` | unavailable | unavailable | unavailable | `reason` (with `decision: "continue"`) |

<!-- markdownlint-enable MD013 -->

Claude's Stop `hookSpecificOutput.additionalContext` would continue the turn,
so it is not an allowed-completion channel, and a blocked Claude Stop sends the
agent text once, as `reason`. Every block carries a non-empty reason: when the
rendered agent message is empty, the runner synthesizes one that points at
`summary.json` and records the agent audience as `synthesized`.

`strict` fails the hook if a configured audience is unavailable. A blocked
Stop that fails this way keeps its work pending, since the block was never
delivered; an allowed Stop ends the turn anyway, so its results are recorded
and not re-run at every later Stop. `best-effort` omits the audience.
`best-effort-with-warnings` adds an omission warning to a representable native
user channel, or to Antigravity's blocked `reason`; an allowed Antigravity
stop has no channel, so its warnings are recorded only in the summary
(`warningsDelivered: false`). The summary records each audience disposition
and any warning. An unrepresentable allowed-stop agent note never turns a
successful result into a block under either best-effort policy.

These policies govern configured templates. An audience rendered entirely
from the built-in templates, such as the default "re-read changed files" agent
text on an allowed Stop or any user text on Antigravity, is omitted without a
warning or a strict failure where the Stop has no channel for it (recorded as
`omitted-builtin`). Customizing any template of an audience makes it
configured.

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
  - puts agent instructions in `additionalContext` and a user note in
    `systemMessage` on `SessionStart`.
- `codex-bash-guard`:
  - parses `PreToolUse` Bash commands with `hookkit-shell` and matches command
    names, flags, and operands (including wrappers, `bash -c`, and `eval`),
  - emits deny JSON for blocked commands and fails closed.
- `shared-posttool-autofix`:
  - triggers on edits, including Codex `apply_patch` and Claude
    `MultiEdit`/`NotebookEdit`, and runs a formatter/linter-autofix pipeline,
  - emits each harness's exact native no-op response on clean success,
  - decides "auto-fixed" from before/after content digests and tells the agent
    which files were rewritten,
  - reports user status through `systemMessage` (exit-0 stderr reaches only
    Claude's debug log and is discarded by Codex),
  - writes verbose manual diagnostics to an artifact keyed by session and tool
    call and gives concise agent guidance when supported.
- `codex-claude-rules`:
  - discovers Claude Code rule files recursively in user-before-project order,
  - evaluates `paths` frontmatter against materialized structured, patch, and
    shell access targets from `hookkit-tool-access`,
  - atomically claims each matched rule in session state before injecting its
    body as additional context.
- `forbidden-file-guard`:
  - uses one aligned handler for Claude, Codex, and Antigravity pre-tool events,
  - merges additive YAML glob policy from home and workspace configuration,
    resolved against the stable project root as well as the current `cwd`,
  - applies inspect-known, deny-unresolved, and deny-all-shell postures (OR-ed
    across settings and layers) to bounded structured, patch, and shell access
    evidence,
  - answers non-matches with the aligned pass-through, never `allow`, and
    fails closed.
- `file-activity-agent-hook`:
  - uses the aligned post-tool API for Codex and Antigravity and the selected
    Claude Code harness API for Claude's `PostToolUse` and
    `PostToolUseFailure` (bind it to both),
  - delegates structured writers, patches, and shell inference to the shared tool-access analyzer and never shells out to Git,
  - appends detailed observations to rotated NDJSON generations whose projection is a versioned per-session path set.
- `post-tool-use-agent-hook`:
  - loads merged Pkl config plus embedded builtin tool catalog,
  - discovers exact candidate paths through the shared file-activity/tool-access analyzer,
  - runs each tool's phases for format/fix/verify commands,
  - classifies clean versus issues and changed versus unchanged from exit policies plus file snapshots,
  - reports missing tools and operational failures per `missingToolPolicy`,
    and run failures (including configuration errors) through `systemMessage`
    rather than an exit-1 stderr the harness would drop,
  - writes remaining diagnostics to artifacts and user notices to
    `systemMessage`,
  - runs every tool in a process group that cannot outlive the hook, bounded
    per command and per invocation,
  - lowers every result through an explicit Claude, Codex, or Antigravity native output arm.
- `turn-completion-agent-hook`:
  - seals the current modified-file generations under an exclusive entity consumer lock,
  - dispatches Pkl-configured check/conditional-remedy/final-check workflows across the accumulated file set,
  - commits detailed per-tool logs and a summary before producing its decision,
  - acknowledges the sealed window after requeueing only unfinished work and recording handled baselines for discharged files,
  - emits configured clean/auto reports without blocking and uses each harness's native continue-working signal for manual or operational results,
  - reports coverage gaps once (persistent reconciliation gaps once per
    session), lets an unchanged repeat block stop on a continuation, and stops
    blocking after eight consecutive blocked attempts, so it cannot trap the
    agent in a Stop loop.

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
