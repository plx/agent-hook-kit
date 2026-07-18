# Notes for future hookkit refinement

Friction points hit while refactoring this example onto `hookkit-shell` (see
[`REFACTOR.md`](REFACTOR.md)). These are **not acted on** — they are recorded to
inform later library work.

## "We needed X that feels like it should be in the core library"

1. **A shared, phase-agnostic "tool call → referenced paths (reads *and*
   writes)" helper.** The structured-field + patch extraction
   (`collect_path_fields`, `is_path_key`, `collect_patch_paths`, `find_string`)
   is duplicated *verbatim* between this example and `hookkit-file-activity`
   (where it's private). It couldn't be reused, so the example still carries its
   own copy.

2. **An aligned `PreToolUse` event.** `session-modified-file-tracker` collapses
   its harnesses into one handler via `run_aligned_event::<PostToolUse>`. There
   is no `PreToolUse` equivalent, and `AlignedEventSpec` is a **sealed** trait,
   so one cannot be added from an example. The three arms must stay, differing
   only in native-output construction. Antigravity would benefit *most* here —
   its pre-tool event carries the tool call that its post-tool event lacks.

3. **Path normalization in `hookkit-core`.** `normalize_path` is hand-written
   identically in this example, `hookkit-shell` (`normalize_utf8`), and
   `hookkit-file-activity` (`normalize_utf8`) — three copies of the same
   `..` / `.` collapsing.

4. **A home-directory (`~`) expansion helper.** The example still expands `~/`
   itself at both policy-compile and candidate time.

## "We used a tool but it wasn't quite what we needed and worked around it"

5. **`hookkit-file-activity::observe_post_tool` is the obvious candidate but is
   shaped only for post-tool modification tracking** — it keys off `may_modify()`
   (drops reads) and takes `hookkit_common::PostToolUseInput`. A *pre-tool
   access* guard cares about reads too, so this refactor dropped down to the raw
   `hookkit-shell` primitives instead. If the file-inference layer exposed a
   "give me *all* access candidates for this tool call" entry point decoupled
   from phase and from modify-only filtering, both the tracker and this guard
   could share it.

6. **`FileAccessReport.unresolved` is computed but there was nowhere clean to act
   on it.** The analyzer *knows* when it couldn't resolve a path (`cat "$SECRET"`),
   which is strictly better than the old lexer silently missing it — but the
   guard's config is a binary (`block_shell_commands`). A natural middle knob
   ("deny when a shell command has *uninspectable* accesses") is enabled by this
   data; a blessed helper/pattern for "fail closed on uncertainty" would make it
   easy to adopt.

7. **The built-in `ShellToolProfile` constants are `pub(crate)`.** The adapters
   encode exactly one tool name per harness (e.g. Antigravity `run_command` →
   `/CommandLine` + `/Cwd`). If a hook needs to guard an alias the adapters don't
   cover, it must re-declare a `ShellToolProfile::new(...)` — duplicating that
   field-pointer knowledge — and the new profile doesn't participate in
   `ShellToolCallExt`. (This refactor dropped the old speculative
   `shell` / `exec_command` aliases, which don't correspond to any modeled
   harness tool.)
