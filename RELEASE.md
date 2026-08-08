# Release process

MSRV is Rust 1.85. The 10 template-facing crates publish in dependency order:

1. `hookkit-core`
2. `hookkit-claude`, `hookkit-codex`, `hookkit-antigravity`,
   `hookkit-session-state`
3. `hookkit-common` and `hookkit-shell`
4. `hookkit-tool-access`
5. `hookkit-file-activity`
6. `hookkit-runtime`

Install the Rust 1.85 toolchain and uv, then run
`scripts/release-check.sh` before tagging. The script validates the canonical
Copier catalogs, runs the pinned Copier 9.17.1 full generated-project matrix,
and compiles the template-facing graph on the declared MSRV. CI checks every
package file set
with `cargo package --list` and fully packages `hookkit-core`. Cargo cannot
fully assemble downstream archives until their versioned internal dependencies
exist in the registry; during an actual staged publish, run `cargo package` and
`cargo publish` at each step after the preceding package group becomes visible.

For the Git-pinned template preview:

1. Push the compatibility commit containing every API used by generated code.
2. Set `templates/hook-project/catalog/compatibility.yml` to that commit's full
   SHA, then run `cargo xtask template-catalog sync` and the release check.
3. From a clean directory, generate against the public SHA and verify that all
   HookKit dependencies resolve to that same revision.
4. Tag the template commit with an immutable PEP 440-compatible version only
   after the public Git smoke succeeds.

Keep all 10 crates on one compatible version train. Do not switch the Copier
template's default from an immutable Git revision to crates.io until every
crate required by its advertised archetypes is published and a clean generated
project passes `cargo check --locked`. The non-published conformance tool,
`xtask`, and examples are never published.

After publishing the final crate group, render a clean crates.io-mode project,
commit its lockfile, run `cargo check --locked`, and inspect the resolved
sources before changing the compatibility catalog's default dependency mode.
