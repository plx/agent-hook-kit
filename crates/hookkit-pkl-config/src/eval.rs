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
use crate::schema::RunnerConfig;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Embedded `Config.pkl` source — schema definitions for project configs.
pub const CONFIG_PKL: &str = include_str!("builtins/Config.pkl");
/// Embedded `Builtins.pkl` source — bundled tool specs.
pub const BUILTINS_PKL: &str = include_str!("builtins/Builtins.pkl");

/// Evaluate a Pkl file and parse the result as a [`RunnerConfig`].
///
/// `file_path` is the Pkl source to evaluate. The embedded `Config.pkl` and
/// `Builtins.pkl` modules are materialized to a sibling temp directory so
/// `amends "Config.pkl"` / `import "Builtins.pkl"` from project configs
/// resolve.
pub fn evaluate_pkl_file(file_path: &Path) -> Result<RunnerConfig, PklConfigError> {
    let staging = stage_builtins()?;
    let staged_target = staging.dir.join(unique_pkl_name("user"));
    copy_to_staging(file_path, &staged_target)?;
    let result = run_pkl_eval(&staged_target);
    drop(staging);
    result
}

/// Evaluate an in-memory Pkl source string.
pub fn evaluate_pkl_source(source: &str) -> Result<RunnerConfig, PklConfigError> {
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

/// Evaluate `Builtins.pkl` and return a temp directory where `Config.pkl`
/// and `Builtins.pkl` live as siblings.
///
/// Useful when callers want to evaluate Pkl that imports `Builtins.pkl`.
pub fn staged_builtins_dir() -> Result<StagedBuiltins, PklConfigError> {
    stage_builtins()
}

/// A temporary directory holding the embedded `Config.pkl` and `Builtins.pkl`.
/// Deleted on drop.
pub struct StagedBuiltins {
    pub dir: PathBuf,
}

impl StagedBuiltins {
    pub fn config_path(&self) -> PathBuf {
        self.dir.join("Config.pkl")
    }

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
    write_file(&dir.join("Config.pkl"), CONFIG_PKL)?;
    write_file(&dir.join("Builtins.pkl"), BUILTINS_PKL)?;
    Ok(StagedBuiltins { dir })
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

fn write_file(path: &Path, content: &str) -> Result<(), PklConfigError> {
    let mut file = std::fs::File::create(path).map_err(|e| PklConfigError::TempIo {
        path: path.to_path_buf(),
        error: e.to_string(),
    })?;
    file.write_all(content.as_bytes())
        .map_err(|e| PklConfigError::TempIo {
            path: path.to_path_buf(),
            error: e.to_string(),
        })?;
    Ok(())
}

fn run_pkl_eval(path: &Path) -> Result<RunnerConfig, PklConfigError> {
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
    serde_json::from_str::<RunnerConfig>(&stdout).map_err(|e| PklConfigError::JsonDecode {
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
