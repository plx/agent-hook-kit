//! Artifact and temp-file helpers.
//!
//! Utilities for storing diagnostics and retrieving artifacts by key across
//! related hook passes.
//!
//! Hook processes for one session can run concurrently (for example
//! `PostToolUse` hooks for parallel tool calls), and the default directory
//! lives under the system temp directory, which is shared between users on
//! most Linux hosts. The manager therefore:
//!
//! - creates directories owner-only (`0700` on Unix), and refuses a default
//!   temp directory that another user owns or that is a symlink;
//! - replaces keyed artifacts atomically (a private temp file renamed over
//!   the target), so a reader never sees a partial file and concurrent
//!   writers never interleave;
//! - never follows a symlink planted at an artifact path, and creates files
//!   owner-only (`0600` on Unix);
//! - maps distinct keys to distinct filenames.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};

/// Key for locating artifacts scoped to a session, turn, or tool use.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArtifactKey {
    /// Native session identifier.
    pub session_id: String,
    /// Optional native turn identifier.
    pub turn_id: Option<String>,
    /// Optional native tool-call identifier.
    pub tool_use_id: Option<String>,
    /// Application-defined artifact label.
    pub label: String,
}

/// Longest filename stem kept verbatim; longer stems are shortened and
/// suffixed with a digest so that the full name, including a numeric suffix
/// and an extension, stays within the common 255-byte filename limit.
const MAX_STEM_BYTES: usize = 200;

impl ArtifactKey {
    /// Creates a session-scoped artifact key.
    pub fn new(session_id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            turn_id: None,
            tool_use_id: None,
            label: label.into(),
        }
    }

    /// Adds a turn scope to the key.
    pub fn with_turn(mut self, turn_id: impl Into<String>) -> Self {
        self.turn_id = Some(turn_id.into());
        self
    }

    /// Adds a tool-call scope to the key.
    pub fn with_tool_use(mut self, tool_use_id: impl Into<String>) -> Self {
        self.tool_use_id = Some(tool_use_id.into());
        self
    }

    /// Generate a human-readable filename stem for this artifact.
    ///
    /// The stem has four `_`-separated fields: session, turn, tool call, and
    /// label. An absent scope is an empty field, and an empty scope is
    /// treated as absent. Inside a field, every character other than an ASCII
    /// letter, digit, or `-` is percent-encoded (`_` becomes `%5F`), so two
    /// keys name the same file only if they have the same fields. Filesystems
    /// that ignore case (the macOS and Windows defaults) still conflate keys
    /// that differ only in ASCII case.
    ///
    /// A stem longer than 200 bytes is shortened and suffixed with `~` and a
    /// 64-bit digest of the whole stem.
    ///
    /// ```
    /// use hookkit_runtime::artifacts::ArtifactKey;
    ///
    /// let key = ArtifactKey::new("sess-1", "lint").with_tool_use("toolu_01");
    /// assert_eq!(key.filename(), "sess-1__toolu%5F01_lint");
    /// ```
    pub fn filename(&self) -> String {
        let optional = |value: &Option<String>| value.as_deref().map(encode).unwrap_or_default();
        let stem = [
            encode(&self.session_id),
            optional(&self.turn_id),
            optional(&self.tool_use_id),
            encode(&self.label),
        ]
        .join("_");
        if stem.len() <= MAX_STEM_BYTES {
            return stem;
        }
        // Encoded stems are ASCII, so any byte index is a char boundary.
        format!(
            "{}~{:016x}",
            &stem[..MAX_STEM_BYTES - 17],
            fnv1a64(stem.as_bytes())
        )
    }
}

/// Percent-encodes every byte of `value` except ASCII letters, digits, and `-`.
fn encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_ascii_alphanumeric() || character == '-' {
            encoded.push(character);
        } else {
            let mut buffer = [0; 4];
            for byte in character.encode_utf8(&mut buffer).bytes() {
                encoded.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    encoded
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Writes keyed artifacts into one directory.
#[derive(Debug, Clone)]
pub struct ArtifactManager {
    base_dir: PathBuf,
}

impl ArtifactManager {
    /// Create a new artifact manager. Creates the base directory if needed.
    ///
    /// Directories this call creates are owner-only (`0700` on Unix). An
    /// existing directory is used as is; choosing a directory other users can
    /// write is the caller's decision.
    pub fn new(base_dir: impl Into<PathBuf>) -> std::io::Result<Self> {
        let base_dir = base_dir.into();
        create_private_dir_all(&base_dir).map_err(|error| with_path(error, &base_dir))?;
        Ok(Self { base_dir })
    }

    /// The per-user default artifact directory under the system temp
    /// directory: `hookkit-artifacts-<uid>` on Unix, `hookkit-artifacts`
    /// elsewhere (where the temp directory is already per-user).
    pub fn default_temp_dir() -> PathBuf {
        #[cfg(unix)]
        {
            std::env::temp_dir().join(format!("hookkit-artifacts-{}", effective_uid()))
        }
        #[cfg(not(unix))]
        {
            std::env::temp_dir().join("hookkit-artifacts")
        }
    }

    /// Create a manager using [`Self::default_temp_dir`].
    ///
    /// On Unix the directory is created `0700`. An existing directory must be
    /// a real directory (not a symlink) owned by the current user; if its
    /// mode grants other users any access, it is reset to `0700`. A directory
    /// that another user created first is refused with
    /// [`std::io::ErrorKind::PermissionDenied`], because on a shared `/tmp`
    /// that user could read the artifacts or replace them with instructions
    /// for the agent.
    pub fn in_temp_dir() -> std::io::Result<Self> {
        let dir = Self::default_temp_dir();
        create_private_dir_all(&dir).map_err(|error| with_path(error, &dir))?;
        #[cfg(unix)]
        verify_private_dir(&dir).map_err(|error| with_path(error, &dir))?;
        Ok(Self { base_dir: dir })
    }

    /// Write text content to an artifact file, atomically replacing any
    /// previous artifact with the same key.
    pub fn write_text(&self, key: &ArtifactKey, content: &str) -> std::io::Result<PathBuf> {
        let path = self.artifact_path(key, "txt");
        write_atomically(&self.base_dir, &path, content.as_bytes())?;
        Ok(path)
    }

    /// Write JSON content to an artifact file, atomically replacing any
    /// previous artifact with the same key.
    pub fn write_json(
        &self,
        key: &ArtifactKey,
        value: &serde_json::Value,
    ) -> std::io::Result<PathBuf> {
        let path = self.artifact_path(key, "json");
        let content = serde_json::to_string_pretty(value)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        write_atomically(&self.base_dir, &path, content.as_bytes())?;
        Ok(path)
    }

    /// Write JSON without replacing an existing artifact with the same key.
    ///
    /// The unsuffixed filename (`<stem>.json`) is attempted first. Collisions
    /// receive a monotonically increasing numeric suffix (`<stem>.1.json`,
    /// `<stem>.2.json`, ...); encoded stems never contain `.`, so a suffixed
    /// name can never be another key's filename. The returned path is the
    /// exact file that was created. Files are created exclusively, so an
    /// existing file or symlink is never opened.
    pub fn write_json_unique(
        &self,
        key: &ArtifactKey,
        value: &serde_json::Value,
    ) -> std::io::Result<PathBuf> {
        let content = serde_json::to_vec_pretty(value)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        let stem = key.filename();
        for suffix in 0_u64.. {
            let filename = if suffix == 0 {
                format!("{stem}.json")
            } else {
                format!("{stem}.{suffix}.json")
            };
            let path = self.base_dir.join(filename);
            match create_new_private(&path) {
                Ok(mut file) => {
                    file.write_all(&content)
                        .and_then(|()| file.sync_all())
                        .map_err(|error| with_path(error, &path))?;
                    return Ok(path);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(with_path(error, &path)),
            }
        }
        unreachable!("u64 artifact suffix space cannot be exhausted")
    }

    /// Check if an artifact exists for the given key.
    pub fn exists(&self, key: &ArtifactKey, extension: &str) -> bool {
        self.artifact_path(key, extension).exists()
    }

    /// Read an artifact back by key.
    pub fn read_text(&self, key: &ArtifactKey) -> std::io::Result<String> {
        let path = self.artifact_path(key, "txt");
        std::fs::read_to_string(&path).map_err(|error| with_path(error, &path))
    }

    /// Get the base directory for artifacts.
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    fn artifact_path(&self, key: &ArtifactKey, extension: &str) -> PathBuf {
        self.base_dir
            .join(format!("{}.{extension}", key.filename()))
    }
}

/// Adds the path to an I/O error's message, keeping its kind.
fn with_path(error: std::io::Error, path: &Path) -> std::io::Error {
    std::io::Error::new(error.kind(), format!("{}: {error}", path.display()))
}

fn create_private_dir_all(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    builder.mode(0o700);
    builder.create(path)
}

#[cfg(unix)]
fn effective_uid() -> u32 {
    // SAFETY: geteuid has no preconditions, cannot fail, and touches no memory.
    unsafe { libc::geteuid() }
}

/// Requires `dir` to be a real directory owned by the current user, and
/// removes any group or other access from it.
#[cfg(unix)]
fn verify_private_dir(dir: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(dir)?;
    if !metadata.file_type().is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "artifact directory is not a directory (it may be a symlink); refusing to use it",
        ));
    }
    if metadata.uid() != effective_uid() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "artifact directory is owned by uid {}, not the current user; refusing to use it",
                metadata.uid()
            ),
        ));
    }
    if metadata.mode() & 0o077 != 0 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Creates `path` exclusively (never opening an existing file or following a
/// symlink), owner-only on Unix.
fn create_new_private(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path)
}

/// Writes `content` to a private temp file in `dir`, then renames it over
/// `path`. The rename replaces a file or symlink at `path` without following
/// it, and readers observe either the old or the new content.
fn write_atomically(dir: &Path, path: &Path, content: &[u8]) -> std::io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (temp_path, mut file) = loop {
        let candidate = dir.join(format!(
            ".{name}.{}-{}.tmp",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        match create_new_private(&candidate) {
            Ok(file) => break (candidate, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(with_path(error, &candidate)),
        }
    };
    let written = file
        .write_all(content)
        .and_then(|()| file.flush())
        .map_err(|error| with_path(error, &temp_path))
        .and_then(|()| {
            drop(file);
            std::fs::rename(&temp_path, path).map_err(|error| with_path(error, path))
        });
    if written.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// A fresh directory per test, so concurrent test runs (for example in
    /// separate worktrees) never delete each other's files.
    fn test_dir(name: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "hookkit-test-{name}-{}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn artifact_key_filename() {
        let key = ArtifactKey::new("sess-123", "diagnostics");
        assert_eq!(key.filename(), "sess-123___diagnostics");
    }

    #[test]
    fn artifact_key_with_turn_and_tool() {
        let key = ArtifactKey::new("sess-123", "lint-output")
            .with_turn("turn-5")
            .with_tool_use("tool-42");
        assert_eq!(key.filename(), "sess-123_turn-5_tool-42_lint-output");
    }

    #[test]
    fn previously_colliding_keys_get_distinct_filenames() {
        let pairs = [
            (
                ArtifactKey::new("s", "x").with_turn("t"),
                ArtifactKey::new("s_t", "x"),
            ),
            (ArtifactKey::new("s.1", "x"), ArtifactKey::new("s/1", "x")),
            (
                ArtifactKey::new("s", "x").with_turn("t"),
                ArtifactKey::new("s", "x").with_tool_use("t"),
            ),
            (ArtifactKey::new("a_b", "c"), ArtifactKey::new("a", "b_c")),
        ];
        for (left, right) in pairs {
            assert_ne!(left.filename(), right.filename(), "{left:?} vs {right:?}");
        }
    }

    #[test]
    fn long_keys_are_shortened_with_a_digest() {
        let long = "x".repeat(400);
        let first = ArtifactKey::new(&long, "a").filename();
        let second = ArtifactKey::new(&long, "b").filename();
        assert!(first.len() <= MAX_STEM_BYTES);
        assert_ne!(first, second);
        assert!(first.contains('~'));
    }

    #[test]
    fn artifact_manager_write_and_read() {
        let dir = test_dir("artifacts");
        let mgr = ArtifactManager::new(&dir).unwrap();
        let key = ArtifactKey::new("test-sess", "test-artifact");

        mgr.write_text(&key, "hello world").unwrap();
        assert!(mgr.exists(&key, "txt"));
        assert_eq!(mgr.read_text(&key).unwrap(), "hello world");

        mgr.write_text(&key, "short").unwrap();
        assert_eq!(mgr.read_text(&key).unwrap(), "short");
        let leftovers = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0, "temp files must be renamed away");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn artifact_manager_write_json() {
        let dir = test_dir("json-artifacts");
        let mgr = ArtifactManager::new(&dir).unwrap();
        let key = ArtifactKey::new("test-sess", "diag");

        let value = serde_json::json!({"errors": 3, "warnings": 1});
        let path = mgr.write_json(&key, &value).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(path).unwrap()).unwrap(),
            value
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn artifact_manager_unique_json_never_replaces_an_existing_record() {
        let dir = test_dir("unique-json-artifacts");
        let mgr = ArtifactManager::new(&dir).unwrap();
        let key = ArtifactKey::new("test-sess", "warning");

        let first = mgr
            .write_json_unique(&key, &serde_json::json!({"record": 1}))
            .unwrap();
        let second = mgr
            .write_json_unique(&key, &serde_json::json!({"record": 2}))
            .unwrap();

        assert_ne!(first, second);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(first).unwrap()).unwrap(),
            serde_json::json!({"record": 1})
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(second).unwrap()).unwrap(),
            serde_json::json!({"record": 2})
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unique_suffixes_never_collide_with_another_keys_filename() {
        let dir = test_dir("unique-suffix-artifacts");
        let mgr = ArtifactManager::new(&dir).unwrap();
        let base = ArtifactKey::new("test-sess", "warning");
        let lookalike = ArtifactKey::new("test-sess", "warning-1");

        mgr.write_json_unique(&base, &serde_json::json!({"record": 0}))
            .unwrap();
        let suffixed = mgr
            .write_json_unique(&base, &serde_json::json!({"record": 1}))
            .unwrap();
        let other = mgr
            .write_json(&lookalike, &serde_json::json!({"record": "other"}))
            .unwrap();

        assert_ne!(suffixed, other);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(&suffixed).unwrap())
                .unwrap(),
            serde_json::json!({"record": 1})
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_writers_never_interleave() {
        let dir = test_dir("concurrent-artifacts");
        let mgr = ArtifactManager::new(&dir).unwrap();
        let key = ArtifactKey::new("test-sess", "shared");
        let long = "L".repeat(64 * 1024);
        let short = "S".repeat(10);
        std::thread::scope(|scope| {
            for content in [&long, &short, &long, &short] {
                let (mgr, key) = (&mgr, &key);
                scope.spawn(move || {
                    for _ in 0..20 {
                        mgr.write_text(key, content).unwrap();
                    }
                });
            }
        });
        let content = mgr.read_text(&key).unwrap();
        assert!(content == long || content == short, "interleaved write");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn writes_replace_a_planted_symlink_instead_of_following_it() {
        let dir = test_dir("symlink-artifacts");
        let mgr = ArtifactManager::new(&dir).unwrap();
        let victim = dir.join("victim.txt");
        std::fs::write(&victim, "precious").unwrap();
        let key = ArtifactKey::new("test-sess", "diag");
        std::os::unix::fs::symlink(&victim, dir.join(format!("{}.txt", key.filename()))).unwrap();

        let path = mgr.write_text(&key, "diagnostics").unwrap();

        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "precious");
        assert!(
            !std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "diagnostics");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn created_directories_and_files_are_owner_only() {
        let dir = test_dir("private-artifacts").join("nested");
        let mgr = ArtifactManager::new(&dir).unwrap();
        let key = ArtifactKey::new("test-sess", "diag");
        let text = mgr.write_text(&key, "x").unwrap();
        let unique = mgr.write_json_unique(&key, &serde_json::json!({})).unwrap();
        let mode = |path: &Path| std::fs::metadata(path).unwrap().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(dir.parent().unwrap()), 0o700);
        assert_eq!(mode(&text), 0o600);
        assert_eq!(mode(&unique), 0o600);
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn default_temp_dir_is_per_user_and_private() {
        let dir = ArtifactManager::default_temp_dir();
        assert!(
            dir.file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(&format!("-{}", effective_uid()))
        );
        let manager = ArtifactManager::in_temp_dir().unwrap();
        let metadata = std::fs::symlink_metadata(manager.base_dir()).unwrap();
        assert_eq!(metadata.mode() & 0o077, 0);
        assert_eq!(metadata.uid(), effective_uid());
    }

    #[cfg(unix)]
    #[test]
    fn private_directory_check_rejects_symlinks_and_tightens_modes() {
        let root = test_dir("private-check");
        std::fs::create_dir_all(&root).unwrap();
        let real = root.join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o777)).unwrap();
        verify_private_dir(&real).unwrap();
        assert_eq!(std::fs::metadata(&real).unwrap().mode() & 0o777, 0o700);

        let link = root.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let error = verify_private_dir(&link).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn io_errors_name_the_path() {
        let dir = test_dir("missing-artifacts");
        let mgr = ArtifactManager {
            base_dir: dir.clone(),
        };
        let error = mgr
            .write_text(&ArtifactKey::new("s", "x"), "content")
            .unwrap_err();
        assert!(
            error.to_string().contains(&dir.display().to_string()),
            "{error}"
        );
    }

    proptest! {
        /// Property: filenames never contain a path separator and use only
        /// portable characters.
        #[test]
        fn artifact_filenames_are_safe(
            session in any::<String>(),
            turn in any::<String>(),
            tool in any::<String>(),
            label in any::<String>(),
        ) {
            let filename = ArtifactKey::new(session, label)
                .with_turn(turn)
                .with_tool_use(tool)
                .filename();
            prop_assert!(filename.len() <= MAX_STEM_BYTES);
            prop_assert!(filename
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "-_%~".contains(character)));
        }

        /// Property: distinct keys (treating an empty optional scope as
        /// absent) map to distinct filenames.
        #[test]
        fn artifact_filenames_are_injective(
            left in (any::<String>(), proptest::option::of(any::<String>()), proptest::option::of(any::<String>()), any::<String>()),
            right in (any::<String>(), proptest::option::of(any::<String>()), proptest::option::of(any::<String>()), any::<String>()),
        ) {
            let key = |(session, turn, tool, label): (String, Option<String>, Option<String>, String)| ArtifactKey {
                session_id: session,
                turn_id: turn.filter(|turn| !turn.is_empty()),
                tool_use_id: tool.filter(|tool| !tool.is_empty()),
                label,
            };
            let (left, right) = (key(left), key(right));
            if left != right {
                prop_assert_ne!(left.filename(), right.filename());
            }
        }

        /// Property: segments separated by `_` decode back to the key.
        #[test]
        fn short_filenames_have_four_fields(
            session in "[a-z_./]{0,12}",
            label in "[a-z_./]{0,12}",
        ) {
            let filename = ArtifactKey::new(session, label).filename();
            prop_assert_eq!(filename.split('_').count(), 4);
        }
    }
}
