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
