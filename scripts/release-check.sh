#!/bin/sh
set -eu

command -v uv >/dev/null 2>&1 || {
  echo "error: uv is required for the pinned Copier acceptance tests" >&2
  exit 1
}

if ! pkl --version 2>/dev/null | grep '^Pkl 0\.31\.1 ' >/dev/null; then
  echo "error: Pkl 0.31.1 is required for non-skipping runner validation" >&2
  exit 1
fi

cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo run --quiet -p hookkit-conformance -- --check
cargo xtask contracts check --snapshot current
cargo xtask contracts report --check
cargo xtask contracts verify-vendor
cargo xtask template-catalog check
templates/hook-project/tests/run.sh --validation full

cargo +1.85.0 check \
  -p hookkit-core \
  -p hookkit-claude \
  -p hookkit-codex \
  -p hookkit-antigravity \
  -p hookkit-session-state \
  -p hookkit-pkl-config \
  -p hookkit-common \
  -p hookkit-shell \
  -p hookkit-tool-access \
  -p hookkit-file-activity \
  -p hookkit-runtime \
  -p hookkit-tool-runner \
  --all-targets

for package in hookkit-core hookkit-claude hookkit-codex hookkit-antigravity hookkit-session-state hookkit-pkl-config hookkit-common hookkit-shell hookkit-tool-access hookkit-file-activity hookkit-runtime hookkit-tool-runner; do
  cargo package -p "$package" --list >/dev/null
done

# The first package in the publish order has no unpublished hookkit dependency,
# so Cargo can fully assemble it before the staged release begins.
cargo package -p hookkit-core
