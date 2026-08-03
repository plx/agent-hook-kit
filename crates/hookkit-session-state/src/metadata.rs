use crate::{
    Result, SessionIdentity, StateError, atomic_replace, atomic_write, create_private_dir_all,
    sha256, sha256_bytes,
};
use fs2::FileExt;
use hookkit_core::{HarnessId, RuntimeContext, SessionBoundaryKind, Utf8PathBuf};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const SCHEMA_VERSION: u32 = 1;
const BOUNDARY_COALESCE_WINDOW: Duration = Duration::from_secs(30);

/// UTC instant serialized as an RFC 3339 string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UtcTimestamp(i128);

impl UtcTimestamp {
    /// Captures the current system clock time.
    pub fn now() -> Self {
        Self::from_system_time(SystemTime::now())
    }

    /// Converts a [`SystemTime`] without losing subsecond precision.
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

    fn to_rfc3339(self) -> std::result::Result<String, time::error::Format> {
        OffsetDateTime::from_unix_timestamp_nanos(self.0)
            .expect("SystemTime-compatible timestamp is in the supported range")
            .format(&Rfc3339)
    }

    fn close_to(self, other: Self, window: Duration) -> bool {
        self.0.abs_diff(other.0) <= window.as_nanos()
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
pub enum TimestampProvenance {
    /// Timestamp supplied explicitly by a native lifecycle event.
    NativeEventTimestamp,
    /// Local observation time of a native lifecycle hook.
    LifecycleHookObservation,
    /// Local observation time of an inferred invocation-number boundary.
    InferredInvocationBoundary,
    /// Fallback time when the session was first observed by any hook.
    FirstHookObservation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
/// Timestamp paired with evidence describing how it was obtained.
pub struct CapturedTimestamp {
    /// Captured UTC instant.
    pub at: UtcTimestamp,
    /// Source of the instant.
    pub provenance: TimestampProvenance,
}

/// Cause of a versioned native session epoch.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionEpochKind {
    /// A newly started native session.
    Startup,
    /// A previously persisted native session resumed.
    Resume,
    /// The conversation context was cleared.
    Clear,
    /// The conversation context was compacted.
    Compact,
    /// The first invocation in an invocation-numbered harness session.
    InvocationStart,
    /// No explicit lifecycle boundary existed; this is the first observation.
    FirstObservedFallback,
}

impl From<SessionBoundaryKind> for SessionEpochKind {
    fn from(value: SessionBoundaryKind) -> Self {
        match value {
            SessionBoundaryKind::Startup => Self::Startup,
            SessionBoundaryKind::Resume => Self::Resume,
            SessionBoundaryKind::Clear => Self::Clear,
            SessionBoundaryKind::Compact => Self::Compact,
            SessionBoundaryKind::InvocationStart => Self::InvocationStart,
            _ => Self::FirstObservedFallback,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
/// Timing metadata for the stable conversation identity.
pub struct ConversationMetadata {
    /// Best available conversation start time and its provenance.
    pub started_at: CapturedTimestamp,
    /// First time any hookkit process observed the conversation.
    pub first_observed_at: UtcTimestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
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
/// Project context aggregated from native hook input.
pub struct ProjectMetadata {
    /// Deduplicated union of every workspace root observed across all
    /// observations (sorted), not just the most recent observation.
    pub workspace_roots: Vec<Utf8PathBuf>,
    /// Native transcript path, when supplied.
    pub transcript_path: Option<Utf8PathBuf>,
    /// Native artifact directory, when supplied.
    pub artifact_directory: Option<Utf8PathBuf>,
}

/// Library-owned metadata that is materialized whenever session state is
/// ensured. Client families cannot overwrite its source observations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionMetadata {
    /// On-disk metadata schema version.
    pub schema_version: u32,
    /// Stable harness identity string.
    pub harness: String,
    /// Identity namespace (`session` or `conversation`).
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectObservation {
    observed_at: UtcTimestamp,
    workspace_roots: Vec<Utf8PathBuf>,
    transcript_path: Option<Utf8PathBuf>,
    artifact_directory: Option<Utf8PathBuf>,
}

pub(crate) fn ensure_metadata(
    directory: &Path,
    harness: &HarnessId,
    identity: &SessionIdentity,
    context: Option<&RuntimeContext<'_>>,
) -> Result<SessionMetadata> {
    let internal = directory.join("_hookkit/metadata/v1");
    create_private_dir_all(&internal)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(internal.join("metadata.lock"))?;
    lock.lock_exclusive()?;
    let result = ensure_metadata_locked(directory, &internal, harness, identity, context);
    let unlock = FileExt::unlock(&lock);
    match (result, unlock) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error.into()),
    }
}

pub(crate) fn read_metadata(directory: &Path) -> Result<SessionMetadata> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("_hookkit/metadata/v1/metadata.lock"))?;
    FileExt::lock_shared(&lock)?;
    let result = std::fs::read(directory.join("metadata.json"))
        .map_err(StateError::from)
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(StateError::from));
    let unlock = FileExt::unlock(&lock);
    match (result, unlock) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error.into()),
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
    if !anchor_path.exists() {
        atomic_write(
            &anchor_path,
            &serde_json::to_vec_pretty(&AnchorObservation {
                observed_at,
                harness: harness.as_str().to_string(),
                identity_kind: identity.kind().to_string(),
                identity_hash: identity_hash.clone(),
            })?,
        )?;
    }
    let anchor: AnchorObservation = serde_json::from_slice(&std::fs::read(&anchor_path)?)?;

    if let Some(context) = context {
        let project = ProjectObservation {
            observed_at,
            workspace_roots: context.workspace_roots().to_vec(),
            transcript_path: context.transcript_path().map(ToOwned::to_owned),
            artifact_directory: context.artifact_directory().map(ToOwned::to_owned),
        };
        write_content_addressed(&internal.join("workspaces"), &project)?;

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
        .filter(|observation| observation.epoch.kind == SessionEpochKind::Startup)
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
        project: aggregate_project(&internal.join("workspaces"))?,
    };
    atomic_replace(
        &directory.join("metadata.json"),
        &serde_json::to_vec_pretty(&metadata)?,
    )?;
    Ok(metadata)
}

fn aggregate_project(directory: &Path) -> Result<ProjectMetadata> {
    let mut observations = read_content_addressed::<ProjectObservation>(directory)?;
    observations.sort_by_key(|observation| observation.observed_at);
    let mut roots = BTreeSet::new();
    let mut transcript_path = None;
    let mut artifact_directory = None;
    for observation in observations {
        roots.extend(observation.workspace_roots);
        if observation.transcript_path.is_some() {
            transcript_path = observation.transcript_path;
        }
        if observation.artifact_directory.is_some() {
            artifact_directory = observation.artifact_directory;
        }
    }
    Ok(ProjectMetadata {
        workspace_roots: roots.into_iter().collect(),
        transcript_path,
        artifact_directory,
    })
}

fn read_start_observations(directory: &Path) -> Result<Vec<StartObservation>> {
    let mut observations = read_content_addressed::<StartObservation>(directory)?;
    observations.sort_by_key(|observation| observation.epoch.first_observed_at);
    Ok(observations)
}

fn write_content_addressed<T: Serialize>(directory: &Path, value: &T) -> Result<PathBuf> {
    create_private_dir_all(directory)?;
    let bytes = serde_json::to_vec_pretty(value)?;
    let path = directory.join(format!("{}.json", sha256_bytes(&[&bytes])));
    atomic_write(&path, &bytes)?;
    Ok(path)
}

fn read_content_addressed<T: for<'de> Deserialize<'de>>(directory: &Path) -> Result<Vec<T>> {
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut files = std::fs::read_dir(directory)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    files.sort();
    files
        .into_iter()
        .map(|path| Ok(serde_json::from_slice(&std::fs::read(path)?)?))
        .collect()
}

pub(crate) fn metadata_path(directory: &Path) -> PathBuf {
    directory.join("metadata.json")
}
