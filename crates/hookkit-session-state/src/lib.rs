//! Concurrent, session-scoped filesystem state for independent hook processes.
#![deny(missing_docs)]
//!
//! A native session owns a flat state root. Independently authored hook
//! families receive versioned subtrees and coordinate only when they choose the
//! same family and primitive. The store deliberately avoids a shared mutable
//! manifest: lifecycle and topology metadata are immutable observations.
//!
//! # Locks
//!
//! Every lock is an advisory file lock scoped to one primitive. Acquiring a
//! lock that the current thread already holds returns
//! [`StateError::LockReentry`] instead of deadlocking. Nested acquisitions of
//! different locks must follow one order in every cooperating hook to avoid
//! cross-process deadlock:
//!
//! 1. family locks ([`StateFamily::exclusive_lock`]) before any entity;
//! 2. an entity's consumer lock before its own append lock, which the library
//!    does for you (appending from inside a consumer closure is safe);
//! 3. when a consumer closure opens a second entity, nest the same entities in
//!    the same order everywhere.

mod entity;
mod metadata;
mod storage;

pub use entity::{
    CompactionPolicy, EntityDisposition, EntityId, EntityJournal, EntityMode, EntityOperationError,
    EntityOutcome, EntityRecord, EntityView, InsertResult, JournalEntity, LoadedRuleEvent,
    LoadedRules, ModifiedFileEvent, ModifiedFiles, SetAggregate, SetEvent, SetJournal,
};
pub use metadata::{
    CapturedTimestamp, ConversationMetadata, ProjectMetadata, SessionEpochKind,
    SessionEpochMetadata, SessionMetadata, TimestampProvenance, UtcTimestamp,
};

use hookkit_core::{HarnessId, RuntimeContext};
use serde::{Serialize, de::DeserializeOwned};
use std::fs::OpenOptions;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use storage::{
    ACTIVITY_REFRESH, Durability, FileIdentity, FileLock, IoContext, LockMode, LockWait,
    atomic_replace, create_private_dir, create_private_dir_all, entry_exists, publish_if_absent,
    read_optional, sha256, sha256_bytes, touch_activity, unique_id, validate_identifier,
    validate_name, validate_relative_path,
};

/// Result alias for session-state operations.
pub type Result<T> = std::result::Result<T, StateError>;

/// Library-owned activity stamp refreshed whenever a session is opened.
const SESSION_ACTIVITY_STAMP: &str = "_hookkit.stamp";

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
/// Error produced by session-state validation, storage, or serialization.
pub enum StateError {
    /// Runtime context supplied neither a native session nor conversation ID.
    #[error("session context has neither a session id nor a conversation id")]
    MissingSessionIdentity,

    /// A family, entity, scope-kind, lock, or journal identifier was unsafe.
    ///
    /// Coordination names must be lowercase so that they mean the same thing
    /// on case-sensitive and case-insensitive filesystems.
    #[error("invalid state identifier `{0}`")]
    InvalidIdentifier(String),

    /// A run-bundle path was absolute or contained a parent component.
    #[error("state path must be relative and contain no parent components: {0}")]
    InvalidRelativePath(PathBuf),

    /// The configured state root was itself a symbolic link.
    #[error("state root must not be a symbolic link: {0}")]
    SymlinkStateRoot(PathBuf),

    /// A per-user state directory is not private to this user.
    #[error("state directory {} is not private to the current user: {reason}", .path.display())]
    UnsafeStateRoot {
        /// Directory that failed the ownership check.
        path: PathBuf,
        /// Why the directory cannot be trusted.
        reason: String,
    },

    /// Filesystem access failed.
    #[error("state I/O error: failed to {operation} {}: {source}", .path.display())]
    Io {
        /// Short description of the failed operation.
        operation: &'static str,
        /// File or directory the operation targeted.
        path: PathBuf,
        /// Underlying operating-system error.
        #[source]
        source: std::io::Error,
    },

    /// Stored JSON could not be decoded.
    #[error("state JSON error: failed to decode {}: {source}", .path.display())]
    Decode {
        /// File whose content could not be decoded.
        path: PathBuf,
        /// Underlying decode error.
        #[source]
        source: serde_json::Error,
    },

    /// A value could not be encoded as JSON.
    #[error("state JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// An entity was opened with mode or representation incompatible with its
    /// existing descriptor, or requested an invalid disposition.
    #[error("entity state configuration error: {0}")]
    EntityConfiguration(String),

    /// The current thread tried to acquire a lock it already holds.
    ///
    /// Waiting would deadlock, because advisory file locks do not nest. This
    /// usually means an entity or family lock was requested again from inside
    /// a closure running under that same lock.
    #[error(
        "state lock {} is already held by this thread; nested acquisition would deadlock",
        .0.display()
    )]
    LockReentry(PathBuf),
}

impl StateError {
    /// Returns the underlying I/O error when this is a filesystem failure.
    pub fn io_error(&self) -> Option<&std::io::Error> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Native identity namespace used to isolate one conversation's state.
pub enum SessionIdentity {
    /// Harness-native session identity.
    Session(String),
    /// Harness-native conversation identity used when no session ID exists.
    Conversation(String),
}

impl SessionIdentity {
    /// Returns the stable on-disk namespace name.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Session(_) => "session",
            Self::Conversation(_) => "conversation",
        }
    }

    /// Returns the opaque native identity value.
    pub fn value(&self) -> &str {
        match self {
            Self::Session(value) | Self::Conversation(value) => value,
        }
    }
}

#[derive(Debug, Clone)]
/// Configured parent directory for all versioned session state.
pub struct StateRoot {
    path: PathBuf,
    /// The per-user directory that contains a per-user root. Only Unix has a
    /// shared temporary directory that needs its owner checked.
    #[cfg_attr(not(unix), allow(dead_code))]
    per_user_base: Option<PathBuf>,
}

impl StateRoot {
    /// Creates a state-root configuration without touching the filesystem.
    ///
    /// An explicit root is used as given: HookKit creates it, and any missing
    /// ancestors, owner-only, rejects it when it is a symbolic link, and never
    /// changes the permissions of an existing directory. Directories HookKit
    /// creates beneath it are owner-only.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            per_user_base: None,
        }
    }

    /// Creates a root at `relative` inside HookKit's per-user directory,
    /// without touching the filesystem.
    ///
    /// On Unix the per-user directory is `$TMPDIR/agent-hook-kit-<uid>`
    /// (`/tmp` when `TMPDIR` is unset). A per-user root gets the same
    /// protection as [`StateRoot::default`], which is
    /// `StateRoot::per_user("session-state")`: the per-user directory is
    /// created owner-only and an existing one must be owned by the effective
    /// user and is made owner-only again, and every directory from it down to
    /// the root must be a real directory owned by the effective user. Users
    /// who share a world-writable temporary directory therefore cannot lock
    /// each other out or plant state for each other. Elsewhere the per-user
    /// directory is `agent-hook-kit` in the already per-user temporary
    /// directory.
    ///
    /// Use this for a tool-specific default beside HookKit's own, such as
    /// `StateRoot::per_user("generated/my-hook")`. `relative` must name at
    /// least one directory and contain only normal components: no root,
    /// prefix, `.`, or `..`.
    pub fn per_user(relative: impl AsRef<Path>) -> Result<Self> {
        let relative = relative.as_ref();
        if !relative
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
            || relative.components().next().is_none()
        {
            return Err(StateError::InvalidRelativePath(relative.to_path_buf()));
        }
        let base = per_user_directory();
        Ok(Self {
            path: base.join(relative),
            per_user_base: Some(base),
        })
    }

    /// Returns the configured root path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Creates and validates the root without opening a session, returning
    /// its path.
    ///
    /// [`SessionState::ensure`] and [`SessionState::open`] do this
    /// themselves. Call it before handing [`StateRoot::path`] to code that
    /// takes a plain directory, such as another program's `--state-dir`, so
    /// a per-user root is checked even though that code builds its own
    /// [`StateRoot::new`].
    pub fn prepare(&self) -> Result<&Path> {
        prepare_root(self)?;
        Ok(&self.path)
    }
}

impl Default for StateRoot {
    /// Returns the per-user default root, `StateRoot::per_user("session-state")`.
    ///
    /// On Unix this is `$TMPDIR/agent-hook-kit-<uid>/session-state` (`/tmp`
    /// when `TMPDIR` is unset). Elsewhere it is `agent-hook-kit\session-state`
    /// in the already per-user temporary directory. See
    /// [`StateRoot::per_user`] for the ownership checks.
    fn default() -> Self {
        let base = per_user_directory();
        Self {
            path: base.join("session-state"),
            per_user_base: Some(base),
        }
    }
}

#[cfg(unix)]
fn per_user_directory() -> PathBuf {
    std::env::temp_dir().join(format!("agent-hook-kit-{}", current_uid()))
}

#[cfg(not(unix))]
fn per_user_directory() -> PathBuf {
    std::env::temp_dir().join("agent-hook-kit")
}

#[cfg(unix)]
fn current_uid() -> u32 {
    // SAFETY: geteuid has no preconditions, cannot fail, and does not touch
    // memory owned by Rust.
    unsafe { libc::geteuid() }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Validated name and nonzero schema version for an independent hook family.
pub struct FamilyId {
    name: String,
    version: u32,
}

impl FamilyId {
    /// Creates a family identity.
    ///
    /// `name` must be a lowercase state identifier (ASCII lowercase letters,
    /// digits, `.`, `_`, and `-`) and `version` must be nonzero.
    pub fn new(name: impl Into<String>, version: u32) -> Result<Self> {
        let name = name.into();
        validate_name(&name)?;
        if version == 0 {
            return Err(StateError::InvalidIdentifier(
                "family version 0".to_string(),
            ));
        }
        Ok(Self { name, version })
    }

    /// Returns the validated family name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the nonzero family schema version.
    pub fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
/// Isolation scope within a versioned state family.
pub enum StateScope {
    /// State shared by every actor and turn in the session.
    Session,
    /// State isolated by a SHA-256 hash of an opaque actor key.
    Actor(String),
    /// State isolated by a SHA-256 hash of an opaque turn key.
    Turn(String),
    /// Application-defined namespace with a hashed opaque key.
    Custom {
        /// Validated lowercase namespace identifier retained in the layout.
        kind: String,
        /// Opaque key stored only through its SHA-256 digest.
        key: String,
    },
}

impl StateScope {
    fn path(&self) -> Result<PathBuf> {
        match self {
            Self::Session => Ok(PathBuf::from("session")),
            Self::Actor(key) => Ok(PathBuf::from("actors").join(sha256(key))),
            Self::Turn(key) => Ok(PathBuf::from("turns").join(sha256(key))),
            Self::Custom { kind, key } => {
                validate_name(kind)?;
                Ok(PathBuf::from("custom").join(kind).join(sha256(key)))
            }
        }
    }
}

#[derive(Debug, Clone)]
/// Open handle to one harness/session state directory.
///
/// Native identity values are never used as path components; the directory is
/// keyed by their SHA-256 digest. Cloning the handle does not acquire locks.
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

    /// Opens state from an explicit harness and native identity.
    ///
    /// This is intended for consumers without a [`RuntimeContext`]. It creates
    /// fallback metadata from the first local observation and therefore cannot
    /// recover native lifecycle timestamps or project paths.
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
        prepare_root(&root)?;
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

    /// Reads the current library-owned session metadata snapshot.
    pub fn metadata(&self) -> Result<SessionMetadata> {
        metadata::read_metadata(&self.directory)
    }

    /// Returns the path of the materialized metadata JSON file.
    pub fn metadata_path(&self) -> PathBuf {
        metadata::metadata_path(&self.directory)
    }

    /// Returns the best available conversation start time.
    pub fn conversation_started_at(&self) -> Result<UtcTimestamp> {
        Ok(self.metadata()?.conversation.started_at.at)
    }

    /// Returns the best available start time of the current session epoch.
    pub fn current_session_started_at(&self) -> Result<UtcTimestamp> {
        Ok(self.metadata()?.current_session.started_at.at)
    }

    /// Returns metadata for the current session epoch.
    pub fn current_epoch(&self) -> Result<SessionEpochMetadata> {
        Ok(self.metadata()?.current_session)
    }

    /// Returns the harness that owns this state directory.
    pub fn harness(&self) -> &HarnessId {
        &self.harness
    }

    /// Returns the native identity namespace and opaque value.
    pub fn identity(&self) -> &SessionIdentity {
        &self.identity
    }

    /// Returns the hashed, harness-scoped session directory.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Opens a versioned family subtree and records family activity for GC.
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

    /// Stores an immutable, content-addressed lifecycle observation.
    ///
    /// Identical JSON serializations map to the same path and are deduplicated;
    /// an existing observation is never rewritten.
    pub fn observe_lifecycle<T: Serialize>(&self, observation: &T) -> Result<PathBuf> {
        write_observation(&self.directory.join("lifecycle/observations"), observation)
    }

    /// Reads all lifecycle observations in deterministic content-hash order.
    ///
    /// Every family in the session shares this directory, so records that do
    /// not decode as `T` are skipped rather than failing the read.
    pub fn lifecycle_observations<T: DeserializeOwned>(&self) -> Result<Vec<T>> {
        read_observations(&self.directory.join("lifecycle/observations"))
    }

    /// Stores an immutable, content-addressed topology observation.
    pub fn observe_topology<T: Serialize>(&self, observation: &T) -> Result<PathBuf> {
        write_observation(&self.directory.join("topology/observations"), observation)
    }

    /// Reads all topology observations in deterministic content-hash order.
    ///
    /// Records that do not decode as `T` are skipped rather than failing the
    /// read.
    pub fn topology_observations<T: DeserializeOwned>(&self) -> Result<Vec<T>> {
        read_observations(&self.directory.join("topology/observations"))
    }

    /// Removes session directories whose newest recorded activity is older
    /// than `max_age`.
    ///
    /// The scan is limited to the versioned subtree under `root`. The session
    /// directory, the library's own activity stamp (refreshed by every
    /// [`SessionState::ensure`] and [`SessionState::open`]), and family
    /// activity stamps participate in the age calculation. Stamps are
    /// rewritten at most every 30 seconds, so a `max_age` shorter than that
    /// is raised to 30 seconds; otherwise a session opened moments ago could
    /// look stale.
    ///
    /// Each session is checked while holding its exclusive metadata lock,
    /// and a session whose lock is busy is skipped. Opening a session takes
    /// that lock and refreshes the activity stamp before releasing it, so a
    /// hook that has opened a session within the last `max_age` keeps it: a
    /// concurrent opener sees either the complete session or, when the pass
    /// wins the lock, a fresh one. A hook that keeps working with a session
    /// for longer than `max_age` after opening it can still lose it, so
    /// choose a retention window well beyond the longest hook run.
    ///
    /// A stale session is renamed into a trash directory under `root` and
    /// then deleted. Sessions already removed by a concurrent pass are
    /// skipped. A failure affecting one session is counted in
    /// [`GcReport::failed`] and does not stop the pass; leftover trash is
    /// retried by the next pass.
    pub fn gc(root: &StateRoot, max_age: Duration) -> Result<GcReport> {
        if !entry_exists(root.path())? {
            return Ok(GcReport::default());
        }
        let max_age = max_age.max(ACTIVITY_REFRESH);
        verify_root(root)?;
        let mut report = GcReport::default();
        let trash = root.path().join(".trash");
        for leftover in read_dirs(&trash)? {
            match std::fs::remove_dir_all(&leftover) {
                Ok(()) => {}
                // A concurrent pass finished deleting it first.
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(_) => report.failed += 1,
            }
        }
        let cutoff = SystemTime::now().checked_sub(max_age).unwrap_or(UNIX_EPOCH);
        for harness in read_dirs(&root.path().join("v1"))? {
            for identity_kind in read_dirs(&harness)? {
                for session in read_dirs(&identity_kind)? {
                    report.scanned += 1;
                    match collect_session(&session, cutoff, &trash) {
                        Ok(true) => report.removed += 1,
                        Ok(false) => {}
                        Err(_) => report.failed += 1,
                    }
                }
            }
        }
        Ok(report)
    }

    /// Returns the configured parent state root.
    pub fn state_root(&self) -> &Path {
        &self.root
    }
}

#[derive(Debug, Clone)]
/// Open handle to one independently versioned hook-family subtree.
pub struct StateFamily {
    id: FamilyId,
    directory: PathBuf,
}

impl StateFamily {
    /// Returns the family identity.
    pub fn id(&self) -> &FamilyId {
        &self.id
    }

    /// Returns the versioned family directory.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Opens an isolation scope within this family.
    pub fn scope(&self, scope: StateScope) -> Result<FamilyScope> {
        let directory = self.directory.join("scopes").join(scope.path()?);
        create_private_dir_all(&directory)?;
        Ok(FamilyScope { directory })
    }

    /// Opens the session-wide family scope.
    pub fn session_scope(&self) -> Result<FamilyScope> {
        self.scope(StateScope::Session)
    }

    /// Opens a named claim set in the session-wide scope.
    pub fn claims(&self, name: &str) -> Result<ClaimSet> {
        self.session_scope()?.claims(name)
    }

    /// Open a sparse, content-addressed journal with one JSON file per record.
    pub fn record_journal(&self, name: &str) -> Result<RecordJournal> {
        self.session_scope()?.record_journal(name)
    }

    /// Starts a uniquely named run bundle in the session-wide scope.
    pub fn start_run(&self, label: &str) -> Result<RunBundle> {
        self.session_scope()?.start_run(label)
    }

    /// Runs `operation` while holding an advisory exclusive family lock.
    ///
    /// All cooperating processes must use the same family and lock name. The
    /// lock is released even when `operation` returns an error. Requesting the
    /// same lock again from inside `operation` fails with
    /// [`StateError::LockReentry`] instead of deadlocking.
    pub fn with_exclusive_lock<T>(
        &self,
        name: &str,
        operation: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let path = self.lock_path(name)?;
        let lock = FileLock::exclusive(&path)?;
        let result = operation();
        let unlock = lock.release(&path);
        match (result, unlock) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    /// Hold an advisory family lock until the returned guard is dropped.
    ///
    /// This form is useful when the protected operation returns a domain error
    /// other than [`StateError`]. All cooperating hook processes must use the
    /// same family and lock name. The call waits indefinitely; use
    /// [`Self::exclusive_lock_timeout`] to stay within a harness deadline.
    pub fn exclusive_lock(&self, name: &str) -> Result<ExclusiveLock> {
        let path = self.lock_path(name)?;
        Ok(ExclusiveLock {
            _lock: FileLock::exclusive(&path)?,
        })
    }

    /// Acquires the family lock only when it is immediately available.
    ///
    /// Returns `Ok(None)` when another holder currently owns the lock.
    pub fn try_exclusive_lock(&self, name: &str) -> Result<Option<ExclusiveLock>> {
        self.acquire_lock(name, LockWait::Try)
    }

    /// Waits at most `timeout` for the family lock.
    ///
    /// Returns `Ok(None)` when the lock is still held elsewhere at the
    /// deadline, so a hook can degrade gracefully before the harness kills
    /// it.
    pub fn exclusive_lock_timeout(
        &self,
        name: &str,
        timeout: Duration,
    ) -> Result<Option<ExclusiveLock>> {
        let wait = Instant::now()
            .checked_add(timeout)
            .map_or(LockWait::Block, LockWait::Until);
        self.acquire_lock(name, wait)
    }

    fn acquire_lock(&self, name: &str, wait: LockWait) -> Result<Option<ExclusiveLock>> {
        let path = self.lock_path(name)?;
        Ok(FileLock::acquire(&path, LockMode::Exclusive, wait)?
            .map(|lock| ExclusiveLock { _lock: lock }))
    }

    fn lock_path(&self, name: &str) -> Result<PathBuf> {
        validate_name(name)?;
        let locks = self.directory.join("locks");
        create_private_dir_all(&locks)?;
        Ok(locks.join(format!("{name}.lock")))
    }
}

#[derive(Debug)]
/// RAII guard for an advisory exclusive family lock.
///
/// The lock is released when the guard is dropped. Drop the guard on the
/// thread that acquired it so same-thread re-entry detection stays accurate.
pub struct ExclusiveLock {
    _lock: FileLock,
}

#[derive(Debug, Clone)]
/// Open handle to one family isolation scope.
pub struct FamilyScope {
    directory: PathBuf,
}

impl FamilyScope {
    /// Returns the scope directory.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Opens a named atomic claim set.
    pub fn claims(&self, name: &str) -> Result<ClaimSet> {
        validate_name(name)?;
        let directory = self.directory.join("claims").join(name);
        create_private_dir_all(&directory)?;
        Ok(ClaimSet { directory })
    }

    /// Open a sparse journal whose records are independently addressable JSON
    /// files. Prefer an [`EntityJournal`] when records are appended frequently
    /// and consumed as an aggregate.
    pub fn record_journal(&self, name: &str) -> Result<RecordJournal> {
        validate_name(name)?;
        let journal = self.directory.join("record-journals").join(name);
        let directory = journal.join("pending");
        create_private_dir_all(&directory)?;
        Ok(RecordJournal {
            directory,
            lock: journal.join("journal.lock"),
        })
    }

    /// Opens a typed aggregate entity.
    ///
    /// Reopening an existing entity with a different `mode` fails rather than
    /// reinterpreting its stored generations, even when two hooks open it for
    /// the first time concurrently.
    pub fn entity<E: JournalEntity>(
        &self,
        id: EntityId,
        mode: EntityMode,
    ) -> Result<EntityJournal<E>> {
        EntityJournal::open(self.directory.join("entities"), id, mode)
    }

    /// Opens a typed sorted-set journal backed by an entity.
    ///
    /// A monotonic set starts with an automatic compaction policy; see
    /// [`SetJournal::with_compaction_policy`].
    pub fn set<T>(&self, id: EntityId, mode: EntityMode) -> Result<SetJournal<T>>
    where
        T: Serialize + DeserializeOwned + Clone + Ord,
    {
        Ok(SetJournal::new(self.entity(id, mode)?))
    }

    /// Creates a uniquely named directory for one multi-artifact run.
    ///
    /// `label` may use either ASCII case; the unique prefix keeps run
    /// directories distinct on case-insensitive filesystems.
    pub fn start_run(&self, label: &str) -> Result<RunBundle> {
        validate_identifier(label)?;
        let runs = self.directory.join("runs");
        create_private_dir_all(&runs)?;
        let directory = loop {
            let candidate = runs.join(format!("{}-{label}", unique_id()));
            match create_private_dir(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error).at("create run directory", &candidate),
            }
        };
        Ok(RunBundle {
            directory,
            committed: false,
        })
    }
}

#[derive(Debug, Clone)]
/// Persistent collection of atomic, hashed-key claims.
pub struct ClaimSet {
    directory: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
/// Result of attempting to create a durable claim.
pub enum ClaimResult {
    /// This process created the claim.
    Claimed,
    /// A claim for the same key already existed.
    AlreadyClaimed,
}

impl ClaimSet {
    /// Atomically claims an opaque key using create-if-absent filesystem
    /// semantics.
    ///
    /// Only a SHA-256 digest of `key` appears in the filename. The claim's
    /// synced content becomes visible in one step, so a peer observes
    /// [`ClaimResult::AlreadyClaimed`] only for a claim that stays in place:
    /// when writing fails, nothing is published and the error is returned, so
    /// a later attempt can still claim the key.
    ///
    /// On filesystems without hard links the claim falls back to exclusive
    /// creation. There, the file's presence is the claim, so a creator whose
    /// content write then fails still reports [`ClaimResult::Claimed`] and
    /// must act.
    pub fn try_claim(&self, key: &str) -> Result<ClaimResult> {
        let path = self.directory.join(format!("{}.claim", sha256(key)));
        if entry_exists(&path)? {
            return Ok(ClaimResult::AlreadyClaimed);
        }
        let temporary = self.directory.join(format!(".tmp-{}", unique_id()));
        let written = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .and_then(|mut file| {
                std::io::Write::write_all(&mut file, b"claimed\n")?;
                file.sync_all()
            });
        if let Err(error) = written {
            let _ = std::fs::remove_file(&temporary);
            return Err(error).at("write claim", &temporary);
        }
        let linked = std::fs::hard_link(&temporary, &path);
        let _ = std::fs::remove_file(&temporary);
        match linked {
            Ok(()) => Ok(ClaimResult::Claimed),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                Ok(ClaimResult::AlreadyClaimed)
            }
            Err(_) => claim_without_links(&path),
        }
    }

    /// Reports whether the claim file for `key` currently exists.
    ///
    /// This is a point-in-time observation, not an atomic claim operation.
    pub fn contains(&self, key: &str) -> bool {
        self.directory
            .join(format!("{}.claim", sha256(key)))
            .is_file()
    }
}

fn claim_without_links(path: &Path) -> Result<ClaimResult> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            // The claim is already visible to peers; keep it even when its
            // informational content cannot be written.
            let _ =
                std::io::Write::write_all(&mut file, b"claimed\n").and_then(|()| file.sync_all());
            Ok(ClaimResult::Claimed)
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(ClaimResult::AlreadyClaimed),
        Err(error) => Err(error).at("create claim", path),
    }
}

#[derive(Debug, Clone)]
/// Sparse content-addressed journal with one pretty-printed JSON file per
/// record.
///
/// Producers share the journal lock, so they never wait for each other;
/// acknowledgement holds it exclusively only while removing captured files.
pub struct RecordJournal {
    directory: PathBuf,
    lock: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// SHA-256-derived identity of a journal entry.
pub struct JournalEntryId(String);

impl JournalEntryId {
    /// Returns the lowercase hexadecimal digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug)]
/// Typed record returned by a sparse-journal snapshot.
pub struct RecordJournalEntry<T> {
    id: JournalEntryId,
    value: T,
    captured: CapturedVersion,
}

impl<T> RecordJournalEntry<T> {
    /// Returns the content-derived entry identity.
    pub fn id(&self) -> &JournalEntryId {
        &self.id
    }

    /// Borrows the decoded entry value.
    pub fn value(&self) -> &T {
        &self.value
    }

    /// Consumes the entry and returns its value.
    pub fn into_value(self) -> T {
        self.value
    }
}

#[derive(Debug)]
/// Captured record whose JSON could not be decoded as the snapshot's type.
///
/// It may have been written by a newer producer or be corrupt. It stays
/// pending unless the batch is acknowledged with
/// [`RecordJournalBatch::acknowledge_including_undecodable`].
pub struct UndecodableRecord {
    id: JournalEntryId,
    error: String,
    captured: CapturedVersion,
}

/// The exact file version a snapshot read.
///
/// On Unix the open file of each of a batch's first [`PINNED_RECORDS`]
/// records is kept for as long as the batch exists. Its inode then stays
/// allocated even after a re-append replaces the path, so no later record
/// can be given the same device and inode numbers and be mistaken for the
/// captured version.
#[derive(Debug)]
struct CapturedVersion {
    identity: FileIdentity,
    #[cfg(unix)]
    _pin: Option<std::fs::File>,
}

/// How many captured records a batch keeps open.
///
/// Pinning every record of a large batch could exhaust a small descriptor
/// limit (256 is a common default) and then fail every snapshot, which would
/// leave the journal unconsumable. Records beyond this bound are identified
/// by device, inode, modification time, and length alone.
const PINNED_RECORDS: usize = 64;

impl UndecodableRecord {
    /// Returns the entry identity taken from its file name.
    pub fn id(&self) -> &JournalEntryId {
        &self.id
    }

    /// Returns the decode error message.
    pub fn error(&self) -> &str {
        &self.error
    }
}

#[derive(Debug)]
/// Point-in-time batch of sparse journal records.
///
/// A batch can be inspected and then acknowledged, which removes only the
/// exact file versions it captured while tolerating files already removed by
/// a peer. A record re-appended after the snapshot is a new occurrence and
/// stays pending even though it has the same entry ID.
///
/// On Unix a batch keeps one open file for each of its first 64 captured
/// records until it is acknowledged or dropped, which makes those captured
/// versions unambiguous. A version is otherwise identified by its device and
/// inode numbers (Unix only), modification time, and length, so a
/// byte-identical record re-appended within the filesystem's timestamp
/// granularity of the captured append, and given a reused inode, can be
/// acknowledged together with it.
pub struct RecordJournalBatch<T> {
    directory: PathBuf,
    lock: PathBuf,
    entries: Vec<RecordJournalEntry<T>>,
    undecodable: Vec<UndecodableRecord>,
}

impl RecordJournal {
    /// Writes a content-addressed JSON record.
    ///
    /// The entry ID hashes `event_key` and the pretty-printed JSON bytes.
    /// Repeating the same pair while the record is still pending replaces the
    /// same path atomically, so pending duplicates coalesce into one entry.
    /// Choose an `event_key` that identifies the occurrence when every
    /// occurrence must be processed separately.
    pub fn append<T: Serialize>(&self, event_key: &str, value: &T) -> Result<JournalEntryId> {
        let bytes = serde_json::to_vec_pretty(value)?;
        let id = JournalEntryId(sha256_bytes(&[
            event_key.as_bytes(),
            b"\0",
            bytes.as_slice(),
        ]));
        // The rename below always creates a new file version, which is what
        // lets an acknowledgement tell a re-append apart from the record it
        // captured.
        let _producer = FileLock::shared(&self.lock)?;
        atomic_replace(
            &self.directory.join(format!("{}.json", id.0)),
            &bytes,
            Durability::Durable,
        )?;
        Ok(id)
    }

    /// Captures and decodes all current records in deterministic ID order.
    ///
    /// Records removed by a concurrent acknowledgement are skipped. A record
    /// that cannot be decoded as `T` does not fail the snapshot; it is
    /// reported through [`RecordJournalBatch::undecodable`].
    pub fn snapshot<T: DeserializeOwned>(&self) -> Result<RecordJournalBatch<T>> {
        let mut entries = Vec::new();
        let mut undecodable = Vec::new();
        for path in json_files(&self.directory)? {
            let Some(id) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            let id = JournalEntryId(id.to_string());
            let pin = entries.len() + undecodable.len() < PINNED_RECORDS;
            let Some((captured, bytes)) = read_captured(&path, pin)? else {
                continue;
            };
            match serde_json::from_slice(&bytes) {
                Ok(value) => entries.push(RecordJournalEntry {
                    id,
                    value,
                    captured,
                }),
                Err(error) => undecodable.push(UndecodableRecord {
                    id,
                    error: error.to_string(),
                    captured,
                }),
            }
        }
        Ok(RecordJournalBatch {
            directory: self.directory.clone(),
            lock: self.lock.clone(),
            entries,
            undecodable,
        })
    }
}

/// Opens a record once and returns that exact file version and its bytes, or
/// `None` when it has already been removed. With `pin`, the version keeps the
/// file open.
fn read_captured(path: &Path, pin: bool) -> Result<Option<(CapturedVersion, Vec<u8>)>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).at("open journal record", path),
    };
    let identity = FileIdentity::of(&file.metadata().at("inspect journal record", path)?);
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .at("read journal record", path)?;
    #[cfg(not(unix))]
    let _ = pin;
    let captured = CapturedVersion {
        identity,
        #[cfg(unix)]
        _pin: pin.then_some(file),
    };
    Ok(Some((captured, bytes)))
}

impl<T> RecordJournalBatch<T> {
    /// Returns the captured, decoded entries in deterministic ID order.
    pub fn entries(&self) -> &[RecordJournalEntry<T>] {
        &self.entries
    }

    /// Returns captured records that could not be decoded.
    pub fn undecodable(&self) -> &[UndecodableRecord] {
        &self.undecodable
    }

    /// Reports whether the batch contains no decoded entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the number of captured, decoded entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Removes every decoded record captured by this batch.
    ///
    /// Only the exact captured file versions are removed: a record re-appended
    /// after the snapshot stays pending. Missing files are treated as already
    /// acknowledged, and undecodable records stay pending. The operation stops
    /// at the first other filesystem error and may therefore be partially
    /// applied.
    pub fn acknowledge(self) -> Result<()> {
        let targets = self
            .entries
            .iter()
            .map(|entry| (&entry.id, &entry.captured.identity))
            .collect::<Vec<_>>();
        remove_captured(&self.directory, &self.lock, &targets)
    }

    /// Removes every captured record, including undecodable ones.
    ///
    /// Use this to discard records that this consumer can never process.
    pub fn acknowledge_including_undecodable(self) -> Result<()> {
        let targets = self
            .entries
            .iter()
            .map(|entry| (&entry.id, &entry.captured.identity))
            .chain(
                self.undecodable
                    .iter()
                    .map(|record| (&record.id, &record.captured.identity)),
            )
            .collect::<Vec<_>>();
        remove_captured(&self.directory, &self.lock, &targets)
    }
}

fn remove_captured(
    directory: &Path,
    lock: &Path,
    targets: &[(&JournalEntryId, &FileIdentity)],
) -> Result<()> {
    if targets.is_empty() {
        return Ok(());
    }
    // Producers hold this lock shared while they replace a record, so no
    // re-append can land between the identity check and the removal.
    let _consumer = FileLock::exclusive(lock)?;
    for (id, identity) in targets {
        let path = directory.join(format!("{}.json", id.0));
        let current = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => FileIdentity::of(&metadata),
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(error).at("inspect journal record", &path),
        };
        if current != **identity {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error).at("remove journal record", &path),
        }
    }
    Ok(())
}

#[derive(Debug)]
/// Directory for artifacts produced by one logical hook/tool run.
///
/// A bundle becomes committed when `summary.json` is written. Dropping an
/// uncommitted bundle does not delete it, allowing postmortem inspection.
pub struct RunBundle {
    directory: PathBuf,
    committed: bool,
}

impl RunBundle {
    /// Returns the unique run directory.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Atomically writes UTF-8 text at a safe relative path, replacing any
    /// earlier content.
    pub fn write_text(&self, relative: impl AsRef<Path>, content: &str) -> Result<PathBuf> {
        self.write_bytes(relative, content.as_bytes())
    }

    /// Pretty-prints JSON and atomically writes it at a safe relative path,
    /// replacing any earlier content.
    pub fn write_json<T: Serialize>(
        &self,
        relative: impl AsRef<Path>,
        value: &T,
    ) -> Result<PathBuf> {
        self.write_bytes(relative, &serde_json::to_vec_pretty(value)?)
    }

    /// Writes `summary.json`, marking the run committed, and consumes the
    /// bundle handle.
    pub fn commit<T: Serialize>(mut self, summary: &T) -> Result<PathBuf> {
        let path = self.write_json("summary.json", summary)?;
        self.committed = true;
        Ok(path)
    }

    /// Reports whether `summary.json` exists or this handle committed it.
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
        atomic_replace(&path, bytes, Durability::Durable)?;
        Ok(path)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
/// Counts from one session-state garbage-collection pass.
pub struct GcReport {
    /// Number of session directories whose activity was inspected.
    pub scanned: usize,
    /// Number of stale session directories removed.
    pub removed: usize,
    /// Number of sessions or leftover trash directories that could not be
    /// inspected or removed; the pass continued past them.
    pub failed: usize,
}

fn write_observation<T: Serialize>(directory: &Path, observation: &T) -> Result<PathBuf> {
    let bytes = serde_json::to_vec_pretty(observation)?;
    let path = directory.join(format!("{}.json", sha256_bytes(&[&bytes])));
    publish_if_absent(&path, &bytes)?;
    Ok(path)
}

fn read_observations<T: DeserializeOwned>(directory: &Path) -> Result<Vec<T>> {
    let mut values = Vec::new();
    for path in json_files(directory)? {
        let Some(bytes) = read_optional(&path)? else {
            continue;
        };
        if let Ok(value) = serde_json::from_slice(&bytes) {
            values.push(value);
        }
    }
    Ok(values)
}

/// Lists `*.json` files in name order; a missing directory is empty.
pub(crate) fn json_files(directory: &Path) -> Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).at("list directory", directory),
    };
    let mut files = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    files.sort();
    Ok(files)
}

fn prepare_root(root: &StateRoot) -> Result<()> {
    #[cfg(unix)]
    if let Some(base) = &root.per_user_base {
        ensure_owned_private_dir(base)?;
        verify_per_user_descendants(base, root.path(), true)?;
    }
    let path = root.path();
    match std::fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                create_private_dir_all(parent)?;
            }
            match create_private_dir(path) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error).at("create state root", path),
            }
        }
        Err(error) => return Err(error).at("inspect state root", path),
    }
    let metadata = std::fs::symlink_metadata(path).at("inspect state root", path)?;
    if metadata.file_type().is_symlink() {
        return Err(StateError::SymlinkStateRoot(path.to_path_buf()));
    }
    if !metadata.is_dir() {
        return Err(StateError::Io {
            operation: "use state root",
            path: path.to_path_buf(),
            source: std::io::Error::new(ErrorKind::NotADirectory, "not a directory"),
        });
    }
    Ok(())
}

/// Checks an existing root before garbage collection deletes anything in it.
fn verify_root(root: &StateRoot) -> Result<()> {
    #[cfg(unix)]
    if let Some(base) = &root.per_user_base {
        ensure_owned_private_dir(base)?;
        verify_per_user_descendants(base, root.path(), false)?;
    }
    let path = root.path();
    if std::fs::symlink_metadata(path)
        .at("inspect state root", path)?
        .file_type()
        .is_symlink()
    {
        return Err(StateError::SymlinkStateRoot(path.to_path_buf()));
    }
    Ok(())
}

/// Creates or validates the per-user directory that isolates a per-user
/// root inside a possibly shared temporary directory.
#[cfg(unix)]
fn ensure_owned_private_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    match create_private_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) if error.kind() == ErrorKind::NotFound => create_private_dir_all(path)?,
        Err(error) => return Err(error).at("create state directory", path),
    }
    let metadata = owned_directory_metadata(path)?;
    if metadata.mode() & 0o077 != 0 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .at("restrict permissions of", path)?;
    }
    Ok(())
}

/// Checks every directory below the per-user `base` down to `path`.
///
/// Once `base` is owner-only, no other user can create anything inside it,
/// but a directory planted while an earlier `base` was still group- or
/// world-writable would otherwise be trusted. With `create`, missing
/// directories are created owner-only on the way down; without it, checking
/// stops at the first missing one.
#[cfg(unix)]
fn verify_per_user_descendants(base: &Path, path: &Path, create: bool) -> Result<()> {
    let relative = path
        .strip_prefix(base)
        .map_err(|_| StateError::InvalidRelativePath(path.to_path_buf()))?;
    let mut current = base.to_path_buf();
    for component in relative.components() {
        current.push(component);
        if create {
            match create_private_dir(&current) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error).at("create state directory", &current),
            }
        } else if !entry_exists(&current)? {
            return Ok(());
        }
        owned_directory_metadata(&current)?;
    }
    Ok(())
}

/// Returns the metadata of `path` after checking that it is a real directory
/// owned by the effective user.
#[cfg(unix)]
fn owned_directory_metadata(path: &Path) -> Result<std::fs::Metadata> {
    use std::os::unix::fs::MetadataExt;

    let metadata = std::fs::symlink_metadata(path).at("inspect state directory", path)?;
    if metadata.file_type().is_symlink() {
        return Err(StateError::SymlinkStateRoot(path.to_path_buf()));
    }
    if !metadata.is_dir() {
        return Err(StateError::UnsafeStateRoot {
            path: path.to_path_buf(),
            reason: "not a directory".to_string(),
        });
    }
    let uid = current_uid();
    if metadata.uid() != uid {
        return Err(StateError::UnsafeStateRoot {
            path: path.to_path_buf(),
            reason: format!(
                "owned by uid {} rather than the current user (uid {uid})",
                metadata.uid()
            ),
        });
    }
    Ok(metadata)
}

/// Lists real subdirectories in name order; a missing directory is empty.
fn read_dirs(path: &Path) -> Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).at("list directory", path),
    };
    let mut paths = entries
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

/// Removes one stale session, returning `false` when it is still active or a
/// concurrent pass already removed it.
fn collect_session(session: &Path, cutoff: SystemTime, trash: &Path) -> Result<bool> {
    // Openers refresh the activity stamp while holding this lock, so once it
    // is held no opener is between taking it and stamping the session.
    let lock_path = metadata::lock_path(session);
    let lock = match FileLock::acquire(&lock_path, LockMode::Exclusive, LockWait::Try) {
        // A hook is opening or reading the session right now.
        Ok(None) => return Ok(false),
        Ok(Some(lock)) => Some(lock),
        // The session was removed concurrently, or it predates the metadata
        // directory and no opener has created it yet. Opening such a session
        // creates that directory, which refreshes the session directory's
        // own modification time.
        Err(error)
            if error
                .io_error()
                .is_some_and(|error| error.kind() == ErrorKind::NotFound) =>
        {
            None
        }
        Err(error) => return Err(error),
    };
    let newest = match newest_activity(session) {
        Ok(newest) => newest,
        Err(error)
            if error
                .io_error()
                .is_some_and(|error| error.kind() == ErrorKind::NotFound) =>
        {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    if newest >= cutoff {
        return Ok(false);
    }
    create_private_dir_all(trash)?;
    let target = trash.join(unique_id());
    // Windows cannot rename a directory while a file inside it is open, so
    // the lock is released first there, leaving a narrow window in which an
    // opener can still lose the session.
    #[cfg(not(unix))]
    drop(lock);
    match std::fs::rename(session, &target) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error).at("move stale session", session),
    }
    // An opener waiting for the lock notices that the lock file moved and
    // starts over in a fresh session directory.
    #[cfg(unix)]
    drop(lock);
    // The session is no longer reachable; a failed delete leaves trash that
    // the next pass retries.
    let _ = std::fs::remove_dir_all(&target);
    Ok(true)
}

/// Keeps the session in `directory` visibly alive for [`SessionState::gc`]
/// even when no family is opened and the materialized metadata does not
/// change. Callers hold the session's exclusive metadata lock.
pub(crate) fn record_session_activity(directory: &Path) -> Result<()> {
    let activity = directory.join("activity");
    create_private_dir_all(&activity)?;
    touch_activity(&activity.join(SESSION_ACTIVITY_STAMP))
}

fn newest_activity(session: &Path) -> Result<SystemTime> {
    let mut newest = std::fs::metadata(session)
        .at("inspect session", session)?
        .modified()
        .unwrap_or(UNIX_EPOCH);
    let activity = session.join("activity");
    let entries = match std::fs::read_dir(&activity) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(newest),
        Err(error) => return Err(error).at("list activity", &activity),
    };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified()) else {
            continue;
        };
        newest = newest.max(modified);
    }
    Ok(newest)
}

#[cfg(test)]
mod tests;
