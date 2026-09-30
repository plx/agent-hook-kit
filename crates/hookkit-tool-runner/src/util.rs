//! Small path, error, and text helpers shared by both runners.

use hookkit_core::{HarnessId, HookkitError};
use std::path::{Path, PathBuf};

pub(crate) fn state_error(error: hookkit_session_state::StateError) -> HookkitError {
    std::io::Error::other(error).into()
}

pub(crate) fn activity_error(error: hookkit_file_activity::FileActivityError) -> HookkitError {
    std::io::Error::other(error).into()
}

pub(crate) fn invalid_data(message: String) -> HookkitError {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message).into()
}

/// The error a runner returns for a harness it has no native lowering for.
pub(crate) fn unsupported_harness(harness: &HarnessId, message: impl Into<String>) -> HookkitError {
    HookkitError::UnsupportedHarness {
        harness: harness.clone(),
        message: message.into(),
    }
}

pub(crate) fn path_arg(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

pub(crate) fn absolute_from(path: &Path, base: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

/// Canonicalize through the nearest existing ancestor, then lexically
/// normalize the missing suffix with [`hookkit_core::normalize_path`].
///
/// Existing paths therefore collapse platform aliases (such as macOS `/var`
/// and `/private/var`), while deleted or not-yet-created paths still share the
/// canonical prefix of their existing parent instead of keeping the alias.
pub(crate) fn normalize_path(path: &Path) -> PathBuf {
    let lexical = hookkit_core::normalize_path(path);
    if let Ok(canonical) = std::fs::canonicalize(&lexical) {
        return canonical;
    }
    for ancestor in lexical.ancestors().skip(1) {
        if ancestor.as_os_str().is_empty() {
            break;
        }
        let Ok(canonical) = std::fs::canonicalize(ancestor) else {
            continue;
        };
        let Ok(suffix) = lexical.strip_prefix(ancestor) else {
            continue;
        };
        return hookkit_core::normalize_path(canonical.join(suffix));
    }
    lexical
}

pub(crate) fn slash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(crate) fn rel_display(path: &Path, project_root: &Path) -> String {
    path.strip_prefix(project_root)
        .map(slash_path)
        .unwrap_or_else(|_| slash_path(path))
}

/// Truncate `text` to at most `max_chars` characters on a character boundary,
/// appending `suffix` (counted within the limit) when truncation occurs.
pub(crate) fn truncate_chars(text: &str, max_chars: usize, suffix: &str) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let keep = max_chars.saturating_sub(suffix.chars().count());
    let mut truncated = text.chars().take(keep).collect::<String>();
    truncated.push_str(suffix);
    truncated
}

/// Lowercase hexadecimal SHA-256 digest of `bytes`.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::path::Component;

    #[test]
    fn missing_paths_share_the_canonical_prefix_of_their_existing_parent() {
        let root = std::env::temp_dir().join(format!(
            "hookkit-normalize-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let existing = root.join("present.rs");
        std::fs::write(&existing, "").unwrap();
        let missing = root.join("deleted.rs");

        let canonical_root = std::fs::canonicalize(&root).unwrap();
        assert_eq!(normalize_path(&existing), canonical_root.join("present.rs"));
        assert_eq!(normalize_path(&missing), canonical_root.join("deleted.rs"));
        assert_eq!(
            normalize_path(&root.join("sub/../deleted.rs")),
            canonical_root.join("deleted.rs")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn relative_parent_segments_are_not_silently_dropped() {
        assert_eq!(
            normalize_path(Path::new("../hookkit-definitely-missing/x")),
            std::fs::canonicalize("..")
                .unwrap()
                .join("hookkit-definitely-missing/x")
        );
    }

    #[test]
    fn truncation_respects_character_boundaries_and_limit() {
        assert_eq!(truncate_chars("short", 10, "…"), "short");
        let truncated = truncate_chars("ééééééééééé", 5, "…");
        assert_eq!(truncated, "éééé…");
        assert_eq!(truncated.chars().count(), 5);
    }

    proptest! {
        /// Property: lexical normalization for not-yet-created output paths is
        /// idempotent, absolute, and cannot retain traversal above root.
        #[test]
        fn non_existing_output_path_normalization_is_stable(
            segments in prop::collection::vec(prop_oneof![Just(".".to_owned()), Just("..".to_owned()), "[a-z]{1,8}"], 0..30),
        ) {
            let path = PathBuf::from(format!(
                "/hookkit-property-path-that-does-not-exist/{}/{}",
                std::process::id(),
                segments.join("/")
            ));
            let once = normalize_path(&path);
            let twice = normalize_path(&once);

            prop_assert_eq!(&once, &twice);
            prop_assert!(once.is_absolute());
            let contains_traversal = once.components().any(|component| {
                matches!(component, Component::CurDir | Component::ParentDir)
            });
            prop_assert!(!contains_traversal);
        }

        /// Property: truncation never exceeds its character budget and keeps
        /// short text unchanged.
        #[test]
        fn truncation_is_bounded(text in ".{0,200}", max in 1usize..100) {
            let truncated = truncate_chars(&text, max, "…");
            prop_assert!(truncated.chars().count() <= max.max(1));
            if text.chars().count() <= max {
                prop_assert_eq!(truncated, text);
            }
        }
    }
}
