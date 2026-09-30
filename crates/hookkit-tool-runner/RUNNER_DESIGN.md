# Post-tool-use runner design

The runner owns tool selection, recursive file discovery, Pkl policy, execution,
classification, diagnostics artifacts, and the `RunnerDomainOutcome` semantic
result. Core, native, common, and runtime crates do not depend on those policies.

The crate is split by responsibility: `cli` parses the four bundled binaries'
arguments (clap, exit 0 for help and exit 1 with a diagnostic naming the binary
for usage errors, never the harness-blocking exit 2); `convert` resolves the Pkl
schema into crate-private `spec` types (reusing the schema's enums); `exec`
selects files, partitions jobs, renders argv, and runs bounded subprocesses;
`snapshot` attributes writes to one command; `post_tool` is the immediate
runner and its native lowering; `turn_completion`, `stop_guard`, and `deferred`
implement the Stop runner. The execution specs are not public: every entry
point takes CLI options and Pkl configuration.

Per-harness lowering is intentionally explicit. The aligned runtime preserves a
lossless Claude, Codex, or Antigravity input arm; runner-local policy
discovers paths from that exact value; and the runner constructs the
corresponding native output arm. No generic output envelope or universal
lowering layer participates.

## Immediate PostToolUse lowering

Neither Claude Code nor Codex shows stderr from a hook that exits 0 (Claude
writes it to the debug log, Codex ignores it), while both show a PostToolUse
response's top-level `systemMessage` to the user. The runner therefore lowers:

| Audience | Claude Code / Codex | Antigravity |
| --- | --- | --- |
| User notices | `systemMessage` | unavailable |
| Agent feedback | `hookSpecificOutput.additionalContext` | unavailable |
| `harness-block` decision | `decision: "block"` with a non-empty `reason` | unavailable |

User notices name each diagnostics artifact; full tool output stays in the
artifact and is never inlined. The user text is truncated to Claude's
10,000-character `systemMessage` cap (for both harnesses) with a pointer to the
first diagnostics artifact. A `harness-block` decision is lowered to the
structured block response on exit 0 so earlier tools' notices and agent
feedback (for example "prettier changed a.ts; re-read it") are delivered
together with the block reason. `loweringPolicy = "strict"` can therefore only
fail on Antigravity. A `hard-failure` missing tool is a hook error whose text
includes the notices and feedback accumulated from earlier tools.

Antigravity `PostToolUse` includes the originating tool call, so the runner can
derive call-scoped file candidates from its name and arguments. It still has no
tool result or changed-file list, and its empty output object cannot carry user
or agent messages or a block. Strict lowering rejects those messages and
best-effort omits them. Best-effort-with-warnings writes a versioned,
collision-safe JSON loss record beneath the event's exact
`artifactDirectoryPath`, then uses successful protocol stderr to point to that
record while preserving exact `{}` stdout. The official payload examples show
that path with a literal `~/` prefix, so a leading `~` is expanded against the
hook's home directory; any other relative path is refused rather than created
below the hook's current directory. Failure to persist the record fails the
hook instead of silently dropping it.

The immediate runner analyzes the tool call before evaluating any Pkl: a call
that wrote no existing file returns the native no-op without staging or
running `pkl`. Full tool output goes to the tool's `diagnostics.directory` or
`settings.diagnosticsDirectory`; without either (only possible when settings
are built in Rust) it goes to the per-user `ArtifactManager::in_temp_dir`
directory, never a shared, predictable temporary directory.

## Configuration

Configuration layers are discovered as home, project chain, and local chain.
The home directory's own file is only the home layer, and the project root is
the deepest directory holding a project or local config (never `$HOME`),
independent of merge order. All layers are evaluated by one `pkl eval` of a
generated aggregator over one private, exclusively created staging directory
(owner-only on Unix); Pkl errors are rewritten to name the real source file.
`settings.fileActivity` merges field by field across layers like
`deferredReporting`.

Phases without `phaseOrder` run by mode (format, fix, verify, check-only) and
then alphabetically by id; Pkl mapping order is not preserved through JSON.

## Execution

Every external command runs with null stdin, captured output, and
`settings.commandTimeoutSeconds` (default 300, `0` disables). On Unix each
command leads its own process group, and a timeout kills the whole group so
descendants such as rustc below `cargo clippy --fix` cannot keep writing after
the hook returns; a timeout is an operational failure. Output collection stops
two seconds after the command exits, so a daemon that inherited the pipes
cannot hold the hook open.

Batch invocations whose `Files` or `WorkspaceFiles` arguments would exceed a
conservative argv budget (128 KiB on Unix) are split into consecutive chunks of
the same workspace, so a large candidate set never fails with `E2BIG`. Chunks
of one workspace run serially when the tool may write beyond its targets.
`WorkspaceFiles` paths that start with `-` are rendered as `./-…` so they cannot
be parsed as options.

Include and exclude globs match the project-relative path; only files outside
the project root fall back to their absolute path. The default global excludes
are `**/.git/**` and `**/node_modules/**`.

Write attribution snapshots store metadata and a streaming SHA-256 digest per
file rather than file contents; the post-command recapture re-hashes only files
whose metadata changed and compares by content. Workspace and matching-glob
walks prune `fileActivity.ignoredDirectoryNames`.

To add a tool, extend the Pkl catalog and its fake-executable orchestration cases;
real-tool fixtures belong to the opt-in compatibility lane. To add a harness,
first prove how its native event yields changed files, then add an explicit final
lowering arm and exact native fixture tests.

Performance budgets are not yet release guarantees. The hermetic smoke lane checks
behavior but does not currently record startup, clean no-op, or subprocess timing;
add a dedicated benchmark/measurement lane before publishing performance claims.

## Turn-completion batching

turn-completion-agent-hook reuses the same Pkl catalog and execution engine,
but obtains candidate files from the `hookkit-file-activity` pending entity
maintained by the bundled `file-activity-agent-hook`. That quiet aligned
PostToolUse observer delegates structured, patch, and shell analysis to
`hookkit-file-activity::observe_post_tool` and the shared tool-access layer, and
keys each journal record by the event family, the tool-call id, and a digest of
the input rather than the raw input. On Claude Code the same binary also
accepts `PostToolUseFailure` (through
`hookkit-file-activity::observe_claude_post_tool_failure`), because Claude
reports a failed call only there even when it wrote files, for example
`sed -i ... && pytest` with failing tests; bind it to both events. Any other
Claude event is a non-blocking hook error. The immediate runner uses the same observation path for
exact file candidates instead of maintaining a second open-payload walker.
Before taking the entity view, it reconciles workspace mtimes from the prior
durable cursor, using current-session start metadata only as the first lower
bound. The aligned lifecycle is Claude, Codex, or Antigravity Stop. Antigravity
lacks a precise session-start producer but its PostToolUse tool-call evidence
can feed the tracker directly.

A relative `--state-dir` resolves against the harness project root
(`CLAUDE_PROJECT_DIR` on Claude Code, otherwise the first workspace root of the
native input), never the hook process's current directory: Claude Code runs
hooks in the agent's current directory, which follows `cd`. On Claude Code the
Stop runner also uses `CLAUDE_PROJECT_DIR` as its first reconciliation and
configuration root.

Reconciliation and target resolution exclude the session state directory and
every configured diagnostics directory, and the default
`ignoredDirectoryNames` include `.agent-hook-kit`, so the hook's own
configuration and artifacts never become Stop candidates.

One runner-family advisory lock serializes stop attempts for a native session.
The consumer seals NDJSON generations and obtains their cached set projection
before executing tools. Stop-time `workflows` are distinct from the immediate
runner's legacy `phases`: all non-mutating initial checks run first, only dirty
workflows receive one ordered remedy, and snapshot-discovered writes invalidate
intersecting target-file or workspace checks for one authoritative final sweep.
Check stages retain bounded job parallelism and deterministic result ordering.
`failFast` is scoped to the failing tool: its remaining remedies are skipped,
while unrelated tools still repair their files.

Under `missingToolPolicy = "user-notice"` (the default) a missing executable is
an unavailable tool, not an operational failure: it is reported to the user,
does not block, and its files are not re-queued. `hard-failure` and
`harness-block` make it an operational failure that blocks. A missing `pkl`
binary is likewise reported without blocking, while the candidates stay queued
until configuration can be evaluated.

When a builtin has no explicit `workflows`, catalog validation proves its
compatibility translation has a read-only final phase before it can ship as
enabled. The generated
[`builtin-deferred-workflow-audit.md`](../../planning/builtin-deferred-workflow-audit.md)
records every command, inferred or explicit scope, invocation granularity, and
known limitation. Immediate PostToolUse continues to use legacy `phases`.

Every executed deferred command writes its own artifact under a deterministic
tool/workflow/job/phase path in a unique run bundle. Artifact metadata includes
structured argv, working directory, candidate and changed files, exit code,
classification, full output, and its report identity. One report/artifact can
therefore be linked by every conservatively attributed file, while a file
covered by several tools retains all distinct links.

The runner commits `summary.json` (schema version 2) only after every command
artifact is durable and before changing pending state. The summary contains run
identity, counts, normal buckets, current groups, artifact paths and a separate
path-to-contents map, the complete result model, the Stop decision and its
loop-guard provenance, rendered-message metadata, and the planned source
disposition. The runner then appends stable retry evidence for only manual and
operationally incomplete files and for targets that were never attempted,
records content-based handled baselines for discharged work, and acknowledges
the sealed source generations. New observations written during execution are
outside the snapshot and remain pending independently. Mtime and opt-in
Git-dirty reconciliation suppress only fingerprints that still match a handled
baseline; direct observations always requeue the path.

Coverage gaps use the Pkl `fileActivity.coverageGapPolicy` and are reported
once. A message-only gap (for example an unanalyzable shell command) or a
target that cannot be materialized is summarized in the Stop that first sees it
and discharged with the source window, because retrying the same analysis
cannot resolve it. The default `best-effort` policy reports gaps without
blocking; `strict` also blocks that one Stop. Recursive target expansion is
bounded by `fileActivity.maxEntries`; the target that exhausts the budget
(`ResolvedFileActivity::exhausted_target`) is reported once, and only the
targets that were never attempted (`ResolvedFileActivity::unattempted_targets`)
are re-queued for the next Stop. Batch/workspace findings are conservatively attributed to all
job candidates, while byte snapshots preserve exact files actually changed by
remedies.

### Stop loop protection

A blocked Stop hands the turn back to the agent; blocking again when nothing
changed cannot make progress (Claude Code only bounds the loop with its
continuation cap, Codex has none, Antigravity re-enters its loop). The runner
fingerprints the blocking conditions (the content of every file that still
needs manual fixes, each operational problem, and strict coverage gaps) and
records the fingerprint of every attempt in the runner family. An identical
repeat is allowed to stop, with a note to the user, when Claude's or Codex's
`stop_hook_active` marks the Stop as a continuation, and always on Antigravity,
which has no such signal. A new turn therefore still gets one block for a
problem that remains, and any change to the conditions (for example a partial
fix) blocks again. Suppressed work stays queued for the next Stop.

### Exact Stop lowering

Rendered deferred messages are lowered without a common output envelope. The
capability matrix is:

| Native event | Allowed user | Allowed agent | Blocked user | Blocked agent |
| --- | --- | --- | --- | --- |
| Claude Stop | `systemMessage` | unavailable | `systemMessage` | `reason` |
| Codex Stop | `systemMessage` | unavailable | `systemMessage` | `reason` |
| Antigravity Stop | unavailable | unavailable | unavailable | `reason` (with `decision: "continue"`) |

Claude's Stop `hookSpecificOutput.additionalContext` continues the conversation
through the same loop protection as `decision: "block"`, so it is not an
allowed-completion channel, and a blocked Claude Stop sends the agent text once,
as `reason`. Antigravity injects `reason` only when `decision` is `"continue"`.

A blocked completion always carries a non-empty reason: when the rendered agent
message is empty (for example an empty template), a fixed reason pointing at
`summary.json` is synthesized and the agent audience is recorded as
`synthesized`. Codex treats a blank block reason as an invalid hook result and
lets the turn end.

`loweringPolicy = "strict"` turns any nonempty unavailable audience into a
hook failure after committing the summary but before changing pending state.
`"best-effort"` omits that audience. `"best-effort-with-warnings"` also emits
an omission warning through `systemMessage` when available, or through the
blocked Antigravity `reason`; an allowed Antigravity stop has no channel, so its
warnings are recorded only in the summary (`warningsDelivered: false`). The
summary records emitted, omitted, empty, synthesized, or unrepresentable status
for each audience. Allowed completion stays allowed under both best-effort
modes.
