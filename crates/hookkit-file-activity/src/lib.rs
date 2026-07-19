//! Loss-aware file activity observation and reconciliation.
//!
//! Tool inspection is evidence, not an audit log. This crate retains both the
//! candidate target and the limits of the inference, then provides timestamp
//! and opt-in VCS fallbacks for deferred consumers.

use globset::Glob;
use hookkit_common::PostToolUseInput;
use hookkit_core::{RuntimeContext, Utf8Path, Utf8PathBuf, normalize_utf8_path};
use hookkit_session_state::{
    EntityId, EntityJournal, EntityMode, EntityOutcome, FamilyId, JournalEntity, SessionState,
    StateRoot, UtcTimestamp,
};
use hookkit_tool_access::{
    AccessCandidate, AccessCertainty, AccessIntent, AccessScope, AccessSource, AccessTarget,
    ToolAccessAnalyzer,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};
use walkdir::{DirEntry, WalkDir};

pub const FILE_ACTIVITY_FAMILY: &str = "agent-hook-kit.file-activity";
pub const PENDING_ACTIVITY_ENTITY: &str = "pending-files";
pub const RECONCILIATION_CURSOR_ENTITY: &str = "reconciliation-cursor";

#[derive(Debug, thiserror::Error)]
pub enum FileActivityError {
    #[error(transparent)]
    State(#[from] hookkit_session_state::StateError),
    #[error("file activity I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("file activity JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("file activity path is not valid UTF-8: {0}")]
    NonUtf8Path(PathBuf),
    #[error("invalid file activity glob `{pattern}`: {message}")]
    InvalidGlob { pattern: String, message: String },
}

pub type Result<T> = std::result::Result<T, FileActivityError>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum FileActivityEffect {
    CreateOrModify,
    Delete,
    MoveSource,
    MoveDestination,
    MaybeWrite,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum FileActivitySource {
    StructuredToolInput,
    Patch,
    ShellInference,
    FilesystemMtime,
    VcsDirty,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ActivityCertainty {
    Direct,
    Conditional,
    Heuristic,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum FileActivityScope {
    Exact,
    Descendants,
    ExactOrDescendants,
    Glob,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum FileActivityTarget {
    Path {
        path: Utf8PathBuf,
        scope: FileActivityScope,
    },
    Workspace {
        root: Option<Utf8PathBuf>,
    },
}

impl FileActivityTarget {
    pub fn exact(path: Utf8PathBuf) -> Self {
        Self::Path {
            path,
            scope: FileActivityScope::Exact,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileActivityEvidence {
    pub target: FileActivityTarget,
    pub effect: FileActivityEffect,
    pub source: FileActivitySource,
    pub certainty: ActivityCertainty,
    pub observed_at: UtcTimestamp,
    pub event: Option<String>,
    pub tool_call_id: Option<String>,
    pub turn_id: Option<String>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileActivityGap {
    pub source: FileActivitySource,
    pub observed_at: UtcTimestamp,
    pub event: Option<String>,
    pub tool_call_id: Option<String>,
    pub turn_id: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum FileActivityEvent {
    Evidence(FileActivityEvidence),
    Gap(FileActivityGap),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PendingFileActivity {
    targets: BTreeSet<FileActivityTarget>,
    evidence_count: usize,
    gap_count: usize,
}

impl PendingFileActivity {
    pub fn targets(&self) -> &BTreeSet<FileActivityTarget> {
        &self.targets
    }

    pub fn evidence_count(&self) -> usize {
        self.evidence_count
    }

    pub fn gap_count(&self) -> usize {
        self.gap_count
    }

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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationCursor {
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActivityReport {
    pub events: Vec<FileActivityEvent>,
}

impl ActivityReport {
    pub fn evidence(&self) -> impl Iterator<Item = &FileActivityEvidence> {
        self.events.iter().filter_map(|event| match event {
            FileActivityEvent::Evidence(evidence) => Some(evidence),
            FileActivityEvent::Gap(_) => None,
        })
    }

    pub fn gaps(&self) -> impl Iterator<Item = &FileActivityGap> {
        self.events.iter().filter_map(|event| match event {
            FileActivityEvent::Evidence(_) => None,
            FileActivityEvent::Gap(gap) => Some(gap),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct FileActivityStore {
    state: SessionState,
    pending: EntityJournal<PendingFileActivity>,
    cursor: EntityJournal<ReconciliationCursor>,
}

impl FileActivityStore {
    pub fn ensure(context: &RuntimeContext<'_>, root: StateRoot) -> Result<Self> {
        Self::from_state(SessionState::ensure(context, root)?)
    }

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

    pub fn state(&self) -> &SessionState {
        &self.state
    }

    pub fn pending(&self) -> &EntityJournal<PendingFileActivity> {
        &self.pending
    }

    pub fn append_report(&self, event_key_prefix: &str, report: &ActivityReport) -> Result<()> {
        for (index, event) in report.events.iter().enumerate() {
            self.pending
                .append(&format!("{event_key_prefix}\0{index}"), event)?;
        }
        Ok(())
    }

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VcsFallback {
    #[default]
    Disabled,
    /// Broad fallback. Includes changes that may predate the agent session.
    GitDirty,
}

#[derive(Debug, Clone)]
pub struct ReconciliationOptions {
    pub roots: Vec<Utf8PathBuf>,
    pub fallback_since: Option<UtcTimestamp>,
    pub through: UtcTimestamp,
    pub timestamp_tolerance: Duration,
    pub filesystem_mtime: bool,
    pub vcs: VcsFallback,
    pub max_entries: usize,
    pub ignored_directory_names: BTreeSet<String>,
    pub excluded_roots: BTreeSet<Utf8PathBuf>,
}

impl ReconciliationOptions {
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconciliationReport {
    pub since: Option<UtcTimestamp>,
    pub through: Option<UtcTimestamp>,
    pub filesystem_files: usize,
    pub vcs_files: usize,
    pub scanned_entries: usize,
    pub truncated: bool,
    pub gaps: Vec<ReconciliationGap>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciliationGap {
    pub source: FileActivitySource,
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

#[derive(Debug, Clone)]
pub struct ResolveOptions {
    pub roots: Vec<Utf8PathBuf>,
    pub max_entries: usize,
    pub ignored_directory_names: BTreeSet<String>,
    pub excluded_roots: BTreeSet<Utf8PathBuf>,
}

impl ResolveOptions {
    pub fn new(roots: Vec<Utf8PathBuf>) -> Self {
        Self {
            roots,
            max_entries: 100_000,
            ignored_directory_names: default_ignored_directory_names(),
            excluded_roots: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedFileActivity {
    pub files: BTreeSet<Utf8PathBuf>,
    pub unresolved_targets: Vec<FileActivityTarget>,
    pub scanned_entries: usize,
    pub truncated: bool,
}

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
            } => {
                if path.is_file() {
                    resolved.files.insert(path.clone());
                }
            }
            FileActivityTarget::Path {
                path,
                scope: FileActivityScope::Descendants,
            } => walk_matching(path, None, options, &mut resolved)?,
            FileActivityTarget::Path {
                path,
                scope: FileActivityScope::ExactOrDescendants,
            } => {
                if path.is_file() {
                    resolved.files.insert(path.clone());
                } else {
                    walk_matching(path, None, options, &mut resolved)?;
                }
            }
            FileActivityTarget::Path {
                path,
                scope: FileActivityScope::Glob,
            } => {
                let pattern = path.as_str();
                let matcher =
                    Glob::new(pattern).map_err(|error| FileActivityError::InvalidGlob {
                        pattern: pattern.to_owned(),
                        message: error.to_string(),
                    })?;
                let matcher = matcher.compile_matcher();
                for root in &options.roots {
                    walk_matching(root, Some(&matcher), options, &mut resolved)?;
                    if resolved.truncated {
                        break;
                    }
                }
            }
            FileActivityTarget::Workspace { root } => {
                if let Some(root) = root {
                    walk_matching(root, None, options, &mut resolved)?;
                } else {
                    for root in &options.roots {
                        walk_matching(root, None, options, &mut resolved)?;
                        if resolved.truncated {
                            break;
                        }
                    }
                }
            }
        }
        if resolved.truncated {
            resolved.unresolved_targets.push(target.clone());
            break;
        }
    }
    Ok(resolved)
}

fn walk_matching(
    root: &Utf8Path,
    matcher: Option<&globset::GlobMatcher>,
    options: &ResolveOptions,
    resolved: &mut ResolvedFileActivity,
) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
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
        if resolved.scanned_entries >= options.max_entries {
            resolved.truncated = true;
            break;
        }
        resolved.scanned_entries += 1;
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(path) = utf8_path(entry.path()) else {
            continue;
        };
        if matcher.is_none_or(|matcher| matcher.is_match(path.as_std_path())) {
            resolved.files.insert(path);
        }
    }
    Ok(())
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
