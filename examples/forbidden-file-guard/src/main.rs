use clap::{Parser, ValueEnum};
use globset::{Glob, GlobSet, GlobSetBuilder};
use hookkit_antigravity::ToolDecision;
use hookkit_core::{RuntimeContext, Utf8Path};
use hookkit_shell::{
    BashAnalyzer, FileAccessAnalyzer, FileAccessCandidate, FileInferenceContext, FileTarget,
    ShellToolCallError, ShellToolCallExt, ShellToolCallMatch, ShellToolCallRef,
};
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
            let roots = workspace_roots(context);
            let reason = deny_reason(
                &cli.config_paths,
                &roots,
                input.shell_tool_call(),
                &input.tool_name,
                &input.tool_input,
            );
            Ok(match reason {
                Some(reason) => hookkit_codex::protocol::PreToolUseOutput::deny(Some(reason)),
                None => hookkit_codex::protocol::PreToolUseOutput::no_op(),
            })
        },
    )
}

fn run_gemini(cli: Cli) -> std::process::ExitCode {
    hookkit_runtime::typed::run_typed::<hookkit_gemini::protocol::BeforeTool, _>(
        move |input, _environment, context| {
            let roots = workspace_roots(context);
            let tool_input = serde_json::Value::Object(input.tool_input.clone());
            let reason = deny_reason(
                &cli.config_paths,
                &roots,
                input.shell_tool_call(),
                &input.tool_name,
                &tool_input,
            );
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
            let roots = workspace_roots(context);
            let tool_input = serde_json::Value::Object(input.tool_call.args.clone());
            let reason = deny_reason(
                &cli.config_paths,
                &roots,
                input.shell_tool_call(),
                &input.tool_call.name,
                &tool_input,
            );
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

fn workspace_roots(context: &RuntimeContext<'_>) -> Vec<PathBuf> {
    context
        .workspace_roots()
        .iter()
        .map(|path| path.as_std_path().to_path_buf())
        .collect()
}

/// Load the active policy and return a deny reason, or `None` to allow. A policy
/// that fails to load is itself a deny so a broken configuration never silently
/// opens the boundary.
fn deny_reason(
    config_paths: &[PathBuf],
    roots: &[PathBuf],
    shell_call: ShellToolCallMatch<'_>,
    tool_name: &str,
    tool_input: &serde_json::Value,
) -> Option<String> {
    let policy = match load_policy(config_paths, roots) {
        Ok(policy) => policy,
        Err(error) => {
            return Some(format!(
                "Forbidden-file policy could not be loaded: {error}"
            ));
        }
    };
    evaluate(&policy, shell_call, tool_name, tool_input, roots)
}

fn load_policy(config_paths: &[PathBuf], roots: &[PathBuf]) -> hookkit_core::Result<Policy> {
    let explicit_paths = !config_paths.is_empty();
    let paths = if config_paths.is_empty() {
        default_config_paths(roots)
    } else {
        config_paths.to_vec()
    };

    let mut builder = GlobSetBuilder::new();
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
        }
    }

    Ok(Policy {
        patterns: builder.build().map_err(invalid_data)?,
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

/// Route one native pre-tool call to the appropriate inspector. Shell tools are
/// recognized by `hookkit-shell`'s native adapters; everything else is treated
/// as a structured tool whose path-bearing fields are inspected directly.
fn evaluate(
    policy: &Policy,
    shell_call: ShellToolCallMatch<'_>,
    tool_name: &str,
    tool_input: &serde_json::Value,
    roots: &[PathBuf],
) -> Option<String> {
    match shell_call {
        ShellToolCallMatch::Matched(call) => evaluate_shell(policy, &call, roots),
        ShellToolCallMatch::Malformed(error) => evaluate_unparseable_shell(policy, &error),
        // `NotShell`, plus any future non-shell classification, fall back to
        // structured field and `apply_patch` inspection.
        _ => evaluate_structured(policy, tool_name, tool_input, roots),
    }
}

/// Inspect a shell command with a bounded Bash parse, matching every resolved
/// read, write, delete, and redirection target against the policy.
fn evaluate_shell(
    policy: &Policy,
    call: &ShellToolCallRef<'_>,
    roots: &[PathBuf],
) -> Option<String> {
    if policy.block_shell_commands {
        return Some(blocked_shell_reason(call.tool_name));
    }
    if policy.patterns.is_empty() {
        return None;
    }
    let outcome = BashAnalyzer::default().analyze(call.command);
    let report = FileAccessAnalyzer::default().infer(&outcome, FileInferenceContext::new(call.cwd));
    report
        .candidates
        .iter()
        .find_map(|candidate| forbidden_target(policy, candidate, roots))
}

/// A shell tool whose command field could not be recovered cannot be inspected.
/// Deny it whenever the policy is active rather than treating it as harmless.
fn evaluate_unparseable_shell(policy: &Policy, error: &ShellToolCallError) -> Option<String> {
    if policy.block_shell_commands {
        return Some(blocked_shell_reason(&error.tool_name));
    }
    if policy.patterns.is_empty() {
        return None;
    }
    Some(format!(
        "Forbidden-file policy denies a shell tool call it cannot parse: {error}"
    ))
}

fn evaluate_structured(
    policy: &Policy,
    tool_name: &str,
    input: &serde_json::Value,
    roots: &[PathBuf],
) -> Option<String> {
    if policy.patterns.is_empty() {
        return None;
    }
    referenced_paths(tool_name, input)
        .into_iter()
        .find_map(|path| {
            matches_policy(policy, &path, roots).then(|| {
                format!(
                    "Forbidden-file policy denies access to `{}`",
                    path.display()
                )
            })
        })
}

fn blocked_shell_reason(tool_name: &str) -> String {
    format!(
        "Forbidden-file policy blocks shell tool `{tool_name}` because shell file access cannot be inspected exhaustively"
    )
}

/// Test a single inferred shell file-access candidate against the policy, trying
/// both the raw argument and the working-directory-resolved absolute form.
fn forbidden_target(
    policy: &Policy,
    candidate: &FileAccessCandidate,
    roots: &[PathBuf],
) -> Option<String> {
    let FileTarget::Path { expression, .. } = &candidate.target else {
        // Whole-workspace scopes (and any future target kind) name no single
        // file, so there is nothing to match against a file pattern here.
        return None;
    };
    let resolved = expression.resolved.as_deref();
    let matched = matches_policy(policy, Path::new(expression.raw.as_str()), roots)
        || resolved.is_some_and(|resolved| matches_policy(policy, resolved.as_std_path(), roots));
    matched.then(|| {
        let shown = resolved.map_or(expression.raw.as_str(), Utf8Path::as_str);
        format!("Forbidden-file policy denies access to `{shown}`")
    })
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

/// Collect path-bearing fields from a structured (non-shell) tool call,
/// including `apply_patch` file headers.
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

fn find_string<'a>(value: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    let map = value.as_object()?;
    keys.iter()
        .find_map(|key| map.get(*key).and_then(serde_json::Value::as_str))
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
    use hookkit_core::{EventId, HarnessId};
    use hookkit_shell::{ShellToolProfile, ToolPhase};
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
            block_shell_commands,
        }
    }

    /// Classify a tool call the way the runtime arms do, using a `Bash` profile
    /// as a stand-in for the per-harness shell adapters. A non-`Bash` tool name
    /// yields `NotShell`, exercising the structured path.
    fn classify<'a>(
        tool_name: &'a str,
        tool_input: &'a serde_json::Value,
        cwd: &'a Utf8Path,
    ) -> ShellToolCallMatch<'a> {
        ShellToolProfile::new("Bash", "/command")
            .unwrap()
            .extract_from_value(
                EventId::builtin(HarnessId::CODEX, "PreToolUse"),
                ToolPhase::Pre,
                tool_name,
                tool_input,
                Some(cwd),
                None,
            )
    }

    #[test]
    fn blocks_structured_root_and_nested_env_paths() {
        let policy = policy(&[".env", "**/.env"], false);
        let roots = vec![PathBuf::from("/repo")];
        let cwd = Utf8Path::new("/repo");

        let root = serde_json::json!({"path": "/repo/.env"});
        assert!(
            evaluate(
                &policy,
                classify("read_file", &root, cwd),
                "read_file",
                &root,
                &roots
            )
            .is_some()
        );
        let nested = serde_json::json!({"file_path": "services/api/.env"});
        assert!(
            evaluate(
                &policy,
                classify("write_file", &nested, cwd),
                "write_file",
                &nested,
                &roots
            )
            .is_some()
        );
    }

    #[test]
    fn inspects_shell_file_access_or_fails_closed() {
        let roots = vec![PathBuf::from("/repo")];
        let cwd = Utf8Path::new("/repo");
        let inspect = policy(&[".env", "**/.env"], false);

        // A literal read, a branch-guarded read, and a redirect write all reach
        // the forbidden target through the Bash parse.
        for command in [
            "cat .env",
            "test -f .env && cat .env",
            "echo secret > config/.env",
        ] {
            let input = serde_json::json!({ "command": command });
            assert!(
                evaluate(
                    &inspect,
                    classify("Bash", &input, cwd),
                    "Bash",
                    &input,
                    &roots
                )
                .is_some(),
                "expected `{command}` to be denied"
            );
        }

        // A command that touches no forbidden path is allowed.
        let benign = serde_json::json!({"command": "echo safe > notes.txt"});
        assert!(
            evaluate(
                &inspect,
                classify("Bash", &benign, cwd),
                "Bash",
                &benign,
                &roots
            )
            .is_none()
        );

        // Failing closed denies every shell tool regardless of its command.
        let closed = policy(&[], true);
        let safe = serde_json::json!({"command": "echo safe"});
        assert!(evaluate(&closed, classify("Bash", &safe, cwd), "Bash", &safe, &roots).is_some());
    }

    #[test]
    fn antigravity_command_line_shell_is_inspected() {
        // Antigravity puts the command in `CommandLine`, not `command`; the
        // native adapter recovers it so the Bash parse can run.
        let input: hookkit_antigravity::PreToolUseInput =
            serde_json::from_value(serde_json::json!({
                "conversationId": "conversation",
                "workspacePaths": ["/repo"],
                "transcriptPath": "/tmp/transcript.jsonl",
                "artifactDirectoryPath": "/tmp/artifacts",
                "toolCall": {
                    "name": "run_command",
                    "args": {"CommandLine": "cat .env", "Cwd": "/repo"}
                },
                "stepIdx": 1
            }))
            .unwrap();

        let policy = policy(&["**/.env"], false);
        let tool_input = serde_json::Value::Object(input.tool_call.args.clone());
        assert!(
            evaluate(
                &policy,
                input.shell_tool_call(),
                &input.tool_call.name,
                &tool_input,
                &[PathBuf::from("/repo")],
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
        assert_eq!(policy.patterns.len(), 2);
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
        let input = serde_json::json!({"patch": "*** Update File: secrets/token.txt\n"});
        let reason = evaluate(
            &policy,
            classify("apply_patch", &input, Utf8Path::new("/repo")),
            "apply_patch",
            &input,
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
