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
        }
    }
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
            FileActivityEvent::Gap(_) => None,
        })
    }

    /// Iterates over gap events.
    pub fn gaps(&self) -> impl Iterator<Item = &FileActivityGap> {
        self.events.iter().filter_map(|event| match event {
            FileActivityEvent::Evidence(_) => None,
            FileActivityEvent::Gap(gap) => Some(gap),
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
            EntityId::new(PENDING_ACTIVITY_ENTITY, 1)?,
            EntityMode::Windowed,
        )?;
        let cursor = scope.entity::<ReconciliationCursor>(
            EntityId::new(RECONCILIATION_CURSOR_ENTITY, 1)?,
            EntityMode::Monotonic,
        )?;
        Ok(Self {
            state,
            pending,
            cursor,
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
    let mut events = Vec::new();

    if options.filesystem_mtime {
        match since {
            Some(since) => collect_mtime_activity(
                &options,
                since,
                cursor.is_none(),
                &metadata,
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
        collect_git_dirty_activity(&options.roots, &metadata, &mut events, &mut report)?;
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
    metadata: &ObservationMetadata,
    events: &mut Vec<FileActivityEvent>,
    report: &mut ReconciliationReport,
) -> Result<()> {
    let mut seen = BTreeSet::new();
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
                    report,
                    FileActivitySource::VcsDirty,
                    format!("git status failed in {} with {}", root, output.status),
                );
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                push_reconciliation_gap(
                    report,
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
            append_git_path(root, path, metadata, events, report, &mut seen)?;
            if status.iter().any(|byte| matches!(*byte, b'R' | b'C')) && index < fields.len() {
                append_git_path(root, fields[index], metadata, events, report, &mut seen)?;
                index += 1;
            }
        }
    }
    Ok(())
}

fn append_git_path(
    root: &Utf8Path,
    raw: &[u8],
    metadata: &ObservationMetadata,
    events: &mut Vec<FileActivityEvent>,
    report: &mut ReconciliationReport,
    seen: &mut BTreeSet<Utf8PathBuf>,
) -> Result<()> {
    let relative = match std::str::from_utf8(raw) {
        Ok(relative) => relative,
        Err(_) => {
            push_reconciliation_gap(
                report,
                FileActivitySource::VcsDirty,
                "git status reported a non-UTF-8 path",
            );
            return Ok(());
        }
    };
    let path = normalize_utf8_path(root.join(relative));
    if seen.insert(path.clone()) {
        events.push(FileActivityEvent::Evidence(metadata.evidence(
            FileActivityTarget::exact(path),
            FileActivityEffect::MaybeWrite,
            FileActivitySource::VcsDirty,
            ActivityCertainty::Heuristic,
            Some("git status dirty path; may predate this session".to_owned()),
        )));
        report.vcs_files += 1;
    }
    Ok(())
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
        resolved
            .files
            .extend(materialized.paths.into_iter().filter(|path| path.is_file()));
        if materialized.budget_exhausted {
            resolved.truncated = true;
            resolved.unresolved_targets.push(target.clone());
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
