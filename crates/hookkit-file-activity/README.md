# hookkit-file-activity

`hookkit-file-activity` records best-effort evidence that an agent may have
changed files and reconciles known observation gaps before a deferred consumer
runs.

The crate deliberately distinguishes evidence from certainty. Native or
structured file paths, parsed patches, shell-command inference, filesystem
modification times, and opt-in VCS dirty state all retain their provenance.
Unresolved shell behavior is recorded as a coverage gap instead of being
silently treated as a complete observation.

Immediate post-tool extraction delegates to `hookkit-tool-access`, then keeps
only candidates whose retained access intent may modify a file. Read-only
evidence remains available to pre-tool and policy consumers through the lower
crate but is not persisted as modified-file activity.

Pending activity is a windowed session-state entity. A consumer acknowledges
the exact generations it discharged; new observations appended while the
consumer runs remain pending. Filesystem reconciliation uses a monotonic
`reconciled_through` cursor, so session-start time is only the bootstrap lower
bound rather than the definition of every batch. A compaction epoch never moves
that bootstrap bound past the conversation's start, because compaction can
begin in the middle of the first turn.

The mtime scan walks each root depth-first in file-name order. Every lower
bound is widened by `timestamp_tolerance`, so writes on coarse or skewed
filesystem clocks near the previous cursor are not lost; files already covered
by a handled baseline stay suppressed, and others inside that overlap may be
reported once more. When a scan reaches its entry budget, it records a
`ScanResume` (a separate version-1 `reconciliation-progress` entity) instead of
silently skipping the tail: the next reconciliation scans the unscanned
remainder first, against the older lower bound it is still owed. Relative roots
and excluded roots are made absolute against the process working directory
before comparison. (The bundled runners resolve a relative `--state-dir`
against the harness project root before it reaches this crate, because Claude
Code's hook working directory follows `cd`.)

`append_report` persists only a SHA-256 digest of its key prefix, so passing
the raw hook input as the prefix does not copy large tool inputs into every
journal record.

The pending entity is version 2. In addition to direct evidence and gaps, it
accepts deterministic retry events. The Stop consumer appends a fresh retry
record for each manual or operationally incomplete file, and for each target
the traversal budget never reached, into the active generation before
acknowledging its sealed source window. Retry ids
deduplicate aggregate contributions, while the fresh record ensures that a
retry is always outside the window being acknowledged. This gives at-least-once
recovery across crashes without retaining unrelated handled files.

Handled fallback state is a separate version-1 monotonic entity keyed by a
normalized path. Each entry stores existence/type, a SHA-256 content digest for
regular files, handled time, and run id. Existing ancestors are canonicalized
so equivalent platform aliases share a key; missing suffixes are preserved.
Only mtime and Git-dirty fallback evidence consult this state. Direct structured,
patch, and shell observations always create pending work. A fingerprint read
failure never suppresses fallback evidence.

There is intentionally no migration from the former version-1 pending entity.
Session state is transient, and the new event variant would be unsafe for an
older projection to interpret. Upgrading creates a fresh version-2 subtree;
mtime and optional Git reconciliation recover best-effort candidates. Restart
the agent session when exact continuity across an in-place binary upgrade is
required.

Git dirty-state reconciliation is intentionally disabled by default because it
cannot distinguish agent edits from changes that were already present. Enable
it only when that broad fallback is appropriate for the calling application.
It runs `git --no-optional-locks status` limited to each root with a `.`
pathspec, re-roots the repository-relative porcelain paths by stripping the
root's own repository prefix (so a workspace in a monorepo subdirectory works),
honors excluded roots, and streams at most `max_entries` paths.

The shipped turn-completion runner exposes its reconciliation choices in Pkl:

```pkl
settings {
  fileActivity = new FileActivity {
    filesystemMtime = true
    vcs = "disabled" // or "git-dirty"
    timestampToleranceMillis = 2000 // widens every scan's lower bound
    maxEntries = 100000
    coverageGapPolicy = "best-effort" // or "strict"
  }
}
```

Callers using the Rust API can supply the same controls through
`ReconciliationOptions` and can independently resolve scoped targets with
`ResolveOptions`. That resolver remains a compatibility wrapper with
existing-files-only behavior; new pre-tool consumers can call
`hookkit_tool_access::resolve_targets` directly for typed unresolved outcomes,
nonexistent-write retention, and explicit symlink/error policies.

The bundled Stop runner's default coverage policy processes materialized files
and reports unresolvable targets and analyzer gaps once, in the run summary of
the Stop that first sees them, without calling resolved files dirty; retrying
the same analysis cannot resolve them, so they are discharged with the source
window. `strict` additionally blocks that one Stop. Only the scopes the
traversal budget never reached are re-queued. `resolve_files` resolves every
exact target first (these probes are not budgeted), so a directly observed
file is never dropped. Scoped resolution then
stops at `maxEntries`; the scope being walked and every later scope remain
unresolved targets, plus an explicit coverage gap, rather than silently
dropping the unwalked tail. `ResolvedFileActivity` names the scope whose walk
ran out (`exhausted_target`) separately from the scopes never attempted
(`unattempted_targets`, always the tail of `unresolved_targets`), so a caller
can retry only the scopes a later attempt can make progress on. A scope whose
root no longer exists (for example
after `rm -rf dist`), or lies in an ignored or excluded directory, has nothing
left to check and resolves as empty instead of being retained forever. Exact
files inside an excluded root or an ignored directory are reported as not
applicable rather than processed. `DEFAULT_IGNORED_DIRECTORY_NAMES` (VCS
metadata, `.context`, `.agent-hook-kit`, and common dependency, virtual
environment, cache, and build-output directories) is the default for both
option types and matches the Pkl `fileActivity.ignoredDirectoryNames` default
used by the bundled runners.

Reconciliation may still stat/hash a fallback candidate once to compare its
current fingerprint with the handled baseline. When the digest matches, it does
not append pending work or spawn configured tools. Direct observations bypass
that suppression by design, even when bytes happen to match an old baseline.

The shipped producer is `file-activity-agent-hook` from
`hookkit-tool-runner`. Bind it to Claude/Codex/Antigravity PostToolUse (and,
on Claude Code, also to `PostToolUseFailure`) and give it the same
`--state-dir` as `turn-completion-agent-hook`
and, where supported, `session-start-state-agent-hook`. The older
`session-modified-file-tracker` example is only a compatibility wrapper around
that library-owned observer.

Claude Code reports a failed tool call (for example a Bash command that edits
files and then exits non-zero) only through `PostToolUseFailure`, which carries
the same `tool_name` and `tool_input`. Library consumers can observe it with
`observe_claude_post_tool_failure`, or with `observe_tool_call` for any borrowed
native input; without that binding such writes are recovered only by the mtime
fallback. An Antigravity `PostToolUse` payload without `toolCall` (the IDE
reference shape) and a relative Antigravity path without any workspace root
are recorded as coverage gaps rather than as "nothing written".

Documented harness built-ins are analyzed from exact argument contracts in
`hookkit-tool-access`: Claude `Write`/`Edit`/`MultiEdit`/`NotebookEdit`, Codex
`apply_patch` (patch text in `command`), and Antigravity `write_to_file`,
`replace_file_content`, and `multi_replace_file_content` (PascalCase
`TargetFile`) all produce direct evidence, as do shell commands. Tools that are
documented not to touch files, such as `TodoWrite`, `WebFetch`, `update_plan`,
or `search_web`, produce no evidence and no coverage gap. Antigravity's missing
precise session-start producer still makes mtime reconciliation a best-effort
fallback.

A relative path whose tool working directory was not observable is resolved
against the hook's session directory (`PathBase::SessionCwd` in
`hookkit-tool-access`). Codex `Bash` payloads omit the `workdir` argument that
can run the command elsewhere, so `echo x > out.txt` may have written
`packages/api/out.txt` rather than `out.txt`. Such evidence is kept but
downgraded to `Heuristic` certainty, and each tool call records one coverage
gap naming those paths, so a missed file is recovered only by the mtime
fallback.
