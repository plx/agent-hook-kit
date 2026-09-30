//! Loss-aware file activity observation and reconciliation.
#![deny(missing_docs)]
//!
//! Tool inspection is evidence, not an audit log. This crate retains both the
//! candidate target and the limits of the inference, then provides timestamp
//! and opt-in VCS fallbacks for deferred consumers.

use hookkit_common::PostToolUseInput;
use hookkit_core::{RuntimeContext, Utf8Path, Utf8PathBuf, normalize_utf8_path, resolve_utf8_path};
use hookkit_session_state::{
    EntityId, EntityJournal, EntityMode, EntityOutcome, FamilyId, JournalEntity, SessionEpochKind,
    SessionState, StateRoot, UtcTimestamp,
};
use hookkit_tool_access::{
    AccessCandidate, AccessCertainty, AccessIntent, AccessScope, AccessSource, AccessTarget,
    ExactPathPolicy, ObservableToolCall, PathBase, PathExpression, ResolutionIssuePolicy,
    SymlinkPolicy, TargetResolutionOptions, TargetResolutionReason, ToolAccessAnalyzer,
    resolve_targets,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};
use walkdir::{DirEntry, WalkDir};

/// Session-state family name used by [`FileActivityStore`].
pub const FILE_ACTIVITY_FAMILY: &str = "agent-hook-kit.file-activity";
/// Windowed entity name containing pending file-activity events.
pub const PENDING_ACTIVITY_ENTITY: &str = "pending-files";
/// Monotonic entity name containing the fallback reconciliation cursor.
pub const RECONCILIATION_CURSOR_ENTITY: &str = "reconciliation-cursor";
/// Monotonic entity name containing the resume point of a truncated
/// filesystem scan.
pub const RECONCILIATION_PROGRESS_ENTITY: &str = "reconciliation-progress";
/// Monotonic entity name containing handled content baselines.
pub const HANDLED_BASELINES_ENTITY: &str = "handled-baselines";

/// Error returned by file-activity persistence, traversal, or reconciliation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FileActivityError {
    /// Session-state operation failed.
    #[error(transparent)]
    State(#[from] hookkit_session_state::StateError),
    /// Filesystem or subprocess I/O failed.
    #[error("file activity I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Persisted JSON could not be decoded.
    #[error("file activity JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// A filesystem path cannot be represented by this UTF-8-only API.
    #[error("file activity path is not valid UTF-8: {0}")]
    NonUtf8Path(PathBuf),
    /// A configured activity glob is invalid.
    #[error("invalid file activity glob `{pattern}`: {message}")]
    InvalidGlob {
        /// Invalid glob text.
        pattern: String,
        /// Glob parser diagnostic.
        message: String,
    },
}

/// Result type returned by file-activity operations.
pub type Result<T> = std::result::Result<T, FileActivityError>;

/// Possible mutating effect retained for a file target.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FileActivityEffect {
    /// Creates a path or changes an existing path.
    CreateOrModify,
    /// Removes a path.
    Delete,
    /// Reads and removes the source side of a move.
    MoveSource,
    /// Creates or replaces the destination side of a move.
    MoveDestination,
    /// Some write may occur, but its exact role is unknown.
    MaybeWrite,
}

/// Analyzer or fallback that produced an activity event.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FileActivitySource {
    /// A path-bearing field in structured tool input.
    StructuredToolInput,
    /// A literal patch payload.
    Patch,
    /// Shell syntax, argv semantics, or a shell patch here-document.
    ShellInference,
    /// Filesystem modification-time reconciliation.
    FilesystemMtime,
    /// Opt-in version-control dirty-state reconciliation.
    VcsDirty,
    /// Retained evidence from an incomplete deferred workflow attempt.
    DeferredRetry,
    /// An application-defined analyzer, or an analyzer source this version
    /// does not recognize.
    Custom,
}

/// Strength of the static or fallback association with a target.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ActivityCertainty {
    /// Directly observed in tool input or fallback state.
    Direct,
    /// Directly identified, but located in conditional or deferred shell code.
    Conditional,
    /// Conservatively inferred and potentially over-inclusive.
    Heuristic,
}

/// Region selected by a file-activity target.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FileActivityScope {
    /// Only the named path.
    Exact,
    /// Children of the directory, excluding the directory itself.
    Descendants,
    /// The named path and, when applicable, its descendants.
    ExactOrDescendants,
    /// Paths matched by the glob expression.
    Glob,
}

/// Persistence-compatible target for deferred file discovery.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum FileActivityTarget {
    /// A lexically resolved path and its selection scope.
    Path {
        /// Resolved path or glob expression.
        path: Utf8PathBuf,
        /// Portion of the file tree selected by `path`.
        scope: FileActivityScope,
    },
    /// A whole workspace, optionally rooted at a known path.
    Workspace {
        /// Known root, or `None` for all roots supplied during resolution.
        root: Option<Utf8PathBuf>,
    },
}

impl FileActivityTarget {
    /// Creates an exact-path target.
    pub fn exact(path: Utf8PathBuf) -> Self {
        Self::Path {
            path,
            scope: FileActivityScope::Exact,
        }
    }
}

/// One possible mutating file activity with correlation metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileActivityEvidence {
    /// File or workspace region that may have changed.
    pub target: FileActivityTarget,
    /// Possible mutating effect.
    pub effect: FileActivityEffect,
    /// Analyzer or fallback that supplied the evidence.
    pub source: FileActivitySource,
    /// Strength of the association.
    pub certainty: ActivityCertainty,
    /// Time at which the evidence was observed, not necessarily the write time.
    pub observed_at: UtcTimestamp,
    /// Native hook event name, when available.
    pub event: Option<String>,
    /// Native tool-call correlation identifier, when available.
    pub tool_call_id: Option<String>,
    /// Native turn identifier, when available.
    pub turn_id: Option<String>,
    /// Human-readable provenance or qualification.
    pub detail: Option<String>,
}

/// Known blind spot encountered while observing file activity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileActivityGap {
    /// Analyzer or fallback that encountered the gap.
    pub source: FileActivitySource,
    /// Time at which the gap was observed.
    pub observed_at: UtcTimestamp,
    /// Native hook event name, when available.
    pub event: Option<String>,
    /// Native tool-call correlation identifier, when available.
    pub tool_call_id: Option<String>,
    /// Native turn identifier, when available.
    pub turn_id: Option<String>,
    /// Human-readable description of the missing evidence.
    pub detail: String,
}

/// Journal event retained by [`PendingFileActivity`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum FileActivityEvent {
    /// A possible mutating file activity.
    Evidence(FileActivityEvidence),
    /// A known gap in activity observation.
    Gap(FileActivityGap),
    /// Idempotent retry evidence retained for a future deferred attempt.
    Retry(FileActivityRetry),
}

/// Idempotent deferred retry evidence. Re-appending the same id keeps one
/// aggregate contribution while still placing a copy outside each sealed
/// source window before that window is acknowledged.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileActivityRetry {
    /// Stable identifier used to deduplicate the retry contribution.
    pub id: String,
    /// File target to retry, or `None` for a retained coverage gap.
    pub target: Option<FileActivityTarget>,
    /// Human-readable explanation for retaining the retry.
    pub reason: String,
}

/// Windowed aggregate of targets awaiting downstream processing.
///
/// Compaction preserves the target set and event counts; individual event
/// provenance remains in the journal only until that window is consumed.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PendingFileActivity {
    targets: BTreeSet<FileActivityTarget>,
    evidence_count: usize,
    gap_count: usize,
    #[serde(default)]
    retry_ids: BTreeSet<String>,
}

impl PendingFileActivity {
    /// Returns the deduplicated targets accumulated in the current window.
    pub fn targets(&self) -> &BTreeSet<FileActivityTarget> {
        &self.targets
    }

    /// Returns the number of evidence events applied to the window.
    pub fn evidence_count(&self) -> usize {
        self.evidence_count
    }

    /// Returns the number of gap events applied to the window.
    pub fn gap_count(&self) -> usize {
        self.gap_count
    }

    /// Returns whether observation recorded at least one known blind spot.
    pub fn has_gaps(&self) -> bool {
        self.gap_count != 0
    }

    /// Returns the number of distinct retry events applied to the window.
    pub fn retry_count(&self) -> usize {
        self.retry_ids.len()
    }
}

impl JournalEntity for PendingFileActivity {
    type Event = FileActivityEvent;

    fn empty() -> Self {
        Self::default()
    }

    fn apply(&mut self, event: &Self::Event) {
        match event {
            FileActivityEvent::Evidence(evidence) => {
                self.evidence_count += 1;
                self.targets.insert(evidence.target.clone());
            }
            FileActivityEvent::Gap(_) => self.gap_count += 1,
            FileActivityEvent::Retry(retry) => {
                if self.retry_ids.insert(retry.id.clone()) {
                    if let Some(target) = &retry.target {
                        self.evidence_count += 1;
                        self.targets.insert(target.clone());
                    } else {
                        self.gap_count += 1;
                    }
                }
            }
        }
    }
}

/// Existence/type and content identity used only to suppress unchanged
/// fallback evidence. Direct observations never consult this baseline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HandledFingerprint {
    /// Kind of filesystem object observed at the handled path.
    pub kind: HandledFileKind,
    /// SHA-256 digest for regular files; absent for other object kinds.
    pub content_sha256: Option<String>,
}

/// Filesystem object kind captured by a handled baseline.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum HandledFileKind {
    /// No filesystem object exists at the path.
    Missing,
    /// The path names a regular file.
    File,
    /// The path names a directory.
    Directory,
    /// The path names a symbolic link.
    Symlink,
    /// The path names another filesystem object kind.
    Other,
}

/// Content baseline recorded after a deferred workflow handles a path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HandledBaseline {
    /// Normalized path associated with the baseline.
    pub path: Utf8PathBuf,
    /// Filesystem identity captured for the path.
    pub fingerprint: HandledFingerprint,
    /// Time at which the baseline was recorded.
    pub handled_at: UtcTimestamp,
    /// Deferred run that produced the baseline.
    pub run_id: String,
}

/// Latest handled baseline for each normalized path.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HandledBaselines {
    entries: std::collections::BTreeMap<Utf8PathBuf, HandledBaseline>,
}

impl HandledBaselines {
    /// Returns the latest baseline for `path`.
    pub fn get(&self, path: &Utf8Path) -> Option<&HandledBaseline> {
        self.entries.get(path)
    }

    /// Returns every recorded path and its latest baseline.
    pub fn entries(&self) -> &std::collections::BTreeMap<Utf8PathBuf, HandledBaseline> {
        &self.entries
    }
}

impl JournalEntity for HandledBaselines {
    type Event = HandledBaseline;

    fn empty() -> Self {
        Self::default()
    }

    fn apply(&mut self, event: &Self::Event) {
        self.entries.insert(event.path.clone(), event.clone());
    }
}

/// Outcome of recording handled baselines for a set of paths.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HandledBaselineReport {
    /// Baselines written successfully.
    pub recorded: Vec<HandledBaseline>,
    /// Paths whose fingerprints could not be read.
    pub failures: Vec<HandledBaselineFailure>,
}

/// One path whose handled baseline could not be recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandledBaselineFailure {
    /// Path whose fingerprint could not be read.
    pub path: Utf8PathBuf,
    /// Human-readable fingerprinting failure.
    pub message: String,
}

/// Monotonic cursor through the interval covered by fallback reconciliation.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationCursor {
    /// Latest instant through which fallback discoveries were durably appended.
    pub reconciled_through: Option<UtcTimestamp>,
}

impl JournalEntity for ReconciliationCursor {
    type Event = UtcTimestamp;

    fn empty() -> Self {
        Self::default()
    }

    fn apply(&mut self, event: &Self::Event) {
        self.reconciled_through = Some(
            self.reconciled_through
                .map_or(*event, |current| current.max(*event)),
        );
    }
}

/// Resume point of a filesystem-mtime scan that stopped at its entry budget.
///
/// The scan walks each root depth-first in file-name order, so path order
/// within a root is walk order. Entries at or before `after` were checked
/// through the reconciliation cursor; later entries were checked only
/// through `tail_since`, and the next reconciliation scans them first.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScanResume {
    /// Roots of the truncated scan, in scan order.
    pub roots: Vec<Utf8PathBuf>,
    /// Index into `roots` of the last examined entry.
    pub root_index: usize,
    /// Last examined entry.
    pub after: Utf8PathBuf,
    /// Inclusive modification-time lower bound still owed to later entries.
    pub tail_since: UtcTimestamp,
}

/// Latest filesystem-scan progress recorded by [`reconcile`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationProgress {
    /// Upper bound of the reconciliation that recorded this progress.
    pub recorded_at: Option<UtcTimestamp>,
    /// Unfinished scan, or `None` when the last scan completed.
    pub resume: Option<ScanResume>,
}

/// One scan-progress update appended by [`reconcile`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationProgressEvent {
    /// Upper bound of the reconciliation that recorded the update.
    pub recorded_at: UtcTimestamp,
    /// Unfinished scan, or `None` when the scan completed.
    pub resume: Option<ScanResume>,
}

impl JournalEntity for ReconciliationProgress {
    type Event = ReconciliationProgressEvent;

    fn empty() -> Self {
        Self::default()
    }

    fn apply(&mut self, event: &Self::Event) {
        if self
            .recorded_at
            .is_none_or(|current| event.recorded_at >= current)
        {
            self.recorded_at = Some(event.recorded_at);
            self.resume = event.resume.clone();
        }
    }
}

/// Activity events derived from one observation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActivityReport {
    /// Evidence and gaps in deterministic analyzer order.
    pub events: Vec<FileActivityEvent>,
}

impl ActivityReport {
    /// Iterates over evidence events.
    pub fn evidence(&self) -> impl Iterator<Item = &FileActivityEvidence> {
        self.events.iter().filter_map(|event| match event {
            FileActivityEvent::Evidence(evidence) => Some(evidence),
            FileActivityEvent::Gap(_) | FileActivityEvent::Retry(_) => None,
        })
    }

    /// Iterates over gap events.
    pub fn gaps(&self) -> impl Iterator<Item = &FileActivityGap> {
        self.events.iter().filter_map(|event| match event {
            FileActivityEvent::Evidence(_) => None,
            FileActivityEvent::Gap(gap) => Some(gap),
            FileActivityEvent::Retry(_) => None,
        })
    }

    /// Returns whether the report contains neither evidence nor gaps.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// Session-scoped journals used for pending activity and reconciliation state.
#[derive(Debug, Clone)]
pub struct FileActivityStore {
    state: SessionState,
    pending: EntityJournal<PendingFileActivity>,
    cursor: EntityJournal<ReconciliationCursor>,
    progress: EntityJournal<ReconciliationProgress>,
    handled: EntityJournal<HandledBaselines>,
}

impl FileActivityStore {
    /// Ensures session state from the runtime context and opens the activity journals.
    pub fn ensure(context: &RuntimeContext<'_>, root: StateRoot) -> Result<Self> {
        Self::from_state(SessionState::ensure(context, root)?)
    }

    /// Opens the activity journals within an existing session state.
    pub fn from_state(state: SessionState) -> Result<Self> {
        let scope = state
            .family(FamilyId::new(FILE_ACTIVITY_FAMILY, 1)?)?
            .session_scope()?;
        let pending = scope.entity::<PendingFileActivity>(
            EntityId::new(PENDING_ACTIVITY_ENTITY, 2)?,
            EntityMode::Windowed,
        )?;
        let cursor = scope.entity::<ReconciliationCursor>(
            EntityId::new(RECONCILIATION_CURSOR_ENTITY, 1)?,
            EntityMode::Monotonic,
        )?;
        let progress = scope.entity::<ReconciliationProgress>(
            EntityId::new(RECONCILIATION_PROGRESS_ENTITY, 1)?,
            EntityMode::Monotonic,
        )?;
        let handled = scope.entity::<HandledBaselines>(
            EntityId::new(HANDLED_BASELINES_ENTITY, 1)?,
            EntityMode::Monotonic,
        )?;
        Ok(Self {
            state,
            pending,
            cursor,
            progress,
            handled,
        })
    }

    /// Returns the underlying session state.
    pub fn state(&self) -> &SessionState {
        &self.state
    }

    /// Returns the windowed pending-activity journal.
    pub fn pending(&self) -> &EntityJournal<PendingFileActivity> {
        &self.pending
    }

    /// Returns the journal containing handled content baselines.
    pub fn handled(&self) -> &EntityJournal<HandledBaselines> {
        &self.handled
    }

    /// Appends every report event under a stable, per-observation key prefix.
    ///
    /// Callers must use a prefix that uniquely and repeatably identifies the
    /// source observation, such as the raw hook input. Only a SHA-256 digest
    /// of the prefix is persisted (followed by the event index), so a large
    /// prefix does not bloat every journal record.
    pub fn append_report(&self, event_key_prefix: &str, report: &ActivityReport) -> Result<()> {
        let prefix = sha256_hex(event_key_prefix.as_bytes());
        for (index, event) in report.events.iter().enumerate() {
            self.pending.append(&format!("{prefix}\0{index}"), event)?;
        }
        Ok(())
    }

    /// Append exact/scoped retry targets into the active generation. Identical
    /// retry ids deduplicate in the pending projection, but a fresh record is
    /// intentionally appended on every attempt so it remains outside the
    /// sealed source window that the caller is about to acknowledge.
    pub fn requeue_targets(
        &self,
        reason: &str,
        targets: impl IntoIterator<Item = FileActivityTarget>,
    ) -> Result<usize> {
        let mut appended = 0;
        for target in targets {
            let target_json = serde_json::to_string(&target)?;
            let id = format!("{reason}\0{target_json}");
            self.pending.append(
                &format!("deferred-retry\0{id}"),
                &FileActivityEvent::Retry(FileActivityRetry {
                    id,
                    target: Some(target),
                    reason: reason.to_owned(),
                }),
            )?;
            appended += 1;
        }
        Ok(appended)
    }

    /// Appends exact-path retry targets to the active generation.
    pub fn requeue_exact(
        &self,
        reason: &str,
        paths: impl IntoIterator<Item = Utf8PathBuf>,
    ) -> Result<usize> {
        self.requeue_targets(reason, paths.into_iter().map(FileActivityTarget::exact))
    }

    /// Appends deduplicated coverage-gap retry events to the active generation.
    pub fn requeue_gaps(
        &self,
        category: &str,
        messages: impl IntoIterator<Item = String>,
    ) -> Result<usize> {
        let mut appended = 0;
        for message in messages.into_iter().collect::<BTreeSet<_>>() {
            let id = format!("{category}\0{message}");
            self.pending.append(
                &format!("deferred-retry\0{id}"),
                &FileActivityEvent::Retry(FileActivityRetry {
                    id,
                    target: None,
                    reason: message,
                }),
            )?;
            appended += 1;
        }
        Ok(appended)
    }

    /// Returns the latest handled content baselines.
    pub fn handled_baselines(&self) -> Result<HandledBaselines> {
        Ok(self
            .handled
            .with_entity(|view| Ok(EntityOutcome::retain(view.state().clone())))?)
    }

    /// Record content-based handled baselines. A file-local fingerprint read
    /// failure is reported and left unsuppressed rather than aborting all
    /// other baseline writes.
    pub fn record_handled_baselines(
        &self,
        paths: impl IntoIterator<Item = Utf8PathBuf>,
        run_id: impl Into<String>,
    ) -> Result<HandledBaselineReport> {
        let run_id = run_id.into();
        let handled_at = UtcTimestamp::now();
        let mut report = HandledBaselineReport::default();
        for path in paths
            .into_iter()
            .map(|path| normalize_handled_path(&path))
            .collect::<BTreeSet<_>>()
        {
            match fingerprint(&path) {
                Ok(fingerprint) => {
                    let baseline = HandledBaseline {
                        path: path.clone(),
                        fingerprint,
                        handled_at,
                        run_id: run_id.clone(),
                    };
                    self.handled
                        .append(&format!("handled\0{}", path.as_str()), &baseline)?;
                    report.recorded.push(baseline);
                }
                Err(error) => report.failures.push(HandledBaselineFailure {
                    path,
                    message: error.to_string(),
                }),
            }
        }
        self.handled
            .with_entity(|_| Ok(EntityOutcome::compact(())))?;
        Ok(report)
    }

    /// Reads the latest durably recorded reconciliation cursor.
    pub fn reconciled_through(&self) -> Result<Option<UtcTimestamp>> {
        Ok(self
            .cursor
            .with_entity(|view| Ok(EntityOutcome::retain(view.state().reconciled_through)))?)
    }

    /// Advance only after all discoveries through this instant have been
    /// appended to the pending journal.
    pub fn record_reconciled_through(&self, through: UtcTimestamp) -> Result<()> {
        self.cursor.append(
            &format!("through:{}", through.unix_milliseconds()),
            &through,
        )?;
        self.cursor
            .with_entity(|_| Ok(EntityOutcome::compact(())))?;
        Ok(())
    }

    /// Returns the resume point of a filesystem scan that stopped at its
    /// entry budget, if the latest reconciliation left one.
    pub fn scan_resume(&self) -> Result<Option<ScanResume>> {
        Ok(self
            .progress
            .with_entity(|view| Ok(EntityOutcome::retain(view.state().resume.clone())))?)
    }

    fn record_scan_progress(
        &self,
        recorded_at: UtcTimestamp,
        resume: Option<ScanResume>,
    ) -> Result<()> {
        self.progress.append(
            &format!("progress:{}", recorded_at.unix_milliseconds()),
            &ReconciliationProgressEvent {
                recorded_at,
                resume,
            },
        )?;
        self.progress
            .with_entity(|_| Ok(EntityOutcome::compact(())))?;
        Ok(())
    }

    /// Returns the lower bound for the first, cursor-less reconciliation.
    ///
    /// This is the current session epoch's start, except that a compaction
    /// epoch (which can begin in the middle of the first turn) never moves
    /// the bound past the conversation's start.
    pub fn bootstrap_started_at(&self) -> Result<UtcTimestamp> {
        let metadata = self.state.metadata()?;
        let current = metadata.current_session.started_at.at;
        Ok(match metadata.current_session.kind {
            SessionEpochKind::Compact => current.min(metadata.conversation.started_at.at),
            _ => current,
        })
    }
}

/// Observe one native post-tool event without claiming complete coverage.
pub fn observe_post_tool(input: &PostToolUseInput, context: &RuntimeContext<'_>) -> ActivityReport {
    observe_tool_call(input, context)
}

/// Observe Claude Code `PostToolUseFailure`, which Claude sends instead of
/// `PostToolUse` when a tool call fails. A failing Bash command (for example
/// `sed -i ... && pytest` with failing tests) may still have written files,
/// so file-activity producers should be bound to both events.
pub fn observe_claude_post_tool_failure(
    input: &hookkit_claude::catalog::CatalogInput,
    context: &RuntimeContext<'_>,
) -> ActivityReport {
    observe_tool_call(input, context)
}

/// Observe any borrowed native or aligned input that carries its originating
/// tool call, without claiming complete coverage.
pub fn observe_tool_call<T: ObservableToolCall + ?Sized>(
    input: &T,
    context: &RuntimeContext<'_>,
) -> ActivityReport {
    let metadata = ObservationMetadata {
        observed_at: UtcTimestamp::now(),
        event: Some(context.event().name().to_owned()),
        tool_call_id: context.tool_call_id().map(ToString::to_string),
        turn_id: context.turn_id().map(ToString::to_string),
    };
    let access = ToolAccessAnalyzer::default().analyze_native(input);
    activity_report(access, &metadata)
}

fn activity_report(
    access: hookkit_tool_access::ToolAccessReport,
    metadata: &ObservationMetadata,
) -> ActivityReport {
    let mut events = Vec::new();
    for candidate in access.may_modify() {
        match activity_evidence(candidate, metadata) {
            Some(evidence) => events.push(FileActivityEvent::Evidence(evidence)),
            None => events.push(FileActivityEvent::Gap(metadata.gap(
                activity_source(candidate.provenance.source()),
                format!(
                    "{}; modifying target has no persistence-compatible lexical resolution",
                    candidate.provenance
                ),
            ))),
        }
    }
    events.extend(access.gaps.into_iter().map(|gap| {
        FileActivityEvent::Gap(metadata.gap(activity_source(gap.source), gap.to_string()))
    }));
    ActivityReport { events }
}

fn activity_evidence(
    candidate: &AccessCandidate,
    metadata: &ObservationMetadata,
) -> Option<FileActivityEvidence> {
    let target = match &candidate.target {
        AccessTarget::Path { expression, scope } => FileActivityTarget::Path {
            path: expression.resolved.clone()?,
            scope: match scope {
                AccessScope::Exact => FileActivityScope::Exact,
                AccessScope::Descendants => FileActivityScope::Descendants,
                AccessScope::ExactOrDescendants => FileActivityScope::ExactOrDescendants,
                AccessScope::Glob => FileActivityScope::Glob,
                _ => return None,
            },
        },
        AccessTarget::Workspace { root } => FileActivityTarget::Workspace { root: root.clone() },
        _ => return None,
    };
    let effect = match candidate.intent {
        AccessIntent::Delete => FileActivityEffect::Delete,
        AccessIntent::MoveSource => FileActivityEffect::MoveSource,
        AccessIntent::MoveDestination => FileActivityEffect::MoveDestination,
        AccessIntent::Modify | AccessIntent::ReadModify => FileActivityEffect::CreateOrModify,
        _ => FileActivityEffect::MaybeWrite,
    };
    let certainty = match candidate.certainty {
        AccessCertainty::Direct => ActivityCertainty::Direct,
        AccessCertainty::Conditional => ActivityCertainty::Conditional,
        AccessCertainty::Heuristic => ActivityCertainty::Heuristic,
        _ => ActivityCertainty::Heuristic,
    };
    Some(metadata.evidence(
        target,
        effect,
        activity_source(candidate.provenance.source()),
        certainty,
        Some(candidate.provenance.to_string()),
    ))
}

fn activity_source(source: AccessSource) -> FileActivitySource {
    match source {
        AccessSource::Structured => FileActivitySource::StructuredToolInput,
        AccessSource::Patch => FileActivitySource::Patch,
        AccessSource::Shell => FileActivitySource::ShellInference,
        _ => FileActivitySource::Custom,
    }
}

#[derive(Debug, Clone)]
struct ObservationMetadata {
    observed_at: UtcTimestamp,
    event: Option<String>,
    tool_call_id: Option<String>,
    turn_id: Option<String>,
}

impl ObservationMetadata {
    fn evidence(
        &self,
        target: FileActivityTarget,
        effect: FileActivityEffect,
        source: FileActivitySource,
        certainty: ActivityCertainty,
        detail: Option<String>,
    ) -> FileActivityEvidence {
        FileActivityEvidence {
            target,
            effect,
            source,
            certainty,
            observed_at: self.observed_at,
            event: self.event.clone(),
            tool_call_id: self.tool_call_id.clone(),
            turn_id: self.turn_id.clone(),
            detail,
        }
    }

    fn gap(&self, source: FileActivitySource, detail: impl Into<String>) -> FileActivityGap {
        FileActivityGap {
            source,
            observed_at: self.observed_at,
            event: self.event.clone(),
            tool_call_id: self.tool_call_id.clone(),
            turn_id: self.turn_id.clone(),
            detail: detail.into(),
        }
    }
}

/// Optional version-control fallback used during reconciliation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum VcsFallback {
    /// Do not inspect version-control state.
    #[default]
    Disabled,
    /// Broad fallback. Includes changes that may predate the agent session.
    GitDirty,
}

/// Bounds and evidence sources for one reconciliation interval.
///
/// Construct with [`ReconciliationOptions::new`] and assign fields. Relative
/// roots and excluded roots are resolved against the process working
/// directory.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ReconciliationOptions {
    /// Workspace roots to scan or query.
    pub roots: Vec<Utf8PathBuf>,
    /// Lower bound used only when no durable cursor is present.
    pub fallback_since: Option<UtcTimestamp>,
    /// Inclusive upper bound recorded after successful reconciliation.
    pub through: UtcTimestamp,
    /// Clock-resolution slack subtracted from every lower bound, so writes on
    /// coarse or skewed filesystem clocks near the previous cursor are not
    /// lost. Unchanged files that already have a handled baseline are still
    /// suppressed; others inside the overlap may be reported again.
    pub timestamp_tolerance: Duration,
    /// Whether to scan filesystem modification times.
    pub filesystem_mtime: bool,
    /// Optional version-control dirty-state fallback.
    pub vcs: VcsFallback,
    /// Maximum directory entries visited by the filesystem scan, and maximum
    /// dirty paths read from version control.
    pub max_entries: usize,
    /// Directory basenames pruned from recursive traversal.
    pub ignored_directory_names: BTreeSet<String>,
    /// Roots pruned from both fallbacks. The session state directory is
    /// always excluded.
    pub excluded_roots: BTreeSet<Utf8PathBuf>,
}

impl ReconciliationOptions {
    /// Creates options with mtime scanning enabled and VCS fallback disabled.
    ///
    /// The lower bound is selected later from the durable cursor, the explicit
    /// fallback, or the session start time, in that precedence order.
    pub fn new(roots: Vec<Utf8PathBuf>, through: UtcTimestamp) -> Self {
        Self {
            roots,
            fallback_since: None,
            through,
            timestamp_tolerance: Duration::from_secs(2),
            filesystem_mtime: true,
            vcs: VcsFallback::Disabled,
            max_entries: 100_000,
            ignored_directory_names: default_ignored_directory_names(),
            excluded_roots: BTreeSet::new(),
        }
    }
}

/// Summary of fallback evidence and traversal limits for one reconciliation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconciliationReport {
    /// Effective lower bound, if one could be established.
    pub since: Option<UtcTimestamp>,
    /// Requested upper bound.
    pub through: Option<UtcTimestamp>,
    /// Files discovered through modification-time scanning.
    pub filesystem_files: usize,
    /// Files discovered through version-control dirty state.
    pub vcs_files: usize,
    /// Fallback filesystem paths omitted because their handled baseline still matches.
    pub suppressed_filesystem_files: usize,
    /// Fallback VCS paths omitted because their handled baseline still matches.
    pub suppressed_vcs_files: usize,
    /// Directory entries charged to the traversal budget.
    pub scanned_entries: usize,
    /// Whether the filesystem scan stopped at its entry budget. Its unscanned
    /// remainder is recorded as a [`ScanResume`] and scanned first next time.
    pub truncated: bool,
    /// Whether the VCS fallback stopped reading dirty paths at the budget.
    pub vcs_truncated: bool,
    /// Non-fatal fallback failures and coverage limitations.
    pub gaps: Vec<ReconciliationGap>,
}

/// Non-fatal gap encountered during fallback reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciliationGap {
    /// Fallback source that encountered the gap.
    pub source: FileActivitySource,
    /// Human-readable description of the limitation.
    pub detail: String,
}

/// Reconcile fallback evidence, then durably record progress.
///
/// Discoveries are appended before any progress is recorded, so if appending
/// fails the cursor remains unchanged and the next attempt safely rescans the
/// same interval. A filesystem scan that reaches its entry budget records
/// where it stopped; the next reconciliation scans that remainder first,
/// against the older lower bound it is still owed, so no part of the tree is
/// permanently skipped.
pub fn reconcile(
    store: &FileActivityStore,
    mut options: ReconciliationOptions,
) -> Result<ReconciliationReport> {
    let working_directory = current_utf8_dir();
    options.roots = options
        .roots
        .iter()
        .map(|root| absolute_path(root, working_directory.as_deref()))
        .collect();
    options.excluded_roots = options
        .excluded_roots
        .iter()
        .map(|root| absolute_path(root, working_directory.as_deref()))
        .collect();
    if let Ok(state_directory) = utf8_path(store.state().directory()) {
        options.excluded_roots.insert(absolute_path(
            &state_directory,
            working_directory.as_deref(),
        ));
    }
    let cursor = store.reconciled_through()?;
    let since = cursor
        .or(options.fallback_since)
        .or_else(|| store.bootstrap_started_at().ok());
    let mut report = ReconciliationReport {
        since,
        through: Some(options.through),
        ..ReconciliationReport::default()
    };
    let metadata = ObservationMetadata {
        observed_at: options.through,
        event: Some("reconciliation".to_owned()),
        tool_call_id: None,
        turn_id: None,
    };
    let handled = store.handled_baselines()?;
    let mut events = Vec::new();
    let mut progress = None;
    let mut advance_cursor = true;

    if options.filesystem_mtime {
        match since {
            Some(since) => {
                let resume = store.scan_resume()?;
                let had_resume = resume.is_some();
                let outcome = MtimeScan {
                    options: &options,
                    metadata: &metadata,
                    handled: &handled,
                    events: &mut events,
                    report: &mut report,
                    seen: BTreeSet::new(),
                    last: None,
                }
                .run(since, cursor.is_none(), resume)?;
                // Complete scans with nothing owed need no progress record.
                if had_resume || outcome.resume.is_some() {
                    progress = Some(outcome.resume);
                }
                advance_cursor = outcome.advance_cursor;
            }
            None => push_reconciliation_gap(
                &mut report,
                FileActivitySource::FilesystemMtime,
                "filesystem fallback has no lower-bound timestamp",
            ),
        }
    }
    if options.vcs == VcsFallback::GitDirty {
        collect_git_dirty_activity(&options, &metadata, &handled, &mut events, &mut report)?;
    }
    for gap in &report.gaps {
        events.push(FileActivityEvent::Gap(
            metadata.gap(gap.source, gap.detail.clone()),
        ));
    }
    let activity = ActivityReport { events };
    store.append_report(
        &format!("reconcile:{}", options.through.unix_milliseconds()),
        &activity,
    )?;
    // The order is intentional: pending evidence, then scan progress, then
    // the cursor. A crash between steps only widens the next scan.
    if let Some(resume) = progress {
        store.record_scan_progress(options.through, resume)?;
    }
    if advance_cursor {
        store.record_reconciled_through(options.through)?;
    }
    Ok(report)
}

/// Modification-time lower bound for one scan pass.
#[derive(Debug, Clone, Copy)]
struct Bound {
    at: SystemTime,
    inclusive: bool,
}

impl Bound {
    fn admits(self, modified: SystemTime) -> bool {
        if self.inclusive {
            modified >= self.at
        } else {
            modified > self.at
        }
    }

    fn min(self, other: Self) -> Self {
        match self.at.cmp(&other.at) {
            Ordering::Less => self,
            Ordering::Greater => other,
            Ordering::Equal => Self {
                at: self.at,
                inclusive: self.inclusive || other.inclusive,
            },
        }
    }
}

/// Which part of the sorted walk a pass covers, relative to a resume point.
#[derive(Debug, Clone, Copy)]
enum ScanRange<'a> {
    /// Every entry of every root.
    All,
    /// Entries strictly after the resume point.
    After(usize, &'a Utf8Path),
    /// Entries at or before the resume point.
    UpTo(usize, &'a Utf8Path),
}

enum ScanStop {
    Complete,
    Stopped,
}

struct ScanOutcome {
    resume: Option<ScanResume>,
    advance_cursor: bool,
}

struct MtimeScan<'a> {
    options: &'a ReconciliationOptions,
    metadata: &'a ObservationMetadata,
    handled: &'a HandledBaselines,
    events: &'a mut Vec<FileActivityEvent>,
    report: &'a mut ReconciliationReport,
    seen: BTreeSet<Utf8PathBuf>,
    /// Last entry examined, in walk order.
    last: Option<(usize, Utf8PathBuf)>,
}

impl MtimeScan<'_> {
    /// Scans the owed remainder of a truncated scan first, then the rest of
    /// the tree, sharing one entry budget.
    ///
    /// Every lower bound is widened by the timestamp tolerance, and the
    /// first, cursor-less scan also includes writes at exactly its bound.
    fn run(
        mut self,
        since: UtcTimestamp,
        is_bootstrap: bool,
        resume: Option<ScanResume>,
    ) -> Result<ScanOutcome> {
        let options = self.options;
        let roots = &options.roots;
        let mut current = Bound {
            at: since
                .as_system_time()
                .checked_sub(options.timestamp_tolerance)
                .unwrap_or(SystemTime::UNIX_EPOCH),
            inclusive: is_bootstrap,
        };
        let resume = match resume {
            Some(resume) if resume.roots == *roots && resume.root_index < roots.len() => {
                Some(resume)
            }
            Some(stale) => {
                // The roots changed, so the old resume point cannot be
                // located; rescan everything against its older bound.
                current = current.min(Bound {
                    at: stale.tail_since.as_system_time(),
                    inclusive: true,
                });
                None
            }
            None => None,
        };
        let start = roots.first().map(|root| (0, root.clone()));
        let Some(resume) = resume else {
            self.last = start;
            return Ok(match self.scan(ScanRange::All, current)? {
                ScanStop::Complete => ScanOutcome {
                    resume: None,
                    advance_cursor: true,
                },
                ScanStop::Stopped => ScanOutcome {
                    resume: self.resume_point(current),
                    advance_cursor: true,
                },
            });
        };

        let owed = Bound {
            at: resume.tail_since.as_system_time(),
            inclusive: true,
        };
        self.last = Some((resume.root_index, resume.after.clone()));
        if let ScanStop::Stopped =
            self.scan(ScanRange::After(resume.root_index, &resume.after), owed)?
        {
            // Entries up to the resume point were not rescanned, so they are
            // still only checked through the unchanged cursor.
            return Ok(ScanOutcome {
                resume: self.resume_point(owed),
                advance_cursor: false,
            });
        }
        self.last = start;
        Ok(
            match self.scan(ScanRange::UpTo(resume.root_index, &resume.after), current)? {
                ScanStop::Complete => ScanOutcome {
                    resume: None,
                    advance_cursor: true,
                },
                ScanStop::Stopped => ScanOutcome {
                    resume: self.resume_point(current),
                    advance_cursor: true,
                },
            },
        )
    }

    fn resume_point(&self, owed: Bound) -> Option<ScanResume> {
        self.last.clone().map(|(root_index, after)| ScanResume {
            roots: self.options.roots.clone(),
            root_index,
            after,
            tail_since: UtcTimestamp::from_system_time(owed.at),
        })
    }

    fn scan(&mut self, range: ScanRange<'_>, bound: Bound) -> Result<ScanStop> {
        let options = self.options;
        let upper = options.through.as_system_time();
        for (root_index, root) in options.roots.iter().enumerate() {
            let marker = match range {
                ScanRange::All => None,
                ScanRange::After(index, _) if root_index < index => continue,
                ScanRange::UpTo(index, _) if root_index > index => break,
                ScanRange::After(index, marker) | ScanRange::UpTo(index, marker) => {
                    (root_index == index).then_some(marker)
                }
            };
            let after_marker = matches!(range, ScanRange::After(..)) && marker.is_some();
            let walker = WalkDir::new(root.as_std_path())
                .follow_links(false)
                .sort_by_file_name()
                .into_iter()
                .filter_entry(|entry| {
                    should_descend(
                        entry,
                        &options.ignored_directory_names,
                        &options.excluded_roots,
                    ) && !(after_marker
                        && marker.is_some_and(|marker| entirely_before(entry.path(), marker)))
                });
            for entry in walker {
                let entry_path = match &entry {
                    Ok(entry) => Some(entry.path()),
                    Err(error) => error.path(),
                };
                if let (Some(marker), Some(path)) = (marker, entry_path) {
                    let order = path.cmp(marker.as_std_path());
                    match range {
                        ScanRange::After(..) if order != Ordering::Greater => continue,
                        ScanRange::UpTo(..) if order == Ordering::Greater => {
                            return Ok(ScanStop::Complete);
                        }
                        _ => {}
                    }
                }
                if self.report.scanned_entries >= options.max_entries {
                    self.report.truncated = true;
                    push_reconciliation_gap(
                        self.report,
                        FileActivitySource::FilesystemMtime,
                        format!(
                            "filesystem fallback stopped after {} entries; the remainder is scanned first next time",
                            options.max_entries
                        ),
                    );
                    return Ok(ScanStop::Stopped);
                }
                self.report.scanned_entries += 1;
                if let Some(path) = entry_path.and_then(|path| utf8_path(path).ok()) {
                    self.last = Some((root_index, path));
                }
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        push_reconciliation_gap(
                            self.report,
                            FileActivitySource::FilesystemMtime,
                            error.to_string(),
                        );
                        continue;
                    }
                };
                self.examine(&entry, bound, upper);
            }
        }
        Ok(ScanStop::Complete)
    }

    fn examine(&mut self, entry: &DirEntry, bound: Bound, upper: SystemTime) {
        if !entry.file_type().is_file() {
            return;
        }
        let modified = match entry
            .metadata()
            .map_err(std::io::Error::from)
            .and_then(|metadata| metadata.modified())
        {
            Ok(modified) => modified,
            Err(error) => {
                push_reconciliation_gap(
                    self.report,
                    FileActivitySource::FilesystemMtime,
                    format!("{}: {error}", entry.path().display()),
                );
                return;
            }
        };
        if !bound.admits(modified) || modified > upper {
            return;
        }
        let path = match utf8_path(entry.path()) {
            Ok(path) => path,
            Err(error) => {
                push_reconciliation_gap(
                    self.report,
                    FileActivitySource::FilesystemMtime,
                    error.to_string(),
                );
                return;
            }
        };
        if !self.seen.insert(path.clone()) {
            return;
        }
        if fallback_matches_handled(&path, self.handled) {
            self.report.suppressed_filesystem_files += 1;
            return;
        }
        self.events
            .push(FileActivityEvent::Evidence(self.metadata.evidence(
                FileActivityTarget::exact(path),
                FileActivityEffect::MaybeWrite,
                FileActivitySource::FilesystemMtime,
                ActivityCertainty::Heuristic,
                Some(format!(
                    "mtime between {} and {}",
                    UtcTimestamp::from_system_time(bound.at).unix_milliseconds(),
                    self.options.through.unix_milliseconds()
                )),
            )));
        self.report.filesystem_files += 1;
    }
}

/// Whether `path` and its whole subtree precede `marker` in sorted walk
/// order. Path ordering compares components, which matches a depth-first
/// walk with children sorted by file name.
fn entirely_before(path: &Path, marker: &Utf8Path) -> bool {
    path < marker.as_std_path() && !marker.as_std_path().starts_with(path)
}

fn should_descend(
    entry: &DirEntry,
    ignored: &BTreeSet<String>,
    excluded_roots: &BTreeSet<Utf8PathBuf>,
) -> bool {
    if excluded_roots
        .iter()
        .any(|excluded| entry.path().starts_with(excluded.as_std_path()))
    {
        return false;
    }
    entry.depth() == 0
        || !entry.file_type().is_dir()
        || !ignored.contains(&entry.file_name().to_string_lossy().into_owned())
}

fn git_command(root: &Utf8Path) -> Command {
    let mut command = Command::new("git");
    // Reconciliation must not contend with the agent's own git commands for
    // `index.lock`.
    command
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(root.as_str());
    command
}

/// Collects dirty paths under each root.
///
/// `git status --porcelain` prints paths relative to the repository top
/// level, whichever directory `-C` names, and lists the whole repository
/// unless given a pathspec. The scan is therefore limited to the root with a
/// `.` pathspec, and each path is re-rooted by stripping the root's own
/// repository prefix (`git rev-parse --show-prefix`). Paths are streamed and
/// charged to the entry budget.
fn collect_git_dirty_activity(
    options: &ReconciliationOptions,
    metadata: &ObservationMetadata,
    handled: &HandledBaselines,
    events: &mut Vec<FileActivityEvent>,
    report: &mut ReconciliationReport,
) -> Result<()> {
    let mut seen = BTreeSet::new();
    let mut collector = GitActivityCollector {
        excluded_roots: &options.excluded_roots,
        metadata,
        handled,
        events,
        report,
        seen: &mut seen,
    };
    let mut remaining = options.max_entries;
    for root in &options.roots {
        let prefix = match git_command(root)
            .args(["rev-parse", "--show-prefix"])
            .stderr(Stdio::null())
            .output()
        {
            Ok(output) if output.status.success() => match String::from_utf8(output.stdout) {
                Ok(prefix) => prefix.trim_end_matches(['\n', '\r']).to_owned(),
                Err(_) => {
                    push_reconciliation_gap(
                        collector.report,
                        FileActivitySource::VcsDirty,
                        format!("git reported a non-UTF-8 prefix for {root}"),
                    );
                    continue;
                }
            },
            Ok(output) => {
                push_reconciliation_gap(
                    collector.report,
                    FileActivitySource::VcsDirty,
                    format!("git rev-parse failed in {} with {}", root, output.status),
                );
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                push_reconciliation_gap(
                    collector.report,
                    FileActivitySource::VcsDirty,
                    "git executable is unavailable",
                );
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        let mut child = match git_command(root)
            .args([
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--ignored=no",
                "--",
                ".",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                push_reconciliation_gap(
                    collector.report,
                    FileActivitySource::VcsDirty,
                    "git executable is unavailable",
                );
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            continue;
        };
        let mut records = BufReader::new(stdout).split(0);
        let mut stopped = false;
        while let Some(record) = records.next() {
            let record = record?;
            if record.len() < 4 {
                continue;
            }
            if remaining == 0 {
                stopped = true;
                break;
            }
            remaining -= 1;
            let renamed = record[..2].iter().any(|byte| matches!(*byte, b'R' | b'C'));
            collector.append(root, &prefix, &record[3..])?;
            if renamed {
                if let Some(source) = records.next() {
                    collector.append(root, &prefix, &source?)?;
                }
            }
        }
        if stopped {
            let _ = child.kill();
            let _ = child.wait();
            collector.report.vcs_truncated = true;
            push_reconciliation_gap(
                collector.report,
                FileActivitySource::VcsDirty,
                format!(
                    "git dirty fallback stopped after {} paths",
                    options.max_entries
                ),
            );
            break;
        }
        let status = child.wait()?;
        if !status.success() {
            push_reconciliation_gap(
                collector.report,
                FileActivitySource::VcsDirty,
                format!("git status failed in {root} with {status}"),
            );
        }
    }
    Ok(())
}

struct GitActivityCollector<'a> {
    excluded_roots: &'a BTreeSet<Utf8PathBuf>,
    metadata: &'a ObservationMetadata,
    handled: &'a HandledBaselines,
    events: &'a mut Vec<FileActivityEvent>,
    report: &'a mut ReconciliationReport,
    seen: &'a mut BTreeSet<Utf8PathBuf>,
}

impl GitActivityCollector<'_> {
    /// Appends one repository-relative porcelain path under `root`, whose
    /// repository-relative prefix is `prefix`.
    fn append(&mut self, root: &Utf8Path, prefix: &str, raw: &[u8]) -> Result<()> {
        let relative = match std::str::from_utf8(raw) {
            Ok(relative) => relative,
            Err(_) => {
                push_reconciliation_gap(
                    self.report,
                    FileActivitySource::VcsDirty,
                    "git status reported a non-UTF-8 path",
                );
                return Ok(());
            }
        };
        // The pathspec limits status to the root, but a rename's other side
        // may lie elsewhere in the repository.
        let Some(relative) = relative.strip_prefix(prefix) else {
            return Ok(());
        };
        let path = normalize_utf8_path(root.join(relative));
        if self
            .excluded_roots
            .iter()
            .any(|excluded| path.starts_with(excluded))
        {
            return Ok(());
        }
        if self.seen.insert(path.clone()) {
            if fallback_matches_handled(&path, self.handled) {
                self.report.suppressed_vcs_files += 1;
                return Ok(());
            }
            self.events
                .push(FileActivityEvent::Evidence(self.metadata.evidence(
                    FileActivityTarget::exact(path),
                    FileActivityEffect::MaybeWrite,
                    FileActivitySource::VcsDirty,
                    ActivityCertainty::Heuristic,
                    Some("git status dirty path; may predate this session".to_owned()),
                )));
            self.report.vcs_files += 1;
        }
        Ok(())
    }
}

fn push_reconciliation_gap(
    report: &mut ReconciliationReport,
    source: FileActivitySource,
    detail: impl Into<String>,
) {
    report.gaps.push(ReconciliationGap {
        source,
        detail: detail.into(),
    });
}

fn fallback_matches_handled(path: &Utf8Path, handled: &HandledBaselines) -> bool {
    let path = normalize_handled_path(path);
    let Some(baseline) = handled.get(&path) else {
        return false;
    };
    fingerprint(&path)
        .map(|current| current == baseline.fingerprint)
        .unwrap_or(false)
}

/// Normalize persisted handled keys through the nearest existing ancestor.
/// This preserves missing/deleted suffixes while collapsing platform aliases
/// such as macOS `/var` and `/private/var` for existing files.
fn normalize_handled_path(path: &Utf8Path) -> Utf8PathBuf {
    let lexical = normalize_utf8_path(path);
    let Some(parent) = lexical.as_std_path().parent() else {
        return lexical;
    };
    for ancestor in parent.ancestors() {
        let Ok(canonical) = std::fs::canonicalize(ancestor) else {
            continue;
        };
        let Ok(suffix) = lexical.as_std_path().strip_prefix(ancestor) else {
            continue;
        };
        let canonical = canonical.join(suffix);
        if let Ok(canonical) = Utf8PathBuf::from_path_buf(canonical) {
            return normalize_utf8_path(canonical);
        }
    }
    lexical
}

/// Fingerprints a path, streaming regular-file content through SHA-256 so
/// memory use does not grow with file size.
fn fingerprint(path: &Utf8Path) -> Result<HandledFingerprint> {
    let metadata = match std::fs::symlink_metadata(path.as_std_path()) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(HandledFingerprint {
                kind: HandledFileKind::Missing,
                content_sha256: None,
            });
        }
        Err(error) => return Err(error.into()),
    };
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Ok(HandledFingerprint {
            kind: HandledFileKind::Symlink,
            content_sha256: None,
        });
    }
    if file_type.is_file() {
        let mut file = std::fs::File::open(path.as_std_path())?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0; 64 * 1024];
        loop {
            let read = match file.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            };
            hasher.update(&buffer[..read]);
        }
        return Ok(HandledFingerprint {
            kind: HandledFileKind::File,
            content_sha256: Some(hex(&hasher.finalize())),
        });
    }
    Ok(HandledFingerprint {
        kind: if file_type.is_dir() {
            HandledFileKind::Directory
        } else {
            HandledFileKind::Other
        },
        content_sha256: None,
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Bounds for materializing pending activity into existing files.
///
/// Construct with [`ResolveOptions::new`] and assign fields. Relative roots
/// and excluded roots are resolved against the process working directory.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ResolveOptions {
    /// Workspace roots used for unrooted workspace and relative glob targets.
    pub roots: Vec<Utf8PathBuf>,
    /// Maximum directory entries visited across all scoped targets.
    pub max_entries: usize,
    /// Directory basenames pruned from recursive traversal. Exact files
    /// inside such a directory (relative to a root) are not applicable.
    pub ignored_directory_names: BTreeSet<String>,
    /// Roots excluded from results and traversal, including exact files.
    pub excluded_roots: BTreeSet<Utf8PathBuf>,
}

impl ResolveOptions {
    /// Creates bounded resolution options with the default ignored directories.
    pub fn new(roots: Vec<Utf8PathBuf>) -> Self {
        Self {
            roots,
            max_entries: 100_000,
            ignored_directory_names: default_ignored_directory_names(),
            excluded_roots: BTreeSet::new(),
        }
    }
}

/// Existing files materialized from a pending activity window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedFileActivity {
    /// Deterministic set of existing regular files.
    pub files: BTreeSet<Utf8PathBuf>,
    /// Exact targets deliberately classified as not applicable: missing or
    /// non-file paths (for example after a deletion), and files inside an
    /// excluded root or an ignored directory.
    pub not_applicable_files: BTreeSet<Utf8PathBuf>,
    /// Scoped targets that may still contain unchecked files: each target
    /// whose traversal hit a failure such as an I/O error, and, once the
    /// entry budget is exhausted, the target being walked plus every scoped
    /// target after it. A scope whose root no longer exists, or lies in an
    /// ignored or excluded directory, has nothing left to check and is not
    /// retained.
    pub unresolved_targets: Vec<FileActivityTarget>,
    /// Directory entries charged to the shared traversal budget.
    pub scanned_entries: usize,
    /// Whether resolution stopped at `ResolveOptions::max_entries`.
    pub truncated: bool,
}

/// Materializes pending targets into existing regular files within explicit bounds.
///
/// Exact targets are resolved first; these probes are cheap and do not
/// consume the traversal-entry budget, so budget exhaustion can never drop a
/// directly observed file. Scoped targets are then walked in order without
/// following directory symlinks. Invalid glob syntax is returned as an error;
/// other target-local traversal failures retain the target in
/// [`ResolvedFileActivity::unresolved_targets`].
pub fn resolve_files(
    activity: &PendingFileActivity,
    options: &ResolveOptions,
) -> Result<ResolvedFileActivity> {
    let working_directory = current_utf8_dir();
    let roots = options
        .roots
        .iter()
        .map(|root| absolute_path(root, working_directory.as_deref()))
        .collect::<Vec<_>>();
    let excluded_roots = options
        .excluded_roots
        .iter()
        .map(|root| absolute_path(root, working_directory.as_deref()))
        .collect::<BTreeSet<_>>();
    let mut resolved = ResolvedFileActivity::default();
    let mut scoped = Vec::new();
    for target in activity.targets() {
        match target {
            FileActivityTarget::Path {
                path,
                scope: FileActivityScope::Exact | FileActivityScope::ExactOrDescendants,
            } if path.is_file() => {
                if is_pruned_file(
                    path,
                    &roots,
                    &excluded_roots,
                    &options.ignored_directory_names,
                ) {
                    resolved.not_applicable_files.insert(path.clone());
                } else {
                    resolved.files.insert(path.clone());
                }
            }
            FileActivityTarget::Path {
                path,
                scope: FileActivityScope::Exact,
            } => {
                // Deleted, missing, or non-file exact targets are resolved as
                // not applicable.
                resolved.not_applicable_files.insert(path.clone());
            }
            FileActivityTarget::Path {
                path,
                scope: FileActivityScope::ExactOrDescendants,
            } if std::fs::symlink_metadata(path)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
            {
                resolved.not_applicable_files.insert(path.clone());
            }
            _ => scoped.push(target),
        }
    }

    for (index, target) in scoped.iter().enumerate() {
        let remaining = options.max_entries.saturating_sub(resolved.scanned_entries);
        if remaining == 0 {
            resolved.truncated = true;
            resolved
                .unresolved_targets
                .extend(scoped[index..].iter().map(|target| (*target).clone()));
            break;
        }
        let mut resolution_options = TargetResolutionOptions::new(roots.clone());
        resolution_options.ignored_directory_names = options.ignored_directory_names.clone();
        resolution_options.excluded_roots = excluded_roots.clone();
        resolution_options.max_entries = remaining;
        resolution_options.symlinks = SymlinkPolicy::DoNotFollow;
        resolution_options.exact_paths = ExactPathPolicy::ExistingOnly;
        resolution_options.io_errors = ResolutionIssuePolicy::Report;
        resolution_options.invalid_globs = ResolutionIssuePolicy::Abort;
        let materialized =
            resolve_targets([activity_target_to_access(target)], &resolution_options).map_err(
                |error| match error.reason {
                    TargetResolutionReason::InvalidGlob { pattern, message } => {
                        FileActivityError::InvalidGlob { pattern, message }
                    }
                    reason => FileActivityError::Io(std::io::Error::other(format!(
                        "target resolution failed: {reason:?}"
                    ))),
                },
            )?;
        resolved.scanned_entries += materialized.scanned_entries;
        resolved
            .files
            .extend(materialized.paths.into_iter().filter(|path| path.is_file()));
        if materialized.budget_exhausted {
            // The walk stopped partway through this target: retain it and
            // every later scope rather than silently dropping the tail.
            resolved.truncated = true;
            resolved
                .unresolved_targets
                .extend(scoped[index..].iter().map(|target| (*target).clone()));
            break;
        }
        if materialized
            .unresolved
            .iter()
            .any(|unresolved| retains_scope(&unresolved.reason))
        {
            resolved.unresolved_targets.push((*target).clone());
        }
    }
    Ok(resolved)
}

/// Whether a resolution issue may hide unchecked files. A missing root has
/// nothing left to check, and ignored or excluded roots are pruned on purpose.
fn retains_scope(reason: &TargetResolutionReason) -> bool {
    !matches!(
        reason,
        TargetResolutionReason::NonexistentExactPath
            | TargetResolutionReason::NonexistentTraversalRoot { .. }
            | TargetResolutionReason::IgnoredDirectory { .. }
            | TargetResolutionReason::ExcludedRoot { .. }
    )
}

/// Whether an exact file lies in an excluded root, or in an ignored directory
/// below the workspace root that contains it.
fn is_pruned_file(
    path: &Utf8Path,
    roots: &[Utf8PathBuf],
    excluded_roots: &BTreeSet<Utf8PathBuf>,
    ignored_directory_names: &BTreeSet<String>,
) -> bool {
    let path = normalize_utf8_path(path);
    if excluded_roots
        .iter()
        .any(|excluded| path.starts_with(excluded))
    {
        return true;
    }
    roots
        .iter()
        .find_map(|root| path.strip_prefix(root).ok())
        .and_then(Utf8Path::parent)
        .is_some_and(|directories| {
            directories
                .components()
                .any(|component| ignored_directory_names.contains(component.as_str()))
        })
}

fn activity_target_to_access(target: &FileActivityTarget) -> AccessTarget {
    match target {
        FileActivityTarget::Path { path, scope } => AccessTarget::Path {
            expression: PathExpression {
                raw: path.to_string(),
                resolved: Some(path.clone()),
                base: PathBase::Absolute,
            },
            scope: match scope {
                FileActivityScope::Exact => AccessScope::Exact,
                FileActivityScope::Descendants => AccessScope::Descendants,
                FileActivityScope::ExactOrDescendants => AccessScope::ExactOrDescendants,
                FileActivityScope::Glob => AccessScope::Glob,
            },
        },
        FileActivityTarget::Workspace { root } => AccessTarget::Workspace { root: root.clone() },
    }
}

/// Directory names pruned by default: VCS metadata, HookKit's `.context`,
/// and common dependency, virtual-environment, cache, and build-tool output
/// directories that agents rarely edit and that would dominate a scan.
fn default_ignored_directory_names() -> BTreeSet<String> {
    [
        ".context",
        ".direnv",
        ".git",
        ".gradle",
        ".hg",
        ".mypy_cache",
        ".next",
        ".nox",
        ".nuxt",
        ".parcel-cache",
        ".pytest_cache",
        ".ruff_cache",
        ".svelte-kit",
        ".svn",
        ".terraform",
        ".tox",
        ".turbo",
        ".venv",
        "__pycache__",
        "node_modules",
        "target",
        "venv",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn utf8_path(path: &Path) -> Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(path.to_path_buf())
        .map_err(FileActivityError::NonUtf8Path)
        .map(normalize_utf8_path)
}

fn current_utf8_dir() -> Option<Utf8PathBuf> {
    std::env::current_dir()
        .ok()
        .and_then(|directory| Utf8PathBuf::from_path_buf(directory).ok())
}

/// Resolves a relative path against the process working directory, when known.
fn absolute_path(path: &Utf8Path, working_directory: Option<&Utf8Path>) -> Utf8PathBuf {
    match working_directory {
        Some(directory) if path.is_relative() => resolve_utf8_path(directory, path),
        _ => normalize_utf8_path(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hookkit_core::HarnessId;
    use hookkit_session_state::{EntityOutcome, SessionIdentity};
    use proptest::prelude::*;
    use std::time::UNIX_EPOCH;

    fn temporary_directory(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "hookkit-file-activity-{label}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn observation_metadata() -> ObservationMetadata {
        ObservationMetadata {
            observed_at: UtcTimestamp::now(),
            event: Some("PostToolUse".to_owned()),
            tool_call_id: Some("tool".to_owned()),
            turn_id: Some("turn".to_owned()),
        }
    }

    fn codex_post_tool(tool_name: &str, tool_input: serde_json::Value) -> PostToolUseInput {
        PostToolUseInput::Codex(
            serde_json::from_value(serde_json::json!({
                "session_id": "session",
                "transcript_path": "/tmp/transcript.jsonl",
                "cwd": "/repo",
                "hook_event_name": "PostToolUse",
                "model": "gpt-test",
                "turn_id": "turn",
                "permission_mode": "default",
                "tool_name": tool_name,
                "tool_use_id": "tool",
                "tool_input": tool_input,
                "tool_response": {"exit_code": 0}
            }))
            .unwrap(),
        )
    }

    fn antigravity_post_tool(command: &str) -> PostToolUseInput {
        PostToolUseInput::Antigravity(
            serde_json::from_value(serde_json::json!({
                "conversationId": "conversation",
                "workspacePaths": ["/repo"],
                "transcriptPath": "/tmp/transcript.jsonl",
                "artifactDirectoryPath": "/tmp/artifacts",
                "toolCall": {
                    "name": "run_command",
                    "args": {"CommandLine": command, "Cwd": "/repo"}
                },
                "stepIdx": 2
            }))
            .unwrap(),
        )
    }

    fn analyze_activity(input: &PostToolUseInput) -> ActivityReport {
        activity_report(
            ToolAccessAnalyzer::default().analyze_post_tool(input),
            &observation_metadata(),
        )
    }

    #[test]
    fn patch_observation_retains_direct_provenance() {
        // Codex's native apply_patch hook input is `{"command": "<patch>"}`.
        let report = analyze_activity(&codex_post_tool(
            "apply_patch",
            serde_json::json!({
                "command": "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n-old\n+new\n*** Add File: src/new.rs\n+new\n*** End Patch"
            }),
        ));

        let evidence = report.evidence().collect::<Vec<_>>();
        assert_eq!(evidence.len(), 2);
        assert!(report.gaps().next().is_none());
        assert!(evidence.iter().all(|item| {
            item.source == FileActivitySource::Patch && item.certainty == ActivityCertainty::Direct
        }));
        assert!(evidence.iter().any(|item| {
            item.target == FileActivityTarget::exact(Utf8PathBuf::from("/repo/src/lib.rs"))
        }));
    }

    #[test]
    fn file_free_tools_record_no_gaps() {
        for tool in ["update_plan", "spawn_agent", "web_search"] {
            let report = analyze_activity(&codex_post_tool(tool, serde_json::json!({})));
            assert!(report.is_empty(), "{tool}: {report:?}");
        }
    }

    #[test]
    fn claude_post_tool_use_failure_is_observed_like_post_tool_use() {
        let input: hookkit_claude::catalog::CatalogInput =
            serde_json::from_value(serde_json::json!({
                "session_id": "session",
                "transcript_path": "/tmp/transcript.jsonl",
                "cwd": "/repo",
                "hook_event_name": "PostToolUseFailure",
                "tool_name": "Bash",
                "tool_input": {"command": "sed -i 's/a/b/' src/x.py && pytest"},
                "tool_use_id": "toolu_1",
                "error": "Exit code 1"
            }))
            .unwrap();
        let report = activity_report(
            ToolAccessAnalyzer::default().analyze_native(&input),
            &observation_metadata(),
        );
        assert!(report.evidence().any(|item| {
            item.target == FileActivityTarget::exact(Utf8PathBuf::from("/repo/src/x.py"))
                && item.effect == FileActivityEffect::CreateOrModify
        }));
    }

    #[test]
    fn append_report_persists_a_digest_of_large_key_prefixes() {
        let project = temporary_directory("append-key");
        let state = SessionState::open(
            HarnessId::CODEX,
            SessionIdentity::Session("session".to_owned()),
            StateRoot::new(project.join("state")),
        )
        .unwrap();
        let store = FileActivityStore::from_state(state).unwrap();
        let prefix = "x".repeat(100_000);
        let report = ActivityReport {
            events: vec![FileActivityEvent::Gap(
                observation_metadata().gap(FileActivitySource::ShellInference, "gap"),
            )],
        };
        store.append_report(&prefix, &report).unwrap();
        store
            .pending()
            .with_entity(|view| {
                let key = view.events()[0].event_key();
                assert!(key.len() < 100, "{}", key.len());
                assert_eq!(key, format!("{}\0{}", sha256_hex(prefix.as_bytes()), 0));
                Ok(EntityOutcome::retain(()))
            })
            .unwrap();
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn shell_observation_keeps_candidates_and_known_gaps() {
        let report = analyze_activity(&codex_post_tool(
            "Bash",
            serde_json::json!({"command": "echo ok > src/out.txt; mystery $TARGET"}),
        ));

        assert!(report.evidence().any(|item| {
            item.target == FileActivityTarget::exact(Utf8PathBuf::from("/repo/src/out.txt"))
        }));
        assert!(report.gaps().next().is_some());
    }

    #[test]
    fn antigravity_post_tool_observes_shell_modifications() {
        let report = analyze_activity(&antigravity_post_tool("printf ok > src/out.txt"));

        assert!(report.evidence().any(|item| {
            item.source == FileActivitySource::ShellInference
                && item.target == FileActivityTarget::exact(Utf8PathBuf::from("/repo/src/out.txt"))
        }));
    }

    #[test]
    fn shared_access_report_keeps_modifications_and_drops_reads() {
        let report = analyze_activity(&codex_post_tool(
            "Bash",
            serde_json::json!({"command": "cat src/input.rs; touch src/new.rs"}),
        ));
        assert!(report.evidence().any(|item| {
            item.source == FileActivitySource::ShellInference
                && item.target == FileActivityTarget::exact(Utf8PathBuf::from("/repo/src/new.rs"))
        }));
        assert!(!report.evidence().any(|item| {
            item.target == FileActivityTarget::exact(Utf8PathBuf::from("/repo/src/input.rs"))
        }));
    }

    #[test]
    fn aggregate_keeps_targets_and_counts_loss() {
        let timestamp = UtcTimestamp::now();
        let target = FileActivityTarget::exact(Utf8PathBuf::from("/repo/src/lib.rs"));
        let mut activity = PendingFileActivity::empty();
        activity.apply(&FileActivityEvent::Evidence(FileActivityEvidence {
            target: target.clone(),
            effect: FileActivityEffect::MaybeWrite,
            source: FileActivitySource::FilesystemMtime,
            certainty: ActivityCertainty::Heuristic,
            observed_at: timestamp,
            event: None,
            tool_call_id: None,
            turn_id: None,
            detail: None,
        }));
        activity.apply(&FileActivityEvent::Gap(FileActivityGap {
            source: FileActivitySource::ShellInference,
            observed_at: timestamp,
            event: None,
            tool_call_id: None,
            turn_id: None,
            detail: "dynamic path".to_owned(),
        }));

        assert_eq!(activity.targets(), &BTreeSet::from([target]));
        assert_eq!(activity.evidence_count(), 1);
        assert_eq!(activity.gap_count(), 1);
    }

    #[test]
    fn compatibility_resolver_keeps_exact_files_outside_the_walk_budget() {
        let project = temporary_directory("resolve-compatibility");
        let file = Utf8PathBuf::from_path_buf(project.join("exact.txt")).unwrap();
        std::fs::write(&file, "exact").unwrap();
        let mut activity = PendingFileActivity::empty();
        activity.apply(&FileActivityEvent::Evidence(FileActivityEvidence {
            target: FileActivityTarget::exact(file.clone()),
            effect: FileActivityEffect::CreateOrModify,
            source: FileActivitySource::ShellInference,
            certainty: ActivityCertainty::Direct,
            observed_at: UtcTimestamp::now(),
            event: None,
            tool_call_id: None,
            turn_id: None,
            detail: None,
        }));
        let mut options =
            ResolveOptions::new(vec![Utf8PathBuf::from_path_buf(project.clone()).unwrap()]);
        options.max_entries = 0;

        let resolved = resolve_files(&activity, &options).unwrap();
        assert_eq!(resolved.files, BTreeSet::from([file]));
        assert_eq!(resolved.scanned_entries, 0);
        assert!(!resolved.truncated);

        std::fs::remove_dir_all(project).unwrap();
    }

    fn pending_with(targets: impl IntoIterator<Item = FileActivityTarget>) -> PendingFileActivity {
        let mut activity = PendingFileActivity::empty();
        for target in targets {
            activity.apply(&FileActivityEvent::Evidence(FileActivityEvidence {
                target,
                effect: FileActivityEffect::MaybeWrite,
                source: FileActivitySource::ShellInference,
                certainty: ActivityCertainty::Heuristic,
                observed_at: UtcTimestamp::now(),
                event: None,
                tool_call_id: None,
                turn_id: None,
                detail: None,
            }));
        }
        activity
    }

    fn scoped(path: PathBuf, scope: FileActivityScope) -> FileActivityTarget {
        FileActivityTarget::Path {
            path: Utf8PathBuf::from_path_buf(path).unwrap(),
            scope,
        }
    }

    fn utf8(path: PathBuf) -> Utf8PathBuf {
        Utf8PathBuf::from_path_buf(path).unwrap()
    }

    #[test]
    fn deleted_ignored_and_excluded_roots_resolve_as_empty_scopes() {
        let project = temporary_directory("resolve-deleted");
        std::fs::create_dir_all(project.join("node_modules/pkg")).unwrap();
        std::fs::write(project.join("node_modules/pkg/index.js"), "x").unwrap();
        std::fs::create_dir_all(project.join("state/v1")).unwrap();
        let missing = utf8(project.join("deleted.rs"));
        let activity = pending_with([
            FileActivityTarget::exact(missing.clone()),
            scoped(
                project.join("missing-scope"),
                FileActivityScope::Descendants,
            ),
            scoped(project.join("build"), FileActivityScope::ExactOrDescendants),
            scoped(project.join("dist/*.js"), FileActivityScope::Glob),
            scoped(
                project.join("node_modules"),
                FileActivityScope::ExactOrDescendants,
            ),
            scoped(project.join("state"), FileActivityScope::Descendants),
        ]);
        let mut options = ResolveOptions::new(vec![utf8(project.clone())]);
        options.excluded_roots.insert(utf8(project.join("state")));

        let resolved = resolve_files(&activity, &options).unwrap();
        assert!(resolved.not_applicable_files.contains(&missing));
        assert!(
            resolved
                .not_applicable_files
                .contains(&utf8(project.join("build")))
        );
        assert!(
            resolved.unresolved_targets.is_empty(),
            "{:?}",
            resolved.unresolved_targets
        );
        assert!(resolved.files.is_empty());
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn budget_exhaustion_retains_every_later_scope_and_never_drops_exact_files() {
        let project = temporary_directory("resolve-tail");
        std::fs::create_dir_all(project.join("a_scope")).unwrap();
        std::fs::create_dir_all(project.join("b_scope")).unwrap();
        for index in 0..10 {
            std::fs::write(project.join(format!("a_scope/f{index}")), "x").unwrap();
        }
        std::fs::write(project.join("b_scope/g"), "x").unwrap();
        std::fs::write(project.join("z_edited.rs"), "x").unwrap();
        let a_scope = scoped(project.join("a_scope"), FileActivityScope::Descendants);
        let b_scope = scoped(project.join("b_scope"), FileActivityScope::Descendants);
        let exact = utf8(project.join("z_edited.rs"));
        let activity = pending_with([
            a_scope.clone(),
            b_scope.clone(),
            FileActivityTarget::exact(exact.clone()),
        ]);
        let mut options = ResolveOptions::new(vec![utf8(project.clone())]);
        options.max_entries = 5;

        let resolved = resolve_files(&activity, &options).unwrap();
        assert!(resolved.truncated);
        assert!(resolved.files.contains(&exact));
        assert_eq!(resolved.unresolved_targets, vec![a_scope, b_scope]);
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn exact_files_in_excluded_or_ignored_directories_are_not_applicable() {
        let project = temporary_directory("resolve-exact-filters");
        std::fs::create_dir_all(project.join("node_modules/pkg")).unwrap();
        std::fs::create_dir_all(project.join("state")).unwrap();
        std::fs::create_dir_all(project.join("src")).unwrap();
        let vendored = utf8(project.join("node_modules/pkg/index.js"));
        let journal = utf8(project.join("state/journal.json"));
        let source = utf8(project.join("src/target"));
        for path in [&vendored, &journal, &source] {
            std::fs::write(path, "x").unwrap();
        }
        let activity = pending_with(
            [&vendored, &journal, &source].map(|path| FileActivityTarget::exact(path.clone())),
        );
        let mut options = ResolveOptions::new(vec![utf8(project.clone())]);
        options
            .excluded_roots
            .insert(journal.parent().unwrap().to_path_buf());

        let resolved = resolve_files(&activity, &options).unwrap();
        // A file named like an ignored directory is still a candidate.
        assert_eq!(resolved.files, BTreeSet::from([source]));
        assert_eq!(
            resolved.not_applicable_files,
            BTreeSet::from([journal, vendored])
        );
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn duplicate_retry_append_is_idempotent_in_pending_projection() {
        let project = temporary_directory("retry-idempotence");
        let path = Utf8PathBuf::from_path_buf(project.join("manual.rs")).unwrap();
        let state = SessionState::open(
            HarnessId::CODEX,
            SessionIdentity::Session("session".to_owned()),
            StateRoot::new(project.join("state")),
        )
        .unwrap();
        let store = FileActivityStore::from_state(state).unwrap();
        store
            .requeue_exact("manual-or-operational", [path.clone()])
            .unwrap();
        store
            .requeue_exact("manual-or-operational", [path.clone()])
            .unwrap();
        store
            .pending()
            .with_entity(|view| {
                assert_eq!(view.state().retry_count(), 1);
                assert_eq!(
                    view.state().targets(),
                    &BTreeSet::from([FileActivityTarget::exact(path.clone())])
                );
                Ok(EntityOutcome::retain(()))
            })
            .unwrap();
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn retry_append_survives_a_crash_before_source_acknowledgement() {
        let project = temporary_directory("retry-crash");
        let path = Utf8PathBuf::from_path_buf(project.join("manual.rs")).unwrap();
        std::fs::write(&path, "manual").unwrap();
        let state = SessionState::open(
            HarnessId::CODEX,
            SessionIdentity::Session("session".to_owned()),
            StateRoot::new(project.join("state")),
        )
        .unwrap();
        let store = FileActivityStore::from_state(state).unwrap();
        let original = ActivityReport {
            events: vec![FileActivityEvent::Evidence(FileActivityEvidence {
                target: FileActivityTarget::exact(path.clone()),
                effect: FileActivityEffect::CreateOrModify,
                source: FileActivitySource::StructuredToolInput,
                certainty: ActivityCertainty::Direct,
                observed_at: UtcTimestamp::now(),
                event: Some("PostToolUse".into()),
                tool_call_id: Some("original".into()),
                turn_id: None,
                detail: None,
            })],
        };
        store.append_report("original", &original).unwrap();

        let crashed = store.pending().try_with_entity(
            |_| -> std::result::Result<EntityOutcome<()>, &'static str> {
                store
                    .requeue_exact("manual-or-operational", [path.clone()])
                    .unwrap();
                Err("simulated crash")
            },
        );
        assert!(crashed.is_err());

        store
            .pending()
            .with_entity(|view| {
                assert!(
                    view.state()
                        .targets()
                        .contains(&FileActivityTarget::exact(path.clone()))
                );
                assert_eq!(view.state().retry_count(), 1);
                // The consumer always writes a fresh copy outside its own
                // sealed window before acknowledging that window.
                store
                    .requeue_exact("manual-or-operational", [path.clone()])
                    .unwrap();
                Ok(EntityOutcome::acknowledge(()))
            })
            .unwrap();
        store
            .pending()
            .with_entity(|view| {
                assert_eq!(view.events().len(), 1);
                assert_eq!(view.state().retry_count(), 1);
                assert_eq!(
                    view.state().targets(),
                    &BTreeSet::from([FileActivityTarget::exact(path.clone())])
                );
                Ok(EntityOutcome::retain(()))
            })
            .unwrap();
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn summary_commit_before_disposition_leaves_source_pending_on_crash() {
        let project = temporary_directory("summary-crash");
        let path = Utf8PathBuf::from_path_buf(project.join("source.rs")).unwrap();
        std::fs::write(&path, "source").unwrap();
        let state = SessionState::open(
            HarnessId::CODEX,
            SessionIdentity::Session("session".to_owned()),
            StateRoot::new(project.join("state")),
        )
        .unwrap();
        let store = FileActivityStore::from_state(state).unwrap();
        store
            .append_report(
                "original",
                &ActivityReport {
                    events: vec![FileActivityEvent::Evidence(FileActivityEvidence {
                        target: FileActivityTarget::exact(path.clone()),
                        effect: FileActivityEffect::CreateOrModify,
                        source: FileActivitySource::StructuredToolInput,
                        certainty: ActivityCertainty::Direct,
                        observed_at: UtcTimestamp::now(),
                        event: Some("PostToolUse".into()),
                        tool_call_id: Some("original".into()),
                        turn_id: None,
                        detail: None,
                    })],
                },
            )
            .unwrap();
        let family = store
            .state()
            .family(FamilyId::new("test.summary-order", 1).unwrap())
            .unwrap();

        let crashed = store.pending().try_with_entity(
            |_| -> std::result::Result<EntityOutcome<()>, &'static str> {
                let summary = family.start_run("turn-completion").unwrap();
                let summary_path = summary
                    .commit(&serde_json::json!({
                        "plannedSourceAcknowledgement": true
                    }))
                    .unwrap();
                assert!(summary_path.is_file());
                Err("simulated crash")
            },
        );
        assert!(crashed.is_err());
        store
            .pending()
            .with_entity(|view| {
                assert!(
                    view.state()
                        .targets()
                        .contains(&FileActivityTarget::exact(path.clone()))
                );
                Ok(EntityOutcome::retain(()))
            })
            .unwrap();
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn observation_appended_during_consumption_survives_acknowledgement() {
        let project = temporary_directory("concurrent-observation");
        let first = Utf8PathBuf::from_path_buf(project.join("first.rs")).unwrap();
        let concurrent = Utf8PathBuf::from_path_buf(project.join("concurrent.rs")).unwrap();
        std::fs::write(&first, "first").unwrap();
        std::fs::write(&concurrent, "concurrent").unwrap();
        let state = SessionState::open(
            HarnessId::CODEX,
            SessionIdentity::Session("session".to_owned()),
            StateRoot::new(project.join("state")),
        )
        .unwrap();
        let store = FileActivityStore::from_state(state).unwrap();
        let report = |path: Utf8PathBuf, id: &str| ActivityReport {
            events: vec![FileActivityEvent::Evidence(FileActivityEvidence {
                target: FileActivityTarget::exact(path),
                effect: FileActivityEffect::CreateOrModify,
                source: FileActivitySource::StructuredToolInput,
                certainty: ActivityCertainty::Direct,
                observed_at: UtcTimestamp::now(),
                event: Some("PostToolUse".into()),
                tool_call_id: Some(id.into()),
                turn_id: None,
                detail: None,
            })],
        };
        store
            .append_report("first", &report(first, "first"))
            .unwrap();
        store
            .pending()
            .with_entity(|_| {
                store
                    .append_report("concurrent", &report(concurrent.clone(), "concurrent"))
                    .unwrap();
                Ok(EntityOutcome::acknowledge(()))
            })
            .unwrap();
        store
            .pending()
            .with_entity(|view| {
                assert_eq!(view.events().len(), 1);
                assert_eq!(
                    view.state().targets(),
                    &BTreeSet::from([FileActivityTarget::exact(concurrent.clone())])
                );
                Ok(EntityOutcome::retain(()))
            })
            .unwrap();
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn partially_materialized_scope_is_requeued_before_acknowledgement() {
        let project = temporary_directory("scoped-retry");
        let scope_dir = project.join("scope");
        std::fs::create_dir_all(&scope_dir).unwrap();
        std::fs::write(scope_dir.join("a.rs"), "a").unwrap();
        std::fs::write(scope_dir.join("b.rs"), "b").unwrap();
        let target = FileActivityTarget::Path {
            path: Utf8PathBuf::from_path_buf(scope_dir).unwrap(),
            scope: FileActivityScope::Descendants,
        };
        let state = SessionState::open(
            HarnessId::CODEX,
            SessionIdentity::Session("session".to_owned()),
            StateRoot::new(project.join("state")),
        )
        .unwrap();
        let store = FileActivityStore::from_state(state).unwrap();
        store
            .append_report(
                "scope",
                &ActivityReport {
                    events: vec![FileActivityEvent::Evidence(FileActivityEvidence {
                        target: target.clone(),
                        effect: FileActivityEffect::MaybeWrite,
                        source: FileActivitySource::ShellInference,
                        certainty: ActivityCertainty::Heuristic,
                        observed_at: UtcTimestamp::now(),
                        event: Some("PostToolUse".into()),
                        tool_call_id: Some("scope".into()),
                        turn_id: None,
                        detail: None,
                    })],
                },
            )
            .unwrap();
        store
            .pending()
            .with_entity(|view| {
                let mut options =
                    ResolveOptions::new(vec![Utf8PathBuf::from_path_buf(project.clone()).unwrap()]);
                options.max_entries = 2;
                let resolved = resolve_files(view.state(), &options).unwrap();
                assert!(resolved.truncated);
                assert!(resolved.unresolved_targets.contains(&target));
                assert_eq!(resolved.files.len(), 1);
                store
                    .requeue_targets("unresolved-scope", resolved.unresolved_targets)
                    .unwrap();
                Ok(EntityOutcome::acknowledge(()))
            })
            .unwrap();
        store
            .pending()
            .with_entity(|view| {
                assert_eq!(view.state().targets(), &BTreeSet::from([target.clone()]));
                Ok(EntityOutcome::retain(()))
            })
            .unwrap();
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn handled_baseline_suppresses_fallback_but_not_direct_observation() {
        let project = temporary_directory("handled-fallback");
        let source = Utf8PathBuf::from_path_buf(project.join("source.rs")).unwrap();
        std::fs::write(&source, "same").unwrap();
        let state = SessionState::open(
            HarnessId::CODEX,
            SessionIdentity::Session("session".to_owned()),
            StateRoot::new(project.join("state")),
        )
        .unwrap();
        let store = FileActivityStore::from_state(state).unwrap();
        let baseline = store
            .record_handled_baselines([source.clone()], "run-1")
            .unwrap();
        assert_eq!(baseline.recorded.len(), 1);

        let mut options = ReconciliationOptions::new(
            vec![Utf8PathBuf::from_path_buf(project.clone()).unwrap()],
            UtcTimestamp::now(),
        );
        options.fallback_since = Some(UtcTimestamp::from_system_time(UNIX_EPOCH));
        let report = reconcile(&store, options).unwrap();
        assert_eq!(report.filesystem_files, 0);
        assert_eq!(report.suppressed_filesystem_files, 1);

        let direct = ActivityReport {
            events: vec![FileActivityEvent::Evidence(FileActivityEvidence {
                target: FileActivityTarget::exact(source.clone()),
                effect: FileActivityEffect::CreateOrModify,
                source: FileActivitySource::StructuredToolInput,
                certainty: ActivityCertainty::Direct,
                observed_at: UtcTimestamp::now(),
                event: Some("PostToolUse".into()),
                tool_call_id: Some("direct".into()),
                turn_id: None,
                detail: None,
            })],
        };
        store.append_report("direct", &direct).unwrap();
        store
            .pending()
            .with_entity(|view| {
                assert!(
                    view.state()
                        .targets()
                        .contains(&FileActivityTarget::exact(source.clone()))
                );
                Ok(EntityOutcome::retain(()))
            })
            .unwrap();
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn same_length_content_change_invalidates_handled_baseline() {
        let project = temporary_directory("handled-same-length");
        let source = Utf8PathBuf::from_path_buf(project.join("source.rs")).unwrap();
        std::fs::write(&source, "one").unwrap();
        let state = SessionState::open(
            HarnessId::CODEX,
            SessionIdentity::Session("session".to_owned()),
            StateRoot::new(project.join("state")),
        )
        .unwrap();
        let store = FileActivityStore::from_state(state).unwrap();
        store
            .record_handled_baselines([source.clone()], "run-1")
            .unwrap();
        std::fs::write(&source, "two").unwrap();

        let mut options = ReconciliationOptions::new(
            vec![Utf8PathBuf::from_path_buf(project.clone()).unwrap()],
            UtcTimestamp::now(),
        );
        options.fallback_since = Some(UtcTimestamp::from_system_time(UNIX_EPOCH));
        let report = reconcile(&store, options).unwrap();
        assert_eq!(report.filesystem_files, 1);
        assert_eq!(report.suppressed_filesystem_files, 0);
        std::fs::remove_dir_all(project).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn handled_fingerprint_preserves_symlink_type() {
        let project = temporary_directory("handled-symlink");
        let target = project.join("target.rs");
        let link = Utf8PathBuf::from_path_buf(project.join("link.rs")).unwrap();
        std::fs::write(&target, "target").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let state = SessionState::open(
            HarnessId::CODEX,
            SessionIdentity::Session("session".to_owned()),
            StateRoot::new(project.join("state")),
        )
        .unwrap();
        let store = FileActivityStore::from_state(state).unwrap();
        let report = store.record_handled_baselines([link], "run-1").unwrap();
        assert_eq!(report.recorded.len(), 1);
        assert_eq!(
            report.recorded[0].fingerprint.kind,
            HandledFileKind::Symlink
        );
        assert_eq!(report.recorded[0].fingerprint.content_sha256, None);
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn git_dirty_fallback_suppresses_unchanged_handled_content() {
        let project = temporary_directory("handled-git");
        let status = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&project)
            .status()
            .unwrap();
        assert!(status.success());
        let source = Utf8PathBuf::from_path_buf(project.join("source.rs")).unwrap();
        std::fs::write(&source, "dirty\n").unwrap();
        let state = SessionState::open(
            HarnessId::CODEX,
            SessionIdentity::Session("session".to_owned()),
            StateRoot::new(project.join("state")),
        )
        .unwrap();
        let store = FileActivityStore::from_state(state).unwrap();
        store.record_handled_baselines([source], "run-1").unwrap();

        let mut options = ReconciliationOptions::new(
            vec![Utf8PathBuf::from_path_buf(project.clone()).unwrap()],
            UtcTimestamp::now(),
        );
        options.filesystem_mtime = false;
        options.vcs = VcsFallback::GitDirty;
        let report = reconcile(&store, options).unwrap();
        assert_eq!(report.vcs_files, 0);
        assert_eq!(report.suppressed_vcs_files, 1);
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn reconciliation_excludes_its_state_and_does_not_repeat_before_cursor() {
        let project = temporary_directory("reconcile");
        let source = project.join("src/lib.rs");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "pub fn example() {}\n").unwrap();
        let state = SessionState::open(
            HarnessId::CODEX,
            SessionIdentity::Session("session".to_owned()),
            StateRoot::new(project.join("state")),
        )
        .unwrap();
        let store = FileActivityStore::from_state(state).unwrap();
        let first_through = UtcTimestamp::now();
        let mut options = ReconciliationOptions::new(
            vec![Utf8PathBuf::from_path_buf(project.clone()).unwrap()],
            first_through,
        );
        options.fallback_since = Some(UtcTimestamp::from_system_time(UNIX_EPOCH));

        let report = reconcile(&store, options).unwrap();
        assert_eq!(report.filesystem_files, 1);
        store
            .pending()
            .with_entity(|view| {
                assert_eq!(
                    view.state().targets(),
                    &BTreeSet::from([FileActivityTarget::exact(
                        Utf8PathBuf::from_path_buf(source.clone()).unwrap()
                    )])
                );
                Ok(EntityOutcome::acknowledge(()))
            })
            .unwrap();

        let second_through = UtcTimestamp::now();
        let mut options = ReconciliationOptions::new(
            vec![Utf8PathBuf::from_path_buf(project.clone()).unwrap()],
            second_through,
        );
        // Without clock slack, the incremental window starts exactly at the cursor.
        options.timestamp_tolerance = Duration::ZERO;
        let report = reconcile(&store, options).unwrap();
        assert_eq!(report.filesystem_files, 0);
        assert_eq!(store.reconciled_through().unwrap(), Some(second_through));
        store
            .pending()
            .with_entity(|view| {
                assert!(view.events().is_empty());
                Ok(EntityOutcome::retain(()))
            })
            .unwrap();

        std::fs::remove_dir_all(project).unwrap();
    }

    fn codex_store(project: &Path) -> FileActivityStore {
        let state = SessionState::open(
            HarnessId::CODEX,
            SessionIdentity::Session("session".to_owned()),
            StateRoot::new(project.join("state")),
        )
        .unwrap();
        FileActivityStore::from_state(state).unwrap()
    }

    fn pending_targets(store: &FileActivityStore) -> BTreeSet<FileActivityTarget> {
        store
            .pending()
            .with_entity(|view| Ok(EntityOutcome::retain(view.state().targets().clone())))
            .unwrap()
    }

    #[test]
    fn incremental_scans_overlap_the_cursor_by_the_tolerance() {
        let project = temporary_directory("reconcile-tolerance");
        let store = codex_store(&project);
        let root = vec![utf8(project.clone())];
        let mut first = ReconciliationOptions::new(root.clone(), UtcTimestamp::now());
        first.fallback_since = Some(UtcTimestamp::now());
        reconcile(&store, first).unwrap();
        let cursor = store.reconciled_through().unwrap().unwrap();

        // A coarse filesystem clock stamps a later write at or before the cursor.
        let late = project.join("late.rs");
        std::fs::write(&late, "late").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&late)
            .unwrap()
            .set_modified(cursor.as_system_time() - Duration::from_millis(500))
            .unwrap();

        let report = reconcile(
            &store,
            ReconciliationOptions::new(root, UtcTimestamp::now()),
        )
        .unwrap();
        assert_eq!(report.filesystem_files, 1);
        assert!(pending_targets(&store).contains(&FileActivityTarget::exact(utf8(late))));
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn truncated_mtime_scan_resumes_where_it_stopped() {
        let project = temporary_directory("reconcile-resume");
        for index in 0..10 {
            std::fs::write(project.join(format!("f{index}.rs")), "x").unwrap();
        }
        let store = codex_store(&project);
        let root = vec![utf8(project.clone())];
        let mut first = ReconciliationOptions::new(root.clone(), UtcTimestamp::now());
        first.fallback_since = Some(UtcTimestamp::from_system_time(UNIX_EPOCH));
        first.timestamp_tolerance = Duration::ZERO;
        first.max_entries = 4;
        let report = reconcile(&store, first).unwrap();
        assert!(report.truncated);
        // The root directory is the first of the four entries.
        assert_eq!(report.filesystem_files, 3);
        let resume = store.scan_resume().unwrap().unwrap();
        assert_eq!(resume.after, utf8(project.join("f2.rs")));

        let mut second = ReconciliationOptions::new(root, UtcTimestamp::now());
        second.timestamp_tolerance = Duration::ZERO;
        let report = reconcile(&store, second).unwrap();
        assert!(!report.truncated);
        assert_eq!(report.filesystem_files, 7);
        assert_eq!(store.scan_resume().unwrap(), None);
        let expected = (0..10)
            .map(|index| FileActivityTarget::exact(utf8(project.join(format!("f{index}.rs")))))
            .collect::<BTreeSet<_>>();
        assert_eq!(pending_targets(&store), expected);
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn a_scan_truncated_while_resuming_keeps_the_cursor_and_advances_the_resume_point() {
        let project = temporary_directory("reconcile-resume-twice");
        for index in 0..10 {
            std::fs::write(project.join(format!("f{index}.rs")), "x").unwrap();
        }
        let store = codex_store(&project);
        let root = vec![utf8(project.clone())];
        let mut first = ReconciliationOptions::new(root.clone(), UtcTimestamp::now());
        first.fallback_since = Some(UtcTimestamp::from_system_time(UNIX_EPOCH));
        first.max_entries = 4;
        reconcile(&store, first).unwrap();
        let cursor = store.reconciled_through().unwrap();

        let mut second = ReconciliationOptions::new(root.clone(), UtcTimestamp::now());
        second.max_entries = 3;
        let report = reconcile(&store, second).unwrap();
        assert!(report.truncated);
        assert_eq!(report.filesystem_files, 3);
        assert_eq!(store.reconciled_through().unwrap(), cursor);
        let resume = store.scan_resume().unwrap().unwrap();
        assert_eq!(resume.after, utf8(project.join("f5.rs")));
        // The unscanned tail still owes the first scan's (widened) lower bound.
        assert!(resume.tail_since <= UtcTimestamp::from_system_time(UNIX_EPOCH));
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn a_relative_state_directory_is_excluded_from_the_mtime_scan() {
        let project = temporary_directory("reconcile-relative-state");
        std::fs::write(project.join("source.rs"), "x").unwrap();
        // Express the state root relative to the test's working directory.
        let working_directory = std::env::current_dir().unwrap();
        let mut relative = PathBuf::new();
        for _ in working_directory.components().skip(1) {
            relative.push("..");
        }
        relative.push(project.join("state").strip_prefix("/").unwrap());
        let state = SessionState::open(
            HarnessId::CODEX,
            SessionIdentity::Session("session".to_owned()),
            StateRoot::new(relative),
        )
        .unwrap();
        assert!(state.directory().is_relative());
        let store = FileActivityStore::from_state(state).unwrap();
        let mut options =
            ReconciliationOptions::new(vec![utf8(project.clone())], UtcTimestamp::now());
        options.fallback_since = Some(UtcTimestamp::from_system_time(UNIX_EPOCH));
        let report = reconcile(&store, options).unwrap();
        assert_eq!(report.filesystem_files, 1, "{:?}", pending_targets(&store));
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn git_dirty_fallback_reroots_paths_from_a_repository_subdirectory() {
        let project = temporary_directory("git-subdirectory");
        let status = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&project)
            .status()
            .unwrap();
        assert!(status.success());
        std::fs::create_dir_all(project.join("packages/web/src")).unwrap();
        std::fs::create_dir_all(project.join("packages/other")).unwrap();
        std::fs::write(project.join("packages/web/src/a.ts"), "a").unwrap();
        std::fs::write(project.join("packages/other/b.ts"), "b").unwrap();
        let store = codex_store(&project);
        let root = utf8(project.join("packages/web"));
        let mut options = ReconciliationOptions::new(vec![root.clone()], UtcTimestamp::now());
        options.filesystem_mtime = false;
        options.vcs = VcsFallback::GitDirty;

        let report = reconcile(&store, options).unwrap();
        assert_eq!(report.vcs_files, 1, "{:?}", report.gaps);
        assert_eq!(
            pending_targets(&store),
            BTreeSet::from([FileActivityTarget::exact(root.join("src/a.ts"))])
        );
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn git_dirty_fallback_charges_paths_to_the_entry_budget() {
        let project = temporary_directory("git-budget");
        let status = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&project)
            .status()
            .unwrap();
        assert!(status.success());
        for index in 0..5 {
            std::fs::write(project.join(format!("dirty{index}.rs")), "x").unwrap();
        }
        let store = codex_store(&project);
        let mut options =
            ReconciliationOptions::new(vec![utf8(project.clone())], UtcTimestamp::now());
        options.filesystem_mtime = false;
        options.vcs = VcsFallback::GitDirty;
        options.max_entries = 2;

        let report = reconcile(&store, options).unwrap();
        assert!(report.vcs_truncated);
        assert_eq!(report.vcs_files, 2);
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn compaction_does_not_move_the_bootstrap_bound_past_the_session_start() {
        let project = temporary_directory("bootstrap-compact");
        let root = StateRoot::new(project.join("state"));
        let raw = hookkit_core::RawInvocation::parse(b"{}".to_vec()).unwrap();
        let ensure = |kind, timestamp: &str, key: &str| {
            let context = RuntimeContext::new(
                HarnessId::CLAUDE_CODE,
                hookkit_core::SnapshotId::builtin("test"),
                hookkit_core::EventId::builtin(HarnessId::CLAUDE_CODE, "SessionStart"),
                hookkit_core::ContractId::builtin("test"),
                hookkit_core::ResolutionProvenance::TypedStatic,
                &raw,
                hookkit_core::NativeContext {
                    workspace_roots: vec![utf8(project.clone())],
                    session_id: hookkit_core::SessionId::new("session-1").ok(),
                    session_boundary: Some(
                        hookkit_core::SessionBoundaryContext::observed(kind)
                            .with_native_timestamp(timestamp)
                            .with_occurrence_key(key),
                    ),
                    ..hookkit_core::NativeContext::default()
                },
                &hookkit_core::DISABLED_DIAGNOSTICS,
            )
            .unwrap();
            FileActivityStore::ensure(&context, root.clone()).unwrap()
        };
        ensure(
            hookkit_core::SessionBoundaryKind::Startup,
            "2026-07-12T10:00:00Z",
            "start",
        );
        let store = ensure(
            hookkit_core::SessionBoundaryKind::Compact,
            "2026-07-12T10:20:00Z",
            "compact",
        );
        assert_eq!(
            store.state().metadata().unwrap().current_session.kind,
            SessionEpochKind::Compact
        );
        assert_eq!(
            store.bootstrap_started_at().unwrap(),
            UtcTimestamp::parse_rfc3339("2026-07-12T10:00:00Z").unwrap()
        );
        std::fs::remove_dir_all(project).unwrap();
    }

    proptest! {
        /// Property: reconciliation cursors are a monotonic maximum. Event
        /// order and duplicate timestamps cannot move the cursor backwards.
        #[test]
        fn reconciliation_cursor_is_the_maximum_observed_time(
            seconds in prop::collection::vec(0u32..2_000_000_000, 0..100),
        ) {
            let timestamps = seconds.into_iter()
                .map(|second| UtcTimestamp::from_system_time(UNIX_EPOCH + Duration::from_secs(second.into())))
                .collect::<Vec<_>>();
            let mut cursor = ReconciliationCursor::empty();
            for timestamp in &timestamps {
                cursor.apply(timestamp);
            }

            prop_assert_eq!(cursor.reconciled_through, timestamps.into_iter().max());
        }

    }
}
