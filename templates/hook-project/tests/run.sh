#!/usr/bin/env bash
set -euo pipefail

test_dir="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
export PYTHONDONTWRITEBYTECODE=1

if ! command -v uv >/dev/null 2>&1; then
  echo "error: uv is required to run the pinned Copier acceptance tests" >&2
  exit 1
fi

exec uv run \
  --quiet \
  --no-project \
  --with 'copier==9.17.1' \
  python "$test_dir/run.py" "$@"
