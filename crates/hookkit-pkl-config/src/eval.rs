//! Pkl evaluation via the `pkl` CLI.
//!
//! The runner does not statically link a Pkl interpreter. It requires the
//! `pkl` binary on PATH and invokes `pkl eval --format json`. The resulting
//! JSON is parsed into [`RunnerConfig`] or a per-layer [`RunnerConfigPatch`].
//!
//! Keeping evaluation out-of-process trades a fork+exec for a smaller binary
//! and full Pkl semantic coverage (abstract classes, amends, imports).
//!
//! Project configs refer to the embedded schema as siblings
//! (`amends "Config.pkl"`, `import "Builtins.pkl"`), so every evaluation
//! stages the embedded tree once into a private temporary directory and gives
//! each configuration layer its own subdirectory that links to that tree and
//! mirrors the layer's real siblings. All layers of one discovery chain are
//! then evaluated by a single `pkl eval` of a generated aggregator module.
//! Error text from Pkl is rewritten so staged paths name the real source
//! files. Imports that climb above the config's own directory (`../`) resolve
//! inside the staging directory and are not supported.

use crate::error::PklConfigError;
use crate::schema::{RunnerConfig, RunnerConfigPatch};
use include_dir::{Dir, DirEntry, include_dir};
use serde::de::DeserializeOwned;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// Embedded `builtins/` tree containing `Config.pkl`, the `Builtins.pkl`
/// aggregator, and `tools/<name>.pkl` per-tool spec modules.
static BUILTINS_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/builtins");

/// Embedded `Config.pkl` source — schema definitions for project configs.
pub const CONFIG_PKL: &str = include_str!("builtins/Config.pkl");
/// Embedded `Builtins.pkl` aggregator source. Requires the sibling `tools/`
/// directory to be staged alongside it to resolve its per-tool imports;
/// [`stage_builtins`](staged_builtins_dir) handles that.
pub const BUILTINS_PKL: &str = include_str!("builtins/Builtins.pkl");

/// File name given to each staged configuration layer. It is unlikely to
/// collide with a mirrored sibling and, unlike `config.pkl`, cannot alias
/// `Config.pkl` on a case-insensitive filesystem.
const STAGED_LAYER_NAME: &str = "hookkit-layer.pkl";
const AGGREGATOR_NAME: &str = "hookkit-layers.pkl";
const BUILTIN_ENTRIES: [&str; 3] = ["Config.pkl", "Builtins.pkl", "tools"];

static STAGING_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Evaluate a Pkl file and parse the result as a [`RunnerConfig`].
///
/// `file_path` is the Pkl source to evaluate. The embedded `Config.pkl`,
/// `Builtins.pkl`, and `tools/*.pkl` modules are materialized next to a
/// staged copy so `amends "Config.pkl"` / `import "Builtins.pkl"` from
/// project configs resolve. Other siblings from the real source directory
/// are mirrored into the staging directory so project-local relative imports
/// keep working.
pub fn evaluate_pkl_file(file_path: &Path) -> Result<RunnerConfig, PklConfigError> {
    evaluate_pkl_file_patch(file_path).map(RunnerConfigPatch::into_config)
}

/// Evaluate a Pkl file and keep per-field presence for multi-file merges.
pub fn evaluate_pkl_file_patch(file_path: &Path) -> Result<RunnerConfigPatch, PklConfigError> {
    let mut patches = evaluate_pkl_files_patch(&[file_path])?;
    Ok(patches.remove(0))
}

/// Evaluate several Pkl files with one staging directory and one `pkl`
/// process, returning one field-preserving patch per file in input order.
pub fn evaluate_pkl_files_patch(
    file_paths: &[&Path],
) -> Result<Vec<RunnerConfigPatch>, PklConfigError> {
    if file_paths.is_empty() {
        return Ok(Vec::new());
    }
    let staging = stage_builtins()?;
    let mut layers = Vec::with_capacity(file_paths.len());
    for (index, source) in file_paths.iter().enumerate() {
        let layer_dir = staging.create_layer_dir(index)?;
        mirror_source_siblings(source, &layer_dir)?;
        copy_to_staging(source, &layer_dir.join(STAGED_LAYER_NAME))?;
        layers.push(StagedLayer {
            source: Some(source.to_path_buf()),
            dir: layer_dir,
        });
    }
    evaluate_layers(&staging, &layers)
}

/// Evaluate an in-memory Pkl source string.
pub fn evaluate_pkl_source(source: &str) -> Result<RunnerConfig, PklConfigError> {
    evaluate_pkl_source_patch(source).map(RunnerConfigPatch::into_config)
}

/// Evaluate an in-memory Pkl source string and keep per-field presence.
pub fn evaluate_pkl_source_patch(source: &str) -> Result<RunnerConfigPatch, PklConfigError> {
    let staging = stage_builtins()?;
    let layer_dir = staging.create_layer_dir(0)?;
    let target = layer_dir.join(STAGED_LAYER_NAME);
    std::fs::write(&target, source).map_err(|e| PklConfigError::TempIo {
        path: target.clone(),
        source: e,
    })?;
    let layers = [StagedLayer {
        source: None,
        dir: layer_dir,
    }];
    let mut patches = evaluate_layers(&staging, &layers)?;
    Ok(patches.remove(0))
}

/// Stage the embedded `builtins/` tree to a private temp directory.
///
/// Useful when callers want to evaluate Pkl that imports `Builtins.pkl`.
pub fn staged_builtins_dir() -> Result<StagedBuiltins, PklConfigError> {
    stage_builtins()
}

/// A private temporary directory holding the embedded `Config.pkl`,
/// `Builtins.pkl`, and `tools/*.pkl` files. The whole staging tree is deleted
/// on drop.
///
/// The directory is created exclusively (never reusing an existing path) with
/// owner-only permissions on Unix, so another local user cannot pre-create it
/// and substitute the modules that define which executables the runner starts.
#[derive(Debug)]
pub struct StagedBuiltins {
    root: PathBuf,
    builtins: PathBuf,
}

impl StagedBuiltins {
    /// Returns the directory containing the staged embedded module tree.
    pub fn dir(&self) -> &Path {
        &self.builtins
    }

    /// Returns the staged `Config.pkl` path.
    pub fn config_path(&self) -> PathBuf {
        self.builtins.join("Config.pkl")
    }

    /// Returns the staged `Builtins.pkl` path.
    pub fn builtins_path(&self) -> PathBuf {
        self.builtins.join("Builtins.pkl")
    }

    /// Create `layer-<index>/` linking the staged builtins as siblings.
    fn create_layer_dir(&self, index: usize) -> Result<PathBuf, PklConfigError> {
        let dir = self.root.join(format!("layer-{index}"));
        create_private_dir(&dir)?;
        for name in BUILTIN_ENTRIES {
            mirror_path(&self.builtins.join(name), &dir.join(name))?;
        }
        Ok(dir)
    }
}

impl Drop for StagedBuiltins {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn stage_builtins() -> Result<StagedBuiltins, PklConfigError> {
    let root = create_private_temp_dir("hookkit-pkl-stage")?;
    // Construct the guard first so a failed write below still cleans up.
    let staged = StagedBuiltins {
        builtins: root.join("builtins"),
        root,
    };
    create_private_dir(&staged.builtins)?;
    write_embedded_dir(&BUILTINS_DIR, &staged.builtins)?;
    overwrite_builtins_aggregator(&staged.builtins)?;
    Ok(staged)
}

/// Create a fresh, uniquely named directory under the system temp directory.
/// Creation is exclusive: an existing path is never reused.
fn create_private_temp_dir(prefix: &str) -> Result<PathBuf, PklConfigError> {
    let base = std::env::temp_dir();
    for _ in 0..64 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let counter = STAGING_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = base.join(format!(
            "{prefix}-{}-{nanos:x}-{counter}",
            std::process::id()
        ));
        match create_private_dir_io(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(PklConfigError::TempIo {
                    path,
                    source: error,
                });
            }
        }
    }
    Err(PklConfigError::TempIo {
        path: base,
        source: std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not create a unique staging directory",
        ),
    })
}

/// Create exactly `path` (not its parents), owner-only on Unix. Fails if the
/// path already exists.
fn create_private_dir(path: &Path) -> Result<(), PklConfigError> {
    create_private_dir_io(path).map_err(|e| PklConfigError::TempIo {
        path: path.to_path_buf(),
        source: e,
    })
}

fn create_private_dir_io(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(path)
}

fn write_embedded_dir(dir: &Dir<'_>, target: &Path) -> Result<(), PklConfigError> {
    for entry in dir.entries() {
        match entry {
            DirEntry::File(file) => {
                let dst = target.join(file.path());
                std::fs::write(&dst, file.contents()).map_err(|e| PklConfigError::TempIo {
                    path: dst,
                    source: e,
                })?;
            }
            DirEntry::Dir(subdir) => {
                create_private_dir(&target.join(subdir.path()))?;
                write_embedded_dir(subdir, target)?;
            }
        }
    }
    Ok(())
}

/// Replace the staged `Builtins.pkl` with a freshly-generated aggregator that
/// imports every Pkl file under `tools/`. This means adding a builtin is a
/// pure "drop a file in `tools/`" operation — no edits to a shared aggregator
/// file, so parallel contributors don't conflict on `Builtins.pkl`.
fn overwrite_builtins_aggregator(dir: &Path) -> Result<(), PklConfigError> {
    let tools_dir = BUILTINS_DIR.get_dir("tools").ok_or_else(|| {
        PklConfigError::PklExec(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "embedded builtins missing tools/ subdirectory",
        ))
    })?;

    let mut tool_files: Vec<&str> = tools_dir
        .files()
        .filter_map(|f| {
            let path = f.path().file_name()?.to_str()?;
            path.strip_suffix(".pkl")
        })
        .collect();
    tool_files.sort();

    let mut out = String::new();
    out.push_str("/// Auto-generated by hookkit-pkl-config at runtime — do not edit.\n");
    out.push_str("///\n");
    out.push_str("/// Imports every tool spec under `tools/` and re-exports it as a top-level\n");
    out.push_str("/// property keyed by camelCase of the filename, matching the pre-split\n");
    out.push_str("/// layout. Snake-case filename `cargo_fmt.pkl` → property `cargoFmt`.\n\n");
    out.push_str("module agent_hook_kit.PostToolUseBuiltins\n\n");
    out.push_str("import \"Config.pkl\" as Config\n");

    for name in &tool_files {
        let camel = snake_to_camel(name);
        out.push_str(&format!("import \"tools/{name}.pkl\" as {camel}Mod\n"));
    }
    out.push('\n');
    for name in &tool_files {
        let camel = snake_to_camel(name);
        out.push_str(&format!("{camel}: Config.ToolSpec = {camel}Mod.spec\n"));
    }

    let target = dir.join("Builtins.pkl");
    std::fs::write(&target, out).map_err(|e| PklConfigError::TempIo {
        path: target,
        source: e,
    })
}

fn snake_to_camel(snake: &str) -> String {
    let mut out = String::with_capacity(snake.len());
    let mut capitalize_next = false;
    for ch in snake.chars() {
        if ch == '_' || ch == '-' {
            capitalize_next = true;
        } else if capitalize_next {
            out.extend(ch.to_uppercase());
            capitalize_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

fn copy_to_staging(src: &Path, dst: &Path) -> Result<(), PklConfigError> {
    let bytes = std::fs::read(src).map_err(|e| PklConfigError::ReadIo {
        path: src.to_path_buf(),
        source: e,
    })?;
    std::fs::write(dst, bytes).map_err(|e| PklConfigError::TempIo {
        path: dst.to_path_buf(),
        source: e,
    })
}

fn mirror_source_siblings(src: &Path, dst_dir: &Path) -> Result<(), PklConfigError> {
    let Some(src_dir) = src.parent() else {
        return Ok(());
    };
    let src_name = src.file_name();
    let entries = std::fs::read_dir(src_dir).map_err(|e| PklConfigError::ReadIo {
        path: src_dir.to_path_buf(),
        source: e,
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| PklConfigError::ReadIo {
            path: src_dir.to_path_buf(),
            source: e,
        })?;
        let name = entry.file_name();
        if Some(name.as_os_str()) == src_name
            || BUILTIN_ENTRIES
                .iter()
                .any(|builtin| name == OsStr::new(builtin))
            || name == OsStr::new(STAGED_LAYER_NAME)
        {
            continue;
        }

        let dst = dst_dir.join(&name);
        if dst.exists() {
            continue;
        }
        mirror_path(&entry.path(), &dst)?;
    }

    Ok(())
}

fn mirror_path(src: &Path, dst: &Path) -> Result<(), PklConfigError> {
    #[cfg(unix)]
    {
        if std::os::unix::fs::symlink(src, dst).is_ok() {
            return Ok(());
        }
    }

    copy_path(src, dst)
}

fn copy_path(src: &Path, dst: &Path) -> Result<(), PklConfigError> {
    let metadata = std::fs::metadata(src).map_err(|e| PklConfigError::ReadIo {
        path: src.to_path_buf(),
        source: e,
    })?;

    if metadata.is_dir() {
        std::fs::create_dir_all(dst).map_err(|e| PklConfigError::TempIo {
            path: dst.to_path_buf(),
            source: e,
        })?;
        for entry in std::fs::read_dir(src).map_err(|e| PklConfigError::ReadIo {
            path: src.to_path_buf(),
            source: e,
        })? {
            let entry = entry.map_err(|e| PklConfigError::ReadIo {
                path: src.to_path_buf(),
                source: e,
            })?;
            copy_path(&entry.path(), &dst.join(entry.file_name()))?;
        }
        return Ok(());
    }

    std::fs::copy(src, dst).map_err(|e| PklConfigError::TempIo {
        path: dst.to_path_buf(),
        source: e,
    })?;
    Ok(())
}

/// One staged configuration layer.
struct StagedLayer {
    /// Real source file, or `None` for in-memory source.
    source: Option<PathBuf>,
    /// Staging directory holding `hookkit-layer.pkl` and its siblings.
    dir: PathBuf,
}

impl StagedLayer {
    fn staged_file(&self) -> PathBuf {
        self.dir.join(STAGED_LAYER_NAME)
    }

    /// Path reported in errors: the real source, or the staged file for
    /// in-memory source.
    fn reported_path(&self) -> PathBuf {
        self.source.clone().unwrap_or_else(|| self.staged_file())
    }
}

#[derive(serde::Deserialize)]
struct AggregatedLayers {
    layers: Vec<serde_json::Value>,
}

/// Evaluate every staged layer with one `pkl eval` of a generated module that
/// imports each layer and renders it as one element of a `layers` listing.
fn evaluate_layers(
    staging: &StagedBuiltins,
    layers: &[StagedLayer],
) -> Result<Vec<RunnerConfigPatch>, PklConfigError> {
    let mut aggregator = String::from(
        "// Auto-generated by hookkit-pkl-config: evaluates every discovered layer.\n",
    );
    for index in 0..layers.len() {
        let _ = writeln!(
            aggregator,
            "import \"layer-{index}/{STAGED_LAYER_NAME}\" as layer{index}"
        );
    }
    aggregator.push_str("\nlayers = new Listing {\n");
    for index in 0..layers.len() {
        let _ = writeln!(aggregator, "  layer{index}");
    }
    aggregator.push_str("}\n");
    let aggregator_path = staging.root.join(AGGREGATOR_NAME);
    std::fs::write(&aggregator_path, aggregator).map_err(|e| PklConfigError::TempIo {
        path: aggregator_path.clone(),
        source: e,
    })?;

    let output = run_pkl(&aggregator_path)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let failing = failing_layer(&stderr, layers).unwrap_or(0);
        return Err(PklConfigError::PklEvalFailed {
            path: layers[failing].reported_path(),
            stderr: rewrite_staged_paths(&stderr, staging, layers),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let aggregated = serde_json::from_str::<AggregatedLayers>(&stdout).map_err(|e| {
        PklConfigError::JsonDecode {
            path: layers[0].reported_path(),
            source: e,
        }
    })?;
    if aggregated.layers.len() != layers.len() {
        return Err(PklConfigError::LayerCountMismatch {
            path: layers[0].reported_path(),
            expected: layers.len(),
            actual: aggregated.layers.len(),
        });
    }
    aggregated
        .layers
        .into_iter()
        .zip(layers)
        .map(|(value, layer)| {
            serde_json::from_value::<RunnerConfigPatch>(value).map_err(|e| {
                PklConfigError::JsonDecode {
                    path: layer.reported_path(),
                    source: e,
                }
            })
        })
        .collect()
}

/// The first layer named in Pkl's error text, if any.
fn failing_layer(stderr: &str, layers: &[StagedLayer]) -> Option<usize> {
    layers
        .iter()
        .enumerate()
        .filter_map(|(index, layer)| {
            staging_aliases(&layer.dir)
                .iter()
                .filter_map(|alias| stderr.find(&format!("{alias}/")))
                .min()
                .map(|position| (position, index))
        })
        .min()
        .map(|(_, index)| index)
}

/// Replace staged paths in Pkl's error text with the real source paths, so a
/// failure names the user's file instead of a deleted temporary copy.
fn rewrite_staged_paths(stderr: &str, staging: &StagedBuiltins, layers: &[StagedLayer]) -> String {
    let mut rewritten = stderr.to_owned();
    for layer in layers {
        let Some(source) = &layer.source else {
            continue;
        };
        let source_dir = source.parent().unwrap_or(Path::new(""));
        for alias in staging_aliases(&layer.dir) {
            rewritten = rewritten.replace(
                &format!("{alias}/{STAGED_LAYER_NAME}"),
                &source.to_string_lossy(),
            );
            rewritten = rewritten.replace(
                &format!("{alias}/"),
                &format!("{}/", source_dir.to_string_lossy()),
            );
        }
    }
    for alias in staging_aliases(&staging.root) {
        rewritten = rewritten.replace(
            &format!("{alias}/{AGGREGATOR_NAME}"),
            "<hookkit layer aggregator>",
        );
        rewritten = rewritten.replace(
            &format!("{alias}/builtins/"),
            "<hookkit embedded builtins>/",
        );
    }
    rewritten
}

/// A staged directory as Pkl may print it: as created and canonicalized
/// (macOS reports `/var/...` temp paths as `/private/var/...`).
fn staging_aliases(dir: &Path) -> Vec<String> {
    let mut aliases = vec![dir.to_string_lossy().into_owned()];
    if let Ok(canonical) = std::fs::canonicalize(dir) {
        let canonical = canonical.to_string_lossy().into_owned();
        if !aliases.contains(&canonical) {
            aliases.push(canonical);
        }
    }
    // Longest first so a canonical prefix is not partially replaced.
    aliases.sort_by_key(|alias| std::cmp::Reverse(alias.len()));
    aliases
}

fn run_pkl(path: &Path) -> Result<std::process::Output, PklConfigError> {
    Command::new("pkl")
        .args(["eval", "--format", "json"])
        .arg(path)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                PklConfigError::PklNotFound
            } else {
                PklConfigError::PklExec(e)
            }
        })
}

/// Evaluate one staged module directly and decode its JSON output.
pub(crate) fn run_pkl_eval<T>(path: &Path) -> Result<T, PklConfigError>
where
    T: DeserializeOwned,
{
    let output = run_pkl(path)?;
    if !output.status.success() {
        return Err(PklConfigError::PklEvalFailed {
            path: path.to_path_buf(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str::<T>(&stdout).map_err(|e| PklConfigError::JsonDecode {
        path: path.to_path_buf(),
        source: e,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn staging_directories_are_private_and_exclusive() {
        use std::os::unix::fs::PermissionsExt as _;

        let staged = stage_builtins().unwrap();
        let mode = std::fs::metadata(&staged.root)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
        assert!(staged.config_path().is_file());
        assert!(staged.builtins_path().is_file());
        assert!(staged.dir().join("tools").is_dir());

        // An existing path is never reused.
        assert!(create_private_dir(&staged.root).is_err());
        let root = staged.root.clone();
        drop(staged);
        assert!(!root.exists(), "staging tree is removed on drop");
    }

    #[test]
    fn staged_paths_in_errors_are_rewritten_to_sources() {
        let staging = stage_builtins().unwrap();
        let layer_dir = staging.create_layer_dir(0).unwrap();
        let layers = [StagedLayer {
            source: Some(PathBuf::from(
                "/repo/.agent-hook-kit/post-tool-use.local.pkl",
            )),
            dir: layer_dir.clone(),
        }];
        let stderr = format!(
            "–– Pkl Error ––\nat file://{}/{STAGED_LAYER_NAME} (line 3)\nimported from {}/shared.pkl\n",
            layer_dir.display(),
            layer_dir.display()
        );
        assert_eq!(failing_layer(&stderr, &layers), Some(0));
        let rewritten = rewrite_staged_paths(&stderr, &staging, &layers);
        assert!(
            rewritten.contains("file:///repo/.agent-hook-kit/post-tool-use.local.pkl (line 3)"),
            "{rewritten}"
        );
        assert!(
            rewritten.contains("/repo/.agent-hook-kit/shared.pkl"),
            "{rewritten}"
        );
        assert!(!rewritten.contains("hookkit-pkl-stage"), "{rewritten}");
    }
}
