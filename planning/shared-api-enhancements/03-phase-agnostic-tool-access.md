# Shared API Enhancement 03: Phase-Agnostic Tool Access Analysis

<!-- markdownlint-disable MD013 -->

- Status: approved
- Priority: P0 central abstraction
- Dependencies: item 01; item 02 for the preferred aligned pre-tool entry point
- Primary deliverable: new `hookkit-tool-access` library crate
- Initial consumers: `hookkit-file-activity`, `codex-claude-rules`, `forbidden-file-guard`
- Controlling plan: [Shared API Enhancement Plan](../shared-api-enhancement-plan.md)

## Objective

Create a shared, phase-agnostic layer that converts an observable native tool call into loss-aware file-access candidates. It must cover structured tool inputs, patch payloads, and shell analysis without coupling the result to post-tool modification tracking or session state.

This is the main abstraction exposed by the two example refactors. The library already has rich shell-only inference and a post-tool, write-focused activity observer, but no reusable "tool call to all referenced file targets" API.

## Crate boundary

Add `hookkit-tool-access` rather than broadening `hookkit-file-activity` into a pre-tool policy crate.

The new crate should depend on:

- `hookkit-core` for identifiers, paths, and lexical path utilities;
- `hookkit-common` and/or feature-gated native types for tool-call adapters; and
- `hookkit-shell` for exact shell-call extraction and Bash file-access inference.

It must not depend on `hookkit-session-state`. It must not own reconciliation cursors, journals, filesystem-mtime scanning, or VCS state.

If implementation reveals a dependency cycle, preserve this semantic boundary and document the smallest lower-level type extraction needed to break the cycle. Do not put the analyzer into a stateful crate merely to avoid introducing the intended crate.

## Required data model

Exact type names are not frozen, but the public report must retain:

- raw path expression, when one is observable;
- lexically resolved path, when resolution is justified;
- target scope: exact, descendants, exact-or-descendants, glob, or workspace;
- access intent: read, modify, read-modify, enumerate, delete, move-source, and move-destination;
- an explicit representation for a path reference whose access semantics cannot be classified without guessing;
- provenance: structured field, patch, shell, or custom;
- detailed structured provenance such as a JSON pointer;
- detailed patch provenance such as operation/header and source line;
- existing shell origin information;
- direct, conditional, or heuristic certainty; and
- typed unresolved/gap records.

The report should provide convenience iterators such as `may_read()` and `may_modify()` plus a completeness query. These are views over the retained evidence, not destructive filters.

Do not deduplicate solely by resolved path: two candidates may have different access roles, scopes, certainty, or provenance. A separate deterministic unique-target view is appropriate for consumers that explicitly want it.

## Tool-call adapters

Define a lossless borrowed tool-call view or local extension trait that can represent:

- event/harness and pre/post phase;
- tool name;
- structured tool input without forcing lossy conversion;
- native cwd or workspace basis;
- optional post-tool response; and
- optional tool-call identifier when available.

Support aligned pre-tool and post-tool inputs. Direct native adapters may also be exposed when they avoid forcing a caller through an aligned runtime.

Antigravity post-tool input lacks the originating tool call. Return a typed gap for that arm rather than pretending the event is file-free.

## Structured input analysis

Provide a reusable structured-field analyzer with documented default path-bearing keys and an extension mechanism for additional exact fields or JSON pointers.

Requirements:

- recursively inspect objects and arrays without double-visiting recognized fields;
- preserve the JSON pointer for each emitted value;
- handle strings and nested string arrays under configured path fields;
- resolve relative paths against the native tool-call cwd;
- distinguish direct configured pointers from broad key-name heuristics; and
- never classify every path-shaped string as a definite write.

Tool-name or profile semantics may classify well-known read/write/move/delete operations. Unknown structured tools should retain a reference or uncertainty rather than acquiring an invented effect.

## Patch analysis

Provide one reusable parser for structured `apply_patch` payloads, including names such as `apply_patch` and qualified forms ending in `.apply_patch` where contract evidence permits.

At minimum, recognize:

- add file;
- update file;
- delete file;
- move source and move destination; and
- unified-diff old/new headers, excluding `/dev/null`.

The parser must preserve operation roles instead of returning an untyped list of strings. It should expose malformed or incomplete patch structure as a typed gap while retaining any safely recovered candidates.

## Shell integration

For a matched native shell call:

1. use `BashAnalyzer`;
2. use `FileAccessAnalyzer` with the native cwd/workspace basis;
3. map every shell candidate into the unified model without discarding scope, origin, or certainty; and
4. map every unresolved shell record into a typed unified gap.

A malformed matched shell call is a gap. A definite non-shell call falls through to structured/patch analysis.

Unknown-command heuristic fallback, custom shell profiles, scope materialization, and shell heredoc patch handling are completed in item 04.

## File-activity integration

Refactor `hookkit-file-activity::observe_post_tool` to consume this shared report:

- retain only candidates whose access may modify;
- map them into the existing persistence-facing activity evidence;
- retain gaps with useful typed detail; and
- preserve the version-1 file-activity family schema unless a separately reviewed migration is unavoidable.

This integration is the proof that the lower layer is genuinely reusable. Do not remove reconciliation or activity-specific metadata from `hookkit-file-activity`.

## Required tests

Cover:

- recursive structured fields, arrays, and exact JSON-pointer provenance;
- no duplicate candidate from one recognized field;
- divergent key spellings currently used by the examples;
- native-cwd resolution for relative paths and preservation of absolute paths;
- add/update/delete/move patch roles;
- malformed patches retaining partial evidence plus a gap;
- shell reads and modifications mapping losslessly;
- unknown and malformed shell calls producing gaps;
- Antigravity post-tool producing a missing-tool-call gap; and
- `hookkit-file-activity` continuing to drop read-only evidence while retaining modifications.

Run at minimum:

```bash
cargo test -p hookkit-tool-access
cargo test -p hookkit-file-activity
cargo clippy -p hookkit-tool-access -p hookkit-file-activity --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

## Exit criteria

- One public analyzer handles structured, patch, and shell tool-call evidence for pre- and post-tool consumers.
- The model retains raw/resolved paths, scopes, access roles, provenance, certainty, and gaps.
- `hookkit-file-activity` delegates extraction/inference to the new crate and remains write-focused.
- Existing file-activity persisted state remains compatible or receives an explicitly reviewed version migration.
- Public docs state the static-analysis boundary and do not claim exhaustive access observation.
