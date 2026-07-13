#!/usr/bin/env bash
set -euo pipefail

mode="${1:---check}"
case "$mode" in
  --check | --update) ;;
  *) echo "usage: $0 [--check|--update]" >&2; exit 2 ;;
esac

root="$(git rev-parse --show-toplevel)"
revision="9e552e9d15ba52bed7077d5357f3e18e330f8f38"
destination="$root/contracts/vendor/codex/$revision/generated"
temporary="$(mktemp -d)"
trap 'rm -rf "$temporary"' EXIT

files=(
  permission-request.command.input.schema.json
  permission-request.command.output.schema.json
  post-compact.command.input.schema.json
  post-compact.command.output.schema.json
  post-tool-use.command.input.schema.json
  post-tool-use.command.output.schema.json
  pre-compact.command.input.schema.json
  pre-compact.command.output.schema.json
  pre-tool-use.command.input.schema.json
  pre-tool-use.command.output.schema.json
  session-start.command.input.schema.json
  session-start.command.output.schema.json
  stop.command.input.schema.json
  stop.command.output.schema.json
  subagent-start.command.input.schema.json
  subagent-start.command.output.schema.json
  subagent-stop.command.input.schema.json
  subagent-stop.command.output.schema.json
  user-prompt-submit.command.input.schema.json
  user-prompt-submit.command.output.schema.json
)

for file in "${files[@]}"; do
  curl --fail --location --silent --show-error \
    "https://raw.githubusercontent.com/openai/codex/$revision/codex-rs/hooks/schema/generated/$file" \
    --output "$temporary/$file"
done

(
  cd "$temporary"
  shasum -a 256 "${files[@]}" > MANIFEST.sha256
)

if [[ "$mode" == "--update" ]]; then
  mkdir -p "$destination"
  for file in "${files[@]}" MANIFEST.sha256; do
    cp "$temporary/$file" "$destination/$file"
  done
  exit 0
fi

if [[ ! -d "$destination" ]]; then
  echo "missing vendored Codex schemas; run $0 --update" >&2
  exit 1
fi

for file in "${files[@]}" MANIFEST.sha256; do
  if ! cmp --silent "$temporary/$file" "$destination/$file"; then
    echo "vendored Codex schema drift: $file" >&2
    exit 1
  fi
done

(
  cd "$destination"
  shasum -a 256 --check MANIFEST.sha256
)
