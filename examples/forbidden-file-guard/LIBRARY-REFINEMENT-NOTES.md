# HookKit refinement status

This document preserves the friction points found during the original
`hookkit-shell` refactor (see [`REFACTOR.md`](REFACTOR.md)). The shared API
enhancement work has now addressed every item below without moving session
policy into the library.

## "We needed X that feels like it should be in the core library"

1. **A shared, phase-agnostic "tool call → referenced paths (reads *and*
   writes)" helper.** The structured-field + patch extraction
   (`collect_path_fields`, `is_path_key`, `collect_patch_paths`, `find_string`)
   is duplicated *verbatim* between this example and `hookkit-file-activity`
   (where it's private). It couldn't be reused, so the example still carries its
   own copy.

   **Addressed:** `hookkit-tool-access::ToolAccessAnalyzer` now owns structured
   field, patch, and shell access extraction for both pre- and post-tool calls.

2. **An aligned `PreToolUse` event.** `session-modified-file-tracker` collapses
   its harnesses into one handler via `run_aligned_event::<PostToolUse>`. There
   is no `PreToolUse` equivalent, and `AlignedEventSpec` is a **sealed** trait,
   so one cannot be added from an example. The three arms must stay, differing
   only in native-output construction. Antigravity would benefit *most* here —
   its pre-tool event carries the tool call that its post-tool event lacks.

   **Addressed:** `hookkit-common::PreToolUse` and
   `hookkit_common::PreToolUseInput` provide the aligned event and native arms
   for all four harnesses.

3. **Path normalization in `hookkit-core`.** `normalize_path` is hand-written
   identically in this example, `hookkit-shell` (`normalize_utf8`), and
   `hookkit-file-activity` (`normalize_utf8`) — three copies of the same
   `..` / `.` collapsing.

   **Addressed:** `hookkit_core::path` provides the shared lexical path
   primitives used by the analyzers and consumers.

4. **A home-directory (`~`) expansion helper.** The example still expands `~/`
   itself at both policy-compile and candidate time.

   **Addressed:** home expansion is part of the shared lexical path API.

## "We used a tool but it wasn't quite what we needed and worked around it"

5. **`hookkit-file-activity::observe_post_tool` is the obvious candidate but is
   shaped only for post-tool modification tracking** — it keys off `may_modify()`
   (drops reads) and takes `hookkit_common::PostToolUseInput`. A *pre-tool
   access* guard cares about reads too, so this refactor dropped down to the raw
   `hookkit-shell` primitives instead. If the file-inference layer exposed a
   "give me *all* access candidates for this tool call" entry point decoupled
   from phase and from modify-only filtering, both the tracker and this guard
   could share it.

   **Addressed:** both consumers now use `hookkit-tool-access`; file activity
   applies its modification-only filter after shared extraction, while this
   guard retains read evidence.

6. **`FileAccessReport.unresolved` is computed but there was nowhere clean to act
   on it.** The analyzer *knows* when it couldn't resolve a path (`cat "$SECRET"`),
   which is strictly better than the old lexer silently missing it — but the
   guard's config is a binary (`block_shell_commands`). A natural middle knob
   ("deny when a shell command has *uninspectable* accesses") is enabled by this
   data; a blessed helper/pattern for "fail closed on uncertainty" would make it
   easy to adopt.

   **Addressed in the consumer:** `access_policy: deny_unresolved` treats typed
   analyzer and resolver gaps as a denial. `inspect_known` remains the
   compatibility posture, and `deny_all_shell` provides the strict shell gate.

7. **The built-in `ShellToolProfile` constants are `pub(crate)`.** The adapters
   encode exactly one tool name per harness (e.g. Antigravity `run_command` →
   `/CommandLine` + `/Cwd`). If a hook needs to guard an alias the adapters don't
   cover, it must re-declare a `ShellToolProfile::new(...)` — duplicating that
   field-pointer knowledge — and the new profile doesn't participate in
   `ShellToolCallExt`. (This refactor dropped the old speculative
   `shell` / `exec_command` aliases, which don't correspond to any modeled
   harness tool.)

   **Addressed:** the exact native profile constants are public, and
   `ToolAccessAnalyzer::with_shell_profile` supports explicit custom aliases
   without weakening the built-in exact-name boundary.
