#!/usr/bin/env bash
# Discover upstream drift in the registry-selected contract snapshots and
# command-environment supplement without mutating anything (see
# contracts/MAINTENANCE.md).
#
# Every input comes from the selected snapshots' and supplement's sources.yaml
# files through `cargo xtask contracts upstream-sources`; this script names no
# revision, URL, or hash of its own. Supplement rows are labeled
# `command-environments/<supplement-id>/<source-id>`. For each selected source:
#
# - a vendored Git tree is fetched at its pinned revision and compared with the
#   vendored MANIFEST.sha256 (evidence integrity), then fetched at the
#   upstream default branch's HEAD and compared again (drift);
# - a pinned-revision Git tree or file whose object differs at HEAD is listed
#   for behavioral review without failing the run; and
# - a URL with a recorded content_sha256 is fetched and hashed. A GitHub file
#   page arrives as its raw.githubusercontent.com URL, because the page itself
#   is HTML with per-request content. When a later selected retrieval of the
#   same URL recorded a newer hash and cited this one after reviewing the
#   difference, matching that newer hash is reported as acknowledged drift
#   rather than failing the run; a successor snapshot should still pin it.
#
# Exit status: 0 when nothing drifted, 1 when drift or an integrity mismatch
# was found, and 2 when discovery itself failed (for example, on a network
# error), so a broken check can never look like a clean one.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

drift=0

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 2
}

sha256_stream() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum | awk '{print $1}'
  else
    shasum -a 256 | awk '{print $1}'
  fi
}

# Print the bare, blobless clone of <url> (at its default branch HEAD), creating
# it on first use. Blobs are fetched lazily, only for the files that are hashed.
upstream_git_dir() {
  local url=$1 directory
  directory="$work/git-$(printf '%s' "$url" | sha256_stream | cut -c1-16)"
  if [[ ! -d "$directory" ]]; then
    git clone --quiet --bare --depth=1 --filter=blob:none "$url" "$directory" \
      || fail "cannot clone $url"
  fi
  printf '%s\n' "$directory"
}

# Write "<sha256>  <file>" lines for every file below <path> at <commit>, in the
# vendored MANIFEST.sha256 format, sorted by file name.
tree_manifest() {
  local git_dir=$1 commit=$2 path=$3 _mode type object name hash
  git --git-dir="$git_dir" ls-tree -r "$commit" -- "$path/" >"$work/tree" \
    || fail "cannot list $path at $commit"
  : >"$work/tree-manifest"
  while IFS=$' \t' read -r _mode type object name; do
    [[ "$type" == blob ]] || continue
    hash="$(git --git-dir="$git_dir" cat-file blob "$object" | sha256_stream)" \
      || fail "cannot read $name at $commit"
    printf '%s  %s\n' "$hash" "${name#"$path"/}" >>"$work/tree-manifest"
  done <"$work/tree"
  sort -k2 "$work/tree-manifest"
}

# Print "+ added", "- removed", and "~ changed" lines comparing two manifests.
manifest_differences() {
  awk '
    FNR == NR { expected[$2] = $1; next }
    { actual[$2] = $1 }
    END {
      for (file in actual) {
        if (!(file in expected)) print "  + " file " (added upstream)"
        else if (expected[file] != actual[file]) print "  ~ " file " (content changed)"
      }
      for (file in expected) if (!(file in actual)) print "  - " file " (removed upstream)"
    }
  ' "$1" "$2" | sort -k2
}

check_vendored_tree() {
  local label=$1 clone_url=$2 path=$3 revision=$4 vendor_dir=$5
  local git_dir head differences
  local expected="$repo_root/$vendor_dir/MANIFEST.sha256"
  [[ -f "$expected" ]] || fail "$label: missing $vendor_dir/MANIFEST.sha256"
  sort -k2 "$expected" >"$work/expected"

  git_dir="$(upstream_git_dir "$clone_url")"
  git --git-dir="$git_dir" fetch --quiet --depth=1 --filter=blob:none origin "$revision" \
    || fail "$label: cannot fetch pinned revision $revision"
  tree_manifest "$git_dir" "$revision" "$path" >"$work/pinned"
  differences="$(manifest_differences "$work/expected" "$work/pinned")"
  if [[ -n "$differences" ]]; then
    drift=1
    printf 'INTEGRITY %s: %s differs from %s at pinned %s\n%s\n' \
      "$label" "$vendor_dir" "$path" "$revision" "$differences"
  fi

  head="$(git --git-dir="$git_dir" rev-parse HEAD)" || fail "$label: cannot resolve upstream HEAD"
  tree_manifest "$git_dir" "$head" "$path" >"$work/head"
  differences="$(manifest_differences "$work/expected" "$work/head")"
  if [[ -n "$differences" ]]; then
    drift=1
    printf 'DRIFT %s: %s at upstream HEAD %s differs from pinned %s\n%s\n' \
      "$label" "$path" "$head" "$revision" "$differences"
  else
    printf 'ok %s: %s at upstream HEAD %s matches pinned %s\n' \
      "$label" "$path" "$head" "$revision"
  fi
}

check_pinned_tree() {
  local label=$1 clone_url=$2 path=$3 revision=$4
  local git_dir head pinned_tree head_tree
  git_dir="$(upstream_git_dir "$clone_url")"
  git --git-dir="$git_dir" fetch --quiet --depth=1 --filter=blob:none origin "$revision" \
    || fail "$label: cannot fetch pinned revision $revision"
  head="$(git --git-dir="$git_dir" rev-parse HEAD)" || fail "$label: cannot resolve upstream HEAD"
  pinned_tree="$(git --git-dir="$git_dir" rev-parse "$revision:$path")" \
    || fail "$label: $path is absent at pinned $revision"
  head_tree="$(git --git-dir="$git_dir" rev-parse "$head:$path" 2>/dev/null || printf 'absent')"
  if [[ "$pinned_tree" == "$head_tree" ]]; then
    printf 'ok %s: %s is unchanged at upstream HEAD %s\n' "$label" "$path" "$head"
  else
    # Source changes are frequent and often behavior-neutral; the vendored
    # wire schemas above are the failing signal, this is review context.
    printf 'review %s: %s changed between pinned %s and upstream HEAD %s\n' \
      "$label" "$path" "$revision" "$head"
  fi
}

check_content_hash() {
  local label=$1 url=$2 recorded=$3 reproducibility=$4 acknowledged=$5 current
  # Recorded hashes cover the decoded body. --compressed decodes a response the
  # server content-encodes even unasked (antigravity.google does, sometimes),
  # and is a no-op for identity responses.
  current="$(curl --fail --silent --show-error --location --compressed --retry 3 --max-time 60 "$url" \
    | sha256_stream)" || fail "$label: cannot fetch $url"
  if [[ "$current" == "$recorded" ]]; then
    printf 'ok %s: %s still hashes to %s\n' "$label" "$url" "$recorded"
    return
  fi
  if [[ "$acknowledged" != - && "$current" == "$acknowledged" ]]; then
    printf 'acknowledged %s: %s hashes to %s, recorded %s; a later selected retrieval pinned %s after review, so cut a successor snapshot that pins it\n' \
      "$label" "$url" "$current" "$recorded" "$acknowledged"
    return
  fi
  drift=1
  printf 'DRIFT %s: %s hashes to %s, recorded %s\n' "$label" "$url" "$current" "$recorded"
  if [[ "$reproducibility" == unreproducible ]]; then
    printf '  the recorded hash is marked unreproducible; record a reproducible capture in the successor snapshot\n'
  fi
}

sources="$(cd "$repo_root" && cargo xtask contracts upstream-sources)" \
  || fail "cannot read the registry-selected snapshot and supplement sources"
[[ -n "$sources" ]] || fail "the registry-selected snapshots list no sources"

printf 'Upstream contract drift report (non-mutating; see contracts/MAINTENANCE.md)\n'
while IFS=$'\t' read -r harness snapshot source reproducibility url revision content_sha256 \
  clone_url path vendor_dir acknowledged_sha256; do
  label="$harness/$snapshot/$source"
  if [[ "$vendor_dir" != - ]]; then
    check_vendored_tree "$label" "$clone_url" "$path" "$revision" "$vendor_dir"
  elif [[ "$clone_url" != - && "$revision" != - ]]; then
    check_pinned_tree "$label" "$clone_url" "$path" "$revision"
  fi
  if [[ "$content_sha256" != - ]]; then
    check_content_hash "$label" "$url" "$content_sha256" "$reproducibility" "$acknowledged_sha256"
  fi
  if [[ "$vendor_dir" == - && "$content_sha256" == - && ( "$clone_url" == - || "$revision" == - ) ]]; then
    printf 'skip %s: %s records no revision or content hash to compare\n' "$label" "$url"
  fi
done <<<"$sources"

if ((drift)); then
  printf 'Upstream drift found. Follow contracts/MAINTENANCE.md to create a successor snapshot.\n'
  exit 1
fi
printf 'No upstream drift found.\n'
