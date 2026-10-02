//! Working-root selection shared by the immediate and Stop runners.
//!
//! Claude Code keeps `CLAUDE_PROJECT_DIR` at the directory where the session
//! started, while the input `cwd` follows the agent: into a subdirectory after
//! `cd`, and to the worktree root after the session enters a Git worktree.
//! The runners therefore anchor on the project directory, except that a
//! session working inside a linked Git worktree works in that worktree.
//! Linked worktrees nested below a root (Claude creates them under
//! `.claude/worktrees/`) belong to other sessions, so the Stop runner's
//! heuristic reconciliation skips them.

use crate::util::normalize_path;
use std::path::{Path, PathBuf};

/// The directory a Claude Code session works in: the root of the linked Git
/// worktree containing `cwd` when that worktree belongs to the same
/// repository as `project_dir` but is not the checkout containing it,
/// otherwise `project_dir`.
///
/// A `cd` into a worktree of an unrelated repository (an additional working
/// directory) therefore never moves the root away from the project.
pub(crate) fn claude_working_root(project_dir: &Path, cwd: Option<&Path>) -> PathBuf {
    let project_dir = normalize_path(project_dir);
    let Some(cwd) = cwd.map(normalize_path) else {
        return project_dir;
    };
    let Some(worktree) = linked_worktree_root(&cwd) else {
        return project_dir;
    };
    let same_repository = repository_common_dir(&worktree).is_some_and(|common| {
        repository_common_dir(&project_dir).is_some_and(|project| project == common)
    });
    if same_repository && linked_worktree_root(&project_dir).as_ref() != Some(&worktree) {
        worktree
    } else {
        project_dir
    }
}

/// Root of the linked Git worktree containing `path`, if any.
///
/// The nearest ancestor holding a `.git` entry decides: a main checkout has a
/// `.git` directory, while a linked worktree has a `.git` file naming its
/// administrative directory, which records the shared repository in a
/// `commondir` file. A submodule also has a `.git` file, but its
/// administrative directory has no `commondir`, so it is not a worktree.
pub(crate) fn linked_worktree_root(path: &Path) -> Option<PathBuf> {
    for ancestor in path.ancestors() {
        let marker = ancestor.join(".git");
        let Ok(metadata) = std::fs::symlink_metadata(&marker) else {
            continue;
        };
        if !metadata.is_file() {
            return None;
        }
        let admin = git_file_target(&marker)?;
        return admin
            .join("commondir")
            .is_file()
            .then(|| ancestor.to_path_buf());
    }
    None
}

/// Linked worktrees of the repository containing `root` that lie strictly
/// below `root`, other than `keep`.
///
/// The repository's `worktrees/<name>/gitdir` files record where each linked
/// worktree's `.git` file lives, so no `git` process is needed.
pub(crate) fn nested_linked_worktrees(root: &Path, keep: &Path) -> Vec<PathBuf> {
    let root = normalize_path(root);
    let keep = normalize_path(keep);
    let Some(common) = repository_common_dir(&root) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(common.join("worktrees")) else {
        return Vec::new();
    };
    let mut nested = entries
        .filter_map(Result::ok)
        .filter_map(|entry| std::fs::read_to_string(entry.path().join("gitdir")).ok())
        .filter_map(|gitdir| {
            let marker = PathBuf::from(gitdir.trim_end_matches(['\n', '\r']));
            marker.is_absolute().then_some(marker)
        })
        .filter_map(|marker| marker.parent().map(normalize_path))
        .filter(|worktree| worktree != &root && worktree.starts_with(&root) && worktree != &keep)
        .collect::<Vec<_>>();
    nested.sort();
    nested.dedup();
    nested
}

/// The shared repository directory (`$GIT_COMMON_DIR`) of the checkout that
/// contains `path`.
fn repository_common_dir(path: &Path) -> Option<PathBuf> {
    for ancestor in path.ancestors() {
        let marker = ancestor.join(".git");
        let Ok(metadata) = std::fs::symlink_metadata(&marker) else {
            continue;
        };
        if metadata.is_dir() {
            return Some(marker);
        }
        let admin = git_file_target(&marker)?;
        return match std::fs::read_to_string(admin.join("commondir")) {
            Ok(common) => Some(normalize_path(
                &admin.join(common.trim_end_matches(['\n', '\r'])),
            )),
            Err(_) => Some(admin),
        };
    }
    None
}

/// The directory a `.git` file points at (`gitdir: <path>`), resolved against
/// the directory holding the file.
fn git_file_target(marker: &Path) -> Option<PathBuf> {
    let contents = std::fs::read_to_string(marker).ok()?;
    let target = contents
        .lines()
        .find_map(|line| line.strip_prefix("gitdir:"))?
        .trim();
    let target = Path::new(target);
    Some(if target.is_absolute() {
        normalize_path(target)
    } else {
        normalize_path(&marker.parent()?.join(target))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "hookkit-roots-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        normalize_path(&root)
    }

    /// Lay out a main checkout at `main` with a linked worktree at `worktree`
    /// the way `git worktree add` does.
    fn add_worktree(main: &Path, name: &str, worktree: &Path) {
        let admin = main.join(".git/worktrees").join(name);
        std::fs::create_dir_all(&admin).unwrap();
        std::fs::create_dir_all(worktree).unwrap();
        std::fs::write(admin.join("commondir"), "../..\n").unwrap();
        std::fs::write(
            admin.join("gitdir"),
            format!("{}\n", worktree.join(".git").display()),
        )
        .unwrap();
        std::fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", admin.display()),
        )
        .unwrap();
    }

    #[test]
    fn a_session_in_a_linked_worktree_works_there_not_in_the_main_checkout() {
        let main = temp_root("worktree-session");
        std::fs::create_dir_all(main.join(".git")).unwrap();
        let mine = main.join(".claude/worktrees/mine");
        let other = main.join(".claude/worktrees/other");
        add_worktree(&main, "mine", &mine);
        add_worktree(&main, "other", &other);
        std::fs::create_dir_all(mine.join("src")).unwrap();

        assert_eq!(claude_working_root(&main, Some(&mine)), mine);
        assert_eq!(claude_working_root(&main, Some(&mine.join("src"))), mine);
        // `cd` inside the main checkout never moves the root.
        std::fs::create_dir_all(main.join("src")).unwrap();
        assert_eq!(claude_working_root(&main, Some(&main.join("src"))), main);
        assert_eq!(claude_working_root(&main, None), main);
        // A session that started inside the worktree stays at its project dir.
        assert_eq!(
            claude_working_root(&mine.join("src"), Some(&mine.join("src"))),
            mine.join("src")
        );

        // Sibling worktrees below the main checkout belong to other sessions.
        assert_eq!(
            nested_linked_worktrees(&main, &main),
            vec![mine.clone(), other.clone()]
        );
        assert_eq!(nested_linked_worktrees(&main, &mine), vec![other]);
        assert!(nested_linked_worktrees(&mine, &mine).is_empty());
        std::fs::remove_dir_all(main).unwrap();
    }

    #[test]
    fn worktrees_of_unrelated_repositories_never_move_the_root() {
        let project = temp_root("unrelated-project");
        std::fs::create_dir_all(project.join(".git")).unwrap();
        let elsewhere = temp_root("unrelated-repository");
        std::fs::create_dir_all(elsewhere.join(".git")).unwrap();
        let foreign = elsewhere.join("wt");
        add_worktree(&elsewhere, "wt", &foreign);

        assert_eq!(linked_worktree_root(&foreign), Some(foreign.clone()));
        assert_eq!(claude_working_root(&project, Some(&foreign)), project);
        // A project that is not a Git checkout at all has no worktrees.
        let plain = temp_root("unrelated-plain");
        assert_eq!(claude_working_root(&plain, Some(&foreign)), plain);
        for root in [project, elsewhere, plain] {
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn submodules_are_not_linked_worktrees() {
        let main = temp_root("submodule");
        let module_admin = main.join(".git/modules/vendor");
        std::fs::create_dir_all(&module_admin).unwrap();
        let vendor = main.join("vendor");
        std::fs::create_dir_all(&vendor).unwrap();
        std::fs::write(vendor.join(".git"), "gitdir: ../.git/modules/vendor\n").unwrap();

        assert_eq!(linked_worktree_root(&vendor), None);
        assert_eq!(claude_working_root(&main, Some(&vendor)), main);
        assert!(nested_linked_worktrees(&main, &main).is_empty());
        std::fs::remove_dir_all(main).unwrap();
    }
}
