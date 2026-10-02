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

Besides the render/compile matrix, the default run renders the project a user
gets by accepting every default (only the name and local HookKit path are
given) and validates it like any other cell. It also checks two crate-mode
instances coexisting in one Cargo workspace, a versioned Copier update that
preserves edits under `src/hooks/`, and deferred-quality policy edits that
survive both recopy and a versioned update while remaining evaluable. The
Git-source project and the unpublished crates.io source are render-only there;
every direct HookKit dependency must use one repository and full Git revision
or one compatible crates.io version, without mixed source keys. The nonexistent-path check verifies that
the questionnaire rejects a missing local HookKit checkout before rendering.
Every negative case passes only when Copier rejects the intended question (or,
for a duplicate crate path, refuses to overwrite); any other exception, such as
a template syntax error, fails the run. Toolchain versions come from
`catalog/compatibility.yml`.
The `pinned` case is a release gate that only runs when named. Git mode, the
template default, builds against `compatibility.hookkit.git_revision` rather
than this checkout, so it compiles the default render and a maximal
single-mode render per harness against a `git archive` export of that
revision, then compiles a Git-source project fetched from the public
repository at that revision. It fails while the pin predates an API the
template emits, which is expected on a branch that adds one until it is merged
and the pin moves (see `RELEASE.md`):

```sh
templates/hook-project/tests/run.sh --case pinned
templates/hook-project/tests/run.sh --case pinned --pinned-revision <sha>
```

The matrix derives native events and aligned-family intersections from the
canonical catalogs while asserting the design's current 33/12/5 event counts
and universal-three plus Claude/Codex-eight family split.
