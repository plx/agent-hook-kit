//! Deterministic lexical path operations.
//!
//! These helpers operate on path syntax only. They never inspect the
//! filesystem, canonicalize a path, resolve symlinks, expand globs, or read
//! ambient process state. Callers that need those behaviors must request them
//! separately.

use camino::{Utf8Path, Utf8PathBuf};
use std::path::{Component, Path, PathBuf};

/// Lexically normalize a platform-native path.
///
/// Current-directory components are removed, normal components followed by
/// `..` are collapsed, leading `..` components are retained on relative paths,
/// and traversal never climbs above a root. Prefixes and roots are preserved
/// according to [`Component`].
pub fn normalize_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let rooted = path.has_root();
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(
                    normalized.components().next_back(),
                    Some(Component::Normal(_))
                ) {
                    normalized.pop();
                } else if !rooted {
                    normalized.push(component.as_os_str());
                }
            }
            _ => normalized.push(component.as_os_str()),
        }
    }

    normalized
}

/// Lexically normalize a UTF-8 path without lossy conversion.
///
/// This has the same component semantics as [`normalize_path`].
pub fn normalize_utf8_path(path: impl AsRef<Utf8Path>) -> Utf8PathBuf {
    let normalized = normalize_path(path.as_ref().as_std_path());
    Utf8PathBuf::from_path_buf(normalized)
        .expect("normalizing an existing UTF-8 path preserves UTF-8")
}

/// Resolve `candidate` against `base` using lexical path semantics only.
///
/// An absolute candidate is independent of the base. Relative candidates are
/// joined to the base before normalization.
pub fn resolve_path(base: impl AsRef<Path>, candidate: impl AsRef<Path>) -> PathBuf {
    let candidate = candidate.as_ref();
    if candidate.is_absolute() {
        normalize_path(candidate)
    } else {
        normalize_path(base.as_ref().join(candidate))
    }
}

/// Resolve a UTF-8 `candidate` against `base` using lexical semantics only.
///
/// This is the lossless UTF-8 counterpart to [`resolve_path`].
pub fn resolve_utf8_path(
    base: impl AsRef<Utf8Path>,
    candidate: impl AsRef<Utf8Path>,
) -> Utf8PathBuf {
    let candidate = candidate.as_ref();
    if candidate.is_absolute() {
        normalize_utf8_path(candidate)
    } else {
        normalize_utf8_path(base.as_ref().join(candidate))
    }
}

/// Expand a leading current-user `~` component using an explicit home path.
///
/// Only `~` and `~/...` are expanded. Forms such as `~other-user`, non-leading
/// `~` components, and paths that do not begin with `~` are returned unchanged.
/// A bare `~` becomes `home` exactly, without a trailing separator. No
/// normalization or filesystem access is performed.
pub fn expand_home(path: impl AsRef<Path>, home: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    match path.strip_prefix(Path::new("~")) {
        Ok(relative) if relative.as_os_str().is_empty() => home.as_ref().to_path_buf(),
        Ok(relative) => home.as_ref().join(relative),
        Err(_) => path.to_path_buf(),
    }
}

/// Losslessly expand a leading current-user `~` component in a UTF-8 path.
///
/// This has the same expansion boundary as [`expand_home`] and does not read
/// the process environment or filesystem.
pub fn expand_utf8_home(path: impl AsRef<Utf8Path>, home: impl AsRef<Utf8Path>) -> Utf8PathBuf {
    let path = path.as_ref();
    match path.strip_prefix(Utf8Path::new("~")) {
        Ok(relative) if relative.as_str().is_empty() => home.as_ref().to_path_buf(),
        Ok(relative) => home.as_ref().join(relative),
        Err(_) => path.to_path_buf(),
    }
}

/// Render a UTF-8 path with `/` separators for portable glob matching.
///
/// Because the input is already UTF-8, this conversion is lossless. It does
/// not normalize path components or inspect the filesystem.
pub fn utf8_path_to_slash(path: impl AsRef<Utf8Path>) -> String {
    path.as_ref().as_str().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn normalization_handles_explicit_edge_cases() {
        assert_eq!(normalize_path(""), PathBuf::new());
        assert_eq!(normalize_path("."), PathBuf::new());
        assert_eq!(normalize_path("a//./b/../c"), PathBuf::from("a/c"));
        assert_eq!(normalize_path("../../a/.."), PathBuf::from("../.."));

        let rooted = Path::new(std::path::MAIN_SEPARATOR_STR);
        assert_eq!(
            normalize_path(rooted.join("../../child")),
            rooted.join("child")
        );
    }

    #[test]
    fn resolution_uses_the_base_only_for_relative_candidates() {
        let (base, expected, absolute, normalized_absolute) = if cfg!(windows) {
            (
                r"C:\workspace\project",
                PathBuf::from(r"C:\workspace\project\README.md"),
                r"C:\workspace\.\README.md",
                PathBuf::from(r"C:\workspace\README.md"),
            )
        } else {
            (
                "/workspace/project",
                PathBuf::from("/workspace/project/README.md"),
                "/workspace/./README.md",
                PathBuf::from("/workspace/README.md"),
            )
        };
        assert_eq!(resolve_path(base, "src/../README.md"), expected);
        assert_eq!(resolve_path(base, absolute), normalized_absolute);

        let (utf8_base, utf8_expected, utf8_absolute, utf8_normalized_absolute) = if cfg!(windows) {
            (
                r"C:\workspace\project",
                Utf8PathBuf::from(r"C:\workspace\shared\file.rs"),
                r"C:\workspace\.\file.rs",
                Utf8PathBuf::from(r"C:\workspace\file.rs"),
            )
        } else {
            (
                "/workspace/project",
                Utf8PathBuf::from("/workspace/shared/file.rs"),
                "/workspace/./file.rs",
                Utf8PathBuf::from("/workspace/file.rs"),
            )
        };
        assert_eq!(
            resolve_utf8_path(utf8_base, "../shared/file.rs"),
            utf8_expected
        );
        assert_eq!(
            resolve_utf8_path(utf8_base, utf8_absolute),
            utf8_normalized_absolute
        );
    }

    #[test]
    fn home_expansion_is_explicit_and_current_user_only() {
        let home = Path::new("/home/example");
        // Compare the rendered text: `PathBuf` equality ignores a trailing
        // separator, which string-based glob matching does not.
        assert_eq!(expand_home("~", home).as_os_str(), home.as_os_str());
        assert_eq!(expand_home("~/", home).as_os_str(), home.as_os_str());
        assert_eq!(expand_home("~/child", home), home.join("child"));
        assert_eq!(expand_home("child/~", home), PathBuf::from("child/~"));
        assert_eq!(
            expand_home("~other-user/child", home),
            PathBuf::from("~other-user/child")
        );

        let utf8_home = Utf8Path::new("/home/example");
        assert_eq!(expand_utf8_home("~", utf8_home).as_str(), "/home/example");
        assert_eq!(expand_utf8_home("~/", utf8_home).as_str(), "/home/example");
        assert_eq!(
            expand_utf8_home("~/child", utf8_home).as_str(),
            "/home/example/child"
        );
        assert_eq!(
            expand_utf8_home("~/child", utf8_home),
            Utf8PathBuf::from("/home/example/child")
        );
        assert_eq!(
            expand_utf8_home("child/~", utf8_home),
            Utf8PathBuf::from("child/~")
        );
        assert_eq!(
            expand_utf8_home("~other-user/child", utf8_home),
            Utf8PathBuf::from("~other-user/child")
        );
    }

    #[test]
    fn slash_rendering_is_utf8_and_lexical() {
        assert_eq!(
            utf8_path_to_slash(Utf8Path::new("src\\nested/file.rs")),
            "src/nested/file.rs"
        );
        assert_eq!(
            utf8_path_to_slash(Utf8Path::new("café/東京.rs")),
            "café/東京.rs"
        );
    }

    proptest! {
        /// Lexical normalization is idempotent and removes current-directory
        /// components. Rooted paths also discard traversal above their root.
        #[test]
        fn normalization_is_idempotent(
            rooted in any::<bool>(),
            segments in prop::collection::vec(
                prop_oneof![Just(".".to_owned()), Just("..".to_owned()), "[a-z]{1,8}"],
                0..30,
            ),
        ) {
            let mut source = if rooted {
                std::path::MAIN_SEPARATOR_STR.to_owned()
            } else {
                String::new()
            };
            source.push_str(&segments.join(std::path::MAIN_SEPARATOR_STR));

            let once = normalize_path(&source);
            let twice = normalize_path(&once);
            prop_assert_eq!(&once, &twice);
            prop_assert!(!once.components().any(|component| matches!(component, Component::CurDir)));
            if rooted {
                prop_assert!(once.has_root());
                prop_assert!(!once.components().any(|component| matches!(component, Component::ParentDir)));
            }
        }

        /// The UTF-8 and native variants agree for inputs representable by
        /// both path types.
        #[test]
        fn utf8_and_native_normalization_agree(
            rooted in any::<bool>(),
            segments in prop::collection::vec(
                prop_oneof![Just(".".to_owned()), Just("..".to_owned()), "[a-zé]{1,8}"],
                0..30,
            ),
        ) {
            let mut source = if rooted {
                std::path::MAIN_SEPARATOR_STR.to_owned()
            } else {
                String::new()
            };
            source.push_str(&segments.join(std::path::MAIN_SEPARATOR_STR));

            let native = normalize_path(&source);
            let utf8 = normalize_utf8_path(Utf8Path::new(&source));
            prop_assert_eq!(native, utf8.into_std_path_buf());
        }
    }
}
