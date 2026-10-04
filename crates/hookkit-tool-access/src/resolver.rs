use crate::{AccessScope, AccessTarget, PathExpression};
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
    /// Do not descend into directory symlinks. A symlinked traversal root is
    /// reported as the link itself rather than walked.
    #[default]
    DoNotFollow,
    /// Allow the filesystem walker to descend through directory symlinks.
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
///
/// Construct with [`TargetResolutionOptions::new`] and assign fields.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TargetResolutionOptions {
    /// Roots used for unrooted workspace targets and for relative glob targets
    /// whose base is observable but was not lexically resolved.
    pub workspace_roots: Vec<Utf8PathBuf>,
    /// Directory basenames pruned from recursive traversal.
    ///
    /// They apply to traversal roots and walked directories only; an exact
    /// target (or an exact-or-descendants target naming a file) is never
    /// pruned by name, so `/repo/.git/hooks/pre-commit` still materializes.
    pub ignored_directory_names: BTreeSet<String>,
    /// Lexically normalized roots excluded from results and traversal.
    pub excluded_roots: BTreeSet<Utf8PathBuf>,
    /// Maximum directory entries and exact-path metadata probes.
    pub max_entries: usize,
    /// Whether recursive traversal follows directory symlinks, including a
    /// traversal root that is itself a symlink.
    pub symlinks: SymlinkPolicy,
    /// Whether nonexistent exact paths are retained.
    pub exact_paths: ExactPathPolicy,
    /// Policy for target-local filesystem errors.
    pub io_errors: ResolutionIssuePolicy,
    /// Policy for invalid glob expressions.
    pub invalid_globs: ResolutionIssuePolicy,
}

impl TargetResolutionOptions {
    /// Creates options with conservative traversal defaults.
    ///
    /// The default budget is 100,000 probed entries, directory symlinks are
    /// not followed, nonexistent exact paths are omitted, and target-local
    /// errors are reported rather than aborting the entire operation.
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
    /// The path expression has no statically resolved path.
    UnresolvedPathExpression,
    /// An exact path does not exist and policy says not to retain it.
    NonexistentExactPath,
    /// The root needed for a descendant walk does not exist.
    NonexistentTraversalRoot {
        /// Missing traversal root.
        path: Utf8PathBuf,
    },
    /// A workspace or relative-glob target has no workspace roots.
    MissingWorkspaceRoots,
    /// The target falls within a configured excluded root.
    ExcludedRoot {
        /// Excluded target or traversal root.
        path: Utf8PathBuf,
    },
    /// Traversal reached a directory whose basename is configured as ignored.
    IgnoredDirectory {
        /// Pruned directory.
        path: Utf8PathBuf,
    },
    /// A glob expression could not be compiled.
    InvalidGlob {
        /// Invalid glob text.
        pattern: String,
        /// Parser diagnostic.
        message: String,
    },
    /// A filesystem probe or directory walk failed.
    Io {
        /// Affected path, when the error can be localized.
        path: Option<Utf8PathBuf>,
        /// Portable I/O error category.
        kind: io::ErrorKind,
        /// Original I/O error message.
        message: String,
    },
    /// Traversal encountered a path that cannot be represented as UTF-8.
    NonUtf8Path {
        /// Lossy display form of the path.
        path: String,
    },
    /// The shared entry/probe budget was exhausted.
    BudgetExhausted,
}

/// One target and the target-local reason it was not fully processed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedTarget {
    /// Original unified target.
    pub target: AccessTarget,
    /// Target-local reason materialization was incomplete.
    pub reason: TargetResolutionReason,
}

/// Bounded materialization result. `paths` may include directories and, when
/// configured, exact paths that do not exist yet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedTargets {
    /// Deterministic set of materialized UTF-8 paths.
    pub paths: BTreeSet<Utf8PathBuf>,
    /// Targets or traversal branches that could not be fully processed.
    pub unresolved: Vec<UnresolvedTarget>,
    /// Number of directory entries and exact-path probes charged to the budget.
    pub scanned_entries: usize,
    /// Whether materialization stopped after reaching `max_entries`.
    pub budget_exhausted: bool,
}

impl ResolvedTargets {
    /// Returns whether every target was processed without a retained issue.
    pub fn is_complete(&self) -> bool {
        self.unresolved.is_empty() && !self.budget_exhausted
    }
}

/// Failure returned when an issue policy is [`ResolutionIssuePolicy::Abort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetResolutionError {
    /// Target being processed when an abort policy triggered.
    pub target: AccessTarget,
    /// Reason materialization failed.
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
                    self.resolve_glob(target, expression)
                } else {
                    let Some(path) = expression.resolved.as_deref() else {
                        self.unresolved(target, TargetResolutionReason::UnresolvedPathExpression);
                        return Ok(false);
                    };
                    self.resolve_path(target, path, *scope)
                }
            }
            AccessTarget::Workspace { root: Some(root) } => {
                self.walk(target, root, None, Walk::workspace())
            }
            AccessTarget::Workspace { root: None } => {
                if self.options.workspace_roots.is_empty() {
                    self.unresolved(target, TargetResolutionReason::MissingWorkspaceRoots);
                    return Ok(false);
                }
                for root in &self.options.workspace_roots {
                    if self.walk(target, root, None, Walk::workspace())? {
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
        if let Some(reason) = self.excluded(&path) {
            self.unresolved(target, reason);
            return Ok(false);
        }
        let follow_root = self.options.symlinks == SymlinkPolicy::Follow;
        match scope {
            AccessScope::Exact => self.exact(target, &path),
            AccessScope::Descendants => self.walk(
                target,
                &path,
                None,
                Walk {
                    include_root: false,
                    follow_root,
                },
            ),
            AccessScope::ExactOrDescendants => {
                if self.is_traversable_directory(&path) {
                    self.walk(
                        target,
                        &path,
                        None,
                        Walk {
                            include_root: true,
                            follow_root,
                        },
                    )
                } else {
                    self.exact(target, &path)
                }
            }
            AccessScope::Glob => unreachable!("glob handled separately"),
        }
    }

    /// Whether `path` is a directory to walk. Under
    /// [`SymlinkPolicy::DoNotFollow`], a symlink to a directory is not.
    fn is_traversable_directory(&self, path: &Utf8Path) -> bool {
        match self.options.symlinks {
            SymlinkPolicy::Follow => path.is_dir(),
            _ => std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir()),
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

    /// Materializes a glob target.
    ///
    /// A relative glob is anchored to the workspace roots only when its base
    /// is observable; after an unknown directory change, without a working
    /// directory, or in another environment it is reported as an unresolved
    /// expression rather than guessed. Syntax is validated first, so an
    /// invalid glob is always reported as such.
    fn resolve_glob(
        &mut self,
        target: &AccessTarget,
        expression: &PathExpression,
    ) -> Result<bool, TargetResolutionError> {
        let resolved = expression.resolved.as_deref();
        let raw = resolved.unwrap_or_else(|| Utf8Path::new(&expression.raw));
        if let Err(error) = GlobBuilder::new(raw.as_str())
            .literal_separator(true)
            .build()
        {
            self.issue(
                target,
                TargetResolutionReason::InvalidGlob {
                    pattern: raw.to_string(),
                    message: error.to_string(),
                },
                self.options.invalid_globs,
            )?;
            return Ok(false);
        }
        let patterns = if raw.is_absolute() {
            vec![normalize_utf8_path(raw)]
        } else if !expression.base.is_observable() {
            self.unresolved(target, TargetResolutionReason::UnresolvedPathExpression);
            return Ok(false);
        } else if self.options.workspace_roots.is_empty() {
            self.unresolved(target, TargetResolutionReason::MissingWorkspaceRoots);
            return Ok(false);
        } else {
            self.options
                .workspace_roots
                .iter()
                .map(|root| normalize_utf8_path(root.join(raw)))
                .collect()
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
            // A shell expands a glob's literal prefix through symlinks, so the
            // prefix is followed even when nested links are not.
            if self.walk(
                target,
                &root,
                Some(&matcher),
                Walk {
                    include_root: true,
                    follow_root: true,
                },
            )? {
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
        walk: Walk,
    ) -> Result<bool, TargetResolutionError> {
        let Walk {
            include_root,
            follow_root,
        } = walk;
        let root = normalize_utf8_path(root);
        if let Some(reason) = self
            .excluded(&root)
            .or_else(|| self.ignored_traversal_root(&root))
        {
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
        let follow_links = self.options.symlinks == SymlinkPolicy::Follow;
        let walker = WalkDir::new(root.as_std_path())
            .follow_links(follow_links)
            .follow_root_links(follow_links || follow_root)
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

    /// Reports a path inside a configured excluded root.
    fn excluded(&self, path: &Utf8Path) -> Option<TargetResolutionReason> {
        self.options
            .excluded_roots
            .iter()
            .map(normalize_utf8_path)
            .find(|excluded| path.starts_with(excluded))
            .map(|path| TargetResolutionReason::ExcludedRoot { path })
    }

    /// Reports a traversal root that lies in, or is, an ignored directory.
    ///
    /// Inside a workspace root, every component relative to that root is
    /// checked, matching what a walk from the workspace root would prune.
    /// Outside every workspace root only the directory's own name is
    /// checked, so ignored names in unrelated ancestors do not apply.
    fn ignored_traversal_root(&self, path: &Utf8Path) -> Option<TargetResolutionReason> {
        let ignored = &self.options.ignored_directory_names;
        let relative = self
            .options
            .workspace_roots
            .iter()
            .map(normalize_utf8_path)
            .find_map(|root| path.strip_prefix(root).ok().map(Utf8Path::to_path_buf));
        let is_ignored = match relative {
            Some(relative) => relative
                .components()
                .any(|component| ignored.contains(component.as_str())),
            None => path.file_name().is_some_and(|name| ignored.contains(name)),
        };
        is_ignored.then(|| TargetResolutionReason::IgnoredDirectory {
            path: path.to_path_buf(),
        })
    }
}

/// Traversal behavior for one walk.
#[derive(Debug, Clone, Copy)]
struct Walk {
    /// Whether the root itself is materialized.
    include_root: bool,
    /// Whether a root that is a directory symlink is walked through.
    follow_root: bool,
}

impl Walk {
    /// Configured workspace roots are walked through a root symlink.
    const fn workspace() -> Self {
        Self {
            include_root: false,
            follow_root: true,
        }
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

/// Returns the longest literal directory prefix of a glob. Any component with
/// wildcard, class, alternation (`{a,b}`), or escape syntax ends the prefix.
fn literal_glob_root(pattern: &Utf8Path) -> Utf8PathBuf {
    let mut prefix = Utf8PathBuf::new();
    for component in pattern.components() {
        if component
            .as_str()
            .contains(['*', '?', '[', ']', '{', '}', '\\'])
        {
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
