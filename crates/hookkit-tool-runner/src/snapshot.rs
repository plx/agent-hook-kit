//! Content snapshots used to attribute writes to one command.
//!
//! A snapshot stores metadata and a streaming SHA-256 digest per file instead
//! of full file contents, so workspace-scoped tools no longer hold two copies
//! of every workspace byte in memory. The post-command recapture reuses the
//! earlier digest for files whose metadata is unchanged and hashes only files
//! whose metadata moved; comparison is by content, so a tool that rewrites a
//! file with identical bytes is not reported as having changed it.
//!
//! Metadata identity only proves a file unchanged when a later write would
//! have produced a different timestamp. On filesystems with coarse clocks
//! (FAT's two seconds; one second on HFS+, ext3, and many network mounts) a
//! same-length rewrite in the same tick as the earlier hash leaves every stamp
//! field equal, so a digest is reused only for files whose timestamps lie
//! more than [`COARSE_TIMESTAMP_WINDOW`] before it was computed. In practice
//! that re-hashes just the files written moments before the capture, such as
//! the agent's own edit.

use crate::exec::{FileMatcher, ToolContext, ToolJob};
use crate::spec::{FileSelection, WriteBehavior};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Coarsest timestamp granularity the digest reuse tolerates: FAT records
/// modification times in two-second steps.
const COARSE_TIMESTAMP_WINDOW: Duration = Duration::from_secs(2);

/// Directory names every snapshot walk prunes in addition to
/// `fileActivity.ignoredDirectoryNames`: version-control metadata, the hook's
/// own configuration and diagnostics, and the dependency and build output
/// directories the runner has always skipped. That list tunes Stop
/// reconciliation and replaces its defaults when set, so without this floor a
/// layer listing only `dist` would make every workspace-scoped snapshot hash
/// `.git` and `node_modules`.
const ALWAYS_PRUNED_DIRECTORY_NAMES: &[&str] = &[
    ".agent-hook-kit",
    ".git",
    ".hg",
    ".jj",
    ".svn",
    "node_modules",
    "target",
];

/// Metadata identity used to decide whether a file must be re-hashed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MetadataStamp {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    changed: (i64, i64),
    #[cfg(unix)]
    inode: (u64, u64),
}

impl MetadataStamp {
    fn of(metadata: &std::fs::Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt as _;
        Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            changed: (metadata.ctime(), metadata.ctime_nsec()),
            #[cfg(unix)]
            inode: (metadata.dev(), metadata.ino()),
        }
    }
}

impl MetadataStamp {
    /// Whether every timestamp lies more than [`COARSE_TIMESTAMP_WINDOW`]
    /// before `instant`, so any write after `instant` must change the stamp
    /// even on a filesystem with coarse timestamps.
    fn settled_before(&self, instant: SystemTime) -> bool {
        let Some(limit) = instant.checked_sub(COARSE_TIMESTAMP_WINDOW) else {
            return false;
        };
        let modified = self.modified.is_some_and(|modified| modified < limit);
        #[cfg(unix)]
        let changed = unix_time(self.changed).is_none_or(|changed| changed < limit);
        #[cfg(not(unix))]
        let changed = true;
        modified && changed
    }
}

/// A `(seconds, nanoseconds)` Unix timestamp; `None` before the epoch.
#[cfg(unix)]
fn unix_time((seconds, nanoseconds): (i64, i64)) -> Option<SystemTime> {
    let seconds = u64::try_from(seconds).ok()?;
    let nanoseconds = u32::try_from(nanoseconds).unwrap_or(0).min(999_999_999);
    SystemTime::UNIX_EPOCH.checked_add(Duration::new(seconds, nanoseconds))
}

#[derive(Debug, Clone)]
struct FileStamp {
    metadata: MetadataStamp,
    digest: [u8; 32],
    /// When hashing started; the digest is reused only for files whose
    /// timestamps had settled by then.
    hashed_at: SystemTime,
}

impl FileStamp {
    /// Whether this digest still describes a file whose metadata is now
    /// `current`.
    fn reusable_for(&self, current: &MetadataStamp) -> bool {
        self.metadata == *current && current.settled_before(self.hashed_at)
    }
}

#[derive(Debug, Default)]
pub(crate) struct Snapshot {
    files: BTreeMap<PathBuf, Option<FileStamp>>,
}

impl Snapshot {
    /// Capture metadata and a content digest for every path.
    pub(crate) fn capture(paths: &BTreeSet<PathBuf>) -> Self {
        Self::capture_with(paths, None)
    }

    /// Capture `paths` after a command, reusing this snapshot's digest for
    /// files whose metadata did not change.
    pub(crate) fn recapture(&self, paths: &BTreeSet<PathBuf>) -> Self {
        Self::capture_with(paths, Some(self))
    }

    fn capture_with(paths: &BTreeSet<PathBuf>, previous: Option<&Self>) -> Self {
        let files = paths
            .iter()
            .map(|path| {
                let previous = previous
                    .and_then(|snapshot| snapshot.files.get(path))
                    .and_then(Option::as_ref);
                (path.clone(), stamp(path, previous))
            })
            .collect();
        Self { files }
    }

    /// Paths whose existence or content differs between the two snapshots.
    pub(crate) fn changed_files(&self, after: &Self) -> Vec<PathBuf> {
        self.files
            .keys()
            .chain(after.files.keys())
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter(|path| {
                let content = |snapshot: &Self| {
                    snapshot
                        .files
                        .get(path)
                        .and_then(Option::as_ref)
                        .map(|stamp| (stamp.metadata.len, stamp.digest))
                };
                content(self) != content(after)
            })
            .collect()
    }
}

fn stamp(path: &Path, previous: Option<&FileStamp>) -> Option<FileStamp> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let metadata = MetadataStamp::of(&metadata);
    if let Some(previous) = previous.filter(|previous| previous.reusable_for(&metadata)) {
        return Some(previous.clone());
    }
    let hashed_at = SystemTime::now();
    let digest = digest_file(path)?;
    Some(FileStamp {
        metadata,
        digest,
        hashed_at,
    })
}

fn digest_file(path: &Path) -> Option<[u8; 32]> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => hasher.update(&buffer[..read]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    Some(hasher.finalize().into())
}

/// Union of the write scopes declared by the tool's enabled immediate phases.
pub(crate) fn snapshot_scope(job: &ToolJob, context: &ToolContext<'_>) -> BTreeSet<PathBuf> {
    let mut scope = BTreeSet::new();
    let mut include_target_files = false;
    let mut include_matching_globs = false;
    let mut include_workspace = false;

    for phase in &context.spec.phases {
        if !phase.enabled {
            continue;
        }
        match phase.writes {
            WriteBehavior::None => {}
            WriteBehavior::TargetFiles => include_target_files = true,
            WriteBehavior::MatchingGlobs => include_matching_globs = true,
            WriteBehavior::Workspace => include_workspace = true,
        }
    }

    if include_target_files {
        scope.extend(job.files.iter().cloned());
    }
    if include_matching_globs {
        scope.extend(collect_matching_files(job, context));
    }
    if include_workspace {
        scope.extend(collect_workspace_files(job, context));
    }
    scope
}

/// Workspace files selected by the tool's globs, evaluated relative to the
/// project root exactly as candidate selection is.
pub(crate) fn collect_matching_files(
    job: &ToolJob,
    context: &ToolContext<'_>,
) -> BTreeSet<PathBuf> {
    matching_files(
        &job.workspace_dir,
        &context.spec.file_selection,
        context.project_root,
        &context.settings.ignored_directory_names,
    )
}

fn matching_files(
    base: &Path,
    selection: &FileSelection,
    project_root: &Path,
    ignored_directory_names: &BTreeSet<String>,
) -> BTreeSet<PathBuf> {
    let matcher = match FileMatcher::new(selection) {
        Ok(matcher) => matcher,
        Err(_) => return BTreeSet::new(),
    };
    walk_files(base, ignored_directory_names)
        .into_iter()
        .filter(|path| matcher.matches(path, project_root))
        .collect()
}

pub(crate) fn collect_workspace_files(
    job: &ToolJob,
    context: &ToolContext<'_>,
) -> BTreeSet<PathBuf> {
    walk_files(
        &job.workspace_dir,
        &context.settings.ignored_directory_names,
    )
    .into_iter()
    .collect()
}

/// Regular files below `base`, pruning configured ignored directory names
/// (`fileActivity.ignoredDirectoryNames`) and [`ALWAYS_PRUNED_DIRECTORY_NAMES`]
/// below the walk root.
fn walk_files(base: &Path, ignored_directory_names: &BTreeSet<String>) -> Vec<PathBuf> {
    walkdir::WalkDir::new(base)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 || !entry.file_type().is_dir() {
                return true;
            }
            let name = entry.file_name().to_string_lossy();
            !ignored_directory_names.contains(name.as_ref())
                && !ALWAYS_PRUNED_DIRECTORY_NAMES.contains(&name.as_ref())
        })
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.path().to_path_buf())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "hookkit-snapshot-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn snapshots_compare_content_not_metadata() {
        let root = temp_root("content");
        let same = root.join("same.txt");
        let changed = root.join("changed.txt");
        let created = root.join("created.txt");
        let deleted = root.join("deleted.txt");
        std::fs::write(&same, "same").unwrap();
        std::fs::write(&changed, "before").unwrap();
        std::fs::write(&deleted, "gone").unwrap();
        let scope = BTreeSet::from([
            same.clone(),
            changed.clone(),
            created.clone(),
            deleted.clone(),
        ]);
        let before = Snapshot::capture(&scope);

        // Identical rewrite: metadata moves, content does not.
        std::fs::write(&same, "same").unwrap();
        // Same-length rewrite: only the digest can detect it.
        std::fs::write(&changed, "after!").unwrap();
        std::fs::write(&created, "new").unwrap();
        std::fs::remove_file(&deleted).unwrap();

        let after = before.recapture(&scope);
        assert_eq!(
            before.changed_files(&after),
            vec![changed, created, deleted]
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// On a filesystem with one- or two-second timestamps, a same-length
    /// rewrite in the same tick as the earlier hash leaves every stamp field
    /// equal; only a digest taken after the timestamps settled is reusable.
    #[test]
    fn digests_are_reused_only_for_settled_timestamps() {
        let now = SystemTime::now();
        let metadata = |age: Duration| MetadataStamp {
            len: 5,
            modified: Some(now - age),
            #[cfg(unix)]
            changed: {
                let since_epoch = (now - age).duration_since(SystemTime::UNIX_EPOCH).unwrap();
                (
                    i64::try_from(since_epoch.as_secs()).unwrap(),
                    i64::from(since_epoch.subsec_nanos()),
                )
            },
            #[cfg(unix)]
            inode: (1, 2),
        };
        let stamp = |age: Duration| FileStamp {
            metadata: metadata(age),
            digest: [0; 32],
            hashed_at: now,
        };

        // Written moments before the hash: a rewrite in the same coarse tick
        // could keep the stamp, so the file is hashed again.
        let fresh = stamp(Duration::from_millis(500));
        assert!(!fresh.reusable_for(&metadata(Duration::from_millis(500))));
        // Settled long before the hash: any later write moves a timestamp.
        let settled = stamp(Duration::from_secs(10));
        assert!(settled.reusable_for(&metadata(Duration::from_secs(10))));
        // Metadata that moved is never reused.
        assert!(!settled.reusable_for(&metadata(Duration::from_secs(9))));
        // A timestamp from a clock ahead of ours is never trusted.
        let future = FileStamp {
            metadata: MetadataStamp {
                modified: Some(now + Duration::from_secs(60)),
                ..metadata(Duration::from_secs(10))
            },
            ..stamp(Duration::from_secs(10))
        };
        assert!(!future.reusable_for(&future.metadata.clone()));
    }

    #[test]
    fn freshly_written_files_are_rehashed_even_with_unchanged_metadata() {
        let root = temp_root("fresh");
        let file = root.join("a.txt");
        std::fs::write(&file, "aaaa\n").unwrap();
        let scope = BTreeSet::from([file.clone()]);
        let before = Snapshot::capture(&scope);
        // Forge the coarse-clock case: the stamp is identical but the recorded
        // digest no longer matches the bytes on disk.
        let mut forged = Snapshot::default();
        let mut stamp = before.files[&file].clone().unwrap();
        stamp.digest = [7; 32];
        forged.files.insert(file.clone(), Some(stamp));
        let after = forged.recapture(&scope);
        assert_eq!(forged.changed_files(&after), vec![file]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn walks_always_prune_metadata_and_dependency_directories() {
        let root = temp_root("always-prune");
        for relative in [
            "src/a.rs",
            ".git/objects/ab",
            "node_modules/pkg/index.js",
            "target/debug/out",
            "nested/.agent-hook-kit/log.txt",
        ] {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "x").unwrap();
        }
        // A customized list (here only `dist`) keeps the fixed floor.
        let ignored = BTreeSet::from(["dist".to_owned()]);
        assert_eq!(walk_files(&root, &ignored), vec![root.join("src/a.rs")]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn walks_prune_configured_ignored_directory_names() {
        let root = temp_root("prune");
        for relative in ["src/a.rs", "dist/bundle.js", ".agent-hook-kit/log.txt"] {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "x").unwrap();
        }
        let ignored = BTreeSet::from(["dist".to_owned(), ".agent-hook-kit".to_owned()]);
        let files = walk_files(&root, &ignored);
        assert_eq!(files, vec![root.join("src/a.rs")]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
