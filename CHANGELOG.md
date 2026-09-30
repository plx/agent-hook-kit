# Changelog

## Unreleased — contract-first reboot

- Added a frozen, source-provenance-aware contract catalog for Claude Code,
  Codex, and Antigravity events (46 at introduction; 50 since the 2026-09-29
  harness refresh below).
- Added event-associated typed command APIs and exact byte/process emission.
- Added harness-native command-environment types for every catalog event,
  deterministic map-based parsing, selective process capture, redundant input
  validation, and explicit environment parameters on every command handler.
- Added selected-harness resolution, optional hints, and a non-executing detector.
- Added a distinct Antigravity crate and lossless three-harness PostToolUse arms.
- Removed Gemini CLI support, including its native crate, contracts, fixtures,
  examples, runtime selection, and runner integrations.
- Split hermetic required tests from opt-in real-tool and live-harness lanes.
- Added generated Rust/catalog parity and support reporting.
- Defined HookKit's runtime scope as command bindings; catalogued HTTP and other
  non-command bindings are explicit unsupported implementation targets.
- Removed the legacy generic native/common runners and protocol-invalid output
  envelopes; use `run_event`, `run_harness`/`dispatch_builtin_harness`, or
  `run_aligned_event`.
- Refreshed Claude Code against the 2026-08-05 hook reference: added
  `DirectoryAdded`, forked session starts, current optional fields and output
  controls, deferred pre-tool decisions, dynamic watch paths, and relative
  worktree paths.
- Fixed `run_aligned_event`, `run_typed`/`run_event_with_diagnostics`,
  `run_harness`, and `dispatch_builtin_harness` so a failed stdin read, a bad
  hook environment, or a handler/parse error now write a concise one-line
  diagnostic (program, hook identity, full cause chain) to stderr before
  `exit 1`, instead of leaving both stdout and stderr byte-for-byte empty.
- Downgraded two Claude Code environment checks that could reject a
  legitimate real-world hook invocation from hard failures to silent
  fallbacks: a missing `CLAUDE_ENV_FILE` on `SessionStart`/`Setup`/
  `CwdChanged`/`FileChanged` now yields `environment_file: None` instead of
  an error, and a lone
  `CLAUDE_PLUGIN_ROOT`/`CLAUDE_PLUGIN_DATA`/`CLAUDE_PLUGIN_OPTION_*` value
  without its pair (ambient state from an unrelated parent process) now
  yields `plugin: None` instead of an error, matching how
  `CodexCommandEnvironment` already treats a partial alias set.
  (Correction: this entry previously called `CLAUDE_ENV_FILE` undocumented.
  The hooks reference documents it for exactly those four events, but no
  sentence promises that it is set and its own examples guard it with
  `[ -n "$CLAUDE_ENV_FILE" ]`, so it is optional.)

### 2026-09-29 harness refresh

All three harnesses moved to new frozen contract snapshots, selected together
with command-environment supplement `command-environments-2026-09-30-r1`. The
selection now covers 50 events: 33 Claude Code, 12 Codex, and 5 Antigravity.
Audits: `planning/audits/2026-09-29-claude-code-hooks.md`,
`planning/audits/2026-09-29-codex-hooks.md`, and
`planning/audits/2026-09-29-antigravity-hooks.md`.

- **Claude Code `docs-2026-09-29-r1`** (Claude Code 2.1.285; hooks reference
  `c20e140e…`, CHANGELOG at `2282079d`, Agent SDK 0.3.285):
  - New `PreModelSwitch` (can allow, deny, ask, or block the switch) and
    `PostModelSwitch` (adds context) events in `hookkit_claude::model_switch`,
    sharing a typed `ModelSwitchInput`. `hookkit_claude::events` re-exports
    every event and output, and `hookkit_claude::SNAPSHOT` names the snapshot.
  - Claude Code now reads JSON stdout on every exit code. New
    `into_blocking_error`/`into_feedback_error` keep structured JSON on stdout
    while exiting 2, and a JSON `reason` replaces stderr as the block message.
  - `PermissionRequest` exit 2 no longer denies (its stderr is discarded);
    deny only with `PermissionRequestOutput::deny`.
  - `Setup`, `InstructionsLoaded`, `Notification`, `StopFailure`,
    `SessionEnd`, and `PostCompact` discard JSON output fields
    (`terminalSequence` at most); builders for discarded fields are
    deprecated, and `SetupOutput::with_context` emits `{}`.
  - `WorktreeRemove` is decided by exit code alone, so a HookKit runtime error
    (exit 1) blocks removal. The `WorktreeCreate` path is the last non-empty
    stdout line; absolute paths with `.`/`..` segments are refused.
  - Plain-text context that starts with `{` and ends with `}` is parsed as
    JSON (and dropped if invalid), so `text_context` now sends such text as
    structured `additionalContext`.
  - Exit-2 stderr routing is recorded per event (user only for
    `UserPromptSubmit`, `UserPromptExpansion`, and manual `PreCompact`; debug
    log for `ConfigChange`; nowhere for `Elicitation`/`ElicitationResult`).
  - New inputs: `scratchpad_dir` everywhere, SessionStart resume/fork cost
    fields, a typed `McpServer` on tool events, new `StopFailure` errors, an
    open `notification_type`, and a nullable `PreCompact.custom_instructions`.
  - New outputs: `PostToolUseOutput::with_classifier_context`, and
    `block(reason)` on `TaskCreated`, `Elicitation`, `ElicitationResult`,
    `UserPromptSubmit`, and `UserPromptExpansion`.
- **Codex `commit-ff6aec9-r1`** (openai/codex `rust-v0.159.2`; schemas vendored
  under `contracts/vendor/codex/ff6aec9…/generated`):
  - New `Interrupt` event (`hookkit_codex::catalog::Interrupt`): main thread
    only, output is empty or `systemMessage`-only JSON
    (`InterruptOutput::no_op`/`system_message`), and it cannot block.
  - SessionStart reports the `fork` source, mapped to a fork session boundary.
  - `no_op()` now emits empty stdout on every event (Codex's no-op outcome)
    instead of `{}`.
  - Output that Codex would silently fail open is refused at emission: a blank
    (after trimming) block reason or blank exit-2 stderr.
  - Process outcomes are corrected: exit-0 stderr is ignored, non-zero exits
    fail open (stderr shown only for `PreCompact`, `PostCompact`, and
    `SessionEnd`, which gain `failure(message)`), and exit 2 blocks only with
    non-blank stderr.
  - New `UserPromptSubmitOutput::{with_context, block}`; PreToolUse decisions
    combine with `with_additional_context` and `with_system_message`; text
    context starting with `{` or `[` is sent as structured
    `additionalContext`.
  - Documented, not claimed as supported: step-local tool-event `cwd`, async
    command hooks (control output ignored), `mcp_tool` handlers, and no hooks
    under cloud orchestration.
- **Antigravity `docs-2026-09-29-r1`** (the unified 2.0/CLI/IDE reference;
  2.0 release notes through v2.18.1, CLI changelog through 1.2.14):
  - Every input gains the optional `modelName` (`model_name`), and
    `workspacePaths` may be empty.
  - `PostToolUse.toolCall` is optional (`Option<ToolCall>`), because IDE
    builds omit it.
  - An empty `error` means success; `error_message()` and `failed()` ignore
    blank values. `stepIdx` counts steps across the whole conversation.
  - Stop decisions are typed (`StopDecision`, `StopOutput::continue_with`,
    `StopOutput::allow_stop`), and near-misses of `continue` such as
    `"Continue"`, which Antigravity treats as allowing the stop, are rejected.
  - Output schemas and exit semantics are unchanged; the reference still
    documents no exit codes (2.0 v2.12.0 and later report a failing hook and
    continue).

### Command environments

- Selected supplement `command-environments-2026-09-30-r1` maps all 50 events
  (it succeeds `-2026-09-29-r1`, which introduced the same profiles).
- Claude Code: new optional `CLAUDE_PID` (v2.1.214+; a malformed value is
  ignored as inherited state) and the cross-session messaging socket
  `CLAUDE_CODE_MESSAGING_SOCKET` (v2.1.224+) plus its token
  `CLAUDE_CODE_MESSAGING_TOKEN` (v2.1.228+, only alongside the socket,
  redacted in `Debug`). `CLAUDE_ENV_FILE` stays optional with the corrected
  rationale above. Cloud sessions include self-hosted environments
  (v2.1.224+), backed by the self-hosted configuration page and the pinned
  Claude Code changelog.
- Codex: `Interrupt` receives the plugin variables. Hooks get a replay of the
  session-creation environment snapshot with five credential variables
  scrubbed; hook configuration cannot set variables. A non-UTF-8 plugin
  variable is skipped with a warning instead of failing every Codex hook.
- `CommandEnvironmentSpec::LENIENT_VARIABLE_NAMES` and
  `capture_command_environment_with_diagnostics`: prefix-matched and lenient
  variables with non-UTF-8 values are skipped with a diagnostic, so stray
  inherited state (for example a `CLAUDE_PLUGIN_OPTION_*`) cannot disable
  every hook.

### BREAKING changes

- **Protocol crates:** `SNAPSHOT_ID`/`SNAPSHOT` and every `ContractId` name
  the new snapshots. Typed input structs are `#[non_exhaustive]` (build them
  through serde), as are the Claude Code and Codex `Event`, `AnyInput`, and
  `AnyCommandOutput` enums. Harness-sent value enums (Claude permission mode,
  effort level, and session source; Codex permission mode, session source,
  compaction trigger, and session-end reason; Antigravity termination reason)
  keep unknown values in an `Unknown(String)` arm, are `#[non_exhaustive]`,
  and are no longer `Copy`.
- **hookkit-claude:**
  - `protocol::SessionStartOutput` and `PostToolUseOutput` are opaque types;
    `StructuredSessionStartOutput`/`StructuredPostToolUseOutput` are gone.
    `SessionStartOutput::with_initial_user_message` is now a builder.
  - `SessionSource`, `PermissionMode`, and `EffortLevel` take `as_str(&self)`;
    `Effort` gains `extra` and is built with `Effort::new`.
  - `PermissionRequestOutput::with_updated_input` takes a JSON object; it,
    `with_updated_permissions`, and `with_interrupt` fail for the wrong
    decision. `PermissionRequestOutput::blocking_error` (deprecated) also
    prints the deny decision.
  - `WorktreeCreateOutput::path` rejects CR, LF, ESC, and absolute paths with
    `.`/`..` segments.
  - `ClaudeCommandEnvironment` gains `claude_pid` and `messaging`, and no
    longer fails on inherited state (a non-`true` `CLAUDE_CODE_REMOTE`, empty
    optional values, a bridge id in a cloud session, or a bare
    `CLAUDE_PLUGIN_OPTION_`).
  - Deprecated: positional `decide`/`structured` constructors, the
    four-argument `block_with_context`, `PostToolUseOutput::blocking_error`,
    constructor-style `with_system_message`, `WorktreeRemoveOutput::no_op`,
    `SetupOutput::with_context`, and builders for discarded fields.
- **hookkit-codex:**
  - `PreToolUseOutput`'s `Block`/`Deny`/`Rewrite`/`AdditionalContext`/
    `SystemMessage` variants became `Structured(StructuredPreToolUseOutput)`
    (constructors keep their names).
  - `no_op()` emits empty stdout; blank block reasons and blank exit-2 stderr
    are `InvalidProcessEmission`; JSON-looking `text_context` becomes
    structured `additionalContext`.
  - An unknown SessionStart `source` parses as `Unknown` with no session
    boundary (it defaulted to `Startup`). `CatalogInput` rejects non-catalog
    event names. Missing-field errors are `InvalidInputForHint`.
    `PreToolUseInput.agent_id`/`agent_type` are typed fields.
- **hookkit-antigravity:** `PostToolUseInput.tool_call` is
  `Option<ToolCall>`; a new `model_name` field (no longer in `extra`);
  `workspacePaths: []` is accepted and `require_workspace` is removed;
  `StopOutput.decision` is `StopDecision` and `StopInput.termination_reason`
  is `TerminationReason`; duplicate `permissionOverrides` are removed instead
  of failing; absent optional input fields are no longer serialized as
  `null`; `PreInvocation`/`PostInvocation` are `EventCategory::Model`;
  `ToolDecision`, `TerminationBehavior`, and `InjectStep` are
  `#[non_exhaustive]`.
- **hookkit-core:** `HookkitError` is `#[non_exhaustive]`; `HintContradiction`
  and `NoEventCandidate` are removed; new `Handler` (`HookkitError::handler`),
  `InvalidInputForEvent`, `DecodedEventMismatch`, and `UnsupportedHarness`.
  The `json_helpers` module and `NativeEventDescriptor::native_input`/
  `native_output` are removed. `EventCategory`, `IdentificationStrength`,
  `ResolutionProvenance`, `DiagnosticLevel`, and `HandlerKind` are
  `#[non_exhaustive]`. `expand_home("~")` returns home without a trailing
  separator.
- **hookkit-runtime:**
  - Selected-harness execution reports `DecodedEventMismatch` (was
    `OutputEventMismatch`) and wraps discriminator parse failures in
    `InvalidInputForEvent`.
  - Stdin runners catch handler panics (exit 1 by default instead of 101) and
    record every failure in the diagnostics sink.
  - Artifacts: filenames always have four percent-encoded fields, unique
    suffixes are `.N.json`, `ArtifactManager::in_temp_dir` uses a per-user
    `hookkit-artifacts-<uid>` directory and refuses one owned by another user,
    and writes are atomic with mode 0600.
  - `BuiltinInput`, `BuiltinOutput`, `BuiltinCommandEnvironment`, and
    `DetectionEvidence` are `#[non_exhaustive]`; the sealed
    `AlignedEventSpec` gains `execute_with_diagnostics`.
- **hookkit-common:**
  - Aligned `PreToolUseOutput::allow` is documented as explicit
    auto-approval; use the new `pass_through` for "no objection".
    `rewrite` takes a `RewriteApproval`.
  - Deny/block helpers reject blank reasons on every harness.
  - `PermissionRequestInput::tool_input` returns `ToolInputRef`.
  - `PostCompactOutput::with_system_notice` emits Claude's `{}`, and Codex
    `UserPromptSubmitOutput::with_context` emits structured JSON.
  - Unsupported harnesses fail with `UnsupportedHarness` (was
    `UnrecognizedEvent`) before the payload is parsed.
  - `MessageAudience`, `FeedbackSeverity`, and `NoticeLevel` are
    `#[non_exhaustive]`.
- **hookkit-shell:**
  - Data structs (`BashAnalysis`, `CommandOccurrence`, `ShellWord`,
    `Redirection`, `HereDocument`, `BashAnalyzerLimits`,
    `FileInferenceContext`, `FileAccessReport`, `ShellToolCallRef`, and
    others) are `#[non_exhaustive]`; build limits with `Default` plus the
    `with_max_*` builders. New fields and enum variants throughout.
  - argv follows Bash: a word after a redirection target is an argument,
    line continuations join words, and brace expansion can add words.
    List- and pipeline-level redirects attach to the last command, and
    compound-statement redirects are reported in `statement_redirections`.
  - `ShellToolCallRef::cwd` is `None` for a relative cwd (use
    `effective_cwd()`); the Codex profile reads `/workdir` and reports
    `ShellCwdOrigin::UnverifiedFallback`. The Claude catalog adapter returns
    `UnsupportedEvent` outside the four tool events.
  - Semantics rules see the command behind wrappers (`sudo`, `env`, `nice`,
    `timeout`, ...). Built-in semantics widened: copy/move/link destinations
    and `mv` sources cover descendants, interpreters, `xargs`, and
    `find -exec` report `IndirectEvaluation`, and `sed` scripts are scanned.
- **hookkit-tool-access:** documented built-in tool contracts take precedence
  over heuristics (`with_builtin_tools(false)` restores them); move keys
  (`source`, `destination`, `from`, `to`, ...) only count for move/copy-like
  tools; tool-name heuristics match whole words; Codex patch payload
  precedence is `command` > `patch` > `input`; new `PathBase` variants
  (`SessionCwd`, `Home`, `UnexpandedHome`, `UnknownEnvironment`) and gap
  reasons; `TargetResolutionOptions` is `#[non_exhaustive]`; resolver and
  `JsonRef::pointer` semantics changed as described under fixes.
- **hookkit-file-activity:** many types are `#[non_exhaustive]`;
  `ResolvedFileActivity` adds `exhausted_target` and `unattempted_targets`;
  `append_report` stores a digest of its key prefix, so journal record ids
  change; the timestamp tolerance widens every scan; missing, ignored, or
  excluded scopes resolve as empty instead of being retained; the default
  ignored-directory list grew to 23 names (`DEFAULT_IGNORED_DIRECTORY_NAMES`).
- **hookkit-session-state:**
  - The default root moved to the per-user
    `$TMPDIR/agent-hook-kit-<uid>/session-state`, so builds relying on the
    default no longer share state with older builds.
  - `StateError` is `#[non_exhaustive]`; `Io` is
    `Io { operation, path, source }` with no `From<io::Error>`; new `Decode`,
    `UnsafeStateRoot`, and `LockReentry`.
  - Family, entity, claim, journal, lock, and custom-scope names must be
    lowercase.
  - `Compact` on a windowed entity fails; monotonic sets from
    `FamilyScope::set` compact after 32 generations; record-journal snapshots
    report undecodable records instead of failing; observations that do not
    decode are skipped; `RunBundle` writes replace existing artifacts.
  - Many metadata and state enums/structs are `#[non_exhaustive]`
    (`SessionEpochKind` gains `Unknown`, `GcReport` gains `failed`).
- **hookkit-pkl-config:** `StagedBuiltins.dir` is private (use `dir()`);
  `SettingsPatch.file_activity` is `Option<FileActivitySettingsPatch>`;
  Pkl `FileActivity` fields are nullable and `ignoredDirectoryNames` replaces
  the default list; the default `exclude` is `**/.git/**`,
  `**/node_modules/**`; `Loaded.project_root` is the deepest project or local
  config directory and never `$HOME`; `PklConfigError` is
  `#[non_exhaustive]` with typed sources and converts to
  `HookkitError::Handler`. Builtins: `jq` verifies with `jq empty`, and
  `check-merge-conflict` no longer matches a bare `=======`.
- **hookkit-tool-runner:**
  - `parse_*_args` return `Result<_, CliError>`; the execution-spec types are
    no longer public; `FileStatus`, `CheckOutcome`, `CommandPhase`, and
    `ArtifactClassification` (new `TimedOut`) are `#[non_exhaustive]`.
  - PostToolUse user notices moved from exit-0 stderr to `systemMessage`, and
    `harness-block` is an exit-0 `decision: "block"` instead of exit 2.
  - Stop output: no allowed-Claude `additionalContext`, blocked Claude Stops
    send `reason` only, blank reasons are replaced by a pointer to
    `summary.json`, and allowed Antigravity Stops carry no `reason`.
    `summary.json` is schema version 2.
  - Stop semantics: gaps are reported once and not re-queued; a missing tool
    under `user-notice`, or a missing `pkl`, does not block; `failFast` is
    per tool; unchanged repeat blocks are suppressed on continuations.
  - A relative `--state-dir` resolves against the project root, not the
    process cwd. Tool commands get null stdin and a default 300 s timeout.
  - `file-activity-agent-hook` accepts Claude `PostToolUseFailure`; other
    Claude events are a handler error. An unsupported harness is
    `HookkitError::UnsupportedHarness`.
- **xtask and contracts:** `contracts check --snapshot` is removed; `check`
  rejects unfrozen selected snapshots and drafts older than them;
  `verify-vendor` rejects unlisted files, empty manifests, duplicates, and
  symlinks; `contracts diff` reports per-event lines; `upstream-sources`
  adds supplement rows.
- **Generated projects and examples:** generated CLIs exit 1 on argument
  errors (`Cli::parse_validated()` returns `Result<Cli, ExitCode>`); PreToolUse
  starters pass through instead of approving (Claude `{}`, Antigravity `ask`,
  cross-mode `pass_through`), the rewrite starter returns `None`, and the
  aligned `PermissionRequest` starter returns `no_op`; the generated default
  state directory is per user. `forbidden-file-guard` answers non-matches
  with a pass-through and fails closed; `codex-bash-guard` fails closed;
  `shared-posttool-autofix` reports through `systemMessage`.

### Security and correctness fixes

- **hookkit-shell** — Bash analysis now follows Bash rather than the
  tree-sitter tree shape wherever they disagree, closing ways a guard could
  receive a "complete" report that omitted a touched file; forms it cannot
  model exactly fail closed with a gap:
  - words after a redirection target or here-document start are arguments
    (`rm 2>/dev/null .env`, `rm <<EOF .env`), and redirections after a
    here-document start belong to the command (`cat <<EOF > .env`);
  - body-less redirections (`> f`, `$(< f)`), backticks in unquoted
    here-documents, line continuations, and comma/sequence brace expansion
    are modeled; the depth limit skips only the over-deep subtree;
  - file-access inference covers statement-level redirections, `sed`
    `r`/`w`/`e`, directory-scoped `mv`/`cp`/`ln`, `grep`/`rg` stdin, quoted
    globs, `~` paths, zsh `>!`, `n>&file`, and a `cd` inside loops, functions,
    or `builtin cd`; interpreters and pagers report `IndirectEvaluation`, and
    PowerShell marks the analysis unresolved;
  - quadratic scans are removed; a new adversarial corpus
    (`tests/adversarial.rs`) has an optional `bash` differential
    (`HOOKKIT_SHELL_DIFFERENTIAL=1`).
- **hookkit-tool-access / hookkit-file-activity** — analysis matches real
  harness payloads:
  - Codex `apply_patch` is read from `tool_input.command`, the real payload
    (it previously produced only a "no patch payload" gap, so guards allowed
    edits to forbidden files), following Codex's patch grammar;
    `cd <dir> && apply_patch <<EOF` and here-documents on a group or subshell
    resolve their paths;
  - Antigravity's PascalCase file-tool arguments (`view_file`
    `AbsolutePath`, `write_to_file` `TargetFile`, ...) and Claude
    `Read`/`Write`/`Edit`/`MultiEdit`/`NotebookEdit`/`Grep`/`Glob` use exact
    per-tool contracts, and documented file-free tools no longer cause
    `deny_unresolved` denials or permanent coverage gaps;
  - shell inference uses the effective working directory (a relative Codex
    `workdir` or Antigravity `Cwd`), and shell `~` paths are candidates
    (`ToolAccessAnalyzer::with_home`);
  - the resolver no longer prunes exact targets by ignored directory name,
    does not follow a symlinked scope root, handles brace globs, and reports
    relative globs with an unknown base as unresolved;
  - `resolve_files` never drops exact targets or later scopes when the budget
    runs out, and the Stop runner retries exactly the scopes never attempted;
    truncated mtime scans resume instead of skipping the tail; git-dirty paths
    are re-rooted for monorepo subdirectories; fingerprints stream file
    contents.
- **hookkit-claude / hookkit-codex / hookkit-antigravity** — harness-sent
  enum values parse as `Unknown` instead of failing the hook; Claude
  `updatedMCPToolOutput` is serialized with its documented key;
  `CatalogInput::event_id` and Codex catalog routing no longer panic; Claude
  parsers no longer clone the whole payload.
- **hookkit-core / hookkit-runtime:**
  - Fail-closed runners: `RunOptions` and `FailurePolicy` (`NonBlocking` by
    default, opt-in `FailClosed`) through `run_event_with_options`,
    `run_harness_with_options`, `dispatch_builtin_harness_with_options`, and
    `hookkit_runtime::aligned::run_aligned_event_with_options`. Under
    `FailClosed` a policy hook that cannot decide denies: Claude Code and
    Codex gates exit 2, Claude `PermissionRequest` gets a JSON deny,
    Antigravity `PreToolUse` gets `{"decision":"deny"}`, and observers and
    Stop-like events stay non-blocking. Also
    `run_aligned_event_with_diagnostics`,
    `execute_aligned_event_with_diagnostics`,
    `execute_builtin_harness_with_diagnostics`, and
    `hookkit_runtime::failure::{failure_response, report_run_failure}`.
  - Payloads with unpaired UTF-16 surrogate escapes (a truncated emoji)
    decode as U+FFFD instead of failing the hook; a non-UTF-8 `argv[0]` no
    longer panics; the detector no longer reports the shared Claude/Codex
    discriminator as a constraint mismatch; selected-harness execution parses
    the payload once.
  - Artifacts no longer use a shared, predictable `/tmp/hookkit-artifacts`:
    the per-user directory is owner-only, writes never follow a planted
    symlink, and distinct keys no longer collide.
  - `BuiltinHarness::{ALL, from_id}`, `Display`, `FromStr` (accepting
    `claude`), and `ArtifactManager::default_temp_dir`.
- **hookkit-common (aligned)** — aligned `PreToolUse` guards and starters no
  longer auto-approve: `pass_through` answers "no objection" (`{}` on Claude
  Code, empty stdout on Codex, `{"decision":"ask"}` on Antigravity), and
  `explicit_allow_supported()` reports that Codex cannot express `allow`.
  Codex `UserPromptSubmit` context is no longer dropped when it starts with
  `[` or `{`. New accessors: `project_roots(environment)`/`project_dir()`
  (Claude Code's `CLAUDE_PROJECT_DIR`, since its `cwd` follows `cd`),
  PostToolUse `cwd`/`tool_name`/`tool_input`/`tool_call_id`, TurnCompletion
  `stop_hook_active`/`last_assistant_message`/`termination_reason`/
  `fully_idle`, and `PermissionRequestOutput::no_op`.
- **hookkit-tool-runner / hookkit-pkl-config:**
  - PostToolUse user notices reach Claude Code and Codex users through
    `systemMessage` (neither shows exit-0 stderr), capped at 10,000
    characters with a pointer to the artifact; calls that wrote no file skip
    Pkl evaluation.
  - Stop no longer loops: an allowed Claude Stop no longer sends
    `additionalContext` (which continued the turn), every block has a
    non-empty reason (Codex treats a blank one as invalid and ends the turn),
    gaps are reported once, and an unchanged repeat block stops on a
    continuation.
  - Large Stop candidate sets are split below the argument limit, so they no
    longer fail with `E2BIG` (ARG_MAX) on every Stop.
  - New `settings.commandTimeoutSeconds` (default 300): a hung tool is killed
    with its process group; tools get null stdin, and output collection stops
    two seconds after exit so a daemon holding the pipes cannot hang the hook.
  - A global `~/.agent-hook-kit/post-tool-use.pkl` no longer makes `$HOME`
    the project root and is evaluated once; an outer `.local.pkl` no longer
    replaces an inner project's root; all layers are evaluated by one `pkl`
    process in an owner-only staging directory; Pkl errors name the real
    file; `settings.fileActivity` merges field by field.
  - A relative `--state-dir` resolves against `CLAUDE_PROJECT_DIR` (or the
    first workspace root), so state no longer splits when the agent runs
    `cd`; include/exclude globs match project-relative paths; the hook's own
    artifacts and `.agent-hook-kit` are never Stop candidates.
  - `file-activity-agent-hook` also records Claude `PostToolUseFailure`, so
    writes from a failed Bash command reach the Stop runner; bind it to both
    events.
  - Diagnostics without a configured directory go to the per-user artifact
    directory; an Antigravity `artifactDirectoryPath` with a leading `~/` is
    expanded and other relative paths are refused.
  - The `jq` builtin no longer reports issues for every valid file, and
    `check-merge-conflict` no longer flags `=======` heading underlines.
  - CLI usage errors print a diagnostic naming the binary and exit 1 (never
    the blocking exit 2); `--help` exits 0.
- **hookkit-session-state:**
  - The default root is per user
    (`$TMPDIR/agent-hook-kit-<uid>/session-state`, owner-checked), so users
    sharing `/tmp` can no longer lock each other out; existing directories are
    never chmodded.
  - Builds sharing a session no longer fail on metadata written by a newer
    build (unknown values decode as `Unknown`), and a `fork` epoch counts as a
    conversation start.
  - `SessionState::ensure` no longer writes one workspace file per call or
    re-reads them under the metadata lock; per-hook latency is flat.
  - Record-journal acknowledgement removes only the captured file versions,
    entity generations sort in true append order, claims are published
    atomically, GC tolerates concurrent passes, and re-entering a held lock
    returns `StateError::LockReentry` instead of deadlocking (new
    `try_exclusive_lock` and `exclusive_lock_timeout`).
- **Tooling, CI, and templates:**
  - Conformance checks that each native crate implements the registry-selected
    snapshot, parses every positive fixture, rejects every negative one
    (except allowlisted open value sets, which must be accepted), and checks
    the repository `fixtures/` and goldens; the golden Codex deny example is
    fixed.
  - The upstream drift check reads every source from the selected snapshots
    and supplement (`cargo xtask contracts upstream-sources`), hashes decoded
    bodies, and exits 1 on drift and 2 on discovery failure.
  - CI installs Pkl and sets `HOOKKIT_REQUIRE_PKL=1`, builds rustdoc with
    warnings denied, fails on untracked generator output, and checks that the
    template's pinned HookKit revision is on `main`; `scripts/release-check.sh`
    gained matching checks.
  - The workspace uses Cargo's MSRV-aware resolver "3", `time` and `globset`
    are caret requirements instead of exact pins, and the Rust 1.85 MSRV is
    restored (let-chains removed from hookkit-shell, ICU locked at 2.1).
  - The Copier template covers the new events, uses non-deprecated
    constructors, and tells Claude Code users to bind the file-activity
    command to `PostToolUseFailure` too.
  - `THIRD_PARTY_LICENSES.md` includes the runner's `clap` dependencies.

This is an intentional pre-1.0 protocol API reboot. Event-scoped output types are
the supported surface.
