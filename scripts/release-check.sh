#!/bin/sh
set -eu

cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo run --quiet -p hookkit-conformance -- --check
cargo xtask contracts check --snapshot current
cargo xtask contracts report --check
cargo xtask contracts verify-vendor

cargo +1.85.0 check \
  -p hookkit-core \
  -p hookkit-claude \
  -p hookkit-codex \
  -p hookkit-gemini \
  -p hookkit-antigravity \
  -p hookkit-common \
  -p hookkit-shell \
  -p hookkit-runtime \
  -p hookkit-pkl-config \
  -p hookkit-tool-runner \
  --all-targets

for package in hookkit-core hookkit-claude hookkit-codex hookkit-gemini hookkit-antigravity hookkit-common hookkit-shell hookkit-runtime; do
  cargo package -p "$package" --list >/dev/null
done

# The first package in the publish order has no unpublished hookkit dependency,
# so Cargo can fully assemble it before the staged release begins.
cargo package -p hookkit-core
