use clap::Parser;
use globset::{Glob, GlobSet, GlobSetBuilder};
use hookkit_codex::protocol::{PreToolUse, PreToolUseOutput};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Parser)]
#[command(about = "Inject path-scoped Claude Code rules into Codex on first match")]
struct Cli {
    /// Override the project root used for `.claude/rules` and glob matching.
    #[arg(long)]
    project_root: Option<PathBuf>,

    /// Override the Claude home containing `rules/` (defaults to `~/.claude`).
    #[arg(long)]
    claude_home: Option<PathBuf>,

    /// Override the directory used for per-session loaded-rule markers.
    #[arg(long)]
    state_dir: Option<PathBuf>,
}

#[derive(Debug)]
struct Rule {
    source: PathBuf,
    patterns: GlobSet,
    body: String,
}

#[derive(Debug, Deserialize)]
struct Frontmatter {
    paths: Option<PathPatterns>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PathPatterns {
    One(String),
    Many(Vec<String>),
}

impl PathPatterns {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::One(pattern) => vec![pattern],
            Self::Many(patterns) => patterns,
        }
    }
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    hookkit_runtime::typed::run_typed::<PreToolUse, _>(move |input, _environment, _ctx| {
        let project_root = absolute_path(
            cli.project_root
                .as_deref()
                .unwrap_or(input.cwd.as_std_path()),
        )?;
        let claude_home = cli
            .claude_home
            .clone()
            .or_else(|| dirs::home_dir().map(|home| home.join(".claude")));
        let state_dir = cli
            .state_dir
            .clone()
            .unwrap_or_else(|| std::env::temp_dir().join("agent-hook-kit/codex-claude-rules"));

        let paths = referenced_paths(&input.tool_name, &input.tool_input, &project_root);
        if paths.is_empty() {
            return Ok(PreToolUseOutput::no_op());
        }

        let rules = discover_rules(claude_home.as_deref(), &project_root)?;
        let session_dir = state_dir.join(sanitize_component(&input.session_id));
        std::fs::create_dir_all(&session_dir)?;

        let mut context = Vec::new();
        for rule in rules {
            if !paths
                .iter()
                .any(|path| rule_matches(&rule, path, &project_root))
            {
                continue;
            }
            if claim_rule(&session_dir, &rule.source)? {
                context.push(format!(
                    "# Claude Code rule: {}\n\n{}",
                    rule.source.display(),
                    rule.body.trim()
                ));
            }
        }

        if context.is_empty() {
            Ok(PreToolUseOutput::no_op())
        } else {
            Ok(PreToolUseOutput::with_context(context.join("\n\n---\n\n")))
        }
    })
}

fn discover_rules(
    claude_home: Option<&Path>,
    project_root: &Path,
) -> hookkit_core::Result<Vec<Rule>> {
    let mut directories = Vec::new();
    if let Some(claude_home) = claude_home {
        directories.push(claude_home.join("rules"));
    }
    directories.push(project_root.join(".claude/rules"));

    let mut rules = Vec::new();
    let mut seen = BTreeSet::new();
    for directory in directories {
        if !directory.is_dir() {
            continue;
        }
        let mut files = WalkDir::new(&directory)
            .follow_links(true)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .map(walkdir::DirEntry::into_path)
            .collect::<Vec<_>>();
        files.sort();

        for file in files {
            if file.extension().and_then(|extension| extension.to_str()) != Some("md") {
                continue;
            }
            let source = std::fs::canonicalize(&file)?;
            if !seen.insert(source.clone()) {
                continue;
            }
            let content = std::fs::read_to_string(&source)?;
            if let Some(rule) = parse_rule(source, &content)? {
                rules.push(rule);
            }
        }
    }
    Ok(rules)
}

fn parse_rule(source: PathBuf, content: &str) -> hookkit_core::Result<Option<Rule>> {
    let Some((yaml, body)) = split_frontmatter(content) else {
        return Ok(None);
    };
    let frontmatter: Frontmatter = serde_yaml_ng::from_str(yaml).map_err(invalid_data)?;
    let Some(patterns) = frontmatter.paths else {
        return Ok(None);
    };

    let mut builder = GlobSetBuilder::new();
    let mut count = 0;
    for pattern in patterns.into_vec() {
        if pattern.trim().is_empty() {
            continue;
        }
        let Ok(pattern) = Glob::new(&slash_string(&pattern)) else {
            // Claude Code treats one invalid path pattern as matching nothing
            // while leaving the rule's remaining patterns active.
            continue;
        };
        builder.add(pattern);
        count += 1;
    }
    if count == 0 {
        return Ok(None);
    }

    Ok(Some(Rule {
        source,
        patterns: builder.build().map_err(invalid_data)?,
        body: body.to_string(),
    }))
}

fn split_frontmatter(content: &str) -> Option<(&str, &str)> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let first_newline = content.find('\n')?;
    if content[..first_newline].trim_end_matches('\r') != "---" {
        return None;
    }

    let yaml_start = first_newline + 1;
    let mut offset = yaml_start;
    for line in content[yaml_start..].split_inclusive('\n') {
        let line_without_newline = line.strip_suffix('\n').unwrap_or(line);
        if line_without_newline.trim_end_matches('\r') == "---" {
            let yaml = &content[yaml_start..offset];
            let body_start = offset + line.len();
            return Some((yaml, &content[body_start..]));
        }
        offset += line.len();
    }
    None
}

fn rule_matches(rule: &Rule, path: &Path, project_root: &Path) -> bool {
    let absolute = normalize_path(if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    });
    let root = normalize_path(project_root.to_path_buf());
    absolute
        .strip_prefix(root)
        .ok()
        .is_some_and(|relative| rule.patterns.is_match(slash_path(relative)))
}

fn claim_rule(session_dir: &Path, source: &Path) -> std::io::Result<bool> {
    let marker = session_dir.join(format!("{}.loaded", sha256(&slash_path(source))));
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)
    {
        Ok(mut file) => {
            if let Err(error) = writeln!(file, "{}", source.display()) {
                let _ = std::fs::remove_file(marker);
                return Err(error);
            }
            Ok(true)
        }
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error),
    }
}

fn referenced_paths(tool_name: &str, input: &serde_json::Value, cwd: &Path) -> Vec<PathBuf> {
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

    let mut seen = BTreeSet::new();
    raw.into_iter()
        .filter(|path| !path.trim().is_empty())
        .map(|path| {
            let path = PathBuf::from(path);
            normalize_path(if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            })
        })
        .filter(|path| seen.insert(slash_path(path)))
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
            .trim_matches(|character: char| "'\"`;|&(){}[]<>".contains(character))
            .rsplit_once('=')
            .map_or(token, |(_, value)| value)
            .trim_matches(|character: char| "'\"`;|&(){}[]<>".contains(character));
        if looks_like_path(token) {
            out.push(token.to_string());
        }
    }
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

fn absolute_path(path: &Path) -> std::io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(normalize_path(path.to_path_buf()))
    } else {
        Ok(normalize_path(std::env::current_dir()?.join(path)))
    }
}

fn slash_path(path: &Path) -> String {
    slash_string(&path.to_string_lossy())
}

fn slash_string(value: &str) -> String {
    value.replace('\\', "/")
}

fn sha256(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sanitize_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn invalid_data(error: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::new(ErrorKind::InvalidData, error.to_string())
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

    #[test]
    fn parses_list_and_single_path_frontmatter() {
        let list = parse_rule(
            PathBuf::from("list.md"),
            "---\npaths:\n  - 'src/**/*.{ts,tsx}'\n---\nUse TypeScript rules.\n",
        )
        .unwrap()
        .unwrap();
        assert!(list.patterns.is_match("src/ui/app.tsx"));
        assert_eq!(list.body, "Use TypeScript rules.\n");

        let one = parse_rule(
            PathBuf::from("one.md"),
            "---\npaths: '**/*.rs'\n---\nUse Rust rules.\n",
        )
        .unwrap()
        .unwrap();
        assert!(one.patterns.is_match("crates/core/src/lib.rs"));

        let partly_invalid = parse_rule(
            PathBuf::from("mixed.md"),
            "---\npaths: ['photos [2024/**', '**/*.md']\n---\nMarkdown.\n",
        )
        .unwrap()
        .unwrap();
        assert!(partly_invalid.patterns.is_match("docs/guide.md"));
    }

    #[test]
    fn ignores_unscoped_rules() {
        assert!(
            parse_rule(PathBuf::from("plain.md"), "# Always loaded\n")
                .unwrap()
                .is_none()
        );
        assert!(
            parse_rule(
                PathBuf::from("unscoped.md"),
                "---\ntitle: general\n---\nAlways loaded.\n"
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn extracts_structured_patch_and_shell_paths() {
        let input = serde_json::json!({
            "patch": "*** Update File: src/lib.rs\n*** Add File: tests/new.rs\n",
            "nested": {"file_path": "README.md"},
            "command": "sed -i '' src/main.rs"
        });
        let paths = referenced_paths("apply_patch", &input, Path::new("/repo"));
        let paths = paths
            .iter()
            .map(|path| slash_path(path))
            .collect::<BTreeSet<_>>();
        assert!(paths.contains("/repo/src/lib.rs"));
        assert!(paths.contains("/repo/tests/new.rs"));
        assert!(paths.contains("/repo/README.md"));

        let paths = referenced_paths(
            "Bash",
            &serde_json::json!({"command": "cat src/main.rs"}),
            Path::new("/repo"),
        );
        assert_eq!(paths, vec![PathBuf::from("/repo/src/main.rs")]);
    }

    #[test]
    fn claims_each_rule_only_once_per_session() {
        let directory = test_dir("rules-claim");
        std::fs::create_dir_all(&directory).unwrap();
        assert!(claim_rule(&directory, Path::new("/repo/.claude/rules/rust.md")).unwrap());
        assert!(!claim_rule(&directory, Path::new("/repo/.claude/rules/rust.md")).unwrap());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn matches_project_relative_paths_only() {
        let rule = parse_rule(
            PathBuf::from("rust.md"),
            "---\npaths: 'src/**/*.rs'\n---\nRust.\n",
        )
        .unwrap()
        .unwrap();
        assert!(rule_matches(
            &rule,
            Path::new("/repo/src/lib.rs"),
            Path::new("/repo")
        ));
        assert!(!rule_matches(
            &rule,
            Path::new("/other/src/lib.rs"),
            Path::new("/repo")
        ));
    }

    #[test]
    fn resolves_relative_project_roots_before_matching() {
        let root = absolute_path(Path::new("examples/codex-claude-rules/fixture-project")).unwrap();
        let rule = parse_rule(
            PathBuf::from("rust.md"),
            "---\npaths: 'src/**/*.rs'\n---\nRust.\n",
        )
        .unwrap()
        .unwrap();
        assert!(rule_matches(&rule, &root.join("src/lib.rs"), &root));
    }
}
