# Release process

MSRV is Rust 1.85. The 12 template-facing crates publish in dependency order:

1. `hookkit-core`
2. `hookkit-claude`, `hookkit-codex`, `hookkit-antigravity`,
   `hookkit-session-state`, and `hookkit-pkl-config`
3. `hookkit-common` and `hookkit-shell`
4. `hookkit-tool-access`
5. `hookkit-file-activity`
6. `hookkit-runtime`
7. `hookkit-tool-runner`

Install the Rust 1.85 toolchain, uv, and Pkl 0.31.1, then run
`scripts/release-check.sh` before tagging. The script validates the canonical
Copier catalogs, runs the pinned Copier 9.17.1 full generated-project matrix,
compiles Git-mode renders against the template's pinned HookKit revision,
requires Pkl so runner tests cannot silently skip, and compiles the full
template-facing graph on the declared MSRV. CI checks every package file set
with `cargo package --list` and fully packages `hookkit-core`. Cargo cannot
fully assemble downstream archives until their versioned internal dependencies
exist in the registry; during an actual staged publish, run `cargo package` and
`cargo publish` at each step after the preceding package group becomes visible.

For the Git-pinned template preview:

1. Merge every change that adds or changes an API the generated code uses.
   The template's default Git mode builds against
   `compatibility.hookkit.git_revision`, which must be reachable from `main`
   (CI's template job fails otherwise), so a branch cannot pin its own new
   APIs: the pin necessarily lags until the change is on `main`.
2. In a follow-up change, set `git_revision` in
   `templates/hook-project/catalog/compatibility.yml` to the full SHA of that
   merge commit (or any later `main` commit), then run
   `cargo xtask template-catalog sync` and `scripts/release-check.sh`. Its
   pinned-revision lane (`templates/hook-project/tests/run.sh --case pinned`)
   compiles the default render and a maximal single-mode render per harness
   against a `git archive` of the pin, then a Git-source project fetched from
   the public repository at the pin, so a stale pin fails before tagging.
   `--pinned-revision <sha>` tries a candidate before editing the catalog.
3. From a clean directory, generate against the public SHA and verify that all
   HookKit dependencies resolve to that same revision.
4. Tag the template commit with an immutable PEP 440-compatible version only
   after the pinned-revision lane and the public Git smoke succeed.

Keep all 12 crates on one compatible version train. Do not switch the Copier
template's default from an immutable Git revision to crates.io until every
crate required by its advertised archetypes is published and a clean generated
project passes `cargo check --locked`. The non-published conformance tool,
`xtask`, and examples are never published.

After publishing the final crate group, render a clean crates.io-mode project,
commit its lockfile, run `cargo check --locked`, and inspect the resolved
sources before changing the compatibility catalog's default dependency mode.
