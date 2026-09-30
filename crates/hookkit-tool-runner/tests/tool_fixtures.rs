//! Fixture-driven E2E tests for the post-tool-use-agent-hook builtins.
//!
//! The harness auto-discovers `tests/fixtures/<tool>/<example>/` directories
//! at test time and runs each against the real `post-tool-use-agent-hook`
//! binary for each post-tool harness (Claude Code and Codex). This compatibility suite
//! is ignored by default because it intentionally executes arbitrary tool
//! versions from `PATH`. Run it explicitly with `--ignored --nocapture`.
//!
//! See `tests/fixtures/README.md` for the on-disk format.

use hookkit_pkl_config::ToolSpec;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const HARNESSES: &[&str] = &["claude", "codex"];
const STABLE_SESSION: &str = "test-session";
const STABLE_TOOL_USE: &str = "fixture-tool";
const STABLE_TURN: &str = "fixture-turn";

/// Comma-separated tool ids to run (for example `HOOKKIT_FIXTURE_TOOLS=jq`);
/// unset runs every fixture.
fn selected_tools() -> Option<Vec<String>> {
    std::env::var("HOOKKIT_FIXTURE_TOOLS").ok().map(|value| {
        value
            .split(',')
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .collect()
    })
}

#[test]
#[ignore = "real-tool compatibility lane; requires controlled PATH versions"]
fn run_all_tool_fixtures() {
    if !pkl_available() {
        assert!(
            !pkl_required(),
            "HOOKKIT_REQUIRE_PKL is set, but the pkl binary is not on PATH"
        );
        eprintln!("skipping: pkl binary not on PATH");
        return;
    }

    let fixtures_root = manifest_dir().join("tests/fixtures");
    if !fixtures_root.exists() {
        eprintln!("no fixtures directory at {fixtures_root:?}; nothing to test");
        return;
    }

    let specs = match hookkit_pkl_config::builtin_specs() {
        Ok(specs) => specs,
        Err(e) => panic!("failed to load builtin specs: {e}"),
    };

    let mut id_to_property: HashMap<String, String> = HashMap::new();
    let mut id_to_spec: HashMap<String, ToolSpec> = HashMap::new();
    for (property, spec) in specs {
        id_to_property.insert(spec.id.clone(), property);
        id_to_spec.insert(spec.id.clone(), spec);
    }

    let mut results: Vec<FixtureOutcome> = Vec::new();

    let tool_entries = match std::fs::read_dir(&fixtures_root) {
        Ok(entries) => entries,
        Err(e) => panic!("failed to read {fixtures_root:?}: {e}"),
    };

    for tool_entry in tool_entries.flatten() {
        if !tool_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let tool_id = tool_entry.file_name().to_string_lossy().into_owned();
        let tool_dir = tool_entry.path();
        if selected_tools().is_some_and(|selected| !selected.contains(&tool_id)) {
            continue;
        }

        let Some(spec) = id_to_spec.get(&tool_id) else {
            results.push(FixtureOutcome::skipped(
                &tool_id,
                "*",
                "*",
                format!("no builtin spec for fixture directory '{tool_id}'"),
            ));
            continue;
        };
        let Some(prop_name) = id_to_property.get(&tool_id) else {
            results.push(FixtureOutcome::skipped(
                &tool_id,
                "*",
                "*",
                "no Pkl property name for tool",
            ));
            continue;
        };

        if !tool_executable_available(&spec.executable) {
            results.push(FixtureOutcome::skipped(
                &tool_id,
                "*",
                "*",
                format!("executable '{}' not on PATH", spec.executable),
            ));
            continue;
        }

        let example_entries = match std::fs::read_dir(&tool_dir) {
            Ok(entries) => entries,
            Err(e) => {
                results.push(FixtureOutcome::failed(
                    &tool_id,
                    "*",
                    "*",
                    format!("read_dir({tool_dir:?}): {e}"),
                ));
                continue;
            }
        };
        for example_entry in example_entries.flatten() {
            if !example_entry
                .file_type()
                .map(|t| t.is_dir())
                .unwrap_or(false)
            {
                continue;
            }
            let example_name = example_entry.file_name().to_string_lossy().into_owned();
            let example_dir = example_entry.path();

            for harness in HARNESSES {
                let outcome = run_fixture_for_harness(
                    &tool_id,
                    &example_name,
                    &example_dir,
                    harness,
                    prop_name,
                );
                results.push(outcome);
            }
        }
    }

    let mut passed = 0;
    let mut skipped = 0;
    let mut failed = 0;
    let mut failures = Vec::new();
    for outcome in &results {
        match &outcome.status {
            FixtureStatus::Pass => {
                passed += 1;
                println!(
                    "PASS  {}/{} ({})",
                    outcome.tool, outcome.example, outcome.harness
                );
            }
            FixtureStatus::Skip(reason) => {
                skipped += 1;
                println!(
                    "SKIP  {}/{} ({}): {reason}",
                    outcome.tool, outcome.example, outcome.harness
                );
            }
            FixtureStatus::Fail(reason) => {
                failed += 1;
                eprintln!(
                    "FAIL  {}/{} ({}):\n{reason}\n",
                    outcome.tool, outcome.example, outcome.harness
                );
                failures.push(format!(
                    "{}/{} ({}): {reason}",
                    outcome.tool, outcome.example, outcome.harness
                ));
            }
        }
    }
    println!("\nSummary: {passed} passed, {skipped} skipped, {failed} failed");
    assert!(
        failed == 0,
        "{failed} fixture(s) failed:\n{}",
        failures.join("\n\n")
    );
}

struct FixtureOutcome {
    tool: String,
    example: String,
    harness: String,
    status: FixtureStatus,
}

enum FixtureStatus {
    Pass,
    Skip(String),
    Fail(String),
}

impl FixtureOutcome {
    fn pass(tool: &str, example: &str, harness: &str) -> Self {
        Self {
            tool: tool.to_string(),
            example: example.to_string(),
            harness: harness.to_string(),
            status: FixtureStatus::Pass,
        }
    }
    fn skipped(tool: &str, example: &str, harness: &str, reason: impl Into<String>) -> Self {
        Self {
            tool: tool.to_string(),
            example: example.to_string(),
            harness: harness.to_string(),
            status: FixtureStatus::Skip(reason.into()),
        }
    }
    fn failed(tool: &str, example: &str, harness: &str, reason: impl Into<String>) -> Self {
        Self {
            tool: tool.to_string(),
            example: example.to_string(),
            harness: harness.to_string(),
            status: FixtureStatus::Fail(reason.into()),
        }
    }
}

fn run_fixture_for_harness(
    tool_id: &str,
    example: &str,
    fixture_dir: &Path,
    harness: &str,
    pkl_property: &str,
) -> FixtureOutcome {
    let temp_project = match prepare_temp_project(tool_id, example, harness, fixture_dir) {
        Ok(p) => p,
        Err(e) => return FixtureOutcome::failed(tool_id, example, harness, e),
    };

    let entry_rel = match find_entry_file(fixture_dir) {
        Ok(p) => p,
        Err(e) => {
            cleanup(&temp_project);
            return FixtureOutcome::failed(tool_id, example, harness, e);
        }
    };

    if let Err(e) = write_pkl_config(&temp_project, tool_id, pkl_property) {
        cleanup(&temp_project);
        return FixtureOutcome::failed(tool_id, example, harness, e);
    }

    let fixture_json = synthesize_hook_event(harness, &temp_project, &entry_rel);
    let output = match run_binary(harness, &fixture_json) {
        Ok(o) => o,
        Err(e) => {
            cleanup(&temp_project);
            return FixtureOutcome::failed(tool_id, example, harness, e);
        }
    };

    if std::env::var_os("HOOKKIT_FIXTURE_BLESS").is_some() {
        if let Err(e) = bless_outputs(harness, fixture_dir, &temp_project, &output) {
            cleanup(&temp_project);
            return FixtureOutcome::failed(tool_id, example, harness, e);
        }
    }
    let result = verify_outputs(
        tool_id,
        example,
        harness,
        fixture_dir,
        &temp_project,
        &output,
    );
    cleanup(&temp_project);
    match result {
        Ok(()) => FixtureOutcome::pass(tool_id, example, harness),
        Err(e) => FixtureOutcome::failed(tool_id, example, harness, e),
    }
}

fn verify_outputs(
    _tool_id: &str,
    _example: &str,
    harness: &str,
    fixture_dir: &Path,
    temp_project: &Path,
    output: &std::process::Output,
) -> Result<(), String> {
    let actual_stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let actual_stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let actual_exit = output.status.code().unwrap_or(-1);
    let project_paths = workspace_path_aliases(temp_project);

    // Compare stdout (parsed as JSON if golden is JSON; else literal string match).
    let stdout_golden_path = fixture_dir.join(format!("{harness}.json"));
    if stdout_golden_path.exists() {
        let golden = std::fs::read_to_string(&stdout_golden_path)
            .map_err(|e| format!("reading {stdout_golden_path:?}: {e}"))?;
        let golden = normalize(&golden, &project_paths);
        let actual_norm = normalize(&actual_stdout, &project_paths);
        match (
            serde_json::from_str::<JsonValue>(&golden),
            serde_json::from_str::<JsonValue>(&actual_norm),
        ) {
            (Ok(g), Ok(a)) => {
                if g != a {
                    return Err(format!(
                        "stdout JSON mismatch:\n  expected: {g}\n  actual:   {a}",
                    ));
                }
            }
            _ => {
                if golden.trim() != actual_norm.trim() {
                    return Err(format!(
                        "stdout mismatch (non-JSON):\n  expected: {golden}\n  actual:   {actual_norm}",
                    ));
                }
            }
        }
    } else {
        let normalized = normalize(&actual_stdout, &project_paths);
        if !normalized.trim().is_empty() {
            return Err(format!(
                "stdout expected empty but got:\n{normalized}\n(write {stdout_golden_path:?} to assert content)",
            ));
        }
    }

    // Compare stderr (literal string match after normalization).
    let stderr_golden_path = fixture_dir.join(format!("{harness}.stderr.txt"));
    if stderr_golden_path.exists() {
        let golden = std::fs::read_to_string(&stderr_golden_path)
            .map_err(|e| format!("reading {stderr_golden_path:?}: {e}"))?;
        let golden_norm = normalize(&golden, &project_paths);
        let actual_norm = normalize(&actual_stderr, &project_paths);
        if golden_norm.trim() != actual_norm.trim() {
            return Err(format!(
                "stderr mismatch:\n  expected:\n{golden_norm}\n  actual:\n{actual_norm}",
            ));
        }
    } else {
        let normalized = normalize(&actual_stderr, &project_paths);
        if !normalized.trim().is_empty() {
            return Err(format!(
                "stderr expected empty but got:\n{normalized}\n(write {stderr_golden_path:?} to assert content)",
            ));
        }
    }

    // Compare exit code.
    let exit_golden_path = fixture_dir.join(format!("{harness}.exit"));
    let expected_exit: i32 = if exit_golden_path.exists() {
        std::fs::read_to_string(&exit_golden_path)
            .map_err(|e| format!("reading {exit_golden_path:?}: {e}"))?
            .trim()
            .parse()
            .map_err(|e| format!("parsing {exit_golden_path:?}: {e}"))?
    } else {
        0
    };
    if actual_exit != expected_exit {
        return Err(format!(
            "exit code mismatch: expected {expected_exit}, got {actual_exit}\nstdout:\n{actual_stdout}\nstderr:\n{actual_stderr}",
        ));
    }

    // Compare post-run file states from expected/ subdir if present.
    let expected_root = fixture_dir.join("expected");
    if expected_root.exists() {
        verify_expected_tree(&expected_root, &expected_root, temp_project)?;
    }

    Ok(())
}

/// Rewrite one fixture's golden stdout, stderr, and exit files from an actual
/// run (`HOOKKIT_FIXTURE_BLESS=1`). Use only with controlled tool versions.
fn bless_outputs(
    harness: &str,
    fixture_dir: &Path,
    temp_project: &Path,
    output: &std::process::Output,
) -> Result<(), String> {
    let aliases = workspace_path_aliases(temp_project);
    let stdout = normalize(&String::from_utf8_lossy(&output.stdout), &aliases);
    let stderr = normalize(&String::from_utf8_lossy(&output.stderr), &aliases);
    let write_or_remove = |name: String, contents: Option<String>| -> Result<(), String> {
        let path = fixture_dir.join(name);
        match contents {
            Some(contents) => {
                std::fs::write(&path, contents).map_err(|e| format!("write {path:?}: {e}"))
            }
            None if path.exists() => {
                std::fs::remove_file(&path).map_err(|e| format!("remove {path:?}: {e}"))
            }
            None => Ok(()),
        }
    };
    let stdout = (!stdout.trim().is_empty()).then(|| {
        serde_json::from_str::<JsonValue>(&stdout)
            .map(|value| format!("{value}\n"))
            .unwrap_or(stdout)
    });
    write_or_remove(format!("{harness}.json"), stdout)?;
    write_or_remove(
        format!("{harness}.stderr.txt"),
        (!stderr.trim().is_empty()).then_some(stderr),
    )?;
    let exit = output.status.code().unwrap_or(-1);
    write_or_remove(
        format!("{harness}.exit"),
        (exit != 0).then(|| format!("{exit}\n")),
    )
}

fn verify_expected_tree(root: &Path, current: &Path, temp_project: &Path) -> Result<(), String> {
    let entries = std::fs::read_dir(current).map_err(|e| format!("read_dir({current:?}): {e}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let ft = entry
            .file_type()
            .map_err(|e| format!("file_type({path:?}): {e}"))?;
        if ft.is_dir() {
            verify_expected_tree(root, &path, temp_project)?;
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map_err(|e| format!("strip_prefix({path:?}): {e}"))?;
        let actual_path = temp_project.join(rel);
        let expected = std::fs::read_to_string(&path)
            .map_err(|e| format!("reading expected {path:?}: {e}"))?;
        let actual = std::fs::read_to_string(&actual_path).map_err(|e| {
            format!("reading post-run {actual_path:?} (for expected/{rel:?}): {e}",)
        })?;
        if expected != actual {
            return Err(format!(
                "post-run file content mismatch for {rel:?}:\n  expected:\n{expected}\n  actual:\n{actual}",
            ));
        }
    }
    Ok(())
}

fn prepare_temp_project(
    tool_id: &str,
    example: &str,
    harness: &str,
    fixture_dir: &Path,
) -> Result<PathBuf, String> {
    let project = unique_temp_dir(&format!("hookkit-fixture-{tool_id}-{example}-{harness}"));
    std::fs::create_dir_all(&project).map_err(|e| format!("create_dir_all: {e}"))?;
    copy_fixture_inputs(fixture_dir, fixture_dir, &project)?;
    Ok(project)
}

fn copy_fixture_inputs(root: &Path, current: &Path, target: &Path) -> Result<(), String> {
    let entries = std::fs::read_dir(current).map_err(|e| format!("read_dir({current:?}): {e}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip golden outputs and expected/ subtree at the fixture root.
        if current == root {
            if name == OsStr::new("expected") {
                continue;
            }
            if is_golden_output(&name_str) || name == OsStr::new("README.md") {
                continue;
            }
        }

        let ft = entry
            .file_type()
            .map_err(|e| format!("file_type({path:?}): {e}"))?;
        let rel = path
            .strip_prefix(root)
            .map_err(|e| format!("strip_prefix({path:?}): {e}"))?;
        let dst = target.join(rel);
        if ft.is_dir() {
            std::fs::create_dir_all(&dst).map_err(|e| format!("mkdir {dst:?}: {e}"))?;
            copy_fixture_inputs(root, &path, target)?;
        } else {
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {parent:?}: {e}"))?;
            }
            std::fs::copy(&path, &dst).map_err(|e| format!("copy {path:?} -> {dst:?}: {e}"))?;
        }
    }
    Ok(())
}

fn is_golden_output(name: &str) -> bool {
    for harness in HARNESSES {
        if name == format!("{harness}.json")
            || name == format!("{harness}.stderr.txt")
            || name == format!("{harness}.exit")
        {
            return true;
        }
    }
    false
}

fn find_entry_file(fixture_dir: &Path) -> Result<PathBuf, String> {
    // Prefer a top-level file named `example.*`. Fall back to the first non-
    // golden, non-expected/ file at the root.
    let mut candidates: Vec<PathBuf> = Vec::new();
    let entries =
        std::fs::read_dir(fixture_dir).map_err(|e| format!("read_dir({fixture_dir:?}): {e}"))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name == OsStr::new("expected") || name == OsStr::new("README.md") {
            continue;
        }
        if is_golden_output(&name_str) {
            continue;
        }
        let ft = entry
            .file_type()
            .map_err(|e| format!("file_type({path:?}): {e}"))?;
        if !ft.is_file() {
            continue;
        }
        if name_str.starts_with("example.") {
            return Ok(PathBuf::from(name));
        }
        candidates.push(PathBuf::from(name));
    }
    candidates.sort();
    candidates.into_iter().next().ok_or_else(|| {
        format!("no entry file in {fixture_dir:?} — add an `example.<ext>` at the fixture root",)
    })
}

fn write_pkl_config(project: &Path, tool_id: &str, pkl_property: &str) -> Result<(), String> {
    let config_dir = project.join(".agent-hook-kit");
    std::fs::create_dir_all(&config_dir).map_err(|e| format!("mkdir {config_dir:?}: {e}"))?;
    let body = format!(
        r#"amends "Config.pkl"
import "Builtins.pkl"

settings {{
  diagnosticsDirectory = ".agent-hook-kit/{tool_id}-agent-hook"
}}

tools {{
  ["{tool_id}"] = Builtins.{pkl_property}
}}
run = new Listing<String> {{ "{tool_id}" }}
"#
    );
    std::fs::write(config_dir.join("post-tool-use.pkl"), body)
        .map_err(|e| format!("write post-tool-use.pkl: {e}"))
}

/// Native snake_case PostToolUse payloads, matching the harness contracts.
fn synthesize_hook_event(harness: &str, project: &Path, entry_rel: &Path) -> Vec<u8> {
    let rel_str = entry_rel.to_string_lossy().to_string();
    let abs_str = project.join(entry_rel).to_string_lossy().to_string();
    let mut fixture = serde_json::json!({
        "session_id": STABLE_SESSION,
        "transcript_path": "/tmp/hookkit-fixture-transcript.jsonl",
        "cwd": project.to_string_lossy(),
        "hook_event_name": "PostToolUse",
        "tool_name": "Write",
        "tool_use_id": STABLE_TOOL_USE,
        "tool_input": {
            "file_path": rel_str,
            "content": "<fixture-test-input>"
        },
        "tool_response": { "filePath": abs_str }
    });
    if harness == "codex" {
        let object = fixture.as_object_mut().unwrap();
        object.insert("model".into(), serde_json::json!("gpt-fixture"));
        object.insert("turn_id".into(), serde_json::json!(STABLE_TURN));
        object.insert("permission_mode".into(), serde_json::json!("default"));
    }
    serde_json::to_vec(&fixture).unwrap()
}

fn run_binary(harness: &str, fixture_json: &[u8]) -> Result<std::process::Output, String> {
    let binary = env!("CARGO_BIN_EXE_post-tool-use-agent-hook");
    let flag = format!("--{harness}");
    let mut command = Command::new(binary);
    command.arg(&flag);
    configure_hook_environment(&mut command, harness, fixture_json)?;
    let mut child = command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn {binary}: {e}"))?;
    {
        use std::io::Write;
        let mut stdin = child.stdin.take().ok_or("stdin not piped")?;
        stdin
            .write_all(fixture_json)
            .map_err(|e| format!("write stdin: {e}"))?;
    }
    child
        .wait_with_output()
        .map_err(|e| format!("wait_with_output: {e}"))
}

fn configure_hook_environment(
    command: &mut Command,
    harness: &str,
    fixture_json: &[u8],
) -> Result<(), String> {
    clear_modeled_hook_environment(command);
    let input: serde_json::Value = serde_json::from_slice(fixture_json)
        .map_err(|error| format!("parse {harness} fixture for hook environment: {error}"))?;
    let field = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| input.get(*name).and_then(serde_json::Value::as_str))
            .ok_or_else(|| {
                format!(
                    "{harness} fixture is missing string field {}",
                    names.join(" or ")
                )
            })
    };

    match harness {
        "claude" => {
            let session_id = field(&["session_id", "sessionId"])?;
            let project_dir = field(&["cwd"])?;
            command
                .env("CLAUDECODE", "1")
                .env("CLAUDE_CODE_CHILD_SESSION", "1")
                .env("CLAUDE_CODE_SESSION_ID", session_id)
                .env("CLAUDE_PROJECT_DIR", project_dir);
        }
        "codex" | "antigravity" => {}
        _ => return Err(format!("unknown harness {harness}")),
    }

    Ok(())
}

fn clear_modeled_hook_environment(command: &mut Command) {
    const EXACT_NAMES: &[&str] = &[
        "CLAUDECODE",
        "CLAUDE_CODE_CHILD_SESSION",
        "CLAUDE_CODE_SESSION_ID",
        "CLAUDE_PROJECT_DIR",
        "CLAUDE_ENV_FILE",
        "CLAUDE_EFFORT",
        "TRACEPARENT",
        "CLAUDE_CODE_REMOTE",
        "CLAUDE_CODE_REMOTE_SESSION_ID",
        "CLAUDE_CODE_BRIDGE_SESSION_ID",
        "CLAUDE_PLUGIN_ROOT",
        "CLAUDE_PLUGIN_DATA",
        "PLUGIN_ROOT",
        "PLUGIN_DATA",
    ];
    for name in EXACT_NAMES {
        command.env_remove(name);
    }
    for name in std::env::vars_os().filter_map(|(name, _)| name.into_string().ok()) {
        if name.starts_with("CLAUDE_PLUGIN_OPTION_") {
            command.env_remove(name);
        }
    }
}

fn normalize(text: &str, project_aliases: &[String]) -> String {
    let mut out = text.to_string();
    // Substitute longest-first so a canonical /private/var/... prefix isn't
    // partially replaced before its shorter /var/... alias.
    let mut aliases: Vec<&String> = project_aliases.iter().collect();
    aliases.sort_by_key(|s| std::cmp::Reverse(s.len()));
    for alias in aliases {
        out = out.replace(alias, "<workspace>");
    }
    out
}

fn workspace_path_aliases(project: &Path) -> Vec<String> {
    let mut aliases = Vec::new();
    aliases.push(project.to_string_lossy().to_string());
    if let Ok(canonical) = project.canonicalize() {
        let canonical = canonical.to_string_lossy().to_string();
        if !aliases.contains(&canonical) {
            aliases.push(canonical);
        }
    }
    aliases
}

fn cleanup(project: &Path) {
    let _ = std::fs::remove_dir_all(project);
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()))
}

/// Whether Pkl-dependent tests must run instead of skipping. CI installs Pkl
/// and sets `HOOKKIT_REQUIRE_PKL=1`, so a missing binary fails the test.
fn pkl_required() -> bool {
    std::env::var_os("HOOKKIT_REQUIRE_PKL").is_some_and(|value| !value.is_empty() && value != "0")
}

fn pkl_available() -> bool {
    Command::new("pkl")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn tool_executable_available(exec: &str) -> bool {
    Command::new(exec)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or_else(|_| {
            // Some tools don't support --version; fall back to `which`.
            Command::new("which")
                .arg(exec)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        })
}
