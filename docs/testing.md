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
catalog/status consistency. The conformance executable runs every declared Rust
case and reconciles the machine-generated implementation registry. Workspace
tests also compare production identification metadata with the selected catalog
snapshots.

### 2. Hermetic library, runtime, examples, and documentation

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
cargo doc --workspace --no-deps
```

The workspace test command deliberately excludes tests marked `ignored`. Native
payloads, fake executables, temporary paths, and checked-in configuration are
allowed; ambient formatter/linter versions are not.

## Scheduled or opt-in

### 3. Live harness conformance

Run only in an isolated, non-sensitive temporary workspace. Record exact harness
version, platform, command event, sanitized invocation, stdout/stderr bytes, and
status. HookKit live-support observations target command bindings only. HTTP and
other non-command bindings may still appear as upstream catalog evidence, but
they are outside the implementation and live-conformance lanes for this
iteration. Store results under `contracts/status/observations/`; frozen
snapshots are immutable.

### 4. Upstream drift discovery

Scheduled retrieval compares pinned sources or deterministic normalized hashes
with a candidate snapshot. It may open a review task and generate artifacts, but
must not modify the selected snapshot or support claims automatically.

## Failure classification

Every failure report uses one of these labels:

- protocol/library defect;
- missing dependency/environment; or
- stale expected output.

Required-lane failures block integration. Opt-in failures block a support or
compatibility claim only when that claim targets the affected pinned version.
