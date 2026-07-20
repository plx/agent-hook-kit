# Shared API Enhancement 01: Core Lexical Path Primitives

<!-- markdownlint-disable MD013 -->

- Status: approved
- Priority: P0 foundation
- Dependencies: none
- Primary crate: `hookkit-core`
- Known consumers: `hookkit-shell`, `hookkit-file-activity`, `codex-claude-rules`, `forbidden-file-guard`
- Controlling plan: [Shared API Enhancement Plan](../shared-api-enhancement-plan.md)

## Objective

Provide one public, deterministic implementation of the lexical path operations currently duplicated across HookKit crates and examples.

The API must normalize and resolve paths without consulting the filesystem. It should also provide explicit-home tilde expansion and lossless slash conversion for UTF-8 paths, so callers no longer carry subtly different local helpers.

## Problem statement

The current tree has independent `normalize_path` or `normalize_utf8` functions in:

- `crates/hookkit-shell/src/file_access.rs`;
- `crates/hookkit-file-activity/src/lib.rs`;
- `examples/codex-claude-rules/src/main.rs`; and
- `examples/forbidden-file-guard/src/main.rs`.

The examples additionally implement absolute anchoring, home expansion, and slash normalization locally. These operations form part of access-analysis correctness: a difference in how `..`, roots, or native working directories are handled can change whether a policy or rule matches.

## Required public semantics

Add a small path module to `hookkit-core`. Exact Rust names are not frozen, but the public surface should cover:

1. lexical normalization for `std::path::Path`/`PathBuf`;
2. lexical normalization for `camino::Utf8Path`/`Utf8PathBuf`;
3. lexical resolution of a candidate against an explicit base path;
4. explicit-home expansion of a leading `~` or `~/`; and
5. slash-separated rendering for UTF-8 paths when matching portable glob syntax.

The contract is:

- remove `.` components;
- collapse `normal/..` pairs;
- preserve leading `..` on relative paths when there is nothing safe to pop;
- never climb above an absolute root;
- preserve platform prefixes and roots according to `std::path::Component`;
- leave an absolute candidate independent of the supplied base;
- expand only the current-user `~` form, not `~other-user`;
- take the home directory as an argument rather than reading ambient process state;
- perform no canonicalization, symlink traversal, existence check, glob expansion, or I/O; and
- avoid lossy conversion in the UTF-8 API.

It is acceptable to expose borrowed/owned results with `Cow` where that materially improves the API. The simpler owned return type is also acceptable at this pre-release stage.

## Implementation outline

1. Add the module and crate-root exports in `hookkit-core`.
2. Move the strongest existing normalization property tests into the core module and extend them with explicit edge cases.
3. Replace all private normalization implementations listed above with the shared functions.
4. Replace example-local absolute anchoring, explicit-home expansion, and UTF-8 slash conversion where the new API applies.
5. Keep filesystem canonicalization in policy code that intentionally needs symlink-aware matching; do not fold it into the lexical helper.
6. Update public crate documentation with the lexical-versus-filesystem boundary.

## Compatibility and design constraints

- `hookkit-core` must remain independent of the native harness crates and higher-level HookKit crates.
- Do not add `dirs` or another ambient-home dependency to `hookkit-core`.
- Preserve Rust 1.85 compatibility.
- Do not change file-activity persistence formats as part of this task.
- If a caller currently relies on lossy `Path` rendering, keep that policy at the caller rather than making the core UTF-8 API lossy.

## Required tests

Add focused unit and property coverage for:

- normalization idempotence;
- relative leading `..` preservation;
- absolute paths not escaping their root;
- empty, `.`, and repeated-separator inputs;
- base-plus-relative and base-plus-absolute resolution;
- UTF-8 and `std::path` variants producing equivalent paths for UTF-8 inputs;
- `~`, `~/child`, non-leading `~`, and `~other-user`; and
- slash rendering without filesystem access.

Run at minimum:

```bash
cargo test -p hookkit-core
cargo test -p hookkit-shell -p hookkit-file-activity
cargo test -p codex-claude-rules -p forbidden-file-guard
cargo clippy -p hookkit-core -p hookkit-shell -p hookkit-file-activity --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

## Exit criteria

- One documented core implementation owns the required lexical path semantics.
- The four known private normalization copies are gone.
- Callers that need canonicalization still request it explicitly.
- Focused and property tests pass.
- No persisted state or native hook wire behavior changes.
