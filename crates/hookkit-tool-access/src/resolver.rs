use crate::{AccessScope, AccessTarget};
use globset::{GlobBuilder, GlobMatcher};
use hookkit_core::{Utf8Path, Utf8PathBuf, normalize_utf8_path};
use std::borrow::Borrow;
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::io;
use walkdir::{DirEntry, WalkDir};

/// Whether an exact target that does not exist is still materialized.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExactPathPolicy {
    /// Only return exact targets currently present on disk.
    #[default]
    ExistingOnly,
    /// Retain a lexical exact target even when a pre-tool write will create it.
    RetainNonexistent,
}

/// Whether directory symlinks may be traversed while materializing scopes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SymlinkPolicy {
    #[default]
    DoNotFollow,
    Follow,
}

/// What to do with a target-local glob or filesystem failure.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResolutionIssuePolicy {
    /// Retain the failure in [`ResolvedTargets::unresolved`], then continue.
    #[default]
    Report,
    /// Stop and return the first failure.
    Abort,
}

/// Explicit controls for bounded filesystem materialization.
#[derive(Debug, Clone)]
pub struct TargetResolutionOptions {
    /// Roots used for unrooted workspace and relative glob targets.
    pub workspace_roots: Vec<Utf8PathBuf>,
    pub ignored_directory_names: BTreeSet<String>,
    pub excluded_roots: BTreeSet<Utf8PathBuf>,
    /// Maximum directory entries and exact-path metadata probes.
    pub max_entries: usize,
    pub symlinks: SymlinkPolicy,
    pub exact_paths: ExactPathPolicy,
    pub io_errors: ResolutionIssuePolicy,
    pub invalid_globs: ResolutionIssuePolicy,
}

impl TargetResolutionOptions {
    pub fn new(workspace_roots: Vec<Utf8PathBuf>) -> Self {
        Self {
            workspace_roots,
            ignored_directory_names: default_ignored_directory_names(),
            excluded_roots: BTreeSet::new(),
            max_entries: 100_000,
            symlinks: SymlinkPolicy::DoNotFollow,
            exact_paths: ExactPathPolicy::ExistingOnly,
            io_errors: ResolutionIssuePolicy::Report,
            invalid_globs: ResolutionIssuePolicy::Report,
        }
    }
}

/// Why a unified target could not be completely materialized.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TargetResolutionReason {
    UnresolvedPathExpression,
    NonexistentExactPath,
    NonexistentTraversalRoot {
        path: Utf8PathBuf,
    },
    MissingWorkspaceRoots,
    ExcludedRoot {
        path: Utf8PathBuf,
    },
    IgnoredDirectory {
        path: Utf8PathBuf,
    },
    InvalidGlob {
        pattern: String,
        message: String,
    },
    Io {
        path: Option<Utf8PathBuf>,
        kind: io::ErrorKind,
        message: String,
    },
    NonUtf8Path {
        path: String,
    },
    BudgetExhausted,
}

/// One target and the target-local reason it was not fully processed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedTarget {
    pub target: AccessTarget,
    pub reason: TargetResolutionReason,
}

/// Bounded materialization result. `paths` may include directories and, when
/// configured, exact paths that do not exist yet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedTargets {
    pub paths: BTreeSet<Utf8PathBuf>,
    pub unresolved: Vec<UnresolvedTarget>,
    pub scanned_entries: usize,
    pub budget_exhausted: bool,
}

impl ResolvedTargets {
    pub fn is_complete(&self) -> bool {
        self.unresolved.is_empty() && !self.budget_exhausted
    }
}

/// Failure returned when an issue policy is [`ResolutionIssuePolicy::Abort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetResolutionError {
    pub target: AccessTarget,
    pub reason: TargetResolutionReason,
}

impl fmt::Display for TargetResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "target resolution failed: {:?}", self.reason)
    }
}

impl Error for TargetResolutionError {}

/// Materialize unified targets within explicit bounds.
///
/// This walks the filesystem. It is not a symbolic proof that two glob or
/// descendant scopes intersect, and changes racing the walk may affect the
/// returned snapshot.
pub fn resolve_targets<I, T>(
    targets: I,
    options: &TargetResolutionOptions,
) -> Result<ResolvedTargets, TargetResolutionError>
where
    I: IntoIterator<Item = T>,
    T: Borrow<AccessTarget>,
{
    let targets = targets
        .into_iter()
        .map(|target| target.borrow().clone())
        .collect::<Vec<_>>();
    let mut resolver = Resolver {
        options,
        result: ResolvedTargets::default(),
    };

    for (index, target) in targets.iter().enumerate() {
        if resolver.result.scanned_entries >= options.max_entries {
            resolver.exhaust(&targets[index..]);
            break;
        }
        if resolver.resolve(target)? {
            resolver.exhaust(&targets[index..]);
            break;
        }
    }
    Ok(resolver.result)
}

struct Resolver<'a> {
    options: &'a TargetResolutionOptions,
    result: ResolvedTargets,
}

impl Resolver<'_> {
    /// Returns whether the budget was exhausted while processing this target.
    fn resolve(&mut self, target: &AccessTarget) -> Result<bool, TargetResolutionError> {
        match target {
            AccessTarget::Path { expression, scope } => {
                if *scope == AccessScope::Glob {
                    self.resolve_glob(
                        target,
                        expression.raw.as_str(),
                        expression.resolved.as_deref(),
                    )
                } else {
                    let Some(path) = expression.resolved.as_deref() else {
                        self.unresolved(target, TargetResolutionReason::UnresolvedPathExpression);
                        return Ok(false);
                    };
                    self.resolve_path(target, path, *scope)
                }
            }
            AccessTarget::Workspace { root: Some(root) } => self.walk(target, root, None, false),
            AccessTarget::Workspace { root: None } => {
                if self.options.workspace_roots.is_empty() {
                    self.unresolved(target, TargetResolutionReason::MissingWorkspaceRoots);
                    return Ok(false);
                }
                for root in &self.options.workspace_roots {
                    if self.walk(target, root, None, false)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
        }
    }

    fn resolve_path(
        &mut self,
        target: &AccessTarget,
        path: &Utf8Path,
        scope: AccessScope,
    ) -> Result<bool, TargetResolutionError> {
        let path = normalize_utf8_path(path);
        if let Some(reason) = self.filtered_root(&path) {
            self.unresolved(target, reason);
            return Ok(false);
        }
        match scope {
            AccessScope::Exact => self.exact(target, &path),
            AccessScope::Descendants => self.walk(target, &path, None, false),
            AccessScope::ExactOrDescendants => {
                if path.is_dir() {
                    self.walk(target, &path, None, true)
                } else {
                    self.exact(target, &path)
                }
            }
            AccessScope::Glob => unreachable!("glob handled separately"),
        }
    }

    fn exact(
        &mut self,
        target: &AccessTarget,
        path: &Utf8Path,
    ) -> Result<bool, TargetResolutionError> {
        if !self.reserve_entry() {
            return Ok(true);
        }
        match std::fs::symlink_metadata(path) {
            Ok(_) => {
                self.result.paths.insert(path.to_path_buf());
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if self.options.exact_paths == ExactPathPolicy::RetainNonexistent {
                    self.result.paths.insert(path.to_path_buf());
                } else {
                    self.unresolved(target, TargetResolutionReason::NonexistentExactPath);
                }
            }
            Err(error) => self.issue(
                target,
                TargetResolutionReason::Io {
                    path: Some(path.to_path_buf()),
                    kind: error.kind(),
                    message: error.to_string(),
                },
                self.options.io_errors,
            )?,
        }
        Ok(false)
    }

    fn resolve_glob(
        &mut self,
        target: &AccessTarget,
        raw: &str,
        resolved: Option<&Utf8Path>,
    ) -> Result<bool, TargetResolutionError> {
        let patterns = if let Some(resolved) = resolved.filter(|path| path.is_absolute()) {
            vec![normalize_utf8_path(resolved)]
        } else {
            let raw = resolved.unwrap_or_else(|| Utf8Path::new(raw));
            if raw.is_absolute() {
                vec![normalize_utf8_path(raw)]
            } else if self.options.workspace_roots.is_empty() {
                self.unresolved(target, TargetResolutionReason::MissingWorkspaceRoots);
                return Ok(false);
            } else {
                self.options
                    .workspace_roots
                    .iter()
                    .map(|root| normalize_utf8_path(root.join(raw)))
                    .collect()
            }
        };

        for pattern in patterns {
            let matcher = match GlobBuilder::new(pattern.as_str())
                .literal_separator(true)
                .build()
            {
                Ok(glob) => glob.compile_matcher(),
                Err(error) => {
                    self.issue(
                        target,
                        TargetResolutionReason::InvalidGlob {
                            pattern: pattern.to_string(),
                            message: error.to_string(),
                        },
                        self.options.invalid_globs,
                    )?;
                    return Ok(false);
                }
            };
            let root = literal_glob_root(&pattern);
            if self.walk(target, &root, Some(&matcher), true)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn walk(
        &mut self,
        target: &AccessTarget,
        root: &Utf8Path,
        matcher: Option<&GlobMatcher>,
        include_root: bool,
    ) -> Result<bool, TargetResolutionError> {
        let root = normalize_utf8_path(root);
        if let Some(reason) = self.filtered_root(&root) {
            self.unresolved(target, reason);
            return Ok(false);
        }
        match std::fs::symlink_metadata(&root) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.unresolved(
                    target,
                    TargetResolutionReason::NonexistentTraversalRoot { path: root },
                );
                return Ok(false);
            }
            Err(error) => {
                self.issue(
                    target,
                    TargetResolutionReason::Io {
                        path: Some(root),
                        kind: error.kind(),
                        message: error.to_string(),
                    },
                    self.options.io_errors,
                )?;
                return Ok(false);
            }
        }
        let ignored = self.options.ignored_directory_names.clone();
        let excluded = self
            .options
            .excluded_roots
            .iter()
            .map(normalize_utf8_path)
            .collect::<BTreeSet<_>>();
        let walker = WalkDir::new(root.as_std_path())
            .follow_links(self.options.symlinks == SymlinkPolicy::Follow)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(|entry| !entry_is_filtered(entry, &ignored, &excluded));
        for entry in walker {
            if !self.reserve_entry() {
                return Ok(true);
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    let path = error
                        .path()
                        .and_then(|path| Utf8PathBuf::from_path_buf(path.to_path_buf()).ok())
                        .map(|path| normalize_utf8_path(&path));
                    let io_error = error.io_error();
                    self.issue(
                        target,
                        TargetResolutionReason::Io {
                            path,
                            kind: io_error.map_or(io::ErrorKind::Other, io::Error::kind),
                            message: error.to_string(),
                        },
                        self.options.io_errors,
                    )?;
                    continue;
                }
            };
            if (!include_root && entry.depth() == 0)
                || entry_is_filtered(&entry, &ignored, &excluded)
            {
                continue;
            }
            let Some(path) = Utf8PathBuf::from_path_buf(entry.path().to_path_buf()).ok() else {
                self.unresolved(
                    target,
                    TargetResolutionReason::NonUtf8Path {
                        path: entry.path().display().to_string(),
                    },
                );
                continue;
            };
            let path = normalize_utf8_path(&path);
            if matcher.is_none_or(|matcher| matcher.is_match(path.as_std_path())) {
                self.result.paths.insert(path);
            }
        }
        Ok(false)
    }

    fn reserve_entry(&mut self) -> bool {
        if self.result.scanned_entries >= self.options.max_entries {
            self.result.budget_exhausted = true;
            false
        } else {
            self.result.scanned_entries += 1;
            true
        }
    }

    fn exhaust(&mut self, targets: &[AccessTarget]) {
        self.result.budget_exhausted = true;
        for target in targets {
            self.unresolved(target, TargetResolutionReason::BudgetExhausted);
        }
    }

    fn issue(
        &mut self,
        target: &AccessTarget,
        reason: TargetResolutionReason,
        policy: ResolutionIssuePolicy,
    ) -> Result<(), TargetResolutionError> {
        if policy == ResolutionIssuePolicy::Abort {
            Err(TargetResolutionError {
                target: target.clone(),
                reason,
            })
        } else {
            self.unresolved(target, reason);
            Ok(())
        }
    }

    fn unresolved(&mut self, target: &AccessTarget, reason: TargetResolutionReason) {
        self.result.unresolved.push(UnresolvedTarget {
            target: target.clone(),
            reason,
        });
    }

    fn filtered_root(&self, path: &Utf8Path) -> Option<TargetResolutionReason> {
        if let Some(excluded) = self
            .options
            .excluded_roots
            .iter()
            .map(normalize_utf8_path)
            .find(|excluded| path == excluded || path.starts_with(excluded))
        {
            return Some(TargetResolutionReason::ExcludedRoot { path: excluded });
        }
        let relative = self
            .options
            .workspace_roots
            .iter()
            .map(normalize_utf8_path)
            .find_map(|root| path.strip_prefix(root).ok());
        let ignored = relative.map_or_else(
            || {
                path.file_name()
                    .is_some_and(|name| self.options.ignored_directory_names.contains(name))
            },
            |relative| {
                relative.components().any(|component| {
                    self.options
                        .ignored_directory_names
                        .contains(component.as_str())
                })
            },
        );
        if ignored {
            return Some(TargetResolutionReason::IgnoredDirectory {
                path: path.to_path_buf(),
            });
        }
        None
    }
}

fn entry_is_filtered(
    entry: &DirEntry,
    ignored_directory_names: &BTreeSet<String>,
    excluded_roots: &BTreeSet<Utf8PathBuf>,
) -> bool {
    let ignored = entry.file_type().is_dir()
        && entry
            .file_name()
            .to_str()
            .is_some_and(|name| ignored_directory_names.contains(name));
    let excluded = excluded_roots.iter().any(|excluded| {
        entry.path() == excluded.as_std_path() || entry.path().starts_with(excluded.as_std_path())
    });
    ignored || excluded
}

fn literal_glob_root(pattern: &Utf8Path) -> Utf8PathBuf {
    let mut prefix = Utf8PathBuf::new();
    for component in pattern.components() {
        if component.as_str().contains(['*', '?', '[', ']']) {
            break;
        }
        prefix.push(component.as_str());
    }
    if prefix.as_str().is_empty() {
        Utf8PathBuf::from(".")
    } else if pattern == prefix {
        prefix.parent().unwrap_or(Utf8Path::new(".")).to_path_buf()
    } else {
        prefix
    }
}

fn default_ignored_directory_names() -> BTreeSet<String> {
    [".context", ".git", ".hg", ".svn", "node_modules", "target"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}
