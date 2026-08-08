# Session-scoped state

`hookkit-session-state` gives independent hook processes a common filesystem
convention and concurrency-safe primitives. It is transient coordination state,
not authoritative application data.

## Identity and layout

`SessionState::ensure` uses the harness's typed session ID, falling back to its
typed conversation ID. The raw identifier is never a path component; HookKit
hashes the harness, identity kind, and identifier:

~~~text
$TMPDIR/agent-hook-kit/session-state/v1/
  <harness>/session|conversation/<identity-hash>/
    metadata.json
    _hookkit/metadata/v1/
      anchor.json
      starts/*.json
      workspaces/*.json
      metadata.lock
    activity/
    lifecycle/observations/
    topology/observations/
    families/<family-name>/v<family-version>/
      locks/
      scopes/session/
        claims/
        record-journals/<journal-name>/pending/*.json
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

A family is an explicit coordination boundary. Unrelated hooks choose different
family names and never share state files. Hooks that intentionally cooperate
choose the same family, version, scope, primitive, and entity version. A version
bump creates a fresh subtree without requiring synchronized migration across
every installed hook.

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
the current session epoch. Epoch causes are startup, resume, clear, compact,
invocation start, or first-observed fallback. It also includes the harness,
hashed identity, accumulated workspace roots, and latest typed transcript and
artifact paths.

Metadata sources are immutable observations. A short metadata lock materializes
their merged view into `metadata.json`; client families never edit that file.
Repeated lifecycle hooks with a native occurrence key deduplicate exactly.
Claude and Codex do not expose such a key, so observations of the same cause
within a 30-second window are coalesced to accommodate multiple independently
installed start hooks.

For the most precise start time, bind a small observer that calls `ensure` to
the native Claude or Codex `SessionStart` event. The complete implementation
used by the deferred quality workflow lives in
[Velvet Glove](https://github.com/plx/velvet-glove).

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

- producers use atomic file creation/rename and do not serialize on a shared
  append lock;
- an identical event key and payload is naturally idempotent;
- acknowledgement deletes exact independently-addressed records;
- corruption or an interrupted write is isolated to one record;
- inspecting, retaining, or removing a single event is straightforward.

That makes it a good fit for sparse journals, bursty independent producers,
and workflows that consume individual events. Its costs are one inode per
pending record and directory scanning/parsing overhead as the record count
grows.

The higher-level `EntityJournal` uses NDJSON, as it is optimized for repeated
appends and aggregation. It does not use one forever-growing shared file:

1. producers briefly take `append.lock` and append one compact JSON line to the
   active `generations/<id>.ndjson` file;
2. a consumer takes `consumer.lock`, briefly takes `append.lock`, repairs any
   crash-truncated final line, and seals the active generation;
3. producers immediately continue in a new generation while the consumer does
   longer work;
4. acknowledge or compact names only the sealed generations in that consumer's
   view, so later appends cannot be consumed accidentally.

NDJSON simplifies and amortizes appends while generation rotation retains the
exact-window property needed by stop-time linting. A malformed non-final line
is treated as corruption; only an incomplete final line is safely discarded.

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
continue appending during the closure.

The projection cache stores the checkpoint revision, covered generation IDs,
and aggregate. A retained retry loads that cache and folds only later
generations. It is disposable: a missing or incompatible cache causes a full
rebuild from the checkpoint plus pending source.

Acknowledge and compact use a small two-phase `transition.json`: write the
intended covered generations and target checkpoint, delete the generations,
write the checkpoint if needed, then delete the transition. A later consumer
finishes an interrupted transition. Windowed delivery is therefore safe for
the linter workflow; as with filesystem queues generally, a crash before a
disposition may cause at-least-once reprocessing.

`EntityMode::Windowed` models pending work such as modified files.
`EntityMode::Monotonic` models accumulated knowledge such as loaded rules; a
monotonic entity must compact rather than acknowledge. Optional
`CompactionPolicy::AfterEntries` and `AfterBytes` policies can compact retained
monotonic state; manual compaction is the default.

## Collection affordances and concrete entities

`SetJournal<T>` provides `insert`, concurrency-safe `insert_once`,
`contains_current`, `current`, `with_current`, and `flush`. It folds
`SetEvent::Insert` into a sorted `SetAggregate`. The rules example uses a
monotonic string set, so two concurrent hooks cannot both decide a rule is new.

Two baseline projections are included:

- `ModifiedFiles` folds detailed `ModifiedFileEvent` values into a set of UTF-8
  paths and is intended for windowed acknowledge/retry processing;
- `LoadedRules` folds `LoadedRuleEvent` values into a monotonic set of UTF-8
  rule paths. The generic `SetJournal` additionally supplies atomic
  `insert_once`, which is what the rules example needs for its immediate
  decision.

General maps are deliberately deferred until callers select an explicit,
deterministic merge policy. Ordered lists are also deferred: independent file
appends do not imply a portable total order. A future list primitive needs a
sequence allocator under the append lock rather than treating directory order
as event order.

## Other primitives

- `ClaimSet::try_claim` is a lightweight atomic first-writer-wins operation.
- `RunBundle` creates a unique directory; `summary.json`, written by `commit`,
  is its commit marker.
- `exclusive_lock` and `with_exclusive_lock` coordinate longer family work.
- `StateScope` separates session, actor, turn, and custom coordination.
- `observe_lifecycle` and `observe_topology` write immutable observations.
- `SessionState::gc` explicitly removes inactive sessions older than a caller's
  retention window.

Directories are private (`0700` on Unix), replacements use temp-write, fsync,
and rename, and the configured root itself may not be a symlink.

## Coordinated downstream consumers

`hookkit-file-activity` builds on these primitives with provenance-bearing
evidence, gap events, generation sealing, and acknowledgement. A coordinated
consumer can reconcile and seal a pending window, commit its own durable run
artifacts, requeue incomplete evidence, and acknowledge only the generations it
successfully handled. The complete deferred linting-and-formatting workflow
using that pattern lives in
[Velvet Glove](https://github.com/plx/velvet-glove).

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

Garbage collection can race a hook whose lifetime exceeds the retention age;
use a conservative window and an external maintenance point. Transient state
loss causes conservative repeated work and lower-precision metadata, not
recovery of authoritative data.
