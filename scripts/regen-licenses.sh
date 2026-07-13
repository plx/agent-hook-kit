#!/usr/bin/env bash
# Regenerate THIRD_PARTY_LICENSES.md from the current dependency graph.
#
# CI re-runs this and rejects the PR if the result differs from the committed
# copy. Run it locally after touching dependencies, then commit the diff.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# Pinned to keep CI and local output byte-identical. Bump in lockstep with the
# version specified by .github/workflows/ci.yml.
CARGO_ABOUT_VERSION="0.9.0"

installed_version=""
if command -v cargo-about >/dev/null 2>&1; then
    installed_version="$(cargo about --version 2>/dev/null | awk '{print $2}')"
fi

if [[ "$installed_version" != "$CARGO_ABOUT_VERSION" ]]; then
    echo "regen-licenses: installing cargo-about ${CARGO_ABOUT_VERSION} (found '${installed_version:-none}')" >&2
    cargo install --locked "cargo-about@${CARGO_ABOUT_VERSION}"
fi

# Attribute dependencies linked into the distributed runner, not unpublished
# workspace-only tools such as xtask and hookkit-conformance.
cargo about generate \
    --manifest-path crates/hookkit-tool-runner/Cargo.toml \
    --locked \
    -c about.toml \
    -o THIRD_PARTY_LICENSES.md \
    about.hbs
