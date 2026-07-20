# Refactor: onto `hookkit-shell`

This example predated the shell-analysis (`hookkit-shell`, #32) and file-activity
(`hookkit-file-activity`, #33) crates and hand-rolled everything. This pass moved
its shell handling onto `hookkit-shell`.

## Follow-up shared API migration

The implementation has since moved again, from direct `hookkit-shell` use to
the phase-agnostic `hookkit-tool-access` analyzer and bounded target resolver.
One aligned `hookkit-common::PreToolUse` handler now covers Claude, Codex,
Gemini, and Antigravity while preserving each harness's native allow/deny
output.

Structured fields, patch operations, shell inference, fallback evidence, and
typed analysis gaps now flow through the same public API. Directory and glob
targets are materialized with explicit bounds, nonexistent exact write targets
are retained, and existing paths are canonicalized before policy matching.
`access_policy` selects `inspect_known`, `deny_unresolved`, or
`deny_all_shell`; the former `block_shell_commands` key remains accepted as a
compatibility mapping.

The historical account below describes the earlier shell-only refactor. Its
local structured extraction and three-arm dispatch no longer describe the
current implementation; see
[`LIBRARY-REFINEMENT-NOTES.md`](LIBRARY-REFINEMENT-NOTES.md) for the resolution
of the gaps it identified.

## What changed

**Shell inspection now uses a real Bash parse instead of a hand-rolled lexer.**
Deleted 84 lines of bespoke shell tokenizing (`shell_tokens`, `push_shell_word`,
`shell_punctuation`, `looks_like_path`, `collect_shell_path_tokens`,
`is_shell_tool`) and replaced them with `BashAnalyzer` + `FileAccessAnalyzer`
(~10 lines of actual library calls). The guard now inspects each *resolved*
read / write / delete / move / redirection target instead of guessing which
whitespace-delimited tokens "look like paths."

**Shell tool-call extraction is now via `ShellToolCallExt`** (the per-harness
native adapters), which fixes a real bug. The old `is_shell_tool` list was
`Bash | run_shell_command | shell | exec_command`, and it read the command from
`command` / `cmd`. Antigravity's shell tool is `run_command` with the command in
`CommandLine` — so **the old code never inspected Antigravity shell commands, and
`block_shell_commands` never fired for them.** The new code handles all three
harnesses correctly (regression test: `antigravity_command_line_shell_is_inspected`,
also verified end-to-end through stdin).

**Behavioral improvements that fell out of the parse:**

- reads vs. writes vs. deletes are now distinguished;
- redirect targets (`echo x > sub/.env`) and branch-guarded reads
  (`test -f .env && cat .env`) are caught;
- paths resolve against the call's real working directory;
- a malformed shell call (`ShellToolCallMatch::Malformed`) now fails closed
  instead of being silently waved through.

Verified end-to-end: `cat .env`, `echo x > sub/.env`, `rm -rf .env` → deny;
`echo hi > notes.txt`, `grep -r secret .` → allow; `cat "$SECRET"` → allow
(documented blind spot).

**Structure.** The three near-identical harness arms now share one
`deny_reason(...)` (load policy → `evaluate`), and `evaluate` dispatches to
`evaluate_shell` / `evaluate_structured`. Structured-tool handling (path fields +
`apply_patch`) is unchanged. Dropped the now-redundant `pattern_count` field in
favor of `GlobSet::len()` / `is_empty()`. The `README.md` explanatory sections
were updated to match.

## Verification

- `cargo test -p forbidden-file-guard` → 7/7 pass
- `cargo clippy -p forbidden-file-guard --all-targets -- -D warnings` → clean
- `cargo fmt -p forbidden-file-guard -- --check` → clean
- `cargo build --workspace` → clean
- End-to-end stdin→stdout runs confirmed for structured deny, all shell access
  kinds, fail-closed, and the Antigravity fix.

See [`LIBRARY-REFINEMENT-NOTES.md`](LIBRARY-REFINEMENT-NOTES.md) for the library
gaps this refactor surfaced.
