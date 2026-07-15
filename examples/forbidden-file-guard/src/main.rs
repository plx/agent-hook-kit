use clap::{Parser, ValueEnum};
use globset::{Glob, GlobSet, GlobSetBuilder};
use hookkit_antigravity::ToolDecision;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

const PROJECT_CONFIG: &str = ".agent-hook-kit/forbidden-files.yaml";

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Harness {
    Codex,
    Gemini,
    Antigravity,
}

#[derive(Debug, Parser)]
#[command(about = "Deny tool calls that reference configured sensitive paths")]
struct Cli {
    /// Hook harness whose native pre-tool contract should be used.
    #[arg(long, value_enum)]
    harness: Harness,

    /// Read only these YAML files instead of the default home and project files.
    #[arg(long = "config", value_name = "PATH")]
    config_paths: Vec<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    #[serde(default)]
    patterns: Vec<String>,
    #[serde(default)]
    block_shell_commands: bool,
}

struct Policy {
    patterns: GlobSet,
    pattern_count: usize,
    block_shell_commands: bool,
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    match cli.harness {
        Harness::Codex => run_codex(cli),
        Harness::Gemini => run_gemini(cli),
        Harness::Antigravity => run_antigravity(cli),
    }
}

fn run_codex(cli: Cli) -> std::process::ExitCode {
    hookkit_runtime::typed::run_typed::<hookkit_codex::protocol::PreToolUse, _>(
        move |input, _environment, context| {
            let roots = context
                .workspace_roots()
                .iter()
                .map(|path| path.as_std_path().to_path_buf())
                .collect::<Vec<_>>();
            let policy = match load_policy(&cli.config_paths, &roots) {
                Ok(policy) => policy,
                Err(error) => {
                    return Ok(hookkit_codex::protocol::PreToolUseOutput::deny(Some(
                        format!("Forbidden-file policy could not be loaded: {error}"),
                    )));
                }
            };
            let reason = evaluate(&policy, &input.tool_name, &input.tool_input, &roots);
            Ok(
                reason.map_or_else(hookkit_codex::protocol::PreToolUseOutput::no_op, |reason| {
                    hookkit_codex::protocol::PreToolUseOutput::deny(Some(reason))
                }),
            )
        },
    )
}

fn run_gemini(cli: Cli) -> std::process::ExitCode {
    hookkit_runtime::typed::run_typed::<hookkit_gemini::protocol::BeforeTool, _>(
        move |input, _environment, context| {
            let roots = context
                .workspace_roots()
                .iter()
                .map(|path| path.as_std_path().to_path_buf())
                .collect::<Vec<_>>();
            let policy = match load_policy(&cli.config_paths, &roots) {
                Ok(policy) => policy,
                Err(error) => {
                    return Ok(hookkit_gemini::protocol::BeforeToolOutput::deny(format!(
                        "Forbidden-file policy could not be loaded: {error}"
                    )));
                }
            };
            let tool_input = serde_json::Value::Object(input.tool_input);
            let reason = evaluate(&policy, &input.tool_name, &tool_input, &roots);
            Ok(reason.map_or_else(
                hookkit_gemini::protocol::BeforeToolOutput::no_op,
                hookkit_gemini::protocol::BeforeToolOutput::deny,
            ))
        },
    )
}

fn run_antigravity(cli: Cli) -> std::process::ExitCode {
    hookkit_runtime::typed::run_typed::<hookkit_antigravity::PreToolUse, _>(
        move |input, _environment, context| {
            let roots = context
                .workspace_roots()
                .iter()
                .map(|path| path.as_std_path().to_path_buf())
                .collect::<Vec<_>>();
            let policy = match load_policy(&cli.config_paths, &roots) {
                Ok(policy) => policy,
                Err(error) => {
                    return Ok(hookkit_antigravity::PreToolUseOutput {
                        decision: ToolDecision::Deny,
                        reason: Some(format!(
                            "Forbidden-file policy could not be loaded: {error}"
                        )),
                        permission_overrides: Vec::new(),
                    });
                }
            };
            let tool_input = serde_json::Value::Object(input.tool_call.args);
            let reason = evaluate(&policy, &input.tool_call.name, &tool_input, &roots);
            Ok(hookkit_antigravity::PreToolUseOutput {
                decision: if reason.is_some() {
                    ToolDecision::Deny
                } else {
                    ToolDecision::Allow
                },
                reason,
                permission_overrides: Vec::new(),
            })
        },
    )
}

fn load_policy(config_paths: &[PathBuf], roots: &[PathBuf]) -> hookkit_core::Result<Policy> {
    let explicit_paths = !config_paths.is_empty();
    let paths = if config_paths.is_empty() {
        default_config_paths(roots)
    } else {
        config_paths.to_vec()
    };

    let mut builder = GlobSetBuilder::new();
    let mut pattern_count = 0;
    let mut block_shell_commands = false;
    for path in paths {
        if !path.is_file() {
            if explicit_paths {
                return Err(std::io::Error::new(
                    ErrorKind::NotFound,
                    format!("configured policy file does not exist: {}", path.display()),
                )
                .into());
            }
            continue;
        }
        let content = std::fs::read_to_string(&path)?;
        let config: FileConfig = serde_yaml_ng::from_str(&content).map_err(invalid_data)?;
        block_shell_commands |= config.block_shell_commands;
        for pattern in config.patterns {
            if pattern.trim().is_empty() {
                continue;
            }
            let pattern = expand_home(&pattern);
            builder.add(Glob::new(&slash_string(&pattern)).map_err(invalid_data)?);
            pattern_count += 1;
        }
    }

    Ok(Policy {
        patterns: builder.build().map_err(invalid_data)?,
        pattern_count,
        block_shell_commands,
    })
}

fn default_config_paths(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = BTreeSet::new();
    if let Some(home) = dirs::home_dir() {
        paths.insert(home.join(PROJECT_CONFIG));
    }
    for root in roots {
        for ancestor in root.ancestors() {
            paths.insert(ancestor.join(PROJECT_CONFIG));
        }
    }
    paths.into_iter().collect()
}

fn evaluate(
    policy: &Policy,
    tool_name: &str,
    input: &serde_json::Value,
    roots: &[PathBuf],
) -> Option<String> {
    if policy.block_shell_commands && is_shell_tool(tool_name) {
        return Some(format!(
            "Forbidden-file policy blocks shell tool `{tool_name}` because shell paths cannot be inspected reliably"
        ));
    }
    if policy.pattern_count == 0 {
        return None;
    }

    let paths = referenced_paths(tool_name, input);
    for path in paths {
        if matches_policy(policy, &path, roots) {
            return Some(format!(
                "Forbidden-file policy denies access to `{}`",
                path.display()
            ));
        }
    }
    None
}

fn matches_policy(policy: &Policy, path: &Path, roots: &[PathBuf]) -> bool {
    let expanded = expand_path_home(path);
    let mut forms = BTreeSet::from([slash_path(path), slash_path(&expanded)]);
    for root in roots {
        let absolute = normalize_path(if expanded.is_absolute() {
            expanded.clone()
        } else {
            root.join(&expanded)
        });
        let normalized_root = normalize_path(root.clone());
        insert_path_forms(&mut forms, &absolute, &normalized_root);
        if let Ok(canonical) = std::fs::canonicalize(&absolute) {
            let canonical_root = std::fs::canonicalize(&normalized_root).unwrap_or(normalized_root);
            insert_path_forms(&mut forms, &canonical, &canonical_root);
        }
    }
    forms.iter().any(|form| policy.patterns.is_match(form))
}

fn insert_path_forms(forms: &mut BTreeSet<String>, path: &Path, root: &Path) {
    forms.insert(slash_path(path));
    if let Ok(relative) = path.strip_prefix(root) {
        forms.insert(slash_path(relative));
    }
}

fn referenced_paths(tool_name: &str, input: &serde_json::Value) -> Vec<PathBuf> {
    let mut raw = Vec::new();
    collect_path_fields(input, None, &mut raw);

    if let Some(patch) = tool_name
        .eq_ignore_ascii_case("apply_patch")
        .then(|| find_string(input, &["patch", "input"]))
        .flatten()
    {
        collect_patch_paths(patch, &mut raw);
    }
    if let Some(command) = is_shell_tool(tool_name)
        .then(|| find_string(input, &["command", "cmd"]))
        .flatten()
    {
        collect_shell_path_tokens(command, &mut raw);
    }

    raw.into_iter()
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn collect_path_fields(value: &serde_json::Value, parent_key: Option<&str>, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if is_path_key(key) {
                    match value {
                        serde_json::Value::String(path) => out.push(path.clone()),
                        serde_json::Value::Array(paths) => out.extend(
                            paths
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .map(str::to_owned),
                        ),
                        _ => {}
                    }
                }
                collect_path_fields(value, Some(key), out);
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                collect_path_fields(value, parent_key, out);
            }
        }
        serde_json::Value::String(path) if parent_key.is_some_and(is_path_key) => {
            out.push(path.clone());
        }
        _ => {}
    }
}

fn is_path_key(key: &str) -> bool {
    matches!(
        key,
        "path"
            | "paths"
            | "file_path"
            | "filePath"
            | "target_file"
            | "targetFile"
            | "absolute_path"
            | "absolutePath"
            | "source"
            | "destination"
            | "old_path"
            | "new_path"
    )
}

fn collect_patch_paths(patch: &str, out: &mut Vec<String>) {
    const PREFIXES: &[&str] = &[
        "*** Add File: ",
        "*** Update File: ",
        "*** Delete File: ",
        "*** Move to: ",
        "+++ b/",
        "--- a/",
    ];
    for line in patch.lines() {
        if let Some(path) = PREFIXES
            .iter()
            .find_map(|prefix| line.strip_prefix(prefix))
            .filter(|path| *path != "/dev/null")
        {
            out.push(path.trim().to_string());
        }
    }
}

fn collect_shell_path_tokens(command: &str, out: &mut Vec<String>) {
    for token in command.split_whitespace() {
        let token = token
            .trim_matches(shell_punctuation)
            .rsplit_once('=')
            .map_or(token, |(_, value)| value)
            .trim_matches(shell_punctuation);
        if looks_like_path(token) {
            out.push(token.to_string());
        }
    }
}

fn shell_punctuation(character: char) -> bool {
    "'\"`;|&(){}[]<>".contains(character)
}

fn looks_like_path(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && (value.contains('/') || value.starts_with('.') || Path::new(value).extension().is_some())
}

fn find_string<'a>(value: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    let map = value.as_object()?;
    keys.iter()
        .find_map(|key| map.get(*key).and_then(serde_json::Value::as_str))
}

fn is_shell_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "Bash" | "run_shell_command" | "shell" | "exec_command"
    )
}

fn expand_home(pattern: &str) -> String {
    let Some(rest) = pattern.strip_prefix("~/") else {
        return pattern.to_string();
    };
    dirs::home_dir().map_or_else(|| pattern.to_string(), |home| slash_path(&home.join(rest)))
}

fn expand_path_home(path: &Path) -> PathBuf {
    let value = path.to_string_lossy();
    let Some(rest) = value.strip_prefix("~/") else {
        return path.to_path_buf();
    };
    dirs::home_dir().map_or_else(|| path.to_path_buf(), |home| home.join(rest))
}

fn normalize_path(path: PathBuf) -> PathBuf {
    let absolute = path.is_absolute();
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let can_pop = matches!(
                    normalized.components().next_back(),
                    Some(Component::Normal(_))
                );
                if can_pop {
                    normalized.pop();
                } else if !absolute {
                    normalized.push(component.as_os_str());
                }
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn slash_path(path: &Path) -> String {
    slash_string(&path.to_string_lossy())
}

fn slash_string(value: &str) -> String {
    value.replace('\\', "/")
}

fn invalid_data(error: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

    fn test_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "hookkit-{label}-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn policy(patterns: &[&str], block_shell_commands: bool) -> Policy {
        let mut builder = GlobSetBuilder::new();
        for pattern in patterns {
            builder.add(Glob::new(pattern).unwrap());
        }
        Policy {
            patterns: builder.build().unwrap(),
            pattern_count: patterns.len(),
            block_shell_commands,
        }
    }

    #[test]
    fn blocks_structured_root_and_nested_env_paths() {
        let policy = policy(&[".env", "**/.env"], false);
        let roots = vec![PathBuf::from("/repo")];
        assert!(
            evaluate(
                &policy,
                "read_file",
                &serde_json::json!({"path": "/repo/.env"}),
                &roots
            )
            .is_some()
        );
        assert!(
            evaluate(
                &policy,
                "write_file",
                &serde_json::json!({"file_path": "services/api/.env"}),
                &roots
            )
            .is_some()
        );
    }

    #[test]
    fn inspects_obvious_shell_tokens_or_fails_closed() {
        let roots = vec![PathBuf::from("/repo")];
        let inspect = policy(&[".env"], false);
        assert!(
            evaluate(
                &inspect,
                "Bash",
                &serde_json::json!({"command": "cat .env"}),
                &roots
            )
            .is_some()
        );

        let closed = policy(&[], true);
        assert!(
            evaluate(
                &closed,
                "run_shell_command",
                &serde_json::json!({"command": "echo safe"}),
                &roots
            )
            .is_some()
        );
    }

    #[test]
    fn explicit_configs_are_merged_additively() {
        let directory = test_dir("forbidden-config");
        std::fs::create_dir_all(&directory).unwrap();
        let first = directory.join("first.yaml");
        let second = directory.join("second.yaml");
        std::fs::write(&first, "patterns: ['.env']\n").unwrap();
        std::fs::write(
            &second,
            "patterns: ['**/*.pem']\nblock_shell_commands: true\n",
        )
        .unwrap();

        let policy = load_policy(&[first, second], &[PathBuf::from("/repo")]).unwrap();
        assert_eq!(policy.pattern_count, 2);
        assert!(policy.block_shell_commands);
        assert!(policy.patterns.is_match("keys/private.pem"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn missing_explicit_config_is_an_error() {
        let missing = test_dir("missing-forbidden-config").join("policy.yaml");
        assert!(load_policy(&[missing], &[PathBuf::from("/repo")]).is_err());
    }

    #[test]
    fn patch_paths_are_guarded() {
        let policy = policy(&["secrets/**"], false);
        let reason = evaluate(
            &policy,
            "apply_patch",
            &serde_json::json!({"patch": "*** Update File: secrets/token.txt\n"}),
            &[PathBuf::from("/repo")],
        );
        assert!(reason.is_some());
    }

    #[test]
    fn normalizes_parent_traversal_without_escaping_the_filesystem_root() {
        let policy = policy(&[".env"], false);
        assert!(matches_policy(
            &policy,
            Path::new("../../repo/.env"),
            &[PathBuf::from("/repo")]
        ));
    }
}
