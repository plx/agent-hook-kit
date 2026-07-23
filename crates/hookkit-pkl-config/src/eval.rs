//! Pkl evaluation via the `pkl` CLI.
//!
//! The runner does not statically link a Pkl interpreter. It requires the
//! `pkl` binary on PATH and invokes `pkl eval --format json` on a temporary
//! file containing the merged Pkl source. The resulting JSON is parsed into
//! [`RunnerConfig`].
//!
//! Keeping evaluation out-of-process trades a fork+exec for a smaller binary
//! and full Pkl semantic coverage (abstract classes, amends, imports).

use crate::error::PklConfigError;
use crate::schema::{RunnerConfig, RunnerConfigPatch};
use include_dir::{Dir, DirEntry, include_dir};
use serde::de::DeserializeOwned;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Embedded `builtins/` tree containing `Config.pkl`, the `Builtins.pkl`
/// aggregator, and `tools/<name>.pkl` per-tool spec modules.
static BUILTINS_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/builtins");

/// Embedded `Config.pkl` source — schema definitions for project configs.
pub const CONFIG_PKL: &str = include_str!("builtins/Config.pkl");
/// Embedded `Builtins.pkl` aggregator source. Requires the sibling `tools/`
/// directory to be staged alongside it to resolve its per-tool imports;
/// [`stage_builtins`](staged_builtins_dir) handles that.
pub const BUILTINS_PKL: &str = include_str!("builtins/Builtins.pkl");

/// Evaluate a Pkl file and parse the result as a [`RunnerConfig`].
///
/// `file_path` is the Pkl source to evaluate. The embedded `Config.pkl`,
/// `Builtins.pkl`, and `tools/*.pkl` modules are materialized to a sibling
/// temp directory so `amends "Config.pkl"` / `import "Builtins.pkl"` from
/// project configs resolve. Other siblings from the real source directory
/// are mirrored into the staging directory so project-local relative imports
/// keep working.
pub fn evaluate_pkl_file(file_path: &Path) -> Result<RunnerConfig, PklConfigError> {
    evaluate_pkl_file_patch(file_path).map(RunnerConfigPatch::into_config)
}

/// Evaluate a Pkl file and keep per-field presence for multi-file merges.
pub fn evaluate_pkl_file_patch(file_path: &Path) -> Result<RunnerConfigPatch, PklConfigError> {
    let staging = stage_builtins()?;
    mirror_source_siblings(file_path, &staging.dir)?;
    let staged_target = staging.dir.join(unique_pkl_name("user"));
    copy_to_staging(file_path, &staged_target)?;
    let result = run_pkl_eval(&staged_target);
    drop(staging);
    result
}

/// Evaluate an in-memory Pkl source string.
pub fn evaluate_pkl_source(source: &str) -> Result<RunnerConfig, PklConfigError> {
    evaluate_pkl_source_patch(source).map(RunnerConfigPatch::into_config)
}

/// Evaluate an in-memory Pkl source string and keep per-field presence.
pub fn evaluate_pkl_source_patch(source: &str) -> Result<RunnerConfigPatch, PklConfigError> {
    let staging = stage_builtins()?;
    let target = staging.dir.join(unique_pkl_name("inline"));
    std::fs::write(&target, source).map_err(|e| PklConfigError::TempIo {
        path: target.clone(),
        error: e.to_string(),
    })?;
    let result = run_pkl_eval(&target);
    drop(staging);
    result
}

/// Stage the embedded `builtins/` tree to a temp directory.
///
/// Useful when callers want to evaluate Pkl that imports `Builtins.pkl`.
pub fn staged_builtins_dir() -> Result<StagedBuiltins, PklConfigError> {
    stage_builtins()
}

/// A temporary directory holding the embedded `Config.pkl`, `Builtins.pkl`,
/// and `tools/*.pkl` files. Deleted on drop.
pub struct StagedBuiltins {
    /// Temporary directory containing the complete embedded module tree.
    pub dir: PathBuf,
}

impl StagedBuiltins {
    /// Returns the staged `Config.pkl` path.
    pub fn config_path(&self) -> PathBuf {
        self.dir.join("Config.pkl")
    }

    /// Returns the staged `Builtins.pkl` path.
    pub fn builtins_path(&self) -> PathBuf {
        self.dir.join("Builtins.pkl")
    }
}

impl Drop for StagedBuiltins {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn stage_builtins() -> Result<StagedBuiltins, PklConfigError> {
    let dir = unique_temp_dir("hookkit-pkl-stage");
    std::fs::create_dir_all(&dir).map_err(|e| PklConfigError::TempIo {
        path: dir.clone(),
        error: e.to_string(),
    })?;
    write_embedded_dir(&BUILTINS_DIR, &dir)?;
    overwrite_builtins_aggregator(&dir)?;
    Ok(StagedBuiltins { dir })
}

fn write_embedded_dir(dir: &Dir<'_>, target: &Path) -> Result<(), PklConfigError> {
    for entry in dir.entries() {
        match entry {
            DirEntry::File(file) => {
                let dst = target.join(file.path());
                if let Some(parent) = dst.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| PklConfigError::TempIo {
                        path: parent.to_path_buf(),
                        error: e.to_string(),
                    })?;
                }
                std::fs::write(&dst, file.contents()).map_err(|e| PklConfigError::TempIo {
                    path: dst,
                    error: e.to_string(),
                })?;
            }
            DirEntry::Dir(subdir) => {
                let dst = target.join(subdir.path());
                std::fs::create_dir_all(&dst).map_err(|e| PklConfigError::TempIo {
                    path: dst.clone(),
                    error: e.to_string(),
                })?;
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
        PklConfigError::PklExec("embedded builtins missing tools/ subdirectory".to_string())
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
        error: e.to_string(),
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
        error: e.to_string(),
    })?;
    std::fs::write(dst, bytes).map_err(|e| PklConfigError::TempIo {
        path: dst.to_path_buf(),
        error: e.to_string(),
    })
}

fn mirror_source_siblings(src: &Path, dst_dir: &Path) -> Result<(), PklConfigError> {
    let Some(src_dir) = src.parent() else {
        return Ok(());
    };
    let src_name = src.file_name();
    let entries = std::fs::read_dir(src_dir).map_err(|e| PklConfigError::ReadIo {
        path: src_dir.to_path_buf(),
        error: e.to_string(),
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| PklConfigError::ReadIo {
            path: src_dir.to_path_buf(),
            error: e.to_string(),
        })?;
        let name = entry.file_name();
        if Some(name.as_os_str()) == src_name
            || name == OsStr::new("Config.pkl")
            || name == OsStr::new("Builtins.pkl")
            || name == OsStr::new("tools")
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
        error: e.to_string(),
    })?;

    if metadata.is_dir() {
        std::fs::create_dir_all(dst).map_err(|e| PklConfigError::TempIo {
            path: dst.to_path_buf(),
            error: e.to_string(),
        })?;
        for entry in std::fs::read_dir(src).map_err(|e| PklConfigError::ReadIo {
            path: src.to_path_buf(),
            error: e.to_string(),
        })? {
            let entry = entry.map_err(|e| PklConfigError::ReadIo {
                path: src.to_path_buf(),
                error: e.to_string(),
            })?;
            copy_path(&entry.path(), &dst.join(entry.file_name()))?;
        }
        return Ok(());
    }

    std::fs::copy(src, dst).map_err(|e| PklConfigError::TempIo {
        path: dst.to_path_buf(),
        error: e.to_string(),
    })?;
    Ok(())
}

fn run_pkl_eval<T>(path: &Path) -> Result<T, PklConfigError>
where
    T: DeserializeOwned,
{
    let output = Command::new("pkl")
        .args(["eval", "--format", "json"])
        .arg(path)
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
            path: path.to_path_buf(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str::<T>(&stdout).map_err(|e| PklConfigError::JsonDecode {
        path: path.to_path_buf(),
        error: e.to_string(),
    })
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()))
}

fn unique_pkl_name(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{prefix}-{}-{nanos}.pkl", std::process::id())
}
