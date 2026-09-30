#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

# The template compatibility catalog is the single source for toolchain
# versions; `cargo xtask template-catalog check` keeps literal copies in sync.
compatibility=templates/hook-project/catalog/compatibility.yml
catalog_value() {
  local value
  value="$(sed -n "s/^$1: *//p" "$compatibility")"
  if [[ -z "$value" ]]; then
    echo "error: $compatibility has no $1" >&2
    exit 1
  fi
  printf '%s\n' "$value"
}
rust_msrv="$(catalog_value rust_msrv)"
pkl_version="$(catalog_value pkl_version)"
hookkit_revision="$(catalog_value '  git_revision')"

command -v uv >/dev/null 2>&1 || {
  echo "error: uv is required for the pinned Copier acceptance tests" >&2
  exit 1
}

if ! pkl --version 2>/dev/null | grep -F "Pkl $pkl_version " >/dev/null; then
  echo "error: Pkl $pkl_version is required for non-skipping runner validation" >&2
  exit 1
fi
# Pkl-dependent tests fail instead of skipping when Pkl is unexpectedly absent.
export HOOKKIT_REQUIRE_PKL=1

if ! git merge-base --is-ancestor "$hookkit_revision" HEAD; then
  echo "error: the template's pinned HookKit revision $hookkit_revision is not an ancestor of HEAD" >&2
  exit 1
fi

cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

# The generators must reproduce the committed catalog exactly.
contracts_before="$(git status --porcelain --untracked-files=all -- contracts)"
cargo run --quiet -p hookkit-conformance
cargo run --quiet -p xtask --bin generate-claude-contracts
cargo run --quiet -p xtask --bin generate-codex-contracts
if [[ "$(git status --porcelain --untracked-files=all -- contracts)" != "$contracts_before" ]]; then
  git status --porcelain --untracked-files=all -- contracts
  echo "error: contract generators changed files under contracts/" >&2
  exit 1
fi
cargo run --quiet -p hookkit-conformance -- --check
cargo xtask contracts check
cargo xtask contracts report --check
cargo xtask contracts verify-vendor
cargo xtask template-catalog check

licenses_before="$(git hash-object THIRD_PARTY_LICENSES.md)"
scripts/regen-licenses.sh
if [[ "$(git hash-object THIRD_PARTY_LICENSES.md)" != "$licenses_before" ]]; then
  echo "error: THIRD_PARTY_LICENSES.md was stale; review and commit the regenerated file" >&2
  exit 1
fi

# Generated projects use resolver "3", but crate-mode cells also render into a
# host workspace that still uses resolver "2", as CI's template job does.
CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback \
  templates/hook-project/tests/run.sh --validation full

# Git-mode projects, the template default, build against the pinned revision
# rather than this checkout. Compile the default render and a maximal
# single-mode render per harness against a `git archive` of that revision, so
# a pin that predates an API the template emits fails here instead of in
# users' projects. A change that adds such an API fails this until it is
# merged and the pin is moved to main (see RELEASE.md).
templates/hook-project/tests/run.sh --case pinned

cargo "+$rust_msrv" check \
  --locked \
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
