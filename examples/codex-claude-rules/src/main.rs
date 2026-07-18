use clap::Parser;
use globset::{Glob, GlobSet, GlobSetBuilder};
use hookkit_codex::protocol::{PreToolUse, PreToolUseInput, PreToolUseOutput};
use hookkit_session_state::{ClaimResult, FamilyId, SessionState, StateRoot};
use hookkit_shell::{
    BashAnalyzer, FileAccessAnalyzer, FileInferenceContext, FileTarget, ShellToolCallExt,
    ShellToolCallMatch, UnresolvedFileAccessReason,
};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

const STATE_FAMILY: &str = "agent-hook-kit.codex-claude-rules";
const STATE_FAMILY_VERSION: u32 = 2;

#[derive(Debug, Parser)]
#[command(about = "Inject path-scoped Claude Code rules into Codex on first match")]
struct Cli {
    /// Override the project root used for `.claude/rules` and glob matching.
    #[arg(long)]
    project_root: Option<PathBuf>,

    /// Override the Claude home containing `rules/` (defaults to `~/.claude`).
    #[arg(long)]
    claude_home: Option<PathBuf>,

    /// Override the common root used for versioned per-session state.
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
    hookkit_runtime::typed::run_event::<PreToolUse, _>(move |input, _environment, runtime| {
        let project_root = absolute_path(
            cli.project_root
                .as_deref()
                .unwrap_or(input.cwd.as_std_path()),
        )?;
        let claude_home = cli
            .claude_home
            .clone()
            .or_else(|| dirs::home_dir().map(|home| home.join(".claude")));
        let state_root = cli
            .state_dir
            .clone()
            .map(StateRoot::new)
            .unwrap_or_default();

        let paths = referenced_paths(&input);
        if paths.is_empty() {
            return Ok(PreToolUseOutput::no_op());
        }

        let rules = discover_rules(claude_home.as_deref(), &project_root)?;
        let loaded_rules = SessionState::ensure(runtime, state_root)
            .and_then(|state| state.family(FamilyId::new(STATE_FAMILY, STATE_FAMILY_VERSION)?))
            .and_then(|family| family.claims("loaded-rules"))
            .map_err(state_error)?;

        let mut additional_context = Vec::new();
        for rule in rules {
            if !paths
                .iter()
                .any(|path| rule_matches(&rule, path, &project_root))
            {
                continue;
            }
            let key = slash_path(&rule.source);
            if loaded_rules.try_claim(&key).map_err(state_error)? == ClaimResult::Claimed {
                additional_context.push(format!(
                    "# Claude Code rule: {}\n\n{}",
                    rule.source.display(),
                    rule.body.trim()
                ));
            }
        }

        if additional_context.is_empty() {
            Ok(PreToolUseOutput::no_op())
        } else {
            Ok(PreToolUseOutput::with_context(
                additional_context.join("\n\n---\n\n"),
            ))
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

fn referenced_paths(input: &PreToolUseInput) -> Vec<PathBuf> {
    let mut raw = Vec::new();
    collect_path_fields(&input.tool_input, &mut raw);

    if let Some(patch) = input
        .tool_name
        .eq_ignore_ascii_case("apply_patch")
        .then(|| find_string(&input.tool_input, &["patch", "input"]))
        .flatten()
    {
        collect_patch_paths(patch, &mut raw);
    }

    let mut seen = BTreeSet::new();
    let structured = raw
        .into_iter()
        .filter(|path| !path.trim().is_empty())
        .map(|path| {
            let path = PathBuf::from(path);
            normalize_path(if path.is_absolute() {
                path
            } else {
                input.cwd.as_std_path().join(path)
            })
        });
    structured
        .chain(shell_referenced_paths(input))
        .filter(|path| seen.insert(slash_path(path)))
        .collect()
}

fn collect_path_fields(value: &serde_json::Value, out: &mut Vec<String>) {
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
                } else {
                    collect_path_fields(value, out);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                collect_path_fields(value, out);
            }
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

fn shell_referenced_paths(input: &PreToolUseInput) -> Vec<PathBuf> {
    let ShellToolCallMatch::Matched(call) = input.shell_tool_call() else {
        return Vec::new();
    };
    let analysis = BashAnalyzer::default().analyze(call.command);
    let report =
        FileAccessAnalyzer::default().infer(&analysis, FileInferenceContext::new(call.cwd));
    let mut paths = report
        .candidates
        .iter()
        .filter_map(|candidate| match &candidate.target {
            FileTarget::Path { expression, .. } => expression
                .resolved
                .as_ref()
                .map(|path| path.as_std_path().to_path_buf()),
            FileTarget::Workspace { root: Some(root) } => Some(root.as_std_path().to_path_buf()),
            FileTarget::Workspace { root: None } => {
                call.cwd.map(|path| path.as_std_path().to_path_buf())
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    let Some(parsed) = analysis.analysis() else {
        return paths;
    };
    let fallback_commands = report
        .unresolved
        .iter()
        .filter(|gap| needs_literal_operand_fallback(&gap.reason))
        .filter_map(|gap| gap.command_index)
        .collect::<BTreeSet<_>>();
    let cwd = call.cwd.unwrap_or(input.cwd.as_path());
    for command_index in fallback_commands {
        // Preserve useful coverage for commands outside HookKit's bounded
        // semantics table without returning to a hand-written shell lexer.
        let Some(argv) = parsed
            .commands
            .get(command_index)
            .and_then(|command| command.literal_argv())
        else {
            continue;
        };
        for argument in argv.iter().skip(1) {
            let candidate = argument
                .rsplit_once('=')
                .map_or(argument.as_str(), |(_, value)| value);
            if looks_like_path(candidate) {
                let path = PathBuf::from(candidate);
                paths.push(normalize_path(if path.is_absolute() {
                    path
                } else {
                    cwd.as_std_path().join(path)
                }));
            }
        }
    }
    paths
}

fn needs_literal_operand_fallback(reason: &UnresolvedFileAccessReason) -> bool {
    matches!(
        reason,
        UnresolvedFileAccessReason::UnknownCommandSemantics { .. }
            | UnresolvedFileAccessReason::AmbiguousArguments { .. }
            | UnresolvedFileAccessReason::IndirectEvaluation { .. }
    )
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

fn invalid_data(error: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::new(ErrorKind::InvalidData, error.to_string())
}

fn state_error(error: hookkit_session_state::StateError) -> std::io::Error {
    std::io::Error::other(error)
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

    fn pre_tool_input(tool_name: &str, tool_input: serde_json::Value) -> PreToolUseInput {
        serde_json::from_value(serde_json::json!({
            "session_id": "session-1",
            "transcript_path": null,
            "cwd": "/repo",
            "hook_event_name": "PreToolUse",
            "model": "gpt-test",
            "turn_id": "turn-1",
            "permission_mode": "default",
            "tool_name": tool_name,
            "tool_use_id": "call-1",
            "tool_input": tool_input
        }))
        .unwrap()
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
        let input = pre_tool_input(
            "apply_patch",
            serde_json::json!({
                "patch": "*** Update File: src/lib.rs\n*** Add File: tests/new.rs\n",
                "nested": {"file_path": "README.md"},
                "command": "sed -i '' src/main.rs"
            }),
        );
        let paths = referenced_paths(&input);
        let paths = paths
            .iter()
            .map(|path| slash_path(path))
            .collect::<BTreeSet<_>>();
        assert!(paths.contains("/repo/src/lib.rs"));
        assert!(paths.contains("/repo/tests/new.rs"));
        assert!(paths.contains("/repo/README.md"));

        let input = pre_tool_input(
            "Bash",
            serde_json::json!({
                "command": "cat 'src/file with spaces.rs'; sed -n '1,20p' tests/unit.rs > build/out.txt"
            }),
        );
        let paths = referenced_paths(&input)
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            paths,
            BTreeSet::from([
                PathBuf::from("/repo/build/out.txt"),
                PathBuf::from("/repo/src/file with spaces.rs"),
                PathBuf::from("/repo/tests/unit.rs"),
            ])
        );
    }

    #[test]
    fn shell_extraction_is_exact_and_does_not_treat_search_queries_as_paths() {
        let input = pre_tool_input(
            "Bash",
            serde_json::json!({"command": "rg 'src/not-a-target.rs' src"}),
        );
        let paths = referenced_paths(&input);
        assert_eq!(paths, vec![PathBuf::from("/repo/src")]);

        let alias = pre_tool_input(
            "exec_command",
            serde_json::json!({"command": "cat src/main.rs"}),
        );
        assert!(referenced_paths(&alias).is_empty());
    }

    #[test]
    fn claims_each_rule_only_once_per_session() {
        let directory = test_dir("rules-claim");
        let loaded_rules = SessionState::open(
            hookkit_core::HarnessId::CODEX,
            hookkit_session_state::SessionIdentity::Session("session-1".into()),
            StateRoot::new(&directory),
        )
        .and_then(|state| state.family(FamilyId::new(STATE_FAMILY, STATE_FAMILY_VERSION)?))
        .and_then(|family| family.claims("loaded-rules"))
        .unwrap();
        let key = slash_path(Path::new("/repo/.claude/rules/rust.md"));
        assert_eq!(loaded_rules.try_claim(&key).unwrap(), ClaimResult::Claimed);
        assert_eq!(
            loaded_rules.try_claim(&key).unwrap(),
            ClaimResult::AlreadyClaimed
        );
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
