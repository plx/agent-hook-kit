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
crate but is not persisted as modified-file activity. The version-1 activity
journal schema remains unchanged.

Pending activity is a windowed session-state entity. A consumer acknowledges
the exact generations it discharged; new observations appended while the
consumer runs remain pending. Filesystem reconciliation uses a monotonic
`reconciled_through` cursor, so session-start time is only the bootstrap lower
bound rather than the definition of every batch.

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
  }
}
```

Callers using the Rust API can supply the same controls through
`ReconciliationOptions` and can independently resolve scoped targets with
`ResolveOptions`.
