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
bound rather than the definition of every batch.

The pending entity is version 2. In addition to direct evidence and gaps, it
accepts deterministic retry events. The Stop consumer appends a fresh retry
record for each manual, operationally incomplete, or unresolved target into
the active generation before acknowledging its sealed source window. Retry ids
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

The shipped turn-completion runner exposes its reconciliation choices in Pkl:

```pkl
settings {
  fileActivity = new FileActivity {
    filesystemMtime = true
    vcs = "disabled" // or "git-dirty"
    timestampToleranceMillis = 2000 // bootstrap scan only
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

The default coverage policy processes materialized files, requeues unresolved
scopes and analyzer gaps, and exposes those gaps in the run summary without
calling resolved files dirty. `strict` additionally blocks Stop while a gap is
present. Both policies retain the unresolved evidence for a later attempt.
Scoped resolution stops at `maxEntries`; truncation remains an unresolved target
plus an explicit coverage gap rather than silently dropping the unwalked tail.

Reconciliation may still stat/hash a fallback candidate once to compare its
current fingerprint with the handled baseline. When the digest matches, it does
not append pending work or spawn configured tools. Direct observations bypass
that suppression by design, even when bytes happen to match an old baseline.

The shipped producer is `file-activity-agent-hook` from
`hookkit-tool-runner`. Bind it to Claude/Codex PostToolUse or Gemini AfterTool
and give it the same `--state-dir` as `session-start-state-agent-hook` and
`turn-completion-agent-hook`. The older `session-modified-file-tracker`
example is only a compatibility wrapper around that library-owned observer.
Antigravity has no supported post-tool producer because its payload omits the
tool call and arguments; its Stop consumer relies on mtime reconciliation.
