# Session-scoped state

`hookkit-session-state` gives independent hook processes a common filesystem
convention and concurrency-safe primitives. It is transient coordination state,
not authoritative application data.

## Identity and layout

`SessionState::ensure` uses the harness's typed session ID, falling back to its
typed conversation ID. The raw identifier is never a path component; HookKit
hashes the harness, identity kind, and identifier:

~~~text
$TMPDIR/agent-hook-kit-<uid>/session-state/   StateRoot::default() on Unix
  .trash/                                     sessions being garbage-collected
  v1/<harness>/session|conversation/<identity-hash>/
    metadata.json
    _hookkit/metadata/v1/
      anchor.json
      starts/*.json
      workspaces/<context-key>.json
      metadata.lock
    activity/
      _hookkit.stamp
      <family-name>.stamp
    lifecycle/observations/
    topology/observations/
    families/<family-name>/v<family-version>/
      locks/
      scopes/session/
        claims/
        record-journals/<journal-name>/
          journal.lock
          pending/*.json
        entities/<entity-name>/v<entity-version>/
          descriptor.json
          active-generation.json
          generations/*.ndjson
          checkpoint.json
          projection-cache.json
          transition.json
        runs/
      scopes/actors/<actor-hash>/
      scopes/turns/<turn-hash>/
~~~

The default root is per user. On Unix, `StateRoot::default()` is
`$TMPDIR/agent-hook-kit-<uid>/session-state` (`/tmp` when `TMPDIR` is unset).
HookKit creates `agent-hook-kit-<uid>` owner-only and refuses to use one owned
by another user, so users sharing `/tmp` cannot lock each other out or tamper
with each other's state. Elsewhere the default is
`agent-hook-kit\session-state` in the already per-user temporary directory.
An explicit `StateRoot::new(path)`, such as a `--state-dir` value, is used as
given: HookKit creates it owner-only when it is missing, rejects it when it is
a symlink, and never changes the permissions of an existing directory.

A family is an explicit coordination boundary. Unrelated hooks choose different
family names and never share state files. Hooks that intentionally cooperate
choose the same family, version, scope, primitive, and entity version. A version
bump creates a fresh subtree without requiring synchronized migration across
every installed hook.

Family, entity, claim-set, journal, lock, and custom-scope names use lowercase
ASCII letters, digits, `.`, `_`, and `-`. Default macOS and Windows
filesystems compare names case-insensitively, so `Rules` and `rules` would
silently share one directory there. Uppercase is rejected so that a name means
the same thing on every platform. Run labels may use either case, because each
run directory has a unique prefix.

The native-session layout is flat. No portable field identifies a parent
session across all three harnesses, and moving a child directory after late
evidence appears would race unrelated processes. Hooks may instead write
immutable, content-addressed topology observations. Actor and turn scopes are
available where native events expose those identifiers. Unknown ancestry stays
unknown.

## Automatic standard metadata

Opening state always creates `metadata.json`; ensuring it from a
`RuntimeContext` also records exact workspace and lifecycle clues. This is
library-owned behavior, so a family does not need to remember to stamp a start
time itself:

```rust
let state = SessionState::ensure(context, StateRoot::default())?;
let metadata: SessionMetadata = state.metadata()?;
let conversation_start = state.conversation_started_at()?;
let current_epoch_start = state.current_session_started_at()?;
let epoch = state.current_epoch()?;
```

`UtcTimestamp` serializes as RFC 3339 and converts to `SystemTime` or Unix
milliseconds. Every `CapturedTimestamp` carries a `TimestampProvenance`:

- `native_event_timestamp`: reserved for a native lifecycle event that supplies
  an exact timestamp;
- `lifecycle_hook_observation`: Claude and Codex expose a start cause but no
  timestamp, so HookKit records when the start hook ran;
- `inferred_invocation_boundary`: Antigravity has no `SessionStart`; invocation
  zero is the best typed boundary available;
- `first_hook_observation`: no start event was seen, so this is a truthful lower
  precision fallback.

`SessionMetadata` separates the conversation's earliest supported start from
the current session epoch. Epoch causes are startup, resume, fork, clear,
compact, invocation start, or first-observed fallback. A fork receives a new
native session identity, so a `fork` epoch, like `startup`, starts the
conversation stored under that identity. The metadata also includes the
harness, hashed identity, accumulated workspace roots, and latest typed
transcript and artifact paths.

Every HookKit build that opens a session reads the same metadata files, so the
persisted enums tolerate values written by newer builds: an unrecognized epoch
kind or timestamp provenance decodes as `unknown` instead of failing, and a
start observation that cannot be decoded at all is skipped. A harness start
cause that this build does not model is also recorded as `unknown`.

Metadata sources are observations written under a short metadata lock, which
materializes their merged view into `metadata.json`; client families never edit
that file, and it is rewritten only when its content changes. Project context
is stored once per distinct combination of workspace roots, transcript path,
and artifact directory. A stored context is updated only when seeing it again
changes the merged view, so a long session does not accumulate one file per
hook. Earlier builds wrote one timestamped file per invocation; the next
`ensure` folds those into the per-context files.

Repeated lifecycle hooks with a native occurrence key deduplicate exactly.
Claude and Codex do not expose such a key, so observations of the same cause
within a 30-second window are coalesced to accommodate multiple independently
installed start hooks.

For the most precise start time, install the shipped no-op observer on the
native `SessionStart` event:

```text
session-start-state-agent-hook --claude [--state-dir PATH]
session-start-state-agent-hook --codex [--state-dir PATH]
```

Without that binding, the first later stateful hook still creates metadata, but
its start value is explicitly marked `first_hook_observation`. Antigravity has
no start hook to bind; an invocation hook calling `ensure` can capture its
inferred invocation-zero boundary.

A linter may use the typed start as a fallback scan boundary when direct file
tracking is incomplete. It should allow a small filesystem timestamp tolerance
and apply its own Git-ignore/file-selection policy; creation-time portability
and repository scanning do not belong in this crate.

## Record journal versus entity journal

`RecordJournal` uses one content-addressed JSON file per pending record. It is a
deliberate option rather than a legacy compatibility format. Its strengths are:

- producers use atomic file creation/rename and share the journal lock, so
  they never wait for one another;
- an identical event key and payload appended while the record is still
  pending coalesces into one entry;
- acknowledgement deletes the exact file versions a snapshot captured, so an
  identical record re-appended after the snapshot stays pending as a new
  occurrence;
- an interrupted write never produces a partial record, and a record that
  cannot be decoded is reported through `RecordJournalBatch::undecodable`
  instead of failing the snapshot; it stays pending unless the consumer calls
  `acknowledge_including_undecodable`;
- inspecting, retaining, or removing a single event is straightforward.

Use an event key that identifies the occurrence, such as a tool-call ID, when
every occurrence must be processed separately.

That makes it a good fit for sparse journals, bursty independent producers,
and workflows that consume individual events. Its costs are one inode per
pending record and directory scanning/parsing overhead as the record count
grows.

The higher-level `EntityJournal` uses NDJSON, as it is optimized for repeated
appends and aggregation. It does not use one forever-growing shared file:

1. producers briefly take `append.lock` and append one compact JSON line to the
   active `generations/<id>.ndjson` file;
2. a consumer takes `consumer.lock`, briefly takes `append.lock`, repairs any
   crash-truncated final line, seals the active generation, and lists the
   sealed ones;
3. producers immediately continue in a new generation while the consumer reads
   the immutable sealed generations and does longer work;
4. acknowledge or compact names only the sealed generations in that consumer's
   view, so later appends cannot be consumed accidentally.

Generations are named `s<20-digit sequence>-<unique>`, with the sequence
allocated under `append.lock`, so name order is append order even across clock
steps or many rotations within one millisecond. Generations named by earlier
builds sort first. `JournalEntity::apply` therefore sees events in append
order.

NDJSON simplifies and amortizes appends while generation rotation retains the
exact-window property needed by stop-time linting. A malformed
newline-terminated line is treated as corruption; only an incomplete final line
left by an append that never returned is safely discarded.

| Workload | Preferred primitive |
| --- | --- |
| Sparse, independently addressed, naturally deduplicated events | `RecordJournal` |
| High-volume appends folded into a set or another aggregate | `EntityJournal` |
| Immediate first-writer-wins decision without event history | `ClaimSet` |

## Journal-to-entity API

An entity defines how typed events fold into a typed aggregate:

```rust
trait JournalEntity: Serialize + DeserializeOwned + Clone {
    type Event: Serialize + DeserializeOwned + Clone;

    fn empty() -> Self;
    fn apply(&mut self, event: &Self::Event);
}
```

`EntityJournal::with_entity` seals a processing window, loads a checkpoint,
applies journal generations, and vends an `EntityView` to a closure. The view
contains the interpreted `state`, all source `events`, only the `new_events`
not already included in the cache, and the covered generation IDs. The closure
chooses one disposition:

- `EntityOutcome::retain(value)` caches the projection and leaves its source
  generations pending;
- `EntityOutcome::acknowledge(value)` removes a windowed entity's exact source
  generations, so their aggregate contribution disappears;
- `EntityOutcome::compact(value)` moves the aggregate into a durable checkpoint
  and removes its source generations.

`try_with_entity` additionally distinguishes a closure's domain error from a
storage error. Only one consumer interprets an entity at a time, but producers
continue appending during the closure. Consuming the same entity again from
inside its own closure, for example `set.contains_current` inside
`set.with_current`, fails with `StateError::LockReentry` instead of
deadlocking.

The projection cache stores the checkpoint revision, covered generation IDs,
and aggregate. A retained retry loads that cache and folds only later
generations. It is disposable: a missing, unreadable, or incompatible cache
causes a full rebuild from the checkpoint plus pending source. It is therefore
not forced to stable storage, and it is rewritten only when the covered
generations change.

Acknowledge and compact use a small two-phase `transition.json`: write the
intended covered generations and target checkpoint, delete the generations,
write the checkpoint if needed, then delete the transition. A later consumer
finishes an interrupted transition. Windowed delivery is therefore safe for
the linter workflow; as with filesystem queues generally, a crash before a
disposition may cause at-least-once reprocessing.

`EntityMode::Windowed` models pending work such as modified files; it must
acknowledge rather than compact, because a checkpoint could never be
acknowledged away. `EntityMode::Monotonic` models accumulated knowledge such as
loaded rules; it must compact rather than acknowledge. Optional
`CompactionPolicy::AfterEntries`, `AfterBytes`, and `AfterGenerations` policies
can compact retained monotonic state; manual compaction is the default for an
`EntityJournal`.

Opening an entity publishes its descriptor without replacing an existing one,
so two hooks that disagree on the mode cannot both succeed, even when both open
the entity for the first time at once.

## Collection affordances and concrete entities

`SetJournal<T>` provides `insert`, concurrency-safe `insert_once`,
`contains_current`, `current`, `with_current`, and `flush`. It folds
`SetEvent::Insert` into a sorted `SetAggregate`. The rules example uses a
monotonic string set, so two concurrent hooks cannot both decide a rule is new.
Each consumer call seals the generation appended since the previous one, so a
monotonic set compacts itself once a call covers 32 sealed generations;
`SetJournal::with_compaction_policy` chooses another policy. `flush` compacts,
so it is valid only for monotonic sets.

Two baseline projections are included:

- `ModifiedFiles` folds detailed `ModifiedFileEvent` values into a set of UTF-8
  paths and is intended for windowed acknowledge/retry processing;
- `LoadedRules` folds `LoadedRuleEvent` values into a monotonic set of UTF-8
  rule paths. The generic `SetJournal` additionally supplies atomic
  `insert_once`, which is what the rules example needs for its immediate
  decision.

General maps are deliberately deferred until callers select an explicit,
deterministic merge policy. Ordered lists are also deferred: independent file
appends do not imply a portable total order. A future list primitive can build
on the generation sequence that is already allocated under the append lock.

## Other primitives

- `ClaimSet::try_claim` is a lightweight atomic first-writer-wins operation.
  The claim's synced content is published in one step through a hard link, so
  a peer never sees a claim that is later withdrawn; a failed write publishes
  nothing and returns the error.
- `RunBundle` creates a unique directory; `summary.json`, written by `commit`,
  is its commit marker. Writing an artifact again replaces its content.
- `exclusive_lock` and `with_exclusive_lock` coordinate longer family work;
  `try_exclusive_lock` and `exclusive_lock_timeout` give up instead of waiting
  past a harness deadline.
- `StateScope` separates session, actor, turn, and custom coordination.
- `observe_lifecycle` and `observe_topology` write immutable observations.
  Every family shares these directories, so reads skip records that do not
  decode as the requested type.
- `SessionState::gc` explicitly removes inactive sessions older than a caller's
  retention window.

## Locks

Every lock is an advisory file lock scoped to one primitive. A thread cannot
acquire a lock it already holds: the attempt returns `StateError::LockReentry`
instead of deadlocking. Other threads and processes wait as usual. Nested
acquisitions of different locks must follow one order in every cooperating
hook:

1. family locks (`exclusive_lock`) before any entity;
2. an entity's `consumer.lock` before its own `append.lock`, which the library
   does for you, so appending from inside a consumer closure is safe;
3. when a consumer closure opens a second entity, nest the same entities in the
   same order everywhere. The turn-completion runner, for example, consumes
   pending activity and then records handled baselines.

## Durability

Every directory HookKit creates is owner-only (`0700` on Unix). Files are
published by writing a temporary file and renaming or linking it into place,
so a killed hook never leaves a torn file. Source-of-truth state is synced
before it becomes visible: generations, checkpoints, transitions, descriptors,
claims, record-journal records, run bundles, and metadata observations. Derived
state that every reader rebuilds or tolerates losing is not forced to stable
storage: `metadata.json`, projection caches, and activity stamps. Immutable
content-addressed files are never rewritten once they exist. The configured
root itself may not be a symlink.

Every I/O error names the operation and the path that failed.

## Batched formatter/linter mechanics

`hookkit-file-activity` builds on these primitives with provenance-bearing
evidence and gap events. The shipped `file-activity-agent-hook` appends those events
to the windowed `agent-hook-kit.file-activity` entity. At turn completion,
`turn-completion-agent-hook`:

1. reconciles workspace mtimes through a captured cutoff and advances a
   monotonic cursor only after discoveries are durable;
2. obtains a `PendingFileActivity` entity view and expands its exact,
   descendant, glob, and workspace targets;
3. runs matching Pkl-configured read-only checks, one remedy for each initially
   dirty workflow, and authoritative checks invalidated by observed writes;
4. writes complete per-tool output and commits a run summary;
5. appends idempotent retry evidence for manual, operationally incomplete, and
   unresolved work into the active generation;
6. records content-based handled baselines for discharged files; and
7. acknowledges only the sealed source generations, then either emits a native
   no-op or asks the harness to continue and points it at the committed run.

The summary describes this planned disposition because it is committed before
the state transition. A crash before disposition leaves the sealed source
pending; a crash after retry append may duplicate an idempotent retry but cannot
lose it. Files modified while linters run also land in the next generation and
remain pending when the completed window is acknowledged.
Normal clean, auto-fixed, and manual results are recorded per file with
worst-wins aggregation across tools. Operational failures remain a separate
bucket, and conservative batch findings retain their shared report provenance.

The older `ModifiedFiles`/`ModifiedFileEvent` projection remains a small
session-state convenience for callers that only need an exact-path set. It does
not model inference provenance, coverage gaps, scoped targets, or
reconciliation; new tracking workflows should use `hookkit-file-activity`.

## Boundaries

The store coordinates cooperating local processes; it is not hardened against
another local process deliberately replacing internal directories with
symlinks. Advisory locks work only when all consumers use the same resource
name. Do not share an entity directory between different event or aggregate
schemas without bumping its version.

`SessionState::gc` renames a stale session into `.trash/` before deleting it,
so a hook opening that session concurrently sees either the complete session
or a fresh one. A session removed by a concurrent pass is skipped, and a
failure affecting one session is counted in `GcReport::failed` without
stopping the pass. Every `ensure` or `open` refreshes the session's
`activity/_hookkit.stamp`, and opening a family refreshes that family's stamp.
Garbage collection can still race a hook whose lifetime exceeds the retention
age; use a conservative window and an external maintenance point. Transient
state loss causes conservative repeated work and lower-precision metadata, not
recovery of authoritative data.
