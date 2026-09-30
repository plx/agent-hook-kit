# Antigravity hook contract refresh: `docs-2026-09-30-r1`

- Audit date: 2026-09-30
- Previous snapshot: `docs-2026-09-29-r1` (selected)
- Successor snapshot: `docs-2026-09-30-r1` (frozen, not yet selected)
- Official hook reference: <https://antigravity.google/docs/hooks> and
  <https://antigravity.google/docs/hooks.md> (site build ETag `"_KFTNQ"`)

No upstream page changed. On 2026-09-30, every source cited by
`docs-2026-09-29-r1` hashed to its recorded value when fetched with
`curl --fail --silent --show-error --location --compressed`. That covers the
reference, the Markdown source, the IDE redirect, the combined changelog, the
permissions reference, and the three Internet Archive captures. The
`antigravity-cli` repository's `main` was still `eaf9e066` (CLI 1.2.14).

The successor exists to correct two review findings that a frozen snapshot
cannot absorb.

`cargo xtask contracts diff antigravity/docs-2026-09-29-r1 antigravity/docs-2026-09-30-r1`:

```text
old: antigravity/docs-2026-09-29-r1 (5 events)
new: antigravity/docs-2026-09-30-r1 (5 events)
= PostInvocation
~ PostToolUse: contract.yaml, fixtures.yaml, input.schema.json
= PreInvocation
~ PreToolUse: contract.yaml, fixtures.yaml, input.schema.json
= Stop
~ snapshot files: sources.yaml
events: 0 added, 0 removed, 2 changed, 3 unchanged
content: changed
```

## LCAE-1: reproducible CLI changelog hash

`docs-2026-09-29-r1` hashed the raw CLI changelog but recorded
`https://github.com/google-antigravity/antigravity-cli/blob/eaf9e066…/CHANGELOG.md`
as its URL. That URL serves an HTML page with per-request nonces, so the drift
script's `curl --compressed <url>` could never reproduce the hash. The
successor records the raw file,
`https://raw.githubusercontent.com/google-antigravity/antigravity-cli/eaf9e06660d2ca8f20f1474ed03f10e3dbfd35e5/CHANGELOG.md`,
which hashes to the recorded `09edf2e8…1d2b`. The revision and
`pinned-revision` classification are unchanged. The xtask validation that
would reject a `github.com/…/blob/` URL with a `content_sha256` is tooling
outside this snapshot.

## LCAE-6: `toolCall.args` may be absent or null

The reference types `toolCall.args` as an object. It also documents a tool
that takes no arguments, `list_permissions` ("Arguments: None"), and never
shows the payload for such a call. No evidence shows Antigravity sending
`args: null` or omitting `args`. The documentation does not rule either out,
so the successor relaxes both tool events:

- `toolCall.required` is `["name"]`, and `args` is typed `["object", "null"]`.
- An uncertainty says that a hook should treat a missing or null `args` as an
  empty object.
- `list-permissions-null-args` and `list-permissions-without-args` positive
  inputs cover both shapes.
- The `PostToolUse` `invalid-tool-call` negative now omits `name` instead of
  `args`, because a missing `args` is no longer invalid.

## Follow-up

- `hookkit-antigravity` needs these changes:
  - `SNAPSHOT` and the contract ids.
  - `ToolCall.args` must accept null and absent values. The crate's
    `snapshot_contract` test requires lossless round-trips of positive
    inputs, so the type has to tell absent, null, and `{}` apart, not only
    default to an empty map.
  - The unit test that rejects a `toolCall` without `args` must change.
- A successor command-environment supplement must target this snapshot. The
  environment contract is unchanged.

Until then `contracts/registry.yaml` keeps `docs-2026-09-29-r1` selected.
