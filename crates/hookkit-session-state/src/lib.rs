//! Concurrent, session-scoped filesystem state for independent hook processes.
//!
//! A native session owns a flat state root. Independently authored hook
//! families receive versioned subtrees and coordinate only when they choose the
//! same family and primitive. The store deliberately avoids a shared mutable
//! manifest: lifecycle and topology metadata are immutable observations.

mod entity;
mod metadata;

pub use entity::{
    CompactionPolicy, EntityDisposition, EntityId, EntityJournal, EntityMode, EntityOperationError,
    EntityOutcome, EntityRecord, EntityView, InsertResult, JournalEntity, LoadedRuleEvent,
    LoadedRules, ModifiedFileEvent, ModifiedFiles, SetAggregate, SetEvent, SetJournal,
};
pub use metadata::{
    CapturedTimestamp, ConversationMetadata, ProjectMetadata, SessionEpochKind,
    SessionEpochMetadata, SessionMetadata, TimestampProvenance, UtcTimestamp,
};

use fs2::FileExt;
use hookkit_core::{HarnessId, RuntimeContext};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub type Result<T> = std::result::Result<T, StateError>;

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("session context has neither a session id nor a conversation id")]
    MissingSessionIdentity,

    #[error("invalid state identifier `{0}`")]
    InvalidIdentifier(String),

    #[error("state path must be relative and contain no parent components: {0}")]
    InvalidRelativePath(PathBuf),

    #[error("state root must not be a symbolic link: {0}")]
    SymlinkStateRoot(PathBuf),

    #[error("state I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("state JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("entity state configuration error: {0}")]
    EntityConfiguration(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionIdentity {
    Session(String),
    Conversation(String),
}

impl SessionIdentity {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Session(_) => "session",
            Self::Conversation(_) => "conversation",
        }
    }

    pub fn value(&self) -> &str {
        match self {
            Self::Session(value) | Self::Conversation(value) => value,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StateRoot {
    path: PathBuf,
}

impl StateRoot {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Default for StateRoot {
    fn default() -> Self {
        Self::new(std::env::temp_dir().join("agent-hook-kit/session-state"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyId {
    name: String,
    version: u32,
}

impl FamilyId {
    pub fn new(name: impl Into<String>, version: u32) -> Result<Self> {
        let name = name.into();
        validate_identifier(&name)?;
        if version == 0 {
            return Err(StateError::InvalidIdentifier(
                "family version 0".to_string(),
            ));
        }
        Ok(Self { name, version })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateScope {
    Session,
    Actor(String),
    Turn(String),
    Custom { kind: String, key: String },
}

impl StateScope {
    fn path(&self) -> Result<PathBuf> {
        match self {
            Self::Session => Ok(PathBuf::from("session")),
            Self::Actor(key) => Ok(PathBuf::from("actors").join(sha256(key))),
            Self::Turn(key) => Ok(PathBuf::from("turns").join(sha256(key))),
            Self::Custom { kind, key } => {
                validate_identifier(kind)?;
                Ok(PathBuf::from("custom").join(kind).join(sha256(key)))
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionState {
    root: PathBuf,
    harness: HarnessId,
    identity: SessionIdentity,
    directory: PathBuf,
}

impl SessionState {
    /// Ensure state and automatically capture standard metadata from this
    /// exact runtime invocation.
    pub fn ensure(context: &RuntimeContext<'_>, root: StateRoot) -> Result<Self> {
        let identity = context
            .session_id()
            .map(|id| SessionIdentity::Session(id.to_string()))
            .or_else(|| {
                context
                    .conversation_id()
                    .map(|id| SessionIdentity::Conversation(id.to_string()))
            })
            .ok_or(StateError::MissingSessionIdentity)?;
        let state = Self::open_uninitialized(context.harness().clone(), identity, root)?;
        metadata::ensure_metadata(
            &state.directory,
            &state.harness,
            &state.identity,
            Some(context),
        )?;
        Ok(state)
    }

    /// Compatibility alias for [`SessionState::ensure`].
    pub fn from_context(context: &RuntimeContext<'_>, root: StateRoot) -> Result<Self> {
        Self::ensure(context, root)
    }

    pub fn open(harness: HarnessId, identity: SessionIdentity, root: StateRoot) -> Result<Self> {
        let state = Self::open_uninitialized(harness, identity, root)?;
        metadata::ensure_metadata(&state.directory, &state.harness, &state.identity, None)?;
        Ok(state)
    }

    fn open_uninitialized(
        harness: HarnessId,
        identity: SessionIdentity,
        root: StateRoot,
    ) -> Result<Self> {
        validate_identifier(harness.as_str())?;
        if identity.value().is_empty() {
            return Err(StateError::InvalidIdentifier(
                "empty session identity".to_string(),
            ));
        }
        prepare_private_root(root.path())?;
        let directory = root
            .path()
            .join("v1")
            .join(harness.as_str())
            .join(identity.kind())
            .join(sha256(&format!(
                "{}\0{}\0{}",
                harness.as_str(),
                identity.kind(),
                identity.value()
            )));
        create_private_dir_all(&directory)?;
        Ok(Self {
            root: root.path,
            harness,
            identity,
            directory,
        })
    }

    pub fn metadata(&self) -> Result<SessionMetadata> {
        metadata::read_metadata(&self.directory)
    }

    pub fn metadata_path(&self) -> PathBuf {
        metadata::metadata_path(&self.directory)
    }

    pub fn conversation_started_at(&self) -> Result<UtcTimestamp> {
        Ok(self.metadata()?.conversation.started_at.at)
    }

    pub fn current_session_started_at(&self) -> Result<UtcTimestamp> {
        Ok(self.metadata()?.current_session.started_at.at)
    }

    pub fn current_epoch(&self) -> Result<SessionEpochMetadata> {
        Ok(self.metadata()?.current_session)
    }

    pub fn harness(&self) -> &HarnessId {
        &self.harness
    }

    pub fn identity(&self) -> &SessionIdentity {
        &self.identity
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn family(&self, id: FamilyId) -> Result<StateFamily> {
        let directory = self
            .directory
            .join("families")
            .join(id.name())
            .join(format!("v{}", id.version()));
        create_private_dir_all(&directory)?;
        let activity = self.directory.join("activity");
        create_private_dir_all(&activity)?;
        touch_activity(&activity.join(format!("{}.stamp", id.name())))?;
        Ok(StateFamily { id, directory })
    }

    pub fn observe_lifecycle<T: Serialize>(&self, observation: &T) -> Result<PathBuf> {
        write_observation(&self.directory.join("lifecycle/observations"), observation)
    }

    pub fn lifecycle_observations<T: DeserializeOwned>(&self) -> Result<Vec<T>> {
        read_observations(&self.directory.join("lifecycle/observations"))
    }

    pub fn observe_topology<T: Serialize>(&self, observation: &T) -> Result<PathBuf> {
        write_observation(&self.directory.join("topology/observations"), observation)
    }

    pub fn topology_observations<T: DeserializeOwned>(&self) -> Result<Vec<T>> {
        read_observations(&self.directory.join("topology/observations"))
    }

    pub fn gc(root: &StateRoot, max_age: Duration) -> Result<GcReport> {
        let version_root = root.path().join("v1");
        if !version_root.exists() {
            return Ok(GcReport::default());
        }
        let cutoff = SystemTime::now().checked_sub(max_age).unwrap_or(UNIX_EPOCH);
        let mut report = GcReport::default();
        for harness in read_dirs(&version_root)? {
            for identity_kind in read_dirs(&harness)? {
                for session in read_dirs(&identity_kind)? {
                    report.scanned += 1;
                    let newest = newest_activity(&session)?;
                    if newest < cutoff {
                        std::fs::remove_dir_all(&session)?;
                        report.removed += 1;
                    }
                }
            }
        }
        Ok(report)
    }

    pub fn state_root(&self) -> &Path {
        &self.root
    }
}

#[derive(Debug, Clone)]
pub struct StateFamily {
    id: FamilyId,
    directory: PathBuf,
}

impl StateFamily {
    pub fn id(&self) -> &FamilyId {
        &self.id
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn scope(&self, scope: StateScope) -> Result<FamilyScope> {
        let directory = self.directory.join("scopes").join(scope.path()?);
        create_private_dir_all(&directory)?;
        Ok(FamilyScope { directory })
    }

    pub fn session_scope(&self) -> Result<FamilyScope> {
        self.scope(StateScope::Session)
    }

    pub fn claims(&self, name: &str) -> Result<ClaimSet> {
        self.session_scope()?.claims(name)
    }

    /// Open a sparse, content-addressed journal with one JSON file per record.
    pub fn record_journal(&self, name: &str) -> Result<RecordJournal> {
        self.session_scope()?.record_journal(name)
    }

    pub fn start_run(&self, label: &str) -> Result<RunBundle> {
        self.session_scope()?.start_run(label)
    }

    pub fn with_exclusive_lock<T>(
        &self,
        name: &str,
        operation: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        validate_identifier(name)?;
        let locks = self.directory.join("locks");
        create_private_dir_all(&locks)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(locks.join(format!("{name}.lock")))?;
        file.lock_exclusive()?;
        let result = operation();
        let unlock = FileExt::unlock(&file);
        match (result, unlock) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error.into()),
        }
    }

    /// Hold an advisory family lock until the returned guard is dropped.
    ///
    /// This form is useful when the protected operation returns a domain error
    /// other than [`StateError`]. All cooperating hook processes must use the
    /// same family and lock name.
    pub fn exclusive_lock(&self, name: &str) -> Result<ExclusiveLock> {
        validate_identifier(name)?;
        let locks = self.directory.join("locks");
        create_private_dir_all(&locks)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(locks.join(format!("{name}.lock")))?;
        file.lock_exclusive()?;
        Ok(ExclusiveLock { file })
    }
}

#[derive(Debug)]
pub struct ExclusiveLock {
    file: std::fs::File,
}

impl Drop for ExclusiveLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

#[derive(Debug, Clone)]
pub struct FamilyScope {
    directory: PathBuf,
}

impl FamilyScope {
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn claims(&self, name: &str) -> Result<ClaimSet> {
        validate_identifier(name)?;
        let directory = self.directory.join("claims").join(name);
        create_private_dir_all(&directory)?;
        Ok(ClaimSet { directory })
    }

    /// Open a sparse journal whose records are independently addressable JSON
    /// files. Prefer an [`EntityJournal`] when records are appended frequently
    /// and consumed as an aggregate.
    pub fn record_journal(&self, name: &str) -> Result<RecordJournal> {
        validate_identifier(name)?;
        let directory = self
            .directory
            .join("record-journals")
            .join(name)
            .join("pending");
        create_private_dir_all(&directory)?;
        Ok(RecordJournal { directory })
    }

    pub fn entity<E: JournalEntity>(
        &self,
        id: EntityId,
        mode: EntityMode,
    ) -> Result<EntityJournal<E>> {
        EntityJournal::open(self.directory.join("entities"), id, mode)
    }

    pub fn set<T>(&self, id: EntityId, mode: EntityMode) -> Result<SetJournal<T>>
    where
        T: Serialize + DeserializeOwned + Clone + Ord,
    {
        Ok(SetJournal::new(self.entity(id, mode)?))
    }

    pub fn start_run(&self, label: &str) -> Result<RunBundle> {
        validate_identifier(label)?;
        let runs = self.directory.join("runs");
        create_private_dir_all(&runs)?;
        let directory = loop {
            let candidate = runs.join(format!("{}-{label}", unique_id()));
            match create_private_dir(&candidate) {
                Ok(()) => break candidate,
                Err(StateError::Io(error)) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        };
        Ok(RunBundle {
            directory,
            committed: false,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ClaimSet {
    directory: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimResult {
    Claimed,
    AlreadyClaimed,
}

impl ClaimSet {
    pub fn try_claim(&self, key: &str) -> Result<ClaimResult> {
        let path = self.directory.join(format!("{}.claim", sha256(key)));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(b"claimed\n").and_then(|()| file.sync_all()) {
                    drop(file);
                    let _ = std::fs::remove_file(path);
                    return Err(error.into());
                }
                Ok(ClaimResult::Claimed)
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                Ok(ClaimResult::AlreadyClaimed)
            }
            Err(error) => Err(error.into()),
        }
    }

    pub fn contains(&self, key: &str) -> bool {
        self.directory
            .join(format!("{}.claim", sha256(key)))
            .is_file()
    }
}

#[derive(Debug, Clone)]
pub struct RecordJournal {
    directory: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JournalEntryId(String);

impl JournalEntryId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug)]
pub struct RecordJournalEntry<T> {
    id: JournalEntryId,
    value: T,
}

impl<T> RecordJournalEntry<T> {
    pub fn id(&self) -> &JournalEntryId {
        &self.id
    }

    pub fn value(&self) -> &T {
        &self.value
    }

    pub fn into_value(self) -> T {
        self.value
    }
}

#[derive(Debug)]
pub struct RecordJournalBatch<T> {
    directory: PathBuf,
    entries: Vec<RecordJournalEntry<T>>,
}

impl RecordJournal {
    pub fn append<T: Serialize>(&self, event_key: &str, value: &T) -> Result<JournalEntryId> {
        let bytes = serde_json::to_vec_pretty(value)?;
        let id = JournalEntryId(sha256_bytes(&[
            event_key.as_bytes(),
            b"\0",
            bytes.as_slice(),
        ]));
        atomic_write(&self.directory.join(format!("{}.json", id.0)), &bytes)?;
        Ok(id)
    }

    pub fn snapshot<T: DeserializeOwned>(&self) -> Result<RecordJournalBatch<T>> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&self.directory)? {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) == Some("json") {
                files.push(path);
            }
        }
        files.sort();
        let mut entries = Vec::with_capacity(files.len());
        for path in files {
            let Some(id) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            let value = serde_json::from_slice(&std::fs::read(&path)?)?;
            entries.push(RecordJournalEntry {
                id: JournalEntryId(id.to_string()),
                value,
            });
        }
        Ok(RecordJournalBatch {
            directory: self.directory.clone(),
            entries,
        })
    }
}

impl<T> RecordJournalBatch<T> {
    pub fn entries(&self) -> &[RecordJournalEntry<T>] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn acknowledge(self) -> Result<()> {
        for entry in self.entries {
            let path = self.directory.join(format!("{}.json", entry.id.0));
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct RunBundle {
    directory: PathBuf,
    committed: bool,
}

impl RunBundle {
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn write_text(&self, relative: impl AsRef<Path>, content: &str) -> Result<PathBuf> {
        self.write_bytes(relative, content.as_bytes())
    }

    pub fn write_json<T: Serialize>(
        &self,
        relative: impl AsRef<Path>,
        value: &T,
    ) -> Result<PathBuf> {
        self.write_bytes(relative, &serde_json::to_vec_pretty(value)?)
    }

    pub fn commit<T: Serialize>(mut self, summary: &T) -> Result<PathBuf> {
        let path = self.write_json("summary.json", summary)?;
        self.committed = true;
        Ok(path)
    }

    pub fn is_committed(&self) -> bool {
        self.committed || self.directory.join("summary.json").is_file()
    }

    fn write_bytes(&self, relative: impl AsRef<Path>, bytes: &[u8]) -> Result<PathBuf> {
        validate_relative_path(relative.as_ref())?;
        let path = self.directory.join(relative.as_ref());
        let Some(parent) = path.parent() else {
            return Err(StateError::InvalidRelativePath(
                relative.as_ref().to_path_buf(),
            ));
        };
        create_private_dir_all(parent)?;
        atomic_write(&path, bytes)?;
        Ok(path)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GcReport {
    pub scanned: usize,
    pub removed: usize,
}

fn write_observation<T: Serialize>(directory: &Path, observation: &T) -> Result<PathBuf> {
    create_private_dir_all(directory)?;
    let bytes = serde_json::to_vec_pretty(observation)?;
    let path = directory.join(format!("{}.json", sha256_bytes(&[&bytes])));
    atomic_write(&path, &bytes)?;
    Ok(path)
}

fn read_observations<T: DeserializeOwned>(directory: &Path) -> Result<Vec<T>> {
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.extension().and_then(|value| value.to_str()) == Some("json") {
            files.push(path);
        }
    }
    files.sort();
    files
        .into_iter()
        .map(|path| Ok(serde_json::from_slice(&std::fs::read(path)?)?))
        .collect()
}

fn touch_activity(path: &Path) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    file.write_all(timestamp_millis().to_string().as_bytes())?;
    file.sync_all()?;
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Err(StateError::InvalidRelativePath(path.to_path_buf()));
    };
    create_private_dir_all(parent)?;
    let temporary = parent.join(format!(".tmp-{}", unique_id()));
    let write_result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        match std::fs::rename(&temporary, path) {
            Ok(()) => sync_directory(parent),
            Err(error) if path.exists() => {
                let _ = std::fs::remove_file(&temporary);
                if error.kind() == ErrorKind::AlreadyExists
                    || error.kind() == ErrorKind::PermissionDenied
                {
                    Ok(())
                } else {
                    Err(error)
                }
            }
            Err(error) => Err(error),
        }
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    write_result.map_err(Into::into)
}

/// Replace mutable state while holding its owning primitive's lock.
fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Err(StateError::InvalidRelativePath(path.to_path_buf()));
    };
    create_private_dir_all(parent)?;
    let temporary = parent.join(format!(".tmp-{}", unique_id()));
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        match std::fs::rename(&temporary, path) {
            Ok(()) => {}
            Err(error)
                if path.exists()
                    && matches!(
                        error.kind(),
                        ErrorKind::AlreadyExists | ErrorKind::PermissionDenied
                    ) =>
            {
                // Windows rename does not replace an existing destination.
                // Callers serialize mutable replacements, so this fallback
                // cannot conflict with another cooperating writer.
                std::fs::remove_file(path)?;
                std::fs::rename(&temporary, path)?;
            }
            Err(error) => return Err(error),
        }
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(Into::into)
}

fn sync_directory(directory: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(directory)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = directory;
        Ok(())
    }
}

fn validate_identifier(value: &str) -> Result<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
    {
        return Err(StateError::InvalidIdentifier(value.to_string()));
    }
    Ok(())
}

fn validate_relative_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(StateError::InvalidRelativePath(path.to_path_buf()));
    }
    Ok(())
}

fn prepare_private_root(path: &Path) -> Result<()> {
    if path.exists() && std::fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(StateError::SymlinkStateRoot(path.to_path_buf()));
    }
    create_private_dir_all(path)
}

fn create_private_dir_all(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn create_private_dir(path: &Path) -> Result<()> {
    std::fs::create_dir(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn read_dirs(path: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = std::fs::read_dir(path)?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|kind| kind.is_dir() && !kind.is_symlink())
                .map(|_| entry.path())
        })
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn newest_activity(session: &Path) -> Result<SystemTime> {
    let mut newest = std::fs::metadata(session)?.modified().unwrap_or(UNIX_EPOCH);
    let activity = session.join("activity");
    if activity.is_dir() {
        for entry in std::fs::read_dir(activity)? {
            let Ok(entry) = entry else { continue };
            let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified()) else {
                continue;
            };
            newest = newest.max(modified);
        }
    }
    Ok(newest)
}

fn unique_id() -> String {
    format!(
        "{}-{}-{}",
        timestamp_millis(),
        std::process::id(),
        UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

fn timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn sha256(value: &str) -> String {
    sha256_bytes(&[value.as_bytes()])
}

fn sha256_bytes(parts: &[&[u8]]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update(part);
    }
    let digest = digest.finalize();
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to a string cannot fail");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeSet;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Arc, Barrier};

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    struct Record {
        path: String,
    }

    fn temporary_root(label: &str) -> StateRoot {
        StateRoot::new(std::env::temp_dir().join(format!(
            "hookkit-session-state-test-{label}-{}",
            unique_id()
        )))
    }

    fn family(root: &StateRoot) -> StateFamily {
        SessionState::open(
            HarnessId::CODEX,
            SessionIdentity::Session("session-1".into()),
            root.clone(),
        )
        .unwrap()
        .family(FamilyId::new("test.family", 1).unwrap())
        .unwrap()
    }

    #[test]
    fn native_identity_is_hashed_and_harness_scoped() {
        let root = temporary_root("identity");
        let state = SessionState::open(
            HarnessId::CODEX,
            SessionIdentity::Session("secret/session".into()),
            root.clone(),
        )
        .unwrap();
        assert!(!state.directory().to_string_lossy().contains("secret"));
        assert!(state.directory().to_string_lossy().contains("codex"));
        let _ = std::fs::remove_dir_all(root.path());
    }

    #[test]
    fn concurrent_claim_has_one_winner() {
        let root = temporary_root("claim");
        let claims = Arc::new(family(&root).claims("loaded").unwrap());
        let barrier = Arc::new(Barrier::new(8));
        let handles = (0..8)
            .map(|_| {
                let claims = Arc::clone(&claims);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    claims.try_claim("rule-1").unwrap()
                })
            })
            .collect::<Vec<_>>();
        let winners = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(|result| *result == ClaimResult::Claimed)
            .count();
        assert_eq!(winners, 1);
        let _ = std::fs::remove_dir_all(root.path());
    }

    #[test]
    fn unrelated_family_names_have_independent_state() {
        let root = temporary_root("families");
        let state = SessionState::open(
            HarnessId::CODEX,
            SessionIdentity::Session("session-1".into()),
            root.clone(),
        )
        .unwrap();
        let first = state
            .family(FamilyId::new("example.first", 1).unwrap())
            .unwrap()
            .claims("once")
            .unwrap();
        let second = state
            .family(FamilyId::new("example.second", 1).unwrap())
            .unwrap()
            .claims("once")
            .unwrap();
        assert_eq!(first.try_claim("same-key").unwrap(), ClaimResult::Claimed);
        assert_eq!(second.try_claim("same-key").unwrap(), ClaimResult::Claimed);
        let _ = std::fs::remove_dir_all(root.path());
    }

    #[test]
    fn exclusive_guard_serializes_cooperating_threads() {
        let root = temporary_root("lock");
        let family = Arc::new(family(&root));
        let barrier = Arc::new(Barrier::new(8));
        let active = Arc::new(AtomicUsize::new(0));
        let handles = (0..8)
            .map(|_| {
                let family = Arc::clone(&family);
                let barrier = Arc::clone(&barrier);
                let active = Arc::clone(&active);
                std::thread::spawn(move || {
                    barrier.wait();
                    let _guard = family.exclusive_lock("consumer").unwrap();
                    assert_eq!(active.fetch_add(1, Ordering::SeqCst), 0);
                    std::thread::yield_now();
                    assert_eq!(active.fetch_sub(1, Ordering::SeqCst), 1);
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }
        let _ = std::fs::remove_dir_all(root.path());
    }

    #[test]
    fn record_journal_acknowledges_only_the_snapshot() {
        let root = temporary_root("journal");
        let journal = family(&root).record_journal("dirty").unwrap();
        journal
            .append(
                "tool-1",
                &Record {
                    path: "a.rs".into(),
                },
            )
            .unwrap();
        let batch = journal.snapshot::<Record>().unwrap();
        journal
            .append(
                "tool-2",
                &Record {
                    path: "b.rs".into(),
                },
            )
            .unwrap();
        batch.acknowledge().unwrap();
        let remaining = journal.snapshot::<Record>().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining.entries()[0].value().path, "b.rs");
        let _ = std::fs::remove_dir_all(root.path());
    }

    #[test]
    fn run_is_discoverable_only_after_summary_commit() {
        let root = temporary_root("run");
        let run = family(&root).start_run("lint").unwrap();
        let run_directory = run.directory().to_path_buf();
        run.write_text("tools/rustfmt/stdout.txt", "changed")
            .unwrap();
        assert!(!run.is_committed());
        assert!(!run_directory.join("summary.json").exists());
        let expected = serde_json::json!({
            "status": "clean",
            "artifacts": ["tools/rustfmt/stdout.txt"]
        });
        let summary = run.commit(&expected).unwrap();
        assert_eq!(summary.file_name().unwrap(), "summary.json");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(summary).unwrap()).unwrap(),
            expected
        );
        let _ = std::fs::remove_dir_all(root.path());
    }

    #[test]
    fn run_artifact_paths_cannot_escape_the_bundle() {
        let root = temporary_root("run-path-escape");
        let run = family(&root).start_run("lint").unwrap();
        assert!(matches!(
            run.write_text("../escaped.txt", "no"),
            Err(StateError::InvalidRelativePath(_))
        ));
        assert!(matches!(
            run.write_text(root.path().join("escaped.txt"), "no"),
            Err(StateError::InvalidRelativePath(_))
        ));
        assert!(!root.path().join("escaped.txt").exists());
        let _ = std::fs::remove_dir_all(root.path());
    }

    #[test]
    fn observations_are_content_addressed() {
        let root = temporary_root("observation");
        let state = SessionState::open(
            HarnessId::CLAUDE_CODE,
            SessionIdentity::Session("s".into()),
            root.clone(),
        )
        .unwrap();
        let first = state
            .observe_topology(&serde_json::json!({"agent": "a"}))
            .unwrap();
        let second = state
            .observe_topology(&serde_json::json!({"agent": "a"}))
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(
            state.topology_observations::<serde_json::Value>().unwrap(),
            vec![serde_json::json!({"agent": "a"})]
        );
        assert!(
            state
                .lifecycle_observations::<serde_json::Value>()
                .unwrap()
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(root.path());
    }

    #[test]
    fn rejects_unsafe_family_and_run_paths() {
        assert!(FamilyId::new("../escape", 1).is_err());
        let root = temporary_root("unsafe");
        assert!(
            SessionState::open(
                HarnessId::new("../escape").unwrap(),
                SessionIdentity::Session("s".into()),
                root.clone(),
            )
            .is_err()
        );
        let run = family(&root).start_run("lint").unwrap();
        assert!(run.write_text("../outside", "no").is_err());
        let _ = std::fs::remove_dir_all(root.path());
    }

    #[test]
    fn open_automatically_materializes_typed_fallback_metadata() {
        let root = temporary_root("metadata-fallback");
        let state = SessionState::open(
            HarnessId::CODEX,
            SessionIdentity::Session("session-1".into()),
            root.clone(),
        )
        .unwrap();
        let metadata = state.metadata().unwrap();
        assert_eq!(metadata.schema_version, 1);
        assert_eq!(metadata.harness, "codex");
        assert_eq!(
            metadata.current_session.kind,
            SessionEpochKind::FirstObservedFallback
        );
        assert_eq!(
            metadata.current_session.started_at.provenance,
            TimestampProvenance::FirstHookObservation
        );
        assert_eq!(
            state.current_session_started_at().unwrap(),
            metadata.current_session.started_at.at
        );
        assert!(state.metadata_path().is_file());
        let _ = std::fs::remove_dir_all(root.path());
    }

    #[test]
    fn ensure_uses_native_start_timestamp_and_captures_project_context() {
        let root = temporary_root("metadata-native");
        let raw = hookkit_core::RawInvocation::parse(b"{}".to_vec()).unwrap();
        let timestamp = "2026-07-12T01:02:03Z";
        let context = hookkit_core::RuntimeContext::new(
            HarnessId::GEMINI_CLI,
            hookkit_core::SnapshotId::builtin("test"),
            hookkit_core::EventId::builtin(HarnessId::GEMINI_CLI, "SessionStart"),
            hookkit_core::ContractId::builtin("test"),
            hookkit_core::ResolutionProvenance::TypedStatic,
            &raw,
            hookkit_core::NativeContext {
                workspace_roots: vec![hookkit_core::Utf8PathBuf::from("/repo")],
                session_id: hookkit_core::SessionId::new("session-1").ok(),
                transcript_path: Some(hookkit_core::Utf8PathBuf::from("/tmp/transcript.json")),
                session_boundary: Some(
                    hookkit_core::SessionBoundaryContext::observed(
                        hookkit_core::SessionBoundaryKind::Startup,
                    )
                    .with_native_timestamp(timestamp)
                    .with_occurrence_key("native-start-1"),
                ),
                ..hookkit_core::NativeContext::default()
            },
            &hookkit_core::DISABLED_DIAGNOSTICS,
        )
        .unwrap();
        let first = SessionState::ensure(&context, root.clone()).unwrap();
        let first_metadata = first.metadata().unwrap();
        assert_eq!(
            first_metadata.current_session.kind,
            SessionEpochKind::Startup
        );
        assert_eq!(
            first_metadata.current_session.started_at.at,
            UtcTimestamp::parse_rfc3339(timestamp).unwrap()
        );
        assert_eq!(
            first_metadata.current_session.started_at.provenance,
            TimestampProvenance::NativeEventTimestamp
        );
        assert_eq!(
            first_metadata.project.workspace_roots,
            vec![hookkit_core::Utf8PathBuf::from("/repo")]
        );

        let second = SessionState::ensure(&context, root.clone()).unwrap();
        assert_eq!(
            first_metadata.current_session.id,
            second.metadata().unwrap().current_session.id
        );
        let _ = std::fs::remove_dir_all(root.path());
    }

    fn modified_entity(root: &StateRoot) -> EntityJournal<ModifiedFiles> {
        family(root)
            .session_scope()
            .unwrap()
            .entity(
                EntityId::new("modified-files", 1).unwrap(),
                EntityMode::Windowed,
            )
            .unwrap()
    }

    fn modified(path: &str) -> ModifiedFileEvent {
        ModifiedFileEvent {
            path: hookkit_core::Utf8PathBuf::from(path),
            event: Some("PostToolUse".into()),
            tool_call_id: None,
        }
    }

    #[test]
    fn entity_cache_applies_only_new_ndjson_generations() {
        let root = temporary_root("entity-cache");
        let entity = modified_entity(&root);
        entity.append("one", &modified("/repo/a.rs")).unwrap();
        entity
            .with_entity(|view| {
                assert_eq!(view.state().paths().len(), 1);
                assert_eq!(view.new_events().len(), 1);
                Ok(EntityOutcome::retain(()))
            })
            .unwrap();
        entity.append("two", &modified("/repo/b.rs")).unwrap();
        entity
            .with_entity(|view| {
                assert_eq!(view.state().paths().len(), 2);
                assert_eq!(view.events().len(), 2);
                assert_eq!(view.new_events().len(), 1);
                Ok(EntityOutcome::acknowledge(()))
            })
            .unwrap();
        entity
            .with_entity(|view| {
                assert!(view.state().paths().is_empty());
                assert!(view.events().is_empty());
                Ok(EntityOutcome::retain(()))
            })
            .unwrap();
        let _ = std::fs::remove_dir_all(root.path());
    }

    #[test]
    fn window_ack_does_not_consume_events_appended_by_the_consumer() {
        let root = temporary_root("entity-window");
        let entity = modified_entity(&root);
        entity.append("one", &modified("/repo/a.rs")).unwrap();
        entity
            .with_entity(|view| {
                assert_eq!(view.state().paths().len(), 1);
                entity.append("two", &modified("/repo/b.rs"))?;
                Ok(EntityOutcome::acknowledge(()))
            })
            .unwrap();
        entity
            .with_entity(|view| {
                assert_eq!(
                    view.state().paths(),
                    &BTreeSet::from([hookkit_core::Utf8PathBuf::from("/repo/b.rs")])
                );
                Ok(EntityOutcome::retain(()))
            })
            .unwrap();
        let generation = read_dirs(&entity.directory().join("generations")).unwrap_or_default();
        assert!(
            generation.is_empty(),
            "generations are NDJSON files, not dirs"
        );
        assert!(
            std::fs::read_dir(entity.directory().join("generations"))
                .unwrap()
                .filter_map(|entry| entry.ok())
                .any(
                    |entry| entry.path().extension().and_then(|value| value.to_str())
                        == Some("ndjson")
                )
        );
        let _ = std::fs::remove_dir_all(root.path());
    }

    #[test]
    fn monotonic_set_compaction_preserves_state_without_source_events() {
        let root = temporary_root("entity-set");
        let set = family(&root)
            .session_scope()
            .unwrap()
            .set::<String>(
                EntityId::new("loaded-rules", 1).unwrap(),
                EntityMode::Monotonic,
            )
            .unwrap();
        assert_eq!(
            set.insert_once("rust", "rust.md".into()).unwrap(),
            InsertResult::Inserted
        );
        assert_eq!(
            set.insert_once("rust", "rust.md".into()).unwrap(),
            InsertResult::AlreadyPresent
        );
        set.flush().unwrap();
        assert_eq!(set.current().unwrap(), BTreeSet::from(["rust.md".into()]));
        assert!(set.entity().directory().join("checkpoint.json").is_file());
        let _ = std::fs::remove_dir_all(root.path());
    }

    #[test]
    fn monotonic_set_insert_once_has_one_concurrent_winner() {
        let root = temporary_root("entity-set-concurrent");
        let set = Arc::new(
            family(&root)
                .session_scope()
                .unwrap()
                .set::<String>(
                    EntityId::new("loaded-rules", 1).unwrap(),
                    EntityMode::Monotonic,
                )
                .unwrap(),
        );
        let barrier = Arc::new(Barrier::new(8));
        let handles = (0..8)
            .map(|_| {
                let set = Arc::clone(&set);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    set.insert_once("rust", "rust.md".into()).unwrap()
                })
            })
            .collect::<Vec<_>>();
        let winners = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(|result| *result == InsertResult::Inserted)
            .count();
        assert_eq!(winners, 1);
        assert_eq!(set.current().unwrap(), BTreeSet::from(["rust.md".into()]));
        let _ = std::fs::remove_dir_all(root.path());
    }
}
