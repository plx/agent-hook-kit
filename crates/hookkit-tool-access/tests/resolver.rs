use hookkit_core::{Utf8Path, Utf8PathBuf};
use hookkit_tool_access::{
    AccessScope, AccessTarget, ExactPathPolicy, PathBase, PathExpression, ResolvedTargets,
    TargetResolutionOptions, TargetResolutionReason, resolve_targets,
};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

struct TempDirectory(Utf8PathBuf);

impl TempDirectory {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = Utf8PathBuf::from_path_buf(std::env::temp_dir())
            .unwrap()
            .join(format!("hookkit-tool-access-{label}-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn target(path: impl Into<Utf8PathBuf>, scope: AccessScope) -> AccessTarget {
    let path = path.into();
    AccessTarget::Path {
        expression: PathExpression {
            raw: path.to_string(),
            resolved: Some(path),
            base: PathBase::Absolute,
        },
        scope,
    }
}

fn resolve(targets: &[AccessTarget], options: &TargetResolutionOptions) -> ResolvedTargets {
    resolve_targets(targets, options).unwrap()
}

#[test]
fn materializes_exact_descendant_exact_or_descendant_glob_and_workspace_scopes() {
    let temporary = TempDirectory::new("scopes");
    let root = &temporary.0;
    fs::create_dir_all(root.join("src/nested")).unwrap();
    fs::write(root.join("README.md"), "readme").unwrap();
    fs::write(root.join("src/lib.rs"), "lib").unwrap();
    fs::write(root.join("src/nested/mod.rs"), "mod").unwrap();
    let options = TargetResolutionOptions::new(vec![root.clone()]);

    let exact = resolve(
        &[target(root.join("README.md"), AccessScope::Exact)],
        &options,
    );
    assert_eq!(exact.paths, [root.join("README.md")].into_iter().collect());

    let descendants = resolve(
        &[target(root.join("src"), AccessScope::Descendants)],
        &options,
    );
    assert!(!descendants.paths.contains(&root.join("src")));
    assert!(descendants.paths.contains(&root.join("src/lib.rs")));
    assert!(descendants.paths.contains(&root.join("src/nested/mod.rs")));

    let exact_or_descendants = resolve(
        &[target(root.join("src"), AccessScope::ExactOrDescendants)],
        &options,
    );
    assert!(exact_or_descendants.paths.contains(&root.join("src")));
    assert!(
        exact_or_descendants
            .paths
            .contains(&root.join("src/nested/mod.rs"))
    );

    let pattern = root.join("**/*.rs");
    let glob = resolve(&[target(pattern, AccessScope::Glob)], &options);
    assert_eq!(glob.paths.len(), 2);
    assert!(glob.paths.iter().all(|path| path.extension() == Some("rs")));

    let workspace = resolve(
        &[AccessTarget::Workspace {
            root: Some(root.clone()),
        }],
        &options,
    );
    assert!(workspace.paths.contains(&root.join("README.md")));
    assert!(workspace.paths.contains(&root.join("src/nested/mod.rs")));
}

#[test]
fn exact_nonexistent_targets_follow_the_selected_existence_policy() {
    let temporary = TempDirectory::new("nonexistent");
    let missing = temporary.0.join("will-be-created.txt");
    let candidate = target(&missing, AccessScope::Exact);

    let existing_only = resolve(
        std::slice::from_ref(&candidate),
        &TargetResolutionOptions::new(vec![temporary.0.clone()]),
    );
    assert!(!existing_only.paths.contains(&missing));
    assert!(matches!(
        existing_only.unresolved[0].reason,
        TargetResolutionReason::NonexistentExactPath
    ));

    let mut retain = TargetResolutionOptions::new(vec![temporary.0.clone()]);
    retain.exact_paths = ExactPathPolicy::RetainNonexistent;
    let retained = resolve(&[candidate], &retain);
    assert!(retained.paths.contains(&missing));
    assert!(retained.unresolved.is_empty());
}

#[test]
fn traversal_honors_ignored_excluded_and_no_follow_defaults() {
    let temporary = TempDirectory::new("filters");
    let outside = TempDirectory::new("outside");
    let root = &temporary.0;
    fs::create_dir_all(root.join("ignored")).unwrap();
    fs::create_dir_all(root.join("private")).unwrap();
    fs::write(root.join("visible.txt"), "visible").unwrap();
    fs::write(root.join("ignored/hidden.txt"), "hidden").unwrap();
    fs::write(root.join("private/secret.txt"), "secret").unwrap();
    fs::write(outside.0.join("outside.txt"), "outside").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside.0, root.join("linked")).unwrap();

    let mut options = TargetResolutionOptions::new(vec![root.clone()]);
    options.ignored_directory_names = ["ignored".to_owned()].into_iter().collect();
    options.excluded_roots.insert(root.join("private"));
    let report = resolve(
        &[AccessTarget::Workspace {
            root: Some(root.clone()),
        }],
        &options,
    );

    assert!(report.paths.contains(&root.join("visible.txt")));
    assert!(!report.paths.contains(&root.join("ignored/hidden.txt")));
    assert!(!report.paths.contains(&root.join("private/secret.txt")));
    #[cfg(unix)]
    assert!(!report.paths.contains(&root.join("linked/outside.txt")));
}

#[test]
fn deterministic_budget_exhaustion_reports_every_unprocessed_target() {
    let temporary = TempDirectory::new("budget");
    let paths = ["one", "two", "three"].map(|name| temporary.0.join(name));
    for path in &paths {
        fs::write(path, path.as_str()).unwrap();
    }
    let targets = paths
        .iter()
        .map(|path| target(path, AccessScope::Exact))
        .collect::<Vec<_>>();
    let mut options = TargetResolutionOptions::new(vec![temporary.0.clone()]);
    options.max_entries = 1;

    let first = resolve(&targets, &options);
    let second = resolve(&targets, &options);
    assert_eq!(first, second);
    assert!(first.budget_exhausted);
    assert_eq!(first.paths, [paths[0].clone()].into_iter().collect());
    assert_eq!(first.unresolved.len(), 2);
    assert!(
        first
            .unresolved
            .iter()
            .all(|unresolved| matches!(unresolved.reason, TargetResolutionReason::BudgetExhausted))
    );
    assert_eq!(first.unresolved[0].target, targets[1]);
    assert_eq!(first.unresolved[1].target, targets[2]);
}

#[test]
fn invalid_globs_and_io_errors_are_typed_per_target() {
    let temporary = TempDirectory::new("failures");
    let invalid_glob = AccessTarget::Path {
        expression: PathExpression {
            raw: "[".to_owned(),
            resolved: None,
            base: PathBase::MissingWorkingDirectory,
        },
        scope: AccessScope::Glob,
    };
    let invalid_path = target(Utf8Path::new("bad\0path"), AccessScope::Exact);
    let report = resolve(
        &[invalid_glob, invalid_path],
        &TargetResolutionOptions::new(vec![temporary.0.clone()]),
    );

    assert!(report.unresolved.iter().any(|unresolved| matches!(
        unresolved.reason,
        TargetResolutionReason::InvalidGlob { .. }
    )));
    assert!(
        report
            .unresolved
            .iter()
            .any(|unresolved| matches!(unresolved.reason, TargetResolutionReason::Io { .. }))
    );
}

#[test]
fn ignored_names_prune_traversal_but_not_exact_targets() {
    let temporary = TempDirectory::new("ignored-exact");
    let root = &temporary.0;
    fs::create_dir_all(root.join(".git/hooks")).unwrap();
    fs::create_dir_all(root.join("scripts")).unwrap();
    fs::write(root.join(".git/hooks/pre-commit"), "#!/bin/sh").unwrap();
    // A regular file whose name matches an ignored directory name.
    fs::write(root.join("scripts/target"), "not a directory").unwrap();
    let options = TargetResolutionOptions::new(vec![root.clone()]);

    let exact = resolve(
        &[
            target(root.join(".git/hooks/pre-commit"), AccessScope::Exact),
            target(root.join("scripts/target"), AccessScope::ExactOrDescendants),
        ],
        &options,
    );
    assert!(exact.is_complete(), "{exact:?}");
    assert!(exact.paths.contains(&root.join(".git/hooks/pre-commit")));
    assert!(exact.paths.contains(&root.join("scripts/target")));

    let walked = resolve(
        &[target(root.join(".git"), AccessScope::Descendants)],
        &options,
    );
    assert!(matches!(
        walked.unresolved[0].reason,
        TargetResolutionReason::IgnoredDirectory { .. }
    ));
}

#[test]
fn brace_alternation_ends_the_literal_glob_root() {
    let temporary = TempDirectory::new("brace-glob");
    let root = &temporary.0;
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("tests")).unwrap();
    fs::write(root.join("src/lib.rs"), "lib").unwrap();
    fs::write(root.join("tests/it.rs"), "it").unwrap();
    let report = resolve(
        &[target(root.join("{src,tests}/*.rs"), AccessScope::Glob)],
        &TargetResolutionOptions::new(vec![root.clone()]),
    );
    assert!(report.is_complete(), "{report:?}");
    assert_eq!(
        report.paths,
        [root.join("src/lib.rs"), root.join("tests/it.rs")]
            .into_iter()
            .collect()
    );
}

#[test]
fn relative_globs_with_an_unknown_base_are_not_guessed() {
    let temporary = TempDirectory::new("unknown-glob-base");
    fs::write(temporary.0.join("a.o"), "object").unwrap();
    let options = TargetResolutionOptions::new(vec![temporary.0.clone()]);
    let unknown = AccessTarget::Path {
        expression: PathExpression {
            raw: "*.o".to_owned(),
            resolved: None,
            base: PathBase::UnknownAfterDirectoryChange,
        },
        scope: AccessScope::Glob,
    };
    let report = resolve(std::slice::from_ref(&unknown), &options);
    assert!(report.paths.is_empty());
    assert!(!report.is_complete());
    assert!(matches!(
        report.unresolved[0].reason,
        TargetResolutionReason::UnresolvedPathExpression
    ));

    // An observable base that was not lexically resolved still anchors to
    // the workspace roots.
    let anchored = AccessTarget::Path {
        expression: PathExpression {
            raw: "*.o".to_owned(),
            resolved: None,
            base: PathBase::InvocationCwd,
        },
        scope: AccessScope::Glob,
    };
    let report = resolve(&[anchored], &options);
    assert!(report.paths.contains(&temporary.0.join("a.o")));
}

#[cfg(unix)]
#[test]
fn a_symlinked_scope_root_is_not_followed_by_default() {
    let temporary = TempDirectory::new("symlink-root");
    let shared = TempDirectory::new("symlink-shared");
    fs::write(shared.0.join("secret.txt"), "secret").unwrap();
    let link = temporary.0.join("vendor-link");
    std::os::unix::fs::symlink(&shared.0, &link).unwrap();
    let options = TargetResolutionOptions::new(vec![temporary.0.clone()]);

    let report = resolve(
        &[
            target(link.clone(), AccessScope::ExactOrDescendants),
            target(link.clone(), AccessScope::Descendants),
        ],
        &options,
    );
    assert_eq!(report.paths, [link.clone()].into_iter().collect());

    let mut follow = options.clone();
    follow.symlinks = hookkit_tool_access::SymlinkPolicy::Follow;
    let followed = resolve(&[target(link.clone(), AccessScope::Descendants)], &follow);
    assert!(followed.paths.contains(&link.join("secret.txt")));
}
