# Shared API Enhancement 05: Consumer Migrations and Regression Closure

<!-- markdownlint-disable MD013 -->

- Status: approved
- Priority: P0 proof and correctness closure
- Dependencies: items 01 through 04
- Primary consumers: `codex-claude-rules`, `forbidden-file-guard`, `hookkit-file-activity`
- Documentation affected: example READMEs/refactor notes and root README
- Controlling plan: [Shared API Enhancement Plan](../shared-api-enhancement-plan.md)

## Objective

Migrate the two examples that exposed the shared API gaps, remove their duplicated extraction and path-analysis glue, and add end-to-end regressions for the behavior that the old local code missed.

This task is not cleanup alone. It is the acceptance proof that the new APIs support both a pre-tool read/write policy and a path-scoped rule injector without discarding native fidelity, scope, provenance, or uncertainty.

## Migration A: `codex-claude-rules`

Replace local implementations of:

- structured path-field collection;
- path-key recognition;
- `apply_patch` header parsing;
- shell candidate flattening;
- unknown-command literal argv fallback;
- deduplication that loses target metadata; and
- lexical path normalization/resolution.

Use the unified analyzer and general target resolver. Rule matching should materialize bounded descendant/glob/workspace targets so a directory search can activate a rule scoped to a descendant pattern.

Keep these semantics explicit:

- relative tool operands resolve against the native tool-call cwd;
- `--project-root` controls rule discovery and project-relative pattern matching, not the meaning of tool operands;
- targets outside the project root do not match project-relative rules; and
- truncation or unresolved access does not silently become a concrete match.

Correct the checked-in fixture or invocation documentation so its native cwd is consistent with the intended project-root demonstration. Do not "fix" the fixture by changing relative operand semantics.

Preserve session-scoped `ClaimSet` behavior and state-family layout unless the task makes an independently justified state change.

## Migration B: `forbidden-file-guard`

Move the executable to aligned `PreToolUse` handling for Claude Code, Codex, Gemini CLI, and Antigravity. Use unified tool-access analysis for shell, structured, and patch calls and the general resolver for scoped candidates.

Replace the binary shell policy with a three-level posture:

- inspect known candidates and allow unresolved gaps;
- deny when access analysis is unresolved; and
- deny all shell calls.

Exact configuration names are not frozen. Preserve compatibility with the existing `block_shell_commands` field or provide a clear, tested migration. A reasonable mapping is `true` to deny-all and `false` to inspect-known.

Policy evaluation must consider:

- raw and resolved exact targets;
- bounded materialization of descendants, globs, and workspace scopes;
- nonexistent exact write targets;
- symlink-aware canonical forms where the guard intentionally requests them; and
- resolver/analyzer gaps according to the configured posture.

Add Claude mode now that aligned pre-tool execution can use the contract-backed catalog event. If current `origin/main` contract evidence changes that premise, document the contradiction before omitting the arm.

## Migration C: activity and tracker consumers

Confirm that `hookkit-file-activity` delegates tool-call analysis to `hookkit-tool-access` and scope resolution to the shared resolver while retaining:

- write-only activity filtering;
- observation metadata;
- pending-window behavior;
- reconciliation behavior; and
- persisted family compatibility.

The former `session-modified-file-tracker` consumer now lives in
[Velvet Glove](https://github.com/plx/velvet-glove). Do not redesign session
batching as part of this historical migration task.

## Required regression scenarios

Add tests that fail on the pre-migration implementation and pass through the public APIs:

1. a structured read and structured write to a forbidden path;
2. structured `apply_patch` add/update/delete/move roles;
3. shell `apply_patch <<'PATCH'` touching a forbidden path;
4. `rm -rf secrets` intersecting a `secrets/**` policy;
5. a directory search activating a `src/**/*.rs` Claude rule;
6. an unknown literal command using the opt-in heuristic candidate path;
7. `cat "$SECRET"` allowed under inspect-known and denied under deny-unresolved;
8. malformed shell-tool input denied whenever active policy cannot safely inspect it;
9. exact native shell names and an explicitly configured alias;
10. native cwd differing from `--project-root`;
11. resolver truncation following the consumer's explicit policy; and
12. concurrent rule claims injecting a rule only once.

Include stdin/stdout integration coverage for every supported forbidden-guard harness and the Codex rule injector.

## Documentation work

Update:

- both example READMEs;
- the root example summary;
- `hookkit-shell`, `hookkit-tool-access`, and `hookkit-file-activity` READMEs; and
- the examples' historical refinement/refactor notes.

Keep the historical notes, but mark addressed items and link to the resulting public APIs. Remove or correct statements that claim Claude pre-tool output is unavailable or that the guard uses "obvious shell path tokens."

Document that:

- access analysis is bounded and not a sandbox;
- resolver truncation and unresolved access are policy inputs;
- native cwd and policy/project roots serve different purposes; and
- canonicalization remains an explicit, filesystem-aware consumer choice.

## Required validation

Run focused tests while iterating, then the entire common validation suite from the controlling plan. At minimum, focused validation includes:

```bash
cargo test -p codex-claude-rules -p forbidden-file-guard
cargo test -p hookkit-tool-access -p hookkit-file-activity -p hookkit-shell
cargo clippy -p codex-claude-rules -p forbidden-file-guard --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

Manually exercise the highest-risk shipped boundaries with checked-in fixtures or temporary state directories:

- one deny and one allow/pass response for each forbidden-guard harness;
- one first-match and one already-claimed rule injection;
- one descendant-scope policy match; and
- one shell heredoc patch match.

Do not commit temporary state, generated diagnostics, or fixture outputs unless they are intentional golden files.

## Exit criteria

- Neither example owns generic structured-field, patch, shell-fallback, target-resolution, or lexical-normalization logic.
- The guard uses one aligned handler across all four supported harnesses.
- The rule injector consumes scoped targets without changing native-cwd semantics.
- All listed regressions are covered at the public API or executable boundary.
- File-activity state and batching behavior remain compatible.
- Public documentation reflects the new API and its limits.
