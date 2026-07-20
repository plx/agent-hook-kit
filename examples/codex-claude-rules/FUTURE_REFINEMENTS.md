# Shared API refinement record

The original refactor exposed the gaps below. The generic analysis/path items
were addressed by the shared API enhancement work; the text is retained as a
historical record. Observable claim metadata remains deliberately deferred.

## Reusable structured and patch target extraction

**Status: addressed.** `hookkit-tool-access::ToolAccessAnalyzer` now owns the
phase-agnostic structured, patch, and shell report consumed by this example.

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

**Status: addressed.** `hookkit-tool-access::resolve_targets` materializes all
retained scopes with explicit bounds and unresolved results.

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

**Status: addressed.** `UnknownCommandFallback::LiteralPathOperands` supplies
the opt-in heuristic, and `sed` has conservative built-in semantics.

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

**Status: addressed.** `hookkit-core` now exports native and UTF-8 lexical
normalization, resolution, explicit-home expansion, and slash rendering.

The example still implements lexical `.`/`..` normalization, absolute-path
anchoring, and slash normalization. Similar private helpers exist in
`hookkit-shell` and `hookkit-file-activity`.

A small public `hookkit-core` path module could define consistent lexical
resolution for both `Path` and `Utf8Path`, without filesystem canonicalization
or symlink assumptions. That would eliminate subtle drift between crates and
examples.

## Atomic loaded-rule state with observability

**Status: deferred.** The example still has no concrete need to expose claim
metadata, so item 06's decision gate is not met. `ClaimSet` and state-family
version 2 remain unchanged.

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
