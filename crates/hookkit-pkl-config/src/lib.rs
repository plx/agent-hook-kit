//! Pkl-driven configuration loader for the post-tool-use agent hook runner.
#![deny(missing_docs)]
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

pub mod catalog;
pub mod discovery;
pub mod error;
pub mod eval;
pub mod merge;
pub mod schema;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use catalog::{
    CatalogValidationError, render_builtin_catalog_markdown, validate_builtin_catalog,
};
pub use error::PklConfigError;
pub use eval::{
    BUILTINS_PKL, CONFIG_PKL, StagedBuiltins, evaluate_pkl_file, evaluate_pkl_file_patch,
    evaluate_pkl_files_patch, evaluate_pkl_source, evaluate_pkl_source_patch, staged_builtins_dir,
};
pub use schema::{
    ArgToken, ArgvElement, CheckScope, CoverageGapPolicy, DEFAULT_COMMAND_TIMEOUT_SECONDS,
    DEFAULT_IGNORED_DIRECTORY_NAMES, DeferredReporting, DeferredReportingPatch, Diagnostics,
    ExitCodes, FileActivitySettings, FileActivitySettingsPatch, FileActivityVcsFallback, FileGroup,
    FileSelection, InvocationGranularity, LoweringPolicy, Merge, MergeResetKey, Messages,
    MissingToolPolicy, Phase, PhaseMode, RunnerConfig, RunnerConfigPatch, Settings, SettingsPatch,
    TemplatePair, TemplatePairPatch, ToolSpec, UnexpectedExitPolicy, Workflow, WorkflowCommand,
    WriteBehavior,
};

/// Result of loading the config chain.
#[derive(Debug, Clone)]
pub struct Loaded {
    /// The merged configuration after the discovery chain.
    pub config: RunnerConfig,
    /// Project root: the deepest directory holding a discovered project or
    /// local config (never the home directory), or the cwd if none was found.
    pub project_root: PathBuf,
}

/// Discover and load Pkl configs around `cwd`.
///
/// When `override_path` is provided, the discovery chain is bypassed and only
/// that file is loaded. Every discovered layer is evaluated by one `pkl`
/// process over one staging directory.
pub fn discover_and_load(
    cwd: &Path,
    override_path: Option<&Path>,
) -> Result<Loaded, PklConfigError> {
    discover_and_load_with_home(cwd, override_path, dirs::home_dir().as_deref())
}

/// [`discover_and_load`] with an explicit home directory (`None` disables the
/// home layer), for embedders and tests that manage their own home.
pub fn discover_and_load_with_home(
    cwd: &Path,
    override_path: Option<&Path>,
    home: Option<&Path>,
) -> Result<Loaded, PklConfigError> {
    if let Some(path) = override_path {
        let config = merge::merge_patch_chain(std::iter::once(evaluate_pkl_file_patch(path)?));
        // `--config PATH` accepts arbitrary locations (e.g. `/tmp/custom.pkl`),
        // so we cannot infer a project root from the file's parents; anchor on
        // cwd as documented in `discovery::project_root_for`.
        return Ok(Loaded {
            config,
            project_root: cwd.to_path_buf(),
        });
    }

    let chain = discovery::discover_with_home(cwd, home);
    let paths = chain
        .iter()
        .map(|discovered| discovered.path.as_path())
        .collect::<Vec<_>>();
    let patches = evaluate_pkl_files_patch(&paths)?;
    let config = if patches.is_empty() {
        RunnerConfig::default()
    } else {
        merge::merge_patch_chain(patches.into_iter())
    };

    Ok(Loaded {
        config,
        project_root: discovery::project_root(&chain, cwd, home),
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
    let specs = eval::run_pkl_eval::<BTreeMap<String, ToolSpec>>(&staging.builtins_path())?;
    validate_builtin_catalog(&specs)
        .map_err(|error| PklConfigError::CatalogValidation(error.to_string()))?;
    Ok(specs)
}
