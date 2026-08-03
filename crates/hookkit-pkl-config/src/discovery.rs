//! Config discovery rules.
//!
//! Discovery layers (each merged over the previous, last-write-wins):
//!
//! 1. **Home/global** — `~/.agent-hook-kit/post-tool-use.pkl`
//! 2. **Project chain** — walking up from `cwd`, each ancestor's
//!    `.agent-hook-kit/post-tool-use.pkl` (root → leaf order)
//! 3. **Local chain** — walking up from `cwd`, each ancestor's
//!    `.agent-hook-kit/post-tool-use.local.pkl` (root → leaf order)
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
    let mut chain = Vec::new();

    if let Some(home) = home_config_path() {
        if home.is_file() {
            chain.push(DiscoveredConfig {
                path: home,
                kind: DiscoveredKind::Home,
            });
        }
    }

    // Walk ancestors root-first so child configs override parent configs.
    let mut ancestors: Vec<PathBuf> = cwd.ancestors().map(Path::to_path_buf).collect();
    ancestors.reverse();

    for ancestor in &ancestors {
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
/// should use `cwd` in those cases.
pub fn project_root_for(config: &DiscoveredConfig, cwd: &Path) -> PathBuf {
    match config.kind {
        DiscoveredKind::Home => cwd.to_path_buf(),
        DiscoveredKind::Project | DiscoveredKind::Local => config
            .path
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| cwd.to_path_buf()),
    }
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

        let chain = discover(&nested);
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
}
