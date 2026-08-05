//! Loss-aware file activity observation and reconciliation.
#![deny(missing_docs)]
//!
//! Tool inspection is evidence, not an audit log. This crate retains both the
//! candidate target and the limits of the inference, then provides timestamp
//! and opt-in VCS fallbacks for deferred consumers.

use hookkit_common::PostToolUseInput;
use hookkit_core::{RuntimeContext, Utf8Path, Utf8PathBuf, normalize_utf8_path};
use hookkit_session_state::{
    EntityId, EntityJournal, EntityMode, EntityOutcome, FamilyId, JournalEntity, SessionState,
    StateRoot, UtcTimestamp,
};
use hookkit_tool_access::{
    AccessCandidate, AccessCertainty, AccessIntent, AccessScope, AccessSource, AccessTarget,
    ExactPathPolicy, PathBase, PathExpression, ResolutionIssuePolicy, SymlinkPolicy,
    TargetResolutionOptions, TargetResolutionReason, ToolAccessAnalyzer, resolve_targets,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};
use walkdir::{DirEntry, WalkDir};

/// Session-state family name used by [`FileActivityStore`].
pub const FILE_ACTIVITY_FAMILY: &str = "agent-hook-kit.file-activity";
/// Windowed entity name containing pending file-activity events.
pub const PENDING_ACTIVITY_ENTITY: &str = "pending-files";
/// Monotonic entity name containing the fallback reconciliation cursor.
pub const RECONCILIATION_CURSOR_ENTITY: &str = "reconciliation-cursor";
/// Monotonic entity name containing handled content baselines.
pub const HANDLED_BASELINES_ENTITY: &str = "handled-baselines";

/// Error returned by file-activity persistence, traversal, or reconciliation.
#[derive(Debug, thiserror::Error)]
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
}

/// Strength of the static or fallback association with a target.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
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
        let handled = scope.entity::<HandledBaselines>(
            EntityId::new(HANDLED_BASELINES_ENTITY, 1)?,
            EntityMode::Monotonic,
        )?;
        Ok(Self {
            state,
            pending,
            cursor,
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
    /// The event index is appended to `event_key_prefix`, so callers must use a
    /// prefix that uniquely and repeatably identifies the source observation.
    pub fn append_report(&self, event_key_prefix: &str, report: &ActivityReport) -> Result<()> {
        for (index, event) in report.events.iter().enumerate() {
            self.pending
                .append(&format!("{event_key_prefix}\0{index}"), event)?;
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

    /// Returns the current session's start time for initial reconciliation.
    pub fn bootstrap_started_at(&self) -> Result<UtcTimestamp> {
        self.state.current_session_started_at().map_err(Into::into)
    }
}

/// Observe one native post-tool event without claiming complete coverage.
pub fn observe_post_tool(input: &PostToolUseInput, context: &RuntimeContext<'_>) -> ActivityReport {
    let observed_at = UtcTimestamp::now();
    let event = Some(context.event().name().to_owned());
    let tool_call_id = context.tool_call_id().map(ToString::to_string);
    let turn_id = context.turn_id().map(ToString::to_string);
    let metadata = ObservationMetadata {
        observed_at,
        event,
        tool_call_id,
        turn_id,
    };

    let access = ToolAccessAnalyzer::default().analyze_post_tool(input);
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
        AccessSource::Custom => FileActivitySource::StructuredToolInput,
        _ => FileActivitySource::StructuredToolInput,
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
pub enum VcsFallback {
    /// Do not inspect version-control state.
    #[default]
    Disabled,
    /// Broad fallback. Includes changes that may predate the agent session.
    GitDirty,
}

/// Bounds and evidence sources for one reconciliation interval.
#[derive(Debug, Clone)]
pub struct ReconciliationOptions {
    /// Workspace roots to scan or query.
    pub roots: Vec<Utf8PathBuf>,
    /// Lower bound used only when no durable cursor is present.
    pub fallback_since: Option<UtcTimestamp>,
    /// Inclusive upper bound recorded after successful reconciliation.
    pub through: UtcTimestamp,
    /// Clock-resolution slack subtracted from the lower bound during the first,
    /// cursor-less reconciliation only; the upper bound and later incremental
    /// runs are unaffected.
    pub timestamp_tolerance: Duration,
    /// Whether to scan filesystem modification times.
    pub filesystem_mtime: bool,
    /// Optional version-control dirty-state fallback.
    pub vcs: VcsFallback,
    /// Maximum directory entries visited by the filesystem scan.
    pub max_entries: usize,
    /// Directory basenames pruned from recursive traversal.
    pub ignored_directory_names: BTreeSet<String>,
    /// Roots pruned from the filesystem-mtime scan and its traversal. The VCS
    /// dirty-state fallback does not honor this set.
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
    /// Whether the filesystem scan stopped at its entry budget.
    pub truncated: bool,
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

/// Reconcile fallback evidence, then durably advance the cursor. If appending
/// discoveries fails, the cursor remains unchanged and the next attempt safely
/// rescans the same interval.
pub fn reconcile(
    store: &FileActivityStore,
    mut options: ReconciliationOptions,
) -> Result<ReconciliationReport> {
    if let Ok(state_directory) = utf8_path(store.state().directory()) {
        options.excluded_roots.insert(state_directory);
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

    if options.filesystem_mtime {
        match since {
            Some(since) => collect_mtime_activity(
                &options,
                since,
                cursor.is_none(),
                &metadata,
                &handled,
                &mut events,
                &mut report,
            )?,
            None => push_reconciliation_gap(
                &mut report,
                FileActivitySource::FilesystemMtime,
                "filesystem fallback has no lower-bound timestamp",
            ),
        }
    }
    if options.vcs == VcsFallback::GitDirty {
        collect_git_dirty_activity(
            &options.roots,
            &options.excluded_roots,
            &metadata,
            &handled,
            &mut events,
            &mut report,
        )?;
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
    // The order is intentional: pending evidence before cursor advancement.
    store.record_reconciled_through(options.through)?;
    Ok(report)
}

fn collect_mtime_activity(
    options: &ReconciliationOptions,
    since: UtcTimestamp,
    is_bootstrap: bool,
    metadata: &ObservationMetadata,
    handled: &HandledBaselines,
    events: &mut Vec<FileActivityEvent>,
    report: &mut ReconciliationReport,
) -> Result<()> {
    let lower = if is_bootstrap {
        since
            .as_system_time()
            .checked_sub(options.timestamp_tolerance)
            .unwrap_or(SystemTime::UNIX_EPOCH)
    } else {
        since.as_system_time()
    };
    let upper = options.through.as_system_time();
    let mut seen = BTreeSet::new();
    'roots: for root in &options.roots {
        let walker = WalkDir::new(root.as_std_path())
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| {
                should_descend(
                    entry,
                    &options.ignored_directory_names,
                    &options.excluded_roots,
                )
            });
        for entry in walker {
            if report.scanned_entries >= options.max_entries {
                report.truncated = true;
                push_reconciliation_gap(
                    report,
                    FileActivitySource::FilesystemMtime,
                    format!(
                        "filesystem fallback stopped after {} entries",
                        options.max_entries
                    ),
                );
                break 'roots;
            }
            report.scanned_entries += 1;
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    push_reconciliation_gap(
                        report,
                        FileActivitySource::FilesystemMtime,
                        error.to_string(),
                    );
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let modified = match entry.metadata() {
                Ok(metadata) => match metadata.modified() {
                    Ok(modified) => modified,
                    Err(error) => {
                        push_reconciliation_gap(
                            report,
                            FileActivitySource::FilesystemMtime,
                            format!("{}: {error}", entry.path().display()),
                        );
                        continue;
                    }
                },
                Err(error) => {
                    push_reconciliation_gap(
                        report,
                        FileActivitySource::FilesystemMtime,
                        format!("{}: {error}", entry.path().display()),
                    );
                    continue;
                }
            };
            let before_window = if is_bootstrap {
                modified < lower
            } else {
                modified <= lower
            };
            if before_window || modified > upper {
                continue;
            }
            let path = match utf8_path(entry.path()) {
                Ok(path) => path,
                Err(error) => {
                    push_reconciliation_gap(
                        report,
                        FileActivitySource::FilesystemMtime,
                        error.to_string(),
                    );
                    continue;
                }
            };
            if !seen.insert(path.clone()) {
                continue;
            }
            if fallback_matches_handled(&path, handled) {
                report.suppressed_filesystem_files += 1;
                continue;
            }
            events.push(FileActivityEvent::Evidence(metadata.evidence(
                FileActivityTarget::exact(path),
                FileActivityEffect::MaybeWrite,
                FileActivitySource::FilesystemMtime,
                ActivityCertainty::Heuristic,
                Some(format!(
                    "mtime between {} and {}",
                    since.unix_milliseconds(),
                    options.through.unix_milliseconds()
                )),
            )));
            report.filesystem_files += 1;
        }
    }
    Ok(())
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

fn collect_git_dirty_activity(
    roots: &[Utf8PathBuf],
    excluded_roots: &BTreeSet<Utf8PathBuf>,
    metadata: &ObservationMetadata,
    handled: &HandledBaselines,
    events: &mut Vec<FileActivityEvent>,
    report: &mut ReconciliationReport,
) -> Result<()> {
    let mut seen = BTreeSet::new();
    let mut collector = GitActivityCollector {
        excluded_roots,
        metadata,
        handled,
        events,
        report,
        seen: &mut seen,
    };
    for root in roots {
        let output = Command::new("git")
            .arg("-C")
            .arg(root.as_str())
            .args([
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--ignored=no",
            ])
            .output();
        let output = match output {
            Ok(output) if output.status.success() => output,
            Ok(output) => {
                push_reconciliation_gap(
                    collector.report,
                    FileActivitySource::VcsDirty,
                    format!("git status failed in {} with {}", root, output.status),
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
        let fields = output.stdout.split(|byte| *byte == 0).collect::<Vec<_>>();
        let mut index = 0;
        while index < fields.len() {
            let field = fields[index];
            index += 1;
            if field.len() < 4 {
                continue;
            }
            let status = &field[..2];
            let path = &field[3..];
            collector.append(root, path)?;
            if status.iter().any(|byte| matches!(*byte, b'R' | b'C')) && index < fields.len() {
                collector.append(root, fields[index])?;
                index += 1;
            }
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
    fn append(&mut self, root: &Utf8Path, raw: &[u8]) -> Result<()> {
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
        let digest = Sha256::digest(std::fs::read(path.as_std_path())?);
        let content_sha256 = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        return Ok(HandledFingerprint {
            kind: HandledFileKind::File,
            content_sha256: Some(content_sha256),
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

/// Bounds for materializing pending activity into existing files.
#[derive(Debug, Clone)]
pub struct ResolveOptions {
    /// Workspace roots used for unrooted workspace and relative glob targets.
    pub roots: Vec<Utf8PathBuf>,
    /// Maximum directory entries visited across all targets.
    pub max_entries: usize,
    /// Directory basenames pruned from recursive traversal.
    pub ignored_directory_names: BTreeSet<String>,
    /// Roots excluded from results and traversal.
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
    /// Existing files that were deliberately classified as not applicable.
    pub not_applicable_files: BTreeSet<Utf8PathBuf>,
    /// The single target whose traversal exhausted the entry budget, if any; it
    /// may be partially resolved (files found before exhaustion are still
    /// included in `files`). Targets ordered after it in the window are skipped
    /// and are not listed here.
    pub unresolved_targets: Vec<FileActivityTarget>,
    /// Directory entries charged to the shared traversal budget.
    pub scanned_entries: usize,
    /// Whether resolution stopped at `ResolveOptions::max_entries`.
    pub truncated: bool,
}

/// Materializes pending targets into existing regular files within explicit bounds.
///
/// Directory symlinks are not followed. Exact regular-file probes are retained
/// for backward compatibility and do not consume the traversal-entry budget.
/// Invalid glob syntax is returned as an error; other target-local traversal
/// failures are ignored by this compatibility API.
pub fn resolve_files(
    activity: &PendingFileActivity,
    options: &ResolveOptions,
) -> Result<ResolvedFileActivity> {
    let mut resolved = ResolvedFileActivity::default();
    for target in activity.targets() {
        match target {
            FileActivityTarget::Path {
                path,
                scope: FileActivityScope::Exact,
            }
            | FileActivityTarget::Path {
                path,
                scope: FileActivityScope::ExactOrDescendants,
            } if path.is_file() => {
                // Compatibility: exact-file probes were never charged against
                // the traversal entry budget.
                resolved.files.insert(path.clone());
                continue;
            }
            FileActivityTarget::Path {
                path,
                scope: FileActivityScope::Exact,
            } => {
                // Deleted, missing, or non-file exact targets are resolved as
                // not applicable. Scoped targets remain unresolved because
                // they may still denote unchecked descendants.
                resolved.not_applicable_files.insert(path.clone());
                continue;
            }
            _ => {}
        }

        let access_target = activity_target_to_access(target);
        let resolution_options = TargetResolutionOptions {
            workspace_roots: options.roots.clone(),
            ignored_directory_names: options.ignored_directory_names.clone(),
            excluded_roots: options.excluded_roots.clone(),
            max_entries: options.max_entries.saturating_sub(resolved.scanned_entries),
            symlinks: SymlinkPolicy::DoNotFollow,
            exact_paths: ExactPathPolicy::ExistingOnly,
            // The compatibility API historically ignored traversal errors but
            // returned invalid glob syntax as an error.
            io_errors: ResolutionIssuePolicy::Report,
            invalid_globs: ResolutionIssuePolicy::Abort,
        };
        let materialized =
            resolve_targets([&access_target], &resolution_options).map_err(|error| match error
                .reason
            {
                TargetResolutionReason::InvalidGlob { pattern, message } => {
                    FileActivityError::InvalidGlob { pattern, message }
                }
                reason => FileActivityError::Io(std::io::Error::other(format!(
                    "target resolution failed: {reason:?}"
                ))),
            })?;
        resolved.scanned_entries += materialized.scanned_entries;
        let target_unresolved = !materialized.unresolved.is_empty();
        resolved
            .files
            .extend(materialized.paths.into_iter().filter(|path| path.is_file()));
        if target_unresolved && !resolved.unresolved_targets.contains(target) {
            resolved.unresolved_targets.push(target.clone());
        }
        if materialized.budget_exhausted {
            resolved.truncated = true;
            if !resolved.unresolved_targets.contains(target) {
                resolved.unresolved_targets.push(target.clone());
            }
            break;
        }
    }
    Ok(resolved)
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

fn default_ignored_directory_names() -> BTreeSet<String> {
    [".context", ".git", ".hg", ".svn", "node_modules", "target"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn utf8_path(path: &Path) -> Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(path.to_path_buf())
        .map_err(FileActivityError::NonUtf8Path)
        .map(normalize_utf8_path)
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
        let report = analyze_activity(&codex_post_tool(
            "apply_patch",
            serde_json::json!({
                "patch": "*** Update File: src/lib.rs\n*** Add File: src/new.rs"
            }),
        ));

        let evidence = report.evidence().collect::<Vec<_>>();
        assert_eq!(evidence.len(), 2);
        assert!(evidence.iter().all(|item| {
            item.source == FileActivitySource::Patch && item.certainty == ActivityCertainty::Direct
        }));
        assert!(evidence.iter().any(|item| {
            item.target == FileActivityTarget::exact(Utf8PathBuf::from("/repo/src/lib.rs"))
        }));
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

    #[test]
    fn exact_deleted_target_is_not_applicable_while_scope_is_retained() {
        let project = temporary_directory("resolve-deleted");
        let missing = Utf8PathBuf::from_path_buf(project.join("deleted.rs")).unwrap();
        let scope = FileActivityTarget::Path {
            path: Utf8PathBuf::from_path_buf(project.join("missing-scope")).unwrap(),
            scope: FileActivityScope::Descendants,
        };
        let mut activity = PendingFileActivity::empty();
        activity.apply(&FileActivityEvent::Evidence(FileActivityEvidence {
            target: FileActivityTarget::exact(missing.clone()),
            effect: FileActivityEffect::Delete,
            source: FileActivitySource::StructuredToolInput,
            certainty: ActivityCertainty::Direct,
            observed_at: UtcTimestamp::now(),
            event: None,
            tool_call_id: None,
            turn_id: None,
            detail: None,
        }));
        activity.apply(&FileActivityEvent::Evidence(FileActivityEvidence {
            target: scope.clone(),
            effect: FileActivityEffect::MaybeWrite,
            source: FileActivitySource::ShellInference,
            certainty: ActivityCertainty::Heuristic,
            observed_at: UtcTimestamp::now(),
            event: None,
            tool_call_id: None,
            turn_id: None,
            detail: None,
        }));

        let resolved = resolve_files(
            &activity,
            &ResolveOptions::new(vec![Utf8PathBuf::from_path_buf(project.clone()).unwrap()]),
        )
        .unwrap();
        assert!(resolved.not_applicable_files.contains(&missing));
        assert!(resolved.unresolved_targets.contains(&scope));
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
        let report = reconcile(
            &store,
            ReconciliationOptions::new(
                vec![Utf8PathBuf::from_path_buf(project.clone()).unwrap()],
                second_through,
            ),
        )
        .unwrap();
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
