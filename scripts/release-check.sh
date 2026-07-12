#!/bin/sh
set -eu

cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo run --quiet -p hookkit-conformance -- --check
cargo xtask contracts check --snapshot current
cargo xtask contracts report --check

for package in hookkit-core hookkit-claude hookkit-codex hookkit-gemini hookkit-antigravity hookkit-common hookkit-runtime; do
  cargo package -p "$package" --no-verify
done
