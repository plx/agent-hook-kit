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
rejected (except one that only closes a harness-sent enumeration, which the
native crates deliberately read as an `Unknown` value), runs every declared Rust
process case, and reconciles the machine-generated implementation registry.
Workspace tests also compare production identification metadata and
command-environment selectors with the selected catalog snapshots and
supplement, and check that the repository `fixtures/` inputs decode and its
golden outputs match typed constructors.

### 2. Hermetic library, runtime, examples, and documentation

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

The workspace test command deliberately excludes tests marked `ignored`. Native
payloads, fake executables, temporary paths, and checked-in configuration are
allowed; ambient formatter/linter versions are not.

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

Scheduled retrieval compares pinned sources or deterministic normalized hashes
with a candidate snapshot. It may open a review task and generate artifacts, but
must not modify the selected snapshot or support claims automatically.

## Failure classification

Every failure report uses one of these labels:

- protocol/library defect;
- deterministic runner defect;
- external-tool compatibility;
- missing dependency/environment; or
- stale expected output.

Required-lane failures block integration. Opt-in failures block a support or
compatibility claim only when that claim targets the affected pinned version.
