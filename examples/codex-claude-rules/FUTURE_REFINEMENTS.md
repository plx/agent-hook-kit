# Future refinements

The refactor exposed several places where a reusable HookKit API could replace
remaining example-local policy or glue. These are observations for future
library work, not requirements for the current example.

## Reusable structured and patch target extraction

The example still owns recursive structured-path discovery, recognized field
names, `apply_patch` header parsing, deduplication, and native-cwd resolution.
Closely related logic exists inside `hookkit-file-activity`, but it is private
and coupled to post-tool modification evidence.

A public pre/post tool-target analyzer could return a shared target type with:

- resolved and raw paths;
- exact, descendant, glob, or workspace scope;
- read, modify, delete, move-source, and move-destination intent;
- structured-field, patch, shell, or custom provenance; and
- direct, conditional, or heuristic certainty.

That would let this hook consume target evidence without duplicating extraction
rules or pretending that every path-shaped field has identical semantics.

## General target-scope resolution

`FileAccessAnalyzer` correctly preserves `Exact`, `Descendants`,
`ExactOrDescendants`, `Glob`, and `Workspace` scope. This example currently
uses only each candidate's resolved path. Consequently, a command that searches
the `src` directory may not activate a rule scoped to `src/**/*.rs` until a
later hook input exposes a specific Rust file.

`hookkit-file-activity::resolve_files` already expands similar scopes, but its
input is a `PendingFileActivity` aggregate rather than a general collection of
targets. A reusable `resolve_targets` API—with explicit roots, ignore policy,
entry limits, and unresolved/truncated results—would make that functionality
available to pre-tool policy hooks without manufacturing file-activity state.

## First-class heuristic shell candidates

The built-in shell semantics deliberately do not model every executable.
Unsupported commands such as `sed` therefore require this example to inspect
literal argv for path-looking operands. That fallback can over-report strings
that resemble paths, and it does not inherit all of the analyzer's uncertainty
handling around earlier `cd`, `pushd`, or `popd` commands.

It would be useful for `FileAccessAnalyzer` to offer an opt-in fallback that
emits these as normal `FileAccessCandidate` values with `Heuristic` certainty,
or to expose enough context for callers to implement the fallback without
reconstructing working-directory uncertainty. Expanding the built-in command
table for common agent inspection/editing commands would reduce the need for
the fallback as well.

## Shared lexical path utilities

The example still implements lexical `.`/`..` normalization, absolute-path
anchoring, and slash normalization. Similar private helpers exist in
`hookkit-shell` and `hookkit-file-activity`.

A small public `hookkit-core` path module could define consistent lexical
resolution for both `Path` and `Utf8Path`, without filesystem canonicalization
or symlink assumptions. That would eliminate subtle drift between crates and
examples.

## Atomic loaded-rule state with observability

`ClaimSet` is the best existing fit for the hook's immediate decision, but its
on-disk claims intentionally retain only hashed keys. This makes the loaded
rule set inexpensive and private, but not directly inspectable. The concrete
`LoadedRules` entity is inspectable, while the generic `SetJournal` supplies an
atomic `insert_once`; combining those affordances currently requires choosing
the heavier journal abstraction.

If loaded-rule observability becomes valuable, a typed atomic insertion helper
for `LoadedRules`, or optional safe metadata on claims, could provide both a
first-writer decision and a debuggable projection. The current hook does not
otherwise need that additional state.
