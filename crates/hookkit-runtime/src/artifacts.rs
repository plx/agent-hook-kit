//! Artifact and temp-file helpers.
//!
//! Utilities for creating scoped temp files, storing diagnostics,
//! and retrieving artifacts by key across related hook passes.

use std::io::Write;
use std::path::{Path, PathBuf};

/// Key for locating artifacts scoped to a session, turn, or tool use.
#[derive(Debug, Clone)]
pub struct ArtifactKey {
    pub session_id: String,
    pub turn_id: Option<String>,
    pub tool_use_id: Option<String>,
    pub label: String,
}

impl ArtifactKey {
    pub fn new(session_id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            turn_id: None,
            tool_use_id: None,
            label: label.into(),
        }
    }

    pub fn with_turn(mut self, turn_id: impl Into<String>) -> Self {
        self.turn_id = Some(turn_id.into());
        self
    }

    pub fn with_tool_use(mut self, tool_use_id: impl Into<String>) -> Self {
        self.tool_use_id = Some(tool_use_id.into());
        self
    }

    /// Generate a human-readable filename for this artifact.
    pub fn filename(&self) -> String {
        let mut parts = vec![sanitize(&self.session_id)];
        if let Some(ref turn) = self.turn_id {
            parts.push(sanitize(turn));
        }
        if let Some(ref tool) = self.tool_use_id {
            parts.push(sanitize(tool));
        }
        parts.push(sanitize(&self.label));
        parts.join("_")
    }
}

/// Simple artifact manager that writes to a temp directory.
pub struct ArtifactManager {
    base_dir: PathBuf,
}

impl ArtifactManager {
    /// Create a new artifact manager. Creates the base directory if needed.
    pub fn new(base_dir: impl Into<PathBuf>) -> std::io::Result<Self> {
        let base_dir = base_dir.into();
        std::fs::create_dir_all(&base_dir)?;
        Ok(Self { base_dir })
    }

    /// Create a manager using a temp directory under the system temp dir.
    pub fn in_temp_dir() -> std::io::Result<Self> {
        let dir = std::env::temp_dir().join("hookkit-artifacts");
        Self::new(dir)
    }

    /// Write text content to an artifact file.
    pub fn write_text(&self, key: &ArtifactKey, content: &str) -> std::io::Result<PathBuf> {
        let path = self.artifact_path(key, "txt");
        let mut file = std::fs::File::create(&path)?;
        file.write_all(content.as_bytes())?;
        Ok(path)
    }

    /// Write JSON content to an artifact file.
    pub fn write_json(
        &self,
        key: &ArtifactKey,
        value: &serde_json::Value,
    ) -> std::io::Result<PathBuf> {
        let path = self.artifact_path(key, "json");
        let content = serde_json::to_string_pretty(value)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        let mut file = std::fs::File::create(&path)?;
        file.write_all(content.as_bytes())?;
        Ok(path)
    }

    /// Check if an artifact exists for the given key.
    pub fn exists(&self, key: &ArtifactKey, extension: &str) -> bool {
        self.artifact_path(key, extension).exists()
    }

    /// Read an artifact back by key.
    pub fn read_text(&self, key: &ArtifactKey) -> std::io::Result<String> {
        let path = self.artifact_path(key, "txt");
        std::fs::read_to_string(path)
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

/// Sanitize a string for use in filenames.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn artifact_key_filename() {
        let key = ArtifactKey::new("sess-123", "diagnostics");
        assert_eq!(key.filename(), "sess-123_diagnostics");
    }

    #[test]
    fn artifact_key_with_turn_and_tool() {
        let key = ArtifactKey::new("sess-123", "lint-output")
            .with_turn("turn-5")
            .with_tool_use("tool-42");
        assert_eq!(key.filename(), "sess-123_turn-5_tool-42_lint-output");
    }

    #[test]
    fn artifact_manager_write_and_read() {
        let dir = std::env::temp_dir().join("hookkit-test-artifacts");
        let mgr = ArtifactManager::new(&dir).unwrap();
        let key = ArtifactKey::new("test-sess", "test-artifact");

        mgr.write_text(&key, "hello world").unwrap();
        assert!(mgr.exists(&key, "txt"));

        let content = mgr.read_text(&key).unwrap();
        assert_eq!(content, "hello world");

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn artifact_manager_write_json() {
        let dir = std::env::temp_dir().join("hookkit-test-json-artifacts");
        let mgr = ArtifactManager::new(&dir).unwrap();
        let key = ArtifactKey::new("test-sess", "diag");

        let value = serde_json::json!({"errors": 3, "warnings": 1});
        let path = mgr.write_json(&key, &value).unwrap();
        assert!(path.exists());

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sanitize_special_chars() {
        assert_eq!(sanitize("hello/world:test"), "hello_world_test");
    }

    proptest! {
        /// Property: sanitization is length-preserving in Unicode scalar values,
        /// idempotent, and its output contains only filename-safe characters.
        /// Thus applying it at more than one artifact layer cannot change a key.
        #[test]
        fn sanitization_is_safe_and_idempotent(value in any::<String>()) {
            let sanitized = sanitize(&value);

            prop_assert_eq!(sanitized.chars().count(), value.chars().count());
            prop_assert!(sanitized
                .chars()
                .all(|character| character.is_alphanumeric() || character == '-' || character == '_'));
            prop_assert_eq!(sanitize(&sanitized), sanitized);
        }

        /// Property: artifact key segments retain their order and no path
        /// separator from external identifiers can escape into the filename.
        #[test]
        fn artifact_filenames_are_ordered_safe_segments(
            session in any::<String>(),
            turn in any::<String>(),
            tool in any::<String>(),
            label in any::<String>(),
        ) {
            let filename = ArtifactKey::new(session.clone(), label.clone())
                .with_turn(turn.clone())
                .with_tool_use(tool.clone())
                .filename();
            let expected = [session, turn, tool, label]
                .iter()
                .map(|part| sanitize(part))
                .collect::<Vec<_>>()
                .join("_");

            prop_assert_eq!(&filename, &expected);
            prop_assert!(!filename.contains('/'));
            prop_assert!(!filename.contains('\\'));
        }
    }
}
