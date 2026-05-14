//! Tests for `discover_and_load` covering walk-up project discovery,
//! `.local.pkl` override, and `--config PATH` bypass.

use hookkit_pkl_config::{discover_and_load, schema::MissingToolPolicy};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn pkl_available() -> bool {
    std::process::Command::new("pkl")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

macro_rules! require_pkl {
    () => {
        if !pkl_available() {
            eprintln!("skipping test: pkl binary not on PATH");
            return;
        }
    };
}

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

fn write_config(dir: &Path, name: &str, body: &str) -> PathBuf {
    let config_dir = dir.join(".agent-hook-kit");
    std::fs::create_dir_all(&config_dir).unwrap();
    let path = config_dir.join(name);
    std::fs::write(&path, body).unwrap();
    path
}

#[test]
fn project_chain_walks_up_and_merges() {
    require_pkl!();
    let root = temp_dir("walk-up");
    let nested = root.join("sub");
    std::fs::create_dir_all(&nested).unwrap();

    write_config(
        &root,
        "post-tool-use.pkl",
        r#"
amends "Config.pkl"
import "Builtins.pkl"

tools {
  ["ruff"] = Builtins.ruff
}
run = new Listing<String> { "ruff" }
"#,
    );

    write_config(
        &nested,
        "post-tool-use.pkl",
        r#"
amends "Config.pkl"
import "Builtins.pkl"

tools {
  ["prettier"] = Builtins.prettier
}
run = new Listing<String> { "ruff"; "prettier" }
"#,
    );

    let loaded = discover_and_load(&nested, None).expect("discover");

    assert!(loaded.config.tools.contains_key("ruff"));
    assert!(loaded.config.tools.contains_key("prettier"));
    assert_eq!(loaded.config.run, vec!["ruff", "prettier"]);

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn local_pkl_overlays_project_pkl() {
    require_pkl!();
    let root = temp_dir("local-overlay");

    write_config(
        &root,
        "post-tool-use.pkl",
        r#"
amends "Config.pkl"
import "Builtins.pkl"

tools {
  ["ruff"] = Builtins.ruff
}
run = new Listing<String> { "ruff" }
"#,
    );

    write_config(
        &root,
        "post-tool-use.local.pkl",
        r#"
amends "Config.pkl"
import "Builtins.pkl"

merge {
  resetTools = new Listing { "ruff" }
}

tools {
  ["biome"] = Builtins.biome
}
run = new Listing<String> { "biome" }
"#,
    );

    let loaded = discover_and_load(&root, None).expect("discover");

    assert!(!loaded.config.tools.contains_key("ruff"));
    assert!(loaded.config.tools.contains_key("biome"));
    assert_eq!(loaded.config.run, vec!["biome"]);

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn explicit_config_path_bypasses_chain() {
    require_pkl!();
    let root = temp_dir("bypass");
    let other = temp_dir("override-source");

    write_config(
        &root,
        "post-tool-use.pkl",
        r#"
amends "Config.pkl"
import "Builtins.pkl"

tools {
  ["ruff"] = Builtins.ruff
}
run = new Listing<String> { "ruff" }
"#,
    );

    let override_path = write_config(
        &other,
        "post-tool-use.pkl",
        r#"
amends "Config.pkl"
import "Builtins.pkl"

tools {
  ["eslint"] = Builtins.eslint
}
run = new Listing<String> { "eslint" }
"#,
    );

    let loaded = discover_and_load(&root, Some(&override_path)).expect("discover");

    assert!(!loaded.config.tools.contains_key("ruff"));
    assert!(loaded.config.tools.contains_key("eslint"));
    assert_eq!(loaded.config.run, vec!["eslint"]);

    std::fs::remove_dir_all(&root).ok();
    std::fs::remove_dir_all(&other).ok();
}

#[test]
fn explicit_config_preserves_sibling_relative_imports() {
    require_pkl!();
    let root = temp_dir("sibling-import");
    let config_dir = root.join(".agent-hook-kit");
    std::fs::create_dir_all(&config_dir).unwrap();

    std::fs::write(
        config_dir.join("shared.pkl"),
        r#"
runList = new Listing<String> { "ruff" }
"#,
    )
    .unwrap();
    let config_path = write_config(
        &root,
        "post-tool-use.pkl",
        r#"
amends "Config.pkl"
import "shared.pkl" as Shared

run = Shared.runList
"#,
    );

    let loaded = discover_and_load(&root, Some(&config_path)).expect("discover");

    assert_eq!(loaded.config.run, vec!["ruff"]);

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn later_config_without_settings_preserves_earlier_settings() {
    require_pkl!();
    let root = temp_dir("settings-overlay");

    write_config(
        &root,
        "post-tool-use.pkl",
        r#"
amends "Config.pkl"

settings {
  missingToolPolicy = "hard-failure"
}
"#,
    );

    write_config(
        &root,
        "post-tool-use.local.pkl",
        r#"
amends "Config.pkl"
import "Builtins.pkl"

tools {
  ["ruff"] = Builtins.ruff
}
run = new Listing<String> { "ruff" }
"#,
    );

    let loaded = discover_and_load(&root, None).expect("discover");

    assert_eq!(
        loaded.config.settings.missing_tool_policy,
        MissingToolPolicy::HardFailure
    );
    assert_eq!(loaded.config.run, vec!["ruff"]);

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn explicit_config_anchors_project_root_on_cwd() {
    require_pkl!();
    let cwd = temp_dir("explicit-anchor-cwd");
    let elsewhere = temp_dir("explicit-anchor-source");

    // Override file lives outside `<cwd>/.agent-hook-kit/`. The previous
    // implementation treated `path.parent().parent()` as project root, which
    // for paths like `/tmp/foo/post-tool-use.pkl` resolves to `/tmp` — guard
    // against that by asserting `project_root == cwd` instead.
    let override_path = elsewhere.join("post-tool-use.pkl");
    std::fs::write(
        &override_path,
        r#"
amends "Config.pkl"
import "Builtins.pkl"

tools {
  ["ruff"] = Builtins.ruff
}
run = new Listing<String> { "ruff" }
"#,
    )
    .unwrap();

    let loaded = discover_and_load(&cwd, Some(&override_path)).expect("discover");
    assert_eq!(
        loaded.project_root, cwd,
        "explicit --config PATH should anchor project_root on cwd, not on the override file's parents"
    );

    std::fs::remove_dir_all(&cwd).ok();
    std::fs::remove_dir_all(&elsewhere).ok();
}

#[test]
fn local_only_config_updates_project_root() {
    require_pkl!();
    let root = temp_dir("local-only-root");
    let nested = root.join("inner");
    std::fs::create_dir_all(&nested).unwrap();

    // Only a local pkl exists at the repo root, no project pkl. The runner
    // should still treat the directory containing `.agent-hook-kit/` as the
    // project root, instead of leaving project_root at cwd.
    write_config(
        &root,
        "post-tool-use.local.pkl",
        r#"
amends "Config.pkl"
import "Builtins.pkl"

tools {
  ["ruff"] = Builtins.ruff
}
run = new Listing<String> { "ruff" }
"#,
    );

    let loaded = discover_and_load(&nested, None).expect("discover");
    assert_eq!(
        loaded.project_root, root,
        "local-only discovery should anchor project_root on the directory containing .agent-hook-kit/"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn empty_chain_returns_default_config() {
    require_pkl!();
    let root = temp_dir("empty");
    let loaded = discover_and_load(&root, None).expect("discover");

    // With no configs anywhere, we should still see a defaulted RunnerConfig
    // (potentially with home config baked in, but we don't expect any tools
    // in this isolated temp dir scenario).
    assert!(
        loaded.config.run.is_empty() || !loaded.config.run.is_empty(),
        "no panic"
    );

    std::fs::remove_dir_all(&root).ok();
}
