# Post-tool-use runner design

The runner owns tool selection, recursive file discovery, Pkl policy, execution,
classification, diagnostics artifacts, and the `RunnerDomainOutcome` semantic
result. Core, native, common, and runtime crates do not depend on those policies.

The crate is split by responsibility: `cli` parses the four bundled binaries'
arguments (clap, exit 0 for help and exit 1 with a diagnostic naming the binary
for usage errors, never the harness-blocking exit 2); `convert` resolves the Pkl
schema into crate-private `spec` types (reusing the schema's enums); `exec`
selects files, partitions jobs, renders argv, and runs bounded subprocesses;
`roots` selects the working root (including Claude Code worktrees);
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
fail on Antigravity.

A failure of the run itself, a `hard-failure` missing tool or a configuration
that cannot be loaded (including a missing `pkl`), stops the run. On Claude
Code and Codex it is lowered to the same exit-0 response: an `error:` notice
leads `systemMessage`, followed by the notices of the tools that already ran,
and their agent feedback stays in `additionalContext`. An exit-1 hook error
would lose all of it: Claude shows the user only the first stderr line and
gives the agent nothing, and Codex discards the stderr of any exit other than
0 or 2. On Antigravity, which has no channel, it remains a hook error whose
text includes the accumulated notices and feedback.

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
(owner-only on Unix); Pkl errors are rewritten to name the real source file,
and frames from the schema modules linked into each layer name
`<hookkit embedded builtins>/…`. Every `pkl eval` is killed after 60 seconds
(`PKL_EVAL_TIMEOUT`), so an import stalled on the network fails the
configuration instead of hanging the hook. `settings.fileActivity` merges
field by field across layers like `deferredReporting`.

Discovery starts from the same working root in both runners: on Claude Code
`CLAUDE_PROJECT_DIR`, or the linked Git worktree of the same repository that
the session entered (Claude keeps `CLAUDE_PROJECT_DIR` at the main checkout
while `cwd` follows the session into the worktree), never the `cwd` that
follows `cd`; elsewhere the first workspace root. An Antigravity conversation
without a workspace starts from the deepest directory containing the written
files. A relative `--config` or `--state-dir` is anchored at
`CLAUDE_PROJECT_DIR` on Claude Code and the first workspace root elsewhere,
and a bare file name (`--config hooks.pkl`) names a file in that directory.

Phases without `phaseOrder` run by mode (format, fix, verify, check-only) and
then alphabetically by id; Pkl mapping order is not preserved through JSON.

## Execution

Every external command runs with null stdin and captured output under two
deadlines: `settings.commandTimeoutSeconds` (default 300) bounds one command,
and `settings.runTimeoutSeconds` (default 540, below the 600-second default
hook timeout of Claude Code and Codex) bounds all commands of one hook
invocation together. Each command gets the smaller of the two, and a command
that would start after the budget is spent is not started; either way the
result is an operational failure. `0` disables either deadline. Output
collection stops two seconds after the command exits, so a daemon that
inherited the pipes cannot hold the hook open.

On Unix each command runs in a process group of its own, led by a tiny
`/bin/sh` watchdog that blocks reading a pipe only the hook process can write.
A runner timeout kills the whole group, so descendants such as rustc below
`cargo clippy --fix` cannot keep writing after the hook returns. The harness
can also kill the hook first: Codex starts every command hook in a new
session and SIGKILLs the hook's process group when the hook times out or the
turn is interrupted, which does not reach a group of its own. The watchdog
then sees end-of-file on its pipe and kills its group, so no tool outlives the
hook, whether the hook died from SIGKILL, SIGTERM, or SIGHUP. When a command
finishes normally the runner releases the watchdog, and a daemon the command
deliberately left behind keeps running. Spawns are serialized so one
command's pipes are close-on-exec before another job can fork.

A phase with `invocation = "per-file"` runs once for each file of its job,
for tools that read several file arguments as one input stream (the `jq`
builtin: `jq empty a.json b.json` parses the files as one concatenated JSON
stream). The Stop-time compatibility translation of such a tool is per-file
as well.

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
whose metadata changed and compares by content. Metadata identity proves a
file unchanged only when a later write would have moved a timestamp: on
filesystems with coarse clocks (FAT's two seconds; one second on HFS+, ext3,
and many network mounts) a same-length rewrite in the same tick leaves every
stamp field equal. A digest is therefore reused only for files whose mtime and
ctime lie more than two seconds before it was computed, which in practice
re-hashes just the files written moments before the capture. Workspace and
matching-glob walks prune `fileActivity.ignoredDirectoryNames` plus a fixed
floor (`.agent-hook-kit`, `.git`, `.hg`, `.jj`, `.svn`, `node_modules`,
`target`) that a customized list cannot remove.

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
Stop runner's first reconciliation and configuration root is the session's
working root (see Configuration): `CLAUDE_PROJECT_DIR`, or the linked worktree
the session entered. Linked worktrees of the same repository nested below a
root (Claude creates them under `.claude/worktrees/`) belong to other
sessions, so the heuristic mtime and Git-dirty fallbacks exclude them.

Antigravity fires Stop for every loop termination. A Stop whose
`terminationReason` is `max_steps_exceeded` or `error`, or that carries an
`error` message, ends a loop the harness is stopping on purpose, and
`decision: "continue"` would re-enter it; such a Stop is allowed without
running any tool, and the pending work stays queued for the next Stop.

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
an unavailable tool, not an operational failure: it is reported to the user
through the `unavailableTool` reporting bucket, does not block, and its files
are not re-queued. `hard-failure` and `harness-block` make it an operational
failure that blocks. An environment problem with the configuration (a missing
or unrunnable `pkl`, a `pkl eval` timeout, an unreadable configuration file,
or a failed staging directory) is likewise reported without blocking, while
the candidates stay queued until configuration can be evaluated; evaluation
and decoding errors, which the agent's own edits may cause, block.

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
baseline; direct observations always requeue the path. Every deliberately
discharged file gets a baseline: clean, auto-fixed, and deleted files, files
no configured tool covers, and files whose only tool is unavailable, so the
fallbacks do not rediscover them (and repeat the same notice) while their
content is unchanged.

`summary.json` records the run's `status`, the most severe condition first:
`operational-failure` (a tool or the configuration failed), `issues` (a file
still needs manual fixes), `incomplete` (nothing blocks, but a configured tool
was unavailable or file-activity coverage has gaps, so some files were not
checked), `not-applicable` (no configured tool applies to any candidate), or
`clean`.

Coverage gaps use the Pkl `fileActivity.coverageGapPolicy` and are reported
once. A message-only gap (for example an unanalyzable shell command) or a
target that cannot be materialized is summarized in the Stop that first sees it
and discharged with the source window, because retrying the same analysis
cannot resolve it. Gaps from mtime or Git-dirty reconciliation describe
persistent conditions (a directory the walker cannot read, a scan that hit
`maxEntries`, a root that is not a Git repository, a missing `git`) that
reconciliation observes again at every Stop, so the runner family remembers
each reconciliation gap message once reported in the session: later Stops
leave it out of the result, the messages, and the blocking fingerprint, and a
window holding nothing but such gaps is discharged without running any tool.
The default `best-effort` policy reports gaps without blocking; `strict` also
blocks that one Stop. Recursive target expansion is
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
fix) blocks again. Conditions that change on every attempt (an agent that
rewrites a file differently each time without fixing it, or alternates
between two broken states) never repeat the previous fingerprint, and only
Claude caps continuations (at eight); the runner therefore allows the Stop,
with a note to the user, after eight consecutive blocked attempts in one
continuation chain. A chain continues while `stop_hook_active` is true, and on
Antigravity until an attempt is allowed. Suppressed work stays queued for the
next Stop.

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
hook failure after committing the summary. A blocked Stop that fails this way
keeps its pending state, since its block was never delivered. An allowed Stop
ends the turn whether or not the hook fails, so its results are recorded as
for any other allowed Stop (the summary's `stateDisposition.source` says which
happened); retaining the window would re-run every tool and re-report the same
message-only gaps at every later Stop. `"best-effort"` omits that audience.
`"best-effort-with-warnings"` also emits an omission warning through
`systemMessage` when available, or through the blocked Antigravity `reason`;
an allowed Antigravity stop has no channel, so its warnings are recorded only
in the summary (`warningsDelivered: false`). Allowed completion stays allowed
under both best-effort modes.

The policy governs configured text only. An audience rendered entirely from
the built-in templates (for example the default "re-read changed files" agent
text on an allowed Stop, or any user text on Antigravity) is dropped where the
Stop has no channel for it, without a warning or a strict failure, and is
recorded as `omitted-builtin`. Customizing any template of an audience (a
bucket pair's side or its master template) makes that audience configured.
The summary records emitted, omitted, omitted-builtin, empty, synthesized, or
unrepresentable status for each audience.
