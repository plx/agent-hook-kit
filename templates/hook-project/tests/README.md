# Copier template acceptance tests

`run.sh` pins Copier 9.17.1 and stages the current working-tree template in a
temporary, non-Git directory before rendering. This prevents Copier from
silently testing the last committed version when the template itself has
uncommitted changes.

The default lane renders the catalog-derived matrix and runs `cargo check
--all-targets` against local path dependencies:

```sh
templates/hook-project/tests/run.sh
```

Use `--validation render` for fast template iteration. The release-equivalent
lane runs formatting, Rust 1.85, stable check, Clippy, and tests for each
generated crate:

```sh
templates/hook-project/tests/run.sh --validation full
```

Individual cells can be selected and retained for inspection:

```sh
templates/hook-project/tests/run.sh --list
templates/hook-project/tests/run.sh \
  --validation render \
  --case cross_claude_codex \
  --keep-workdir
```

Besides the render/compile matrix, the default run checks two crate-mode
instances coexisting in one Cargo workspace and a versioned Copier update that
preserves edits under `src/hooks/`. It also compiles a generated project against
the public Git compatibility revision. The unpublished crates.io source is
render-only; every direct HookKit dependency must use one repository and full
Git revision or one compatible crates.io version, without mixed source keys.
The nonexistent-path check verifies that the questionnaire rejects a missing
local HookKit checkout before rendering.
The matrix derives native events and aligned-family intersections from the
canonical catalogs while asserting the design's current 31/11/5 event counts
and universal-three plus Claude/Codex-eight family split.
