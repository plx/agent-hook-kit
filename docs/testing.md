# Test lanes

The required pull-request path is offline and deterministic. Tests that discover
mutable tools or live harnesses are separate compatibility and observation lanes;
their results never rewrite protocol snapshots or goldens automatically.

## Required on every pull request

### 1. Contract and metadata conformance

```sh
cargo xtask contracts check
cargo run -p hookkit-conformance -- --check
cargo xtask contracts report --check
cargo xtask contracts verify-vendor
```

It validates catalog YAML/meta-schemas, offline JSON Schema references, positive
and negative fixtures, exact stream checksums/framing, immutable snapshots, and
catalog/status consistency. The conformance executable checks that each native
crate implements the registry-selected snapshot, parses every positive input
fixture with the native parser, requires every negative input fixture to be
rejected, runs every declared Rust process case, and reconciles the
machine-generated implementation registry. The only exceptions to rejection
are the `(contract, pointer)` pairs in `OPEN_VALUE_SET_NEGATIVES`
(`crates/hookkit-conformance/src/lib.rs`): negatives that only close a
harness-sent value set (an `enum`, or a `const` on a field other than the
`/hook_event_name` discriminator) that the native crate deliberately reads as
an `Unknown` value. Those negatives must be *accepted*, which locks in the
forward compatibility, and every allowlist entry must match such a fixture in
the selected snapshot.
Workspace tests also compare production identification metadata and
command-environment selectors with the selected catalog snapshots and
supplement, and check that the repository `fixtures/` inputs decode and its
golden outputs match typed constructors.

### 2. Hermetic library, runtime, examples, and documentation

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
HOOKKIT_REQUIRE_PKL=1 cargo test --workspace --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

The workspace test command deliberately excludes tests marked `ignored`. Native
payloads, fake executables, temporary paths, and checked-in configuration are
allowed; ambient formatter/linter versions are not.

Tests that evaluate Pkl (`hookkit-pkl-config`'s `merge`, `builtins`, and
`discovery` suites, the runtime integration suite, and the tool runner's
`post_tool_e2e`, `turn_completion_e2e`, and `tool_fixtures` suites) skip
themselves when `pkl` is not on `PATH`, so a local run without Pkl still
passes. `HOOKKIT_REQUIRE_PKL=1` turns every such skip into a failure. CI's
check job installs the Pkl version pinned in
`templates/hook-project/catalog/compatibility.yml` and sets it, as does
`scripts/release-check.sh`, so Pkl-dependent coverage can never silently
disappear from the required lane.

The stdin/stdout runners are tested as real processes, because only a process
shows the bytes and exit status a harness observes:

- `crates/hookkit-runtime/tests/stdin_runners.rs` drives `run_event*`,
  `run_harness*`, and `dispatch_builtin_harness*`, including the one-line
  stderr failure report, `FailurePolicy::FailClosed` lowering, caught handler
  panics, and the diagnostics sink.
- `crates/hookkit-runtime/tests/aligned_stdin.rs` does the same for
  `run_aligned_event`, `run_aligned_event_with_diagnostics`, and
  `run_aligned_event_with_options`: fail-closed exit 2 on Claude Code and
  Codex gating families, a JSON deny on Antigravity `PreToolUse`, and
  non-blocking observers and turn completion.
- `crates/hookkit-runtime/tests/integration.rs` runs the example and runner
  binaries end to end. It finds them in the same target directory, profile,
  and target triple as the test executable, so `CARGO_TARGET_DIR`,
  `--release`, and `--target` exercise freshly built binaries.

`crates/hookkit-shell/tests/adversarial.rs` is a table-driven corpus of Bash
analysis bypasses, each tagged with its review finding, plus bounded-cost and
benign-precision checks. With `HOOKKIT_SHELL_DIFFERENTIAL=1` it also runs every
executable case under `bash` and checks that the case had the effect the
analysis must report:

```sh
HOOKKIT_SHELL_DIFFERENTIAL=1 cargo test -p hookkit-shell --test adversarial
```

### 3. Hermetic runner orchestration

```sh
cargo test -p hookkit-tool-runner --lib hermetic_fake_executable_smoke
```

This smoke creates a temporary fake checker, executes the normal tool phase,
checks clean classification and captured diagnostics, and removes its workspace.
It does not load a formatter/linter from `PATH`. Broader runtime integration tests
also use controlled fake executables when Pkl is available.

## Scheduled or opt-in

### 4. Pinned real-tool compatibility

Run in a toolchain image or environment whose manifest records every executable
and version:

```sh
cargo test -p hookkit-tool-runner --test tool_fixtures \
  -- --ignored --nocapture
```

`HOOKKIT_FIXTURE_TOOLS=jq,check-merge-conflict` limits the run to the listed
tool ids, and `HOOKKIT_FIXTURE_BLESS=1` rewrites the goldens from the installed
tools; use it only with controlled tool versions. See
[`crates/hookkit-tool-runner/tests/fixtures/README.md`](../crates/hookkit-tool-runner/tests/fixtures/README.md).
The aggregate skips missing tools and compares installed-tool output, so it is
diagnostic unless the job supplies the repository's pinned compatibility
manifest. Record OS, architecture, locale, color settings, Pkl version, tool
versions, pass/fail/skip counts, and produced artifacts. Never update goldens to
match an arbitrary developer `PATH`.

### 5. Live harness conformance

Run only in an isolated, non-sensitive temporary workspace. Record exact harness
version, platform, command event, sanitized invocation, stdout/stderr bytes, and
status. HookKit live-support observations target command bindings only. HTTP and
other non-command bindings may still appear as upstream catalog evidence, but
they are outside the implementation and live-conformance lanes for this
iteration. Store results under `contracts/status/observations/`; frozen
snapshots are immutable.

### 6. Upstream drift discovery

```sh
scripts/check-upstream-contract-drift.sh
```

The scheduled drift workflow runs this script. It reads every revision, URL,
and hash from the registry-selected snapshots and command-environment
supplement through `cargo xtask contracts upstream-sources`; supplement rows
are labeled `command-environments/<supplement-id>/<source-id>`. It checks the
vendored Codex schemas against upstream at the pinned revision and at upstream
HEAD, lists pinned source trees that changed for review, and re-hashes every
documentation URL that records a `content_sha256`. It exits 0 when nothing
drifted, 1 on drift or a vendored-evidence mismatch, and 2 when discovery
itself fails, so a broken check never looks clean. It never modifies the
checkout: a drift report is a prompt to author a successor snapshot or
supplement, never to edit the selected one.

## Failure classification

Every failure report uses one of these labels:

- protocol/library defect;
- deterministic runner defect;
- external-tool compatibility;
- missing dependency/environment; or
- stale expected output.

Required-lane failures block integration. Opt-in failures block a support or
compatibility claim only when that claim targets the affected pinned version.
