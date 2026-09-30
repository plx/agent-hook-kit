//! Private filesystem plumbing shared by the session-state primitives.
//!
//! Every helper attaches the failed operation and path to I/O errors, so a
//! hook's stderr names the file that could not be read or written.

use crate::{Result, StateError};
use fs2::FileExt;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread::ThreadId;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);
static LOCK_TOKENS: AtomicU64 = AtomicU64::new(0);
static HELD_LOCKS: Mutex<Vec<HeldLock>> = Mutex::new(Vec::new());

/// Minimum age before an activity stamp is rewritten.
pub(crate) const ACTIVITY_REFRESH: Duration = Duration::from_secs(30);

/// Attaches an operation and path to a raw I/O result.
pub(crate) trait IoContext<T> {
    /// Converts an I/O failure into [`StateError::Io`] naming `path`.
    fn at(self, operation: &'static str, path: &Path) -> Result<T>;
}

impl<T> IoContext<T> for std::io::Result<T> {
    fn at(self, operation: &'static str, path: &Path) -> Result<T> {
        self.map_err(|source| StateError::Io {
            operation,
            path: path.to_path_buf(),
            source,
        })
    }
}

/// How strongly a replacement must survive power loss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Durability {
    /// Source-of-truth state: sync the content before it becomes visible and
    /// sync the directory entry afterwards.
    Durable,
    /// Derived state that every reader can rebuild or tolerate losing. It is
    /// still published atomically, so a killed process never leaves a torn
    /// file, but it is not forced to stable storage.
    Disposable,
}

/// Atomically replaces `path` with `bytes`.
///
/// Callers serialize replacements of mutable state through the owning
/// primitive's lock; concurrent replacements are last-writer-wins.
pub(crate) fn atomic_replace(path: &Path, bytes: &[u8], durability: Durability) -> Result<()> {
    let parent = parent_of(path)?;
    create_private_dir_all(parent)?;
    let temporary = temporary_path(parent);
    let result = write_temporary(&temporary, bytes, durability)
        .and_then(|()| rename_replacing(&temporary, path))
        .and_then(|()| match durability {
            Durability::Durable => sync_directory(parent),
            Durability::Disposable => Ok(()),
        });
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// Publishes immutable content at `path` unless an entry already exists.
///
/// Returns `true` when this call published the file. The complete, synced
/// content becomes visible in one step through a hard link, which fails
/// rather than replacing an existing entry. On filesystems without hard links
/// the content is renamed into place instead; that fallback may replace a
/// concurrently published entry. Content-addressed callers tolerate that
/// because both writers publish identical bytes; a caller whose concurrent
/// writers may publish different bytes must serialize publication under a
/// lock that every writer takes.
pub(crate) fn publish_if_absent(path: &Path, bytes: &[u8]) -> Result<bool> {
    if entry_exists(path)? {
        return Ok(false);
    }
    let parent = parent_of(path)?;
    create_private_dir_all(parent)?;
    let temporary = temporary_path(parent);
    let result = write_temporary(&temporary, bytes, Durability::Durable).and_then(|()| {
        match hard_link(&temporary, path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(false),
            Err(_) => rename_replacing(&temporary, path).map(|()| true),
        }
    });
    // After a successful link the temporary name is redundant; after the
    // rename fallback it no longer exists.
    let _ = std::fs::remove_file(&temporary);
    let published = result?;
    if published {
        sync_directory(parent)?;
    }
    Ok(published)
}

#[cfg(not(test))]
fn hard_link(original: &Path, link: &Path) -> std::io::Result<()> {
    std::fs::hard_link(original, link)
}

#[cfg(test)]
thread_local! {
    /// Makes this thread's `publish_if_absent` behave as on a filesystem
    /// without hard links (FAT, exFAT, some SMB and FUSE mounts).
    pub(crate) static WITHOUT_HARD_LINKS: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn hard_link(original: &Path, link: &Path) -> std::io::Result<()> {
    if WITHOUT_HARD_LINKS.with(std::cell::Cell::get) {
        return Err(std::io::Error::from(ErrorKind::Unsupported));
    }
    std::fs::hard_link(original, link)
}

/// Reports whether any directory entry, including a dangling symlink, exists.
pub(crate) fn entry_exists(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).at("inspect", path),
    }
}

/// Reads a whole file, returning `None` when it does not exist.
pub(crate) fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).at("read", path),
    }
}

/// Decodes stored JSON, naming the file on failure.
pub(crate) fn decode<T: DeserializeOwned>(path: &Path, bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|source| StateError::Decode {
        path: path.to_path_buf(),
        source,
    })
}

/// Removes a file, treating an already-missing file as success.
pub(crate) fn remove_if_present(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).at("remove", path),
    }
}

/// Syncs a directory so a completed rename or unlink survives power loss.
pub(crate) fn sync_directory(directory: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        File::open(directory)
            .and_then(|handle| handle.sync_all())
            .at("sync directory", directory)
    }
    #[cfg(not(unix))]
    {
        let _ = directory;
        Ok(())
    }
}

/// Creates `path` and any missing ancestors as owner-only directories.
///
/// Existing directories are left untouched; in particular their permissions
/// are never changed.
pub(crate) fn create_private_dir_all(path: &Path) -> Result<()> {
    private_dir_builder(true)
        .create(path)
        .at("create directory", path)
}

/// Creates exactly `path` as an owner-only directory.
pub(crate) fn create_private_dir(path: &Path) -> std::io::Result<()> {
    private_dir_builder(false).create(path)
}

fn private_dir_builder(recursive: bool) -> std::fs::DirBuilder {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(recursive);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
}

/// Refreshes a garbage-collection activity stamp.
///
/// The stamp's modification time is its only meaning, so it is not synced,
/// and a stamp refreshed within the last few seconds is left alone.
pub(crate) fn touch_activity(path: &Path) -> Result<()> {
    let fresh = std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age < ACTIVITY_REFRESH);
    if fresh {
        return Ok(());
    }
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .and_then(|mut file| file.write_all(timestamp_millis().to_string().as_bytes()))
        .at("write activity stamp", path)
}

/// Stable identity of one file version, used to tell a captured file apart
/// from a later replacement at the same path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    modified: Option<SystemTime>,
    length: u64,
}

impl FileIdentity {
    /// Captures the identity reported by `metadata`.
    pub(crate) fn of(metadata: &std::fs::Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Self {
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            modified: metadata.modified().ok(),
            length: metadata.len(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LockMode {
    Shared,
    Exclusive,
}

/// How long an acquisition may wait for a contended lock.
#[derive(Debug, Clone, Copy)]
pub(crate) enum LockWait {
    /// Wait indefinitely.
    Block,
    /// Return immediately when contended.
    Try,
    /// Poll until the deadline passes.
    Until(Instant),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LockKey {
    #[cfg(unix)]
    Inode { device: u64, inode: u64 },
    #[cfg(not(unix))]
    Path(PathBuf),
}

#[derive(Debug)]
struct HeldLock {
    token: u64,
    key: LockKey,
    thread: ThreadId,
    mode: LockMode,
}

fn held_locks() -> MutexGuard<'static, Vec<HeldLock>> {
    HELD_LOCKS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Advisory file lock that detects same-thread re-entry.
///
/// Advisory locks belong to an open file description, so a second
/// acquisition of the same lock file by the thread that already holds it
/// would wait for itself forever. A process-local registry turns that
/// deadlock into [`StateError::LockReentry`]. Other threads and processes
/// still wait normally. Re-entry detection assumes a guard is dropped on the
/// thread that acquired it.
#[derive(Debug)]
pub(crate) struct FileLock {
    file: File,
    token: u64,
}

impl FileLock {
    /// Waits for an exclusive lock on `path`.
    pub(crate) fn exclusive(path: &Path) -> Result<Self> {
        Self::acquire(path, LockMode::Exclusive, LockWait::Block)?
            .ok_or_else(|| unreachable_contention(path))
    }

    /// Waits for a shared lock on `path`.
    pub(crate) fn shared(path: &Path) -> Result<Self> {
        Self::acquire(path, LockMode::Shared, LockWait::Block)?
            .ok_or_else(|| unreachable_contention(path))
    }

    /// Acquires a lock on `path`, returning `None` when `wait` expires first.
    pub(crate) fn acquire(path: &Path, mode: LockMode, wait: LockWait) -> Result<Option<Self>> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .at("open lock", path)?;
        let key = lock_key(&file, path)?;
        let thread = std::thread::current().id();
        if held_locks().iter().any(|held| {
            held.key == key
                && held.thread == thread
                && (mode == LockMode::Exclusive || held.mode == LockMode::Exclusive)
        }) {
            return Err(StateError::LockReentry(path.to_path_buf()));
        }
        let acquired = match wait {
            LockWait::Block => {
                match mode {
                    LockMode::Shared => FileExt::lock_shared(&file),
                    LockMode::Exclusive => FileExt::lock_exclusive(&file),
                }
                .at("acquire lock", path)?;
                true
            }
            LockWait::Try => try_lock(&file, mode, path)?,
            LockWait::Until(deadline) => {
                let mut delay = Duration::from_millis(1);
                loop {
                    if try_lock(&file, mode, path)? {
                        break true;
                    }
                    let now = Instant::now();
                    if now >= deadline {
                        break false;
                    }
                    std::thread::sleep(delay.min(deadline - now));
                    delay = (delay * 2).min(Duration::from_millis(50));
                }
            }
        };
        if !acquired {
            return Ok(None);
        }
        let token = LOCK_TOKENS.fetch_add(1, Ordering::Relaxed);
        held_locks().push(HeldLock {
            token,
            key,
            thread,
            mode,
        });
        Ok(Some(Self { file, token }))
    }

    /// Reports whether `path` still names the locked file.
    ///
    /// A lock protects the file it was taken on. When that file has since
    /// been moved or replaced, for example because garbage collection moved
    /// its whole session directory, a process that opens `path` now locks a
    /// different file and is not excluded by this lock. Only Unix exposes a
    /// file identity to compare; elsewhere this always reports `true`.
    pub(crate) fn is_current(&self, path: &Path) -> Result<bool> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let locked = self.file.metadata().at("inspect lock", path)?;
            match std::fs::metadata(path) {
                Ok(current) => Ok(current.dev() == locked.dev() && current.ino() == locked.ino()),
                Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
                Err(error) => Err(error).at("inspect lock", path),
            }
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Ok(true)
        }
    }

    /// Releases the lock now, reporting an unlock failure.
    pub(crate) fn release(self, path: &Path) -> Result<()> {
        // Dropping afterwards unlocks again, which is harmless, and removes
        // the registry entry.
        FileExt::unlock(&self.file).at("release lock", path)
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
        held_locks().retain(|held| held.token != self.token);
    }
}

fn unreachable_contention(path: &Path) -> StateError {
    StateError::Io {
        operation: "acquire lock",
        path: path.to_path_buf(),
        source: std::io::Error::other("blocking lock acquisition returned without a lock"),
    }
}

fn try_lock(file: &File, mode: LockMode, path: &Path) -> Result<bool> {
    let result = match mode {
        LockMode::Shared => FileExt::try_lock_shared(file),
        LockMode::Exclusive => FileExt::try_lock_exclusive(file),
    };
    match result {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == fs2::lock_contended_error().kind() => Ok(false),
        Err(error) => Err(error).at("acquire lock", path),
    }
}

#[cfg(unix)]
fn lock_key(file: &File, path: &Path) -> Result<LockKey> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata().at("inspect lock", path)?;
    Ok(LockKey::Inode {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(not(unix))]
fn lock_key(_file: &File, path: &Path) -> Result<LockKey> {
    Ok(LockKey::Path(
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
    ))
}

/// Returns a process-unique, roughly time-ordered identifier.
pub(crate) fn unique_id() -> String {
    format!(
        "{}-{}-{}",
        timestamp_millis(),
        std::process::id(),
        UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

pub(crate) fn timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub(crate) fn sha256(value: &str) -> String {
    sha256_bytes(&[value.as_bytes()])
}

pub(crate) fn sha256_bytes(parts: &[&[u8]]) -> String {
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

/// Validates a path-safe identifier that may use either ASCII case.
///
/// Used for harness identifiers and run labels, which never collide on a
/// case-insensitive filesystem because they are either library-defined or
/// prefixed by a unique ID.
pub(crate) fn validate_identifier(value: &str) -> Result<()> {
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

/// Validates a lowercase coordination name.
///
/// Family, entity, claim-set, journal, lock, and custom-scope names become
/// directory or file names. Default macOS and Windows filesystems compare
/// names case-insensitively, so two names differing only in case would share
/// state there while staying separate on Linux; uppercase is rejected so the
/// layout means the same thing everywhere.
pub(crate) fn validate_name(value: &str) -> Result<()> {
    validate_identifier(value)?;
    if value
        .chars()
        .any(|character| character.is_ascii_uppercase())
    {
        return Err(StateError::InvalidIdentifier(value.to_string()));
    }
    Ok(())
}

pub(crate) fn validate_relative_path(path: &Path) -> Result<()> {
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

fn parent_of(path: &Path) -> Result<&Path> {
    path.parent()
        .ok_or_else(|| StateError::InvalidRelativePath(path.to_path_buf()))
}

fn temporary_path(parent: &Path) -> PathBuf {
    parent.join(format!(".tmp-{}", unique_id()))
}

fn write_temporary(path: &Path, bytes: &[u8], durability: Durability) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .at("create temporary file", path)?;
    file.write_all(bytes).at("write temporary file", path)?;
    if durability == Durability::Durable {
        file.sync_all().at("sync temporary file", path)?;
    }
    Ok(())
}

fn rename_replacing(from: &Path, to: &Path) -> Result<()> {
    // `std::fs::rename` replaces an existing destination on every supported
    // platform, including Windows. There it can fail with a sharing violation,
    // reported as `PermissionDenied`, while another process (an editor or a
    // virus scanner) holds the destination open without FILE_SHARE_DELETE.
    // Retrying briefly is safer than deleting the destination first, which
    // would leave a crash window in which the file does not exist at all.
    #[cfg(windows)]
    {
        let mut delay = Duration::from_millis(2);
        for _ in 0..8 {
            match std::fs::rename(from, to) {
                Err(error) if error.kind() == ErrorKind::PermissionDenied => {
                    std::thread::sleep(delay);
                    delay *= 2;
                }
                result => return result.at("replace", to),
            }
        }
    }
    std::fs::rename(from, to).at("replace", to)
}
