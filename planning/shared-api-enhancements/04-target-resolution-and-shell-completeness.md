# Shared API Enhancement 04: Target Resolution and Shell Completeness

<!-- markdownlint-disable MD013 -->

- Status: approved
- Priority: P0 correctness and P1 ergonomics
- Dependencies: items 01 and 03
- Primary crates: `hookkit-tool-access`, `hookkit-shell`
- Secondary consumer: `hookkit-file-activity`
- Controlling plan: [Shared API Enhancement Plan](../shared-api-enhancement-plan.md)

## Objective

Preserve and act on target scope instead of flattening every candidate to one path, and move the examples' remaining shell fallbacks into reusable, uncertainty-aware library APIs.

This task closes two correctness gaps found after the example refactors:

- a descendant-scoped operation such as `rm -rf secrets` can miss a `secrets/**` policy when only the directory path is matched; and
- a shell-invoked `apply_patch` heredoc is not covered by structured-tool patch extraction.

## Workstream A: general target resolution

Add a general resolver in `hookkit-tool-access` that accepts an iterator or collection of unified targets directly. It must not require a `PendingFileActivity` aggregate.

The options must make these policies explicit:

- workspace roots used for unrooted workspace/glob targets;
- ignored directory names;
- excluded roots;
- maximum entries or work budget;
- symlink-following policy, defaulting to no traversal;
- treatment of exact nonexistent paths, including a mode that retains them for pre-tool create/write policy; and
- treatment of I/O errors and invalid glob expressions.

The result must include:

- concrete resolved files/paths;
- unresolved targets with per-target reasons;
- the number of scanned entries;
- whether the budget was exhausted; and
- enough detail to identify targets not processed after truncation.

An exact write target must not disappear merely because it does not exist yet. An existing-files-only mode remains necessary for deferred linter execution.

Refactor `hookkit-file-activity::resolve_files` to delegate to the general resolver or provide a compatibility wrapper over it. Preserve its externally documented behavior unless a change is explicitly called out and tested.

The resolver is bounded filesystem materialization, not symbolic glob-intersection proof. Document that distinction.

## Workstream B: heuristic unknown-command candidates

Add an opt-in policy to `FileAccessAnalyzer` for unknown or ambiguous commands with fully recovered literal argv.

When enabled, the fallback should:

- examine literal operands using one documented path-likeness rule;
- handle `name=value` operands consistently;
- emit normal `FileAccessCandidate` values;
- mark them `Heuristic`;
- retain argument provenance and the analyzer's working-directory basis;
- respect uncertainty after `cd`, `pushd`, or `popd`; and
- retain the underlying unknown/ambiguous-semantics unresolved record.

The default must remain conservative and backward-compatible: no heuristic candidates unless the caller opts in.

Extend the built-in command table where semantics are sufficiently well understood, beginning with `sed`. Model option layouts conservatively and emit uncertainty rather than guessing when `-i`, scripts, or operands are ambiguous.

## Workstream C: reusable shell profiles

Publish the exact contract-backed built-in `ShellToolProfile` values or equivalent constructors for Claude/Codex Bash, Gemini `run_shell_command`, and Antigravity `run_command`.

Allow the unified tool-call analyzer to match an explicit caller-supplied profile or list of profiles without re-declaring native field pointers. Custom aliases remain opt-in. The library must not restore speculative aliases such as `shell` or `exec_command` as defaults without contract evidence.

## Workstream D: shell-invoked patch payloads

Recognize an observable literal patch passed to a shell `apply_patch` invocation, including the common heredoc form. Reuse the shared patch parser from item 03.

If `BashAnalysis` does not expose a heredoc body safely enough, enrich its redirection model with bounded, source-span-backed heredoc delimiter/body information. Do not parse arbitrary generated shell input or claim that dynamic heredocs are known.

Requirements:

- only recover literal, statically delimited patch bodies;
- retain shell and patch provenance;
- resolve patch paths against the shell call's effective cwd when known;
- emit gaps for dynamic delimiters, truncated analysis, or uncertain cwd; and
- apply the same resource bounds as the Bash analysis and patch parser.

## Uncertainty policy boundary

The library reports facts and known gaps. It should expose straightforward completeness queries, but it must not decide whether an application allows or denies an unresolved command.

`FileAccessReport::is_fully_resolved()` already expresses the basic fact. Add more classification only if it distinguishes actionable categories; do not add a security-sounding helper that silently embeds one policy.

## Required tests

Cover:

- exact, descendant, exact-or-descendant, glob, and workspace materialization;
- exact nonexistent targets under both existence policies;
- ignored/excluded directories and no symlink following by default;
- deterministic truncation with every unprocessed target reported;
- invalid globs and traversal I/O errors as typed unresolved results;
- `rm -rf secrets` producing descendant files that can match `secrets/**`;
- `sed` read versus in-place-modify behavior and ambiguous options;
- opt-in literal fallback for an unknown command;
- fallback after a possible directory change withholding an unjustified resolved path;
- default analyzer behavior remaining unchanged when fallback is disabled;
- built-in and explicit custom shell profiles; and
- literal shell `apply_patch` heredoc targets, including add/delete/move and malformed/dynamic cases.

Run at minimum:

```bash
cargo test -p hookkit-shell -p hookkit-tool-access -p hookkit-file-activity
cargo clippy -p hookkit-shell -p hookkit-tool-access -p hookkit-file-activity --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

## Exit criteria

- Pre-tool consumers can resolve scoped targets without constructing activity state.
- Resolver bounds, ignore policy, existence policy, and unresolved/truncated outcomes are explicit.
- Unknown-command fallback is opt-in, heuristic, and uncertainty-preserving.
- Exact native shell profiles are reusable and custom aliases are explicit.
- Literal shell-invoked patches use the shared patch parser.
- The documented static-analysis boundary remains honest.
