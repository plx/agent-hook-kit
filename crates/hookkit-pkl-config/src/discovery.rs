//! Config discovery rules.
//!
//! Discovery layers (each merged over the previous, last-write-wins):
//!
//! 1. **Home/global** — `~/.agent-hook-kit/post-tool-use.pkl`
//! 2. **Project chain** — walking up from `cwd`, each ancestor's
//!    `.agent-hook-kit/post-tool-use.pkl` (root → leaf order). The home
//!    directory's own file is the home layer and is never re-added here.
//! 3. **Local chain** — walking up from `cwd`, each ancestor's
//!    `.agent-hook-kit/post-tool-use.local.pkl` (root → leaf order)
//!
//! The project root is the deepest directory that holds a project or local
//! config, independent of merge order; configs in the home directory never
//! make the home directory the project root. Without such a config the
//! caller's `cwd` is the project root.
//!
//! When `--config PATH` is passed, the entire chain is bypassed and only that
//! file is used.

use std::path::{Path, PathBuf};

/// Filename used for inherited home and project configuration.
pub const PROJECT_CONFIG_NAME: &str = "post-tool-use.pkl";
/// Filename used for local, normally uncommitted configuration.
pub const LOCAL_CONFIG_NAME: &str = "post-tool-use.local.pkl";
/// Directory searched at the home and project-ancestor levels.
pub const CONFIG_DIR: &str = ".agent-hook-kit";

/// One step in the config discovery chain.
#[derive(Debug, Clone)]
pub struct DiscoveredConfig {
    /// Existing configuration file.
    pub path: PathBuf,
    /// Discovery layer that supplied the file.
    pub kind: DiscoveredKind,
}

/// Layer in the configuration discovery and precedence chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveredKind {
    /// User-wide configuration under the home directory.
    Home,
    /// Inherited project configuration.
    Project,
    /// Local project configuration, merged after every project file.
    Local,
}

/// Discover the configs that should be loaded, in merge order (earliest first).
pub fn discover(cwd: &Path) -> Vec<DiscoveredConfig> {
    discover_with_home(cwd, dirs::home_dir().as_deref())
}

/// [`discover`] with an explicit home directory (`None` disables the home
/// layer), for embedders and tests that manage their own home.
pub fn discover_with_home(cwd: &Path, home: Option<&Path>) -> Vec<DiscoveredConfig> {
    let mut chain = Vec::new();
    let home_dir = home.map(canonical_or_lexical);

    if let Some(home) = home {
        let home_config = home.join(CONFIG_DIR).join(PROJECT_CONFIG_NAME);
        if home_config.is_file() {
            chain.push(DiscoveredConfig {
                path: home_config,
                kind: DiscoveredKind::Home,
            });
        }
    }

    // Walk ancestors root-first so child configs override parent configs.
    let mut ancestors: Vec<PathBuf> = cwd.ancestors().map(Path::to_path_buf).collect();
    ancestors.reverse();

    for ancestor in &ancestors {
        // The home directory's project-named file is already the home layer;
        // adding it again would evaluate it twice and make `$HOME` the
        // project root for every project below it.
        if home_dir.as_deref() == Some(canonical_or_lexical(ancestor).as_path()) {
            continue;
        }
        let candidate = ancestor.join(CONFIG_DIR).join(PROJECT_CONFIG_NAME);
        if candidate.is_file() {
            chain.push(DiscoveredConfig {
                path: candidate,
                kind: DiscoveredKind::Project,
            });
        }
    }

    for ancestor in &ancestors {
        let candidate = ancestor.join(CONFIG_DIR).join(LOCAL_CONFIG_NAME);
        if candidate.is_file() {
            chain.push(DiscoveredConfig {
                path: candidate,
                kind: DiscoveredKind::Local,
            });
        }
    }

    chain
}

/// Returns the conventional home configuration path, if a home directory exists.
pub fn home_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(CONFIG_DIR).join(PROJECT_CONFIG_NAME))
}

/// Project root associated with a discovered config (or the cwd as fallback).
///
/// Home configs and `--config PATH` files do not imply a project root; callers
/// should use `cwd` in those cases. To pick the root for a whole chain, use
/// [`project_root`].
pub fn project_root_for(config: &DiscoveredConfig, cwd: &Path) -> PathBuf {
    match config.kind {
        DiscoveredKind::Home => cwd.to_path_buf(),
        DiscoveredKind::Project | DiscoveredKind::Local => config_owner_dir(config)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| cwd.to_path_buf()),
    }
}

/// Project root for a discovery chain: the deepest directory holding a
/// project or local config, excluding the home directory, or `cwd`.
///
/// Merge order does not matter: an outer `.local.pkl` layer (merged after
/// every project layer) never replaces the root of an inner project config.
pub fn project_root(chain: &[DiscoveredConfig], cwd: &Path, home: Option<&Path>) -> PathBuf {
    let home_dir = home.map(canonical_or_lexical);
    chain
        .iter()
        .filter(|config| config.kind != DiscoveredKind::Home)
        .filter_map(config_owner_dir)
        .filter(|dir| home_dir.as_deref() != Some(canonical_or_lexical(dir).as_path()))
        .max_by_key(|dir| dir.components().count())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| cwd.to_path_buf())
}

/// Directory that contains a config's `.agent-hook-kit` directory.
fn config_owner_dir(config: &DiscoveredConfig) -> Option<&Path> {
    config.path.parent().and_then(Path::parent)
}

fn canonical_or_lexical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| hookkit_core::normalize_path(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "hookkit-pkl-discovery-{name}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_config(dir: &Path, name: &str) -> PathBuf {
        let config_dir = dir.join(CONFIG_DIR);
        std::fs::create_dir_all(&config_dir).unwrap();
        let path = config_dir.join(name);
        std::fs::write(&path, "// stub").unwrap();
        path
    }

    #[test]
    fn discovers_project_then_local_in_root_first_order() {
        let root = temp_dir("project-local");
        let nested = root.join("a/b/c");
        std::fs::create_dir_all(&nested).unwrap();
        let root_project = write_config(&root, PROJECT_CONFIG_NAME);
        let inner_project = write_config(&nested, PROJECT_CONFIG_NAME);
        let root_local = write_config(&root, LOCAL_CONFIG_NAME);

        let chain = discover_with_home(&nested, None);
        let kinds: Vec<_> = chain.iter().map(|c| (c.kind, c.path.clone())).collect();
        assert!(
            kinds.contains(&(DiscoveredKind::Project, root_project)),
            "root project missing: {kinds:?}"
        );
        assert!(
            kinds.contains(&(DiscoveredKind::Project, inner_project)),
            "nested project missing: {kinds:?}"
        );
        assert!(
            kinds.contains(&(DiscoveredKind::Local, root_local)),
            "root local missing: {kinds:?}"
        );

        // Projects should appear before locals, and within each kind ancestors
        // should appear before descendants.
        let project_positions: Vec<usize> = chain
            .iter()
            .enumerate()
            .filter_map(|(i, c)| (c.kind == DiscoveredKind::Project).then_some(i))
            .collect();
        let local_positions: Vec<usize> = chain
            .iter()
            .enumerate()
            .filter_map(|(i, c)| (c.kind == DiscoveredKind::Local).then_some(i))
            .collect();
        if let (Some(&p_max), Some(&l_min)) =
            (project_positions.iter().max(), local_positions.iter().min())
        {
            assert!(p_max < l_min, "project should come before local: {chain:?}");
        }

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn home_config_is_discovered_once_and_never_becomes_the_project_root() {
        let home = temp_dir("home-once");
        let project = home.join("code/proj");
        std::fs::create_dir_all(&project).unwrap();
        let home_config = write_config(&home, PROJECT_CONFIG_NAME);

        let chain = discover_with_home(&project, Some(&home));
        assert_eq!(chain.len(), 1, "{chain:?}");
        assert_eq!(chain[0].kind, DiscoveredKind::Home);
        assert_eq!(chain[0].path, home_config);
        assert_eq!(project_root(&chain, &project, Some(&home)), project);

        // Running from the home directory itself still evaluates it once.
        let chain = discover_with_home(&home, Some(&home));
        assert_eq!(chain.len(), 1, "{chain:?}");

        // A home-level local override does not make `$HOME` the root either.
        write_config(&home, LOCAL_CONFIG_NAME);
        let chain = discover_with_home(&project, Some(&home));
        assert_eq!(chain.len(), 2, "{chain:?}");
        assert_eq!(project_root(&chain, &project, Some(&home)), project);

        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn outer_local_layer_does_not_replace_the_inner_project_root() {
        let repo = temp_dir("outer-local");
        let package = repo.join("pkg");
        let cwd = package.join("src");
        std::fs::create_dir_all(&cwd).unwrap();
        write_config(&repo, LOCAL_CONFIG_NAME);
        write_config(&package, PROJECT_CONFIG_NAME);

        let chain = discover_with_home(&cwd, None);
        assert_eq!(
            chain.last().map(|config| config.kind),
            Some(DiscoveredKind::Local),
            "the outer local layer still merges last: {chain:?}"
        );
        assert_eq!(project_root(&chain, &cwd, None), package);

        std::fs::remove_dir_all(&repo).ok();
    }
}
