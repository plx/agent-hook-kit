//! Pkl-driven configuration loader for the post-tool-use agent hook runner.
//!
//! Public entry points:
//!
//! - [`discover_and_load`] — find user/project/local Pkl configs around `cwd`,
//!   merge them, and return the resolved [`RunnerConfig`] plus a project root.
//! - [`load_explicit`] — load a single Pkl file by path, bypassing discovery.
//! - [`evaluate_pkl_source`] — evaluate an in-memory Pkl source (used for
//!   testing and for synthesizing builtin-only configs).
//! - [`builtin_specs`] — evaluate the embedded `Builtins.pkl` module and
//!   return the bundled tool specs.
//!
//! The runtime entry-point `hookkit-tool-runner` consumes a [`Loaded`] value
//! and translates `schema::ToolSpec` into its execution-time `ToolSpec`.

pub mod discovery;
pub mod error;
pub mod eval;
pub mod merge;
pub mod schema;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use error::PklConfigError;
pub use eval::{
    BUILTINS_PKL, CONFIG_PKL, StagedBuiltins, evaluate_pkl_file, evaluate_pkl_file_patch,
    evaluate_pkl_source, evaluate_pkl_source_patch, staged_builtins_dir,
};
pub use schema::{
    ArgToken, ArgvElement, Diagnostics, ExitCodes, FileSelection, LoweringPolicy, Merge,
    MergeResetKey, Messages, MissingToolPolicy, Phase, PhaseMode, RunnerConfig, RunnerConfigPatch,
    Settings, SettingsPatch, ToolSpec, UnexpectedExitPolicy, WriteBehavior,
};

/// Result of loading the config chain.
#[derive(Debug, Clone)]
pub struct Loaded {
    /// The merged configuration after the discovery chain.
    pub config: RunnerConfig,
    /// Project root inferred from the inner-most project config (or the cwd
    /// if none was found).
    pub project_root: PathBuf,
}

/// Discover and load Pkl configs around `cwd`.
///
/// When `override_path` is provided, the discovery chain is bypassed and only
/// that file is loaded.
pub fn discover_and_load(
    cwd: &Path,
    override_path: Option<&Path>,
) -> Result<Loaded, PklConfigError> {
    if let Some(path) = override_path {
        let config = evaluate_pkl_file_patch(path)?.into_config();
        let project_root = path
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| cwd.to_path_buf());
        return Ok(Loaded {
            config,
            project_root,
        });
    }

    let chain = discovery::discover(cwd);

    let mut configs = Vec::with_capacity(chain.len());
    let mut project_root = cwd.to_path_buf();
    for discovered in &chain {
        let config = evaluate_pkl_file_patch(&discovered.path)?;
        if matches!(discovered.kind, discovery::DiscoveredKind::Project) {
            project_root = discovery::project_root_for(discovered, cwd);
        }
        configs.push(config);
    }

    let config = if configs.is_empty() {
        RunnerConfig::default()
    } else {
        merge::merge_patch_chain(configs.into_iter())
    };

    Ok(Loaded {
        config,
        project_root,
    })
}

/// Load a single Pkl file, bypassing discovery.
pub fn load_explicit(path: &Path, cwd: &Path) -> Result<Loaded, PklConfigError> {
    discover_and_load(cwd, Some(path))
}

/// Evaluate the embedded `Builtins.pkl` and return the bundled tool specs
/// keyed by their Pkl identifier (e.g. `"ruff"`, `"cargoFmt"`).
pub fn builtin_specs() -> Result<BTreeMap<String, ToolSpec>, PklConfigError> {
    let staging = staged_builtins_dir()?;
    let builtins_path = staging.builtins_path();

    let output = std::process::Command::new("pkl")
        .args(["eval", "--format", "json"])
        .arg(&builtins_path)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                PklConfigError::PklNotFound
            } else {
                PklConfigError::PklExec(e.to_string())
            }
        })?;

    if !output.status.success() {
        return Err(PklConfigError::PklEvalFailed {
            path: builtins_path,
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str::<BTreeMap<String, ToolSpec>>(&stdout).map_err(|e| {
        PklConfigError::JsonDecode {
            path: builtins_path,
            error: e.to_string(),
        }
    })
}
