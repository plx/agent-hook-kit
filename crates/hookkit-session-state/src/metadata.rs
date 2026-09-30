use crate::storage::{
    Durability, FileLock, IoContext, atomic_replace, create_private_dir_all, decode,
    publish_if_absent, read_optional, remove_if_present, sha256, sha256_bytes,
};
use crate::{Result, SessionIdentity, StateError, json_files};
use hookkit_core::{HarnessId, RuntimeContext, SessionBoundaryKind, Utf8PathBuf};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const SCHEMA_VERSION: u32 = 1;
const BOUNDARY_COALESCE_WINDOW: Duration = Duration::from_secs(30);

/// Domain separator for the identity of a project observation.
const PROJECT_KEY_DOMAIN: &[u8] = b"hookkit.project-observation.v1\0";

/// UTC instant serialized as an RFC 3339 string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UtcTimestamp(i128);

impl UtcTimestamp {
    /// Captures the current system clock time.
    pub fn now() -> Self {
        Self::from_system_time(SystemTime::now())
    }

    /// Converts a [`SystemTime`] without losing subsecond precision.
    ///
    /// Any `SystemTime` is accepted, but only instants between the years
    /// -9999 and 9999 can be serialized; serializing one outside that range
    /// returns an error.
    pub fn from_system_time(value: SystemTime) -> Self {
        match value.duration_since(UNIX_EPOCH) {
            Ok(duration) => Self(duration.as_nanos() as i128),
            Err(error) => Self(-(error.duration().as_nanos() as i128)),
        }
    }

    /// Converts this timestamp back to [`SystemTime`].
    pub fn as_system_time(self) -> SystemTime {
        if self.0 >= 0 {
            UNIX_EPOCH + nanos_duration(self.0 as u128)
        } else {
            UNIX_EPOCH - nanos_duration(self.0.unsigned_abs())
        }
    }

    /// Returns whole milliseconds relative to the Unix epoch, truncating any
    /// fractional millisecond toward zero.
    pub fn unix_milliseconds(self) -> i128 {
        self.0 / 1_000_000
    }

    /// Parses an RFC 3339 timestamp, returning `None` for invalid or
    /// out-of-range input.
    pub fn parse_rfc3339(value: &str) -> Option<Self> {
        OffsetDateTime::parse(value, &Rfc3339)
            .ok()
            .map(|value| Self(value.unix_timestamp_nanos()))
    }

    fn to_rfc3339(self) -> std::result::Result<String, String> {
        OffsetDateTime::from_unix_timestamp_nanos(self.0)
            .map_err(|error| {
                format!(
                    "timestamp {} ns from the Unix epoch cannot be represented in RFC 3339: {error}",
                    self.0
                )
            })?
            .format(&Rfc3339)
            .map_err(|error| error.to_string())
    }

    fn close_to(self, other: Self, window: Duration) -> bool {
        self.0.abs_diff(other.0) <= window.as_nanos()
    }

    /// Returns the smallest representable instant after `self`.
    fn successor(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl Serialize for UtcTimestamp {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_rfc3339().map_err(serde::ser::Error::custom)?)
    }
}

impl<'de> Deserialize<'de> for UtcTimestamp {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse_rfc3339(&value)
            .ok_or_else(|| de::Error::custom(format!("invalid RFC 3339 timestamp `{value}`")))
    }
}

fn nanos_duration(value: u128) -> Duration {
    let seconds = (value / 1_000_000_000).min(u64::MAX as u128) as u64;
    let nanos = (value % 1_000_000_000) as u32;
    Duration::new(seconds, nanos)
}

/// Origin of a timestamp retained in automatic session metadata.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TimestampProvenance {
    /// Timestamp supplied explicitly by a native lifecycle event.
    NativeEventTimestamp,
    /// Local observation time of a native lifecycle hook.
    LifecycleHookObservation,
    /// Local observation time of an inferred invocation-number boundary.
    InferredInvocationBoundary,
    /// Fallback time when the session was first observed by any hook.
    FirstHookObservation,
    /// Provenance recorded by a newer HookKit build that this build does not
    /// recognize.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
/// Timestamp paired with evidence describing how it was obtained.
pub struct CapturedTimestamp {
    /// Captured UTC instant.
    pub at: UtcTimestamp,
    /// Source of the instant.
    pub provenance: TimestampProvenance,
}

/// Cause of a versioned native session epoch.
///
/// Stored epochs are shared by every HookKit build that opens the session, so
/// a kind recorded by a newer build decodes as [`SessionEpochKind::Unknown`]
/// instead of failing the read.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SessionEpochKind {
    /// A newly started native session.
    Startup,
    /// A previously persisted native session resumed.
    Resume,
    /// A new native session forked from an existing session.
    Fork,
    /// The conversation context was cleared.
    Clear,
    /// The conversation context was compacted.
    Compact,
    /// The first invocation in an invocation-numbered harness session.
    InvocationStart,
    /// No explicit lifecycle boundary existed; this is the first observation.
    FirstObservedFallback,
    /// An observed lifecycle boundary that this build does not recognize,
    /// either recorded by a newer HookKit build or reported by a newer
    /// harness.
    #[serde(other)]
    Unknown,
}

impl From<SessionBoundaryKind> for SessionEpochKind {
    fn from(value: SessionBoundaryKind) -> Self {
        match value {
            SessionBoundaryKind::Startup => Self::Startup,
            SessionBoundaryKind::Resume => Self::Resume,
            SessionBoundaryKind::Fork => Self::Fork,
            SessionBoundaryKind::Clear => Self::Clear,
            SessionBoundaryKind::Compact => Self::Compact,
            SessionBoundaryKind::InvocationStart => Self::InvocationStart,
            _ => Self::Unknown,
        }
    }
}

impl SessionEpochKind {
    /// Reports whether this boundary starts the conversation stored under the
    /// session's identity. A fork receives a new native identity, so its
    /// epoch is the first one in that identity's directory.
    fn starts_conversation(self) -> bool {
        matches!(self, Self::Startup | Self::Fork)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
/// Timing metadata for the stable conversation identity.
pub struct ConversationMetadata {
    /// Best available conversation start time and its provenance.
    ///
    /// This is the earliest observed `startup` or `fork` epoch, falling back
    /// to the first hook observation.
    pub started_at: CapturedTimestamp,
    /// First time any hookkit process observed the conversation.
    pub first_observed_at: UtcTimestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
/// Metadata for the current lifecycle epoch within a conversation.
pub struct SessionEpochMetadata {
    /// Content-derived epoch identifier.
    pub id: String,
    /// Lifecycle event that began the epoch.
    pub kind: SessionEpochKind,
    /// Best available epoch start time and its provenance.
    pub started_at: CapturedTimestamp,
    /// First local observation associated with this epoch.
    pub first_observed_at: UtcTimestamp,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
/// Project context aggregated from native hook input.
pub struct ProjectMetadata {
    /// Deduplicated union of every workspace root observed across all
    /// observations (sorted), not just the most recent observation.
    pub workspace_roots: Vec<Utf8PathBuf>,
    /// Most recently observed native transcript path, when any was supplied.
    pub transcript_path: Option<Utf8PathBuf>,
    /// Most recently observed native artifact directory, when any was
    /// supplied.
    pub artifact_directory: Option<Utf8PathBuf>,
}

/// Library-owned metadata that is materialized whenever session state is
/// ensured. Client families cannot overwrite its source observations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct SessionMetadata {
    /// On-disk metadata schema version.
    pub schema_version: u32,
    /// Stable harness identity string; see [`Self::harness_id`].
    pub harness: String,
    /// Identity namespace (`session` or `conversation`), matching
    /// [`SessionIdentity::kind`].
    pub identity_kind: String,
    /// SHA-256 digest of the harness identifier, identity namespace, and opaque
    /// native identity value (joined by NUL bytes); this is also the on-disk
    /// session directory key.
    pub identity_hash: String,
    /// Conversation-wide timing metadata.
    pub conversation: ConversationMetadata,
    /// Current lifecycle epoch.
    pub current_session: SessionEpochMetadata,
    /// Latest observed project context.
    pub project: ProjectMetadata,
}

impl SessionMetadata {
    /// Returns the typed harness identity, or `None` when the stored string
    /// is not a valid harness identifier.
    pub fn harness_id(&self) -> Option<HarnessId> {
        HarnessId::new(self.harness.clone()).ok()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnchorObservation {
    observed_at: UtcTimestamp,
    harness: String,
    identity_kind: String,
    identity_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StartObservation {
    epoch: SessionEpochMetadata,
    occurrence_hash: Option<String>,
}

/// One distinct project context, stored at `workspaces/<key>.json`.
///
/// Earlier builds named each observation by the hash of its full content,
/// including a fresh timestamp, and so wrote one file per hook invocation.
/// Those files use the same fields; [`load_project_observations`] folds them
/// into keyed files.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectObservation {
    /// First time this exact project context was observed.
    observed_at: UtcTimestamp,
    /// Latest time this context changed the aggregated project view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_observed_at: Option<UtcTimestamp>,
    workspace_roots: Vec<Utf8PathBuf>,
    transcript_path: Option<Utf8PathBuf>,
    artifact_directory: Option<Utf8PathBuf>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectKey<'a> {
    workspace_roots: &'a [Utf8PathBuf],
    transcript_path: &'a Option<Utf8PathBuf>,
    artifact_directory: &'a Option<Utf8PathBuf>,
}

impl ProjectObservation {
    /// Time used to order observations when choosing the latest paths.
    fn effective_at(&self) -> UtcTimestamp {
        self.last_observed_at
            .map_or(self.observed_at, |last| last.max(self.observed_at))
    }

    /// Content identity that ignores observation times.
    fn key(&self) -> Result<String> {
        let mut roots = self.workspace_roots.clone();
        roots.sort();
        roots.dedup();
        let key = serde_json::to_vec(&ProjectKey {
            workspace_roots: &roots,
            transcript_path: &self.transcript_path,
            artifact_directory: &self.artifact_directory,
        })?;
        Ok(sha256_bytes(&[PROJECT_KEY_DOMAIN, &key]))
    }

    fn merge(&mut self, other: &Self) {
        let effective = self.effective_at().max(other.effective_at());
        self.observed_at = self.observed_at.min(other.observed_at);
        self.last_observed_at = (effective > self.observed_at).then_some(effective);
    }
}

/// Path of the session's metadata lock, which [`crate::SessionState::gc`]
/// also takes before collecting a session.
pub(crate) fn lock_path(directory: &Path) -> PathBuf {
    directory.join(METADATA_LOCK)
}

const METADATA_LOCK: &str = "_hookkit/metadata/v1/metadata.lock";

/// How many times an opener starts over after garbage collection moved the
/// session away while the opener waited for its lock.
const MOVED_SESSION_RETRIES: usize = 8;

/// Materializes the session's metadata and refreshes its activity stamp
/// under the exclusive metadata lock.
///
/// Stamping under the lock is what lets garbage collection, which takes the
/// same lock before checking a session's age, never remove a session that
/// an opener has just stamped.
pub(crate) fn ensure_metadata(
    directory: &Path,
    harness: &HarnessId,
    identity: &SessionIdentity,
    context: Option<&RuntimeContext<'_>>,
) -> Result<SessionMetadata> {
    let internal = directory.join("_hookkit/metadata/v1");
    let lock_path = lock_path(directory);
    let mut attempts = 0;
    let lock = loop {
        create_private_dir_all(&internal)?;
        let lock = FileLock::exclusive(&lock_path)?;
        if lock.is_current(&lock_path)? {
            break lock;
        }
        // Garbage collection moved this session into its trash while we
        // waited; start over in a fresh session directory.
        attempts += 1;
        if attempts == MOVED_SESSION_RETRIES {
            return Err(StateError::Io {
                operation: "acquire lock",
                path: lock_path,
                source: std::io::Error::other(
                    "the session directory kept moving while waiting for its lock",
                ),
            });
        }
    };
    let result = ensure_metadata_locked(directory, &internal, harness, identity, context)
        .and_then(|metadata| crate::record_session_activity(directory).map(|()| metadata));
    let unlock = lock.release(&lock_path);
    match (result, unlock) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

pub(crate) fn read_metadata(directory: &Path) -> Result<SessionMetadata> {
    let lock_path = lock_path(directory);
    let lock = FileLock::shared(&lock_path)?;
    let path = metadata_path(directory);
    let result = std::fs::read(&path)
        .at("read session metadata", &path)
        .and_then(|bytes| decode(&path, &bytes));
    let unlock = lock.release(&lock_path);
    match (result, unlock) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}

fn ensure_metadata_locked(
    directory: &Path,
    internal: &Path,
    harness: &HarnessId,
    identity: &SessionIdentity,
    context: Option<&RuntimeContext<'_>>,
) -> Result<SessionMetadata> {
    let observed_at = UtcTimestamp::now();
    let identity_hash = sha256(&format!(
        "{}\0{}\0{}",
        harness.as_str(),
        identity.kind(),
        identity.value()
    ));
    let anchor_path = internal.join("anchor.json");
    publish_if_absent(
        &anchor_path,
        &serde_json::to_vec_pretty(&AnchorObservation {
            observed_at,
            harness: harness.as_str().to_string(),
            identity_kind: identity.kind().to_string(),
            identity_hash: identity_hash.clone(),
        })?,
    )?;
    let anchor: AnchorObservation = decode(
        &anchor_path,
        &std::fs::read(&anchor_path).at("read metadata anchor", &anchor_path)?,
    )?;

    let workspaces = internal.join("workspaces");
    let mut projects = load_project_observations(&workspaces)?;
    if let Some(context) = context {
        record_project_observation(&workspaces, &mut projects, context, observed_at)?;

        if let Some(boundary) = context.session_boundary() {
            let existing = read_start_observations(&internal.join("starts"))?;
            let kind = SessionEpochKind::from(boundary.kind);
            let occurrence_hash = boundary.occurrence_key.as_deref().map(sha256);
            let duplicate = existing.iter().any(|observation| {
                occurrence_hash.is_some() && observation.occurrence_hash == occurrence_hash
            }) || (occurrence_hash.is_none()
                && existing.last().is_some_and(|observation| {
                    observation.epoch.kind == kind
                        && observation
                            .epoch
                            .first_observed_at
                            .close_to(observed_at, BOUNDARY_COALESCE_WINDOW)
                }));
            if !duplicate {
                let native_timestamp = boundary
                    .native_timestamp
                    .as_deref()
                    .and_then(UtcTimestamp::parse_rfc3339);
                let provenance = if native_timestamp.is_some() {
                    TimestampProvenance::NativeEventTimestamp
                } else if kind == SessionEpochKind::InvocationStart {
                    TimestampProvenance::InferredInvocationBoundary
                } else {
                    TimestampProvenance::LifecycleHookObservation
                };
                let started_at = CapturedTimestamp {
                    at: native_timestamp.unwrap_or(observed_at),
                    provenance,
                };
                let epoch_id = occurrence_hash.clone().unwrap_or_else(|| {
                    sha256(&format!(
                        "{:?}\0{}\0{}",
                        kind,
                        observed_at.unix_milliseconds(),
                        std::process::id()
                    ))
                });
                write_content_addressed(
                    &internal.join("starts"),
                    &StartObservation {
                        epoch: SessionEpochMetadata {
                            id: epoch_id,
                            kind,
                            started_at,
                            first_observed_at: observed_at,
                        },
                        occurrence_hash,
                    },
                )?;
            }
        }
    }

    let starts = read_start_observations(&internal.join("starts"))?;
    let fallback = SessionEpochMetadata {
        id: sha256(&format!(
            "fallback\0{}",
            anchor.observed_at.unix_milliseconds()
        )),
        kind: SessionEpochKind::FirstObservedFallback,
        started_at: CapturedTimestamp {
            at: anchor.observed_at,
            provenance: TimestampProvenance::FirstHookObservation,
        },
        first_observed_at: anchor.observed_at,
    };
    let current_session = starts
        .iter()
        .max_by_key(|observation| observation.epoch.first_observed_at)
        .map(|observation| observation.epoch.clone())
        .unwrap_or_else(|| fallback.clone());
    let startup = starts
        .iter()
        .filter(|observation| observation.epoch.kind.starts_conversation())
        .min_by_key(|observation| observation.epoch.started_at.at)
        .map(|observation| observation.epoch.started_at.clone());
    let conversation = ConversationMetadata {
        started_at: startup.unwrap_or_else(|| fallback.started_at.clone()),
        first_observed_at: anchor.observed_at,
    };
    let metadata = SessionMetadata {
        schema_version: SCHEMA_VERSION,
        harness: anchor.harness,
        identity_kind: anchor.identity_kind,
        identity_hash: anchor.identity_hash,
        conversation,
        current_session,
        project: aggregate_project(projects.values()),
    };
    // metadata.json is a derived view that every ensure rebuilds, so it is
    // rewritten only when it changes and is not forced to stable storage.
    let bytes = serde_json::to_vec_pretty(&metadata)?;
    let path = metadata_path(directory);
    if read_optional(&path)?.as_deref() != Some(bytes.as_slice()) {
        atomic_replace(&path, &bytes, Durability::Disposable)?;
    }
    Ok(metadata)
}

/// Loads distinct project observations keyed by [`ProjectObservation::key`].
///
/// Observations written by earlier builds under content-hash names are
/// folded into the keyed file for the same context and then removed. Every
/// writer of this directory holds the exclusive metadata lock, including
/// earlier builds, so no reader or writer can observe a half-folded state.
/// Undecodable files are ignored and left in place.
fn load_project_observations(directory: &Path) -> Result<BTreeMap<String, ProjectObservation>> {
    let mut keyed = BTreeMap::<String, ProjectObservation>::new();
    let mut legacy = Vec::new();
    let mut folded = BTreeSet::new();
    for path in json_files(directory)? {
        let Some(bytes) = read_optional(&path)? else {
            continue;
        };
        let Ok(observation) = serde_json::from_slice::<ProjectObservation>(&bytes) else {
            continue;
        };
        let key = observation.key()?;
        if path.file_stem().and_then(|stem| stem.to_str()) != Some(key.as_str()) {
            legacy.push(path);
            folded.insert(key.clone());
        }
        match keyed.get_mut(&key) {
            Some(existing) => existing.merge(&observation),
            None => {
                keyed.insert(key, observation);
            }
        }
    }
    for key in &folded {
        atomic_replace(
            &project_observation_path(directory, key),
            &serde_json::to_vec_pretty(&keyed[key])?,
            Durability::Durable,
        )?;
    }
    for path in legacy {
        remove_if_present(&path)?;
    }
    Ok(keyed)
}

/// Records the invocation's project context without writing when it adds no
/// information.
///
/// A new context gets its own file. A known context is rewritten, with a new
/// `lastObservedAt`, only when seeing it again changes the aggregated view,
/// for example when it restores an earlier transcript path. Skipping the
/// write otherwise cannot change any later aggregate: the latest value of
/// each field is unaffected by moving this context later in time when it
/// already agrees with every later observation.
fn record_project_observation(
    directory: &Path,
    projects: &mut BTreeMap<String, ProjectObservation>,
    context: &RuntimeContext<'_>,
    now: UtcTimestamp,
) -> Result<()> {
    let mut workspace_roots = context.workspace_roots().to_vec();
    workspace_roots.sort();
    workspace_roots.dedup();
    let current = ProjectObservation {
        observed_at: now,
        last_observed_at: None,
        workspace_roots,
        transcript_path: context.transcript_path().map(ToOwned::to_owned),
        artifact_directory: context.artifact_directory().map(ToOwned::to_owned),
    };
    let key = current.key()?;
    // Order this observation after every stored one even if the clock stepped
    // backwards since they were written.
    let at = projects
        .values()
        .map(ProjectObservation::effective_at)
        .max()
        .map_or(now, |latest| now.max(latest.successor()));
    let candidate = match projects.get(&key) {
        Some(existing) => ProjectObservation {
            last_observed_at: Some(at),
            ..existing.clone()
        },
        None => ProjectObservation {
            observed_at: at,
            ..current
        },
    };
    if projects.contains_key(&key) {
        let before = aggregate_project(projects.values());
        let after = aggregate_project(
            projects
                .iter()
                .filter(|(stored, _)| **stored != key)
                .map(|(_, observation)| observation)
                .chain(std::iter::once(&candidate)),
        );
        if before == after {
            return Ok(());
        }
    }
    atomic_replace(
        &project_observation_path(directory, &key),
        &serde_json::to_vec_pretty(&candidate)?,
        Durability::Durable,
    )?;
    projects.insert(key, candidate);
    Ok(())
}

fn project_observation_path(directory: &Path, key: &str) -> PathBuf {
    directory.join(format!("{key}.json"))
}

fn aggregate_project<'a>(
    observations: impl Iterator<Item = &'a ProjectObservation>,
) -> ProjectMetadata {
    let mut observations = observations.collect::<Vec<_>>();
    observations.sort_by_key(|observation| observation.effective_at());
    let mut roots = BTreeSet::new();
    let mut transcript_path = None;
    let mut artifact_directory = None;
    for observation in observations {
        roots.extend(observation.workspace_roots.iter().cloned());
        if observation.transcript_path.is_some() {
            transcript_path.clone_from(&observation.transcript_path);
        }
        if observation.artifact_directory.is_some() {
            artifact_directory.clone_from(&observation.artifact_directory);
        }
    }
    ProjectMetadata {
        workspace_roots: roots.into_iter().collect(),
        transcript_path,
        artifact_directory,
    }
}

/// Reads start observations in observation order.
///
/// A start written by a build with an incompatible schema is skipped rather
/// than failing every later hook in the session.
fn read_start_observations(directory: &Path) -> Result<Vec<StartObservation>> {
    let mut observations = Vec::new();
    for path in json_files(directory)? {
        let Some(bytes) = read_optional(&path)? else {
            continue;
        };
        if let Ok(observation) = serde_json::from_slice::<StartObservation>(&bytes) {
            observations.push(observation);
        }
    }
    observations.sort_by_key(|observation| observation.epoch.first_observed_at);
    Ok(observations)
}

fn write_content_addressed<T: Serialize>(directory: &Path, value: &T) -> Result<PathBuf> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let path = directory.join(format!("{}.json", sha256_bytes(&[&bytes])));
    publish_if_absent(&path, &bytes)?;
    Ok(path)
}

pub(crate) fn metadata_path(directory: &Path) -> PathBuf {
    directory.join("metadata.json")
}
