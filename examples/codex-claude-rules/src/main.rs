use clap::Parser;
use globset::{Glob, GlobSet, GlobSetBuilder};
use hookkit_codex::protocol::{PreToolUse, PreToolUseInput, PreToolUseOutput};
use hookkit_core::{Utf8Path, Utf8PathBuf, normalize_utf8_path, resolve_path, utf8_path_to_slash};
use hookkit_session_state::{ClaimResult, FamilyId, SessionState, StateRoot};
use hookkit_shell::{BashAnalyzer, FileAccessAnalyzer, UnknownCommandFallback};
use hookkit_tool_access::{
    ExactPathPolicy, ResolvedTargets, TargetResolutionOptions, ToolAccessAnalyzer, resolve_targets,
};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const STATE_FAMILY: &str = "agent-hook-kit.codex-claude-rules";
const STATE_FAMILY_VERSION: u32 = 2;
const TARGET_RESOLUTION_BUDGET: usize = 100_000;

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
    source: Utf8PathBuf,
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
    // Clap exits with status 2 on a usage error, which Codex treats as a
    // blocking hook decision. Report argument errors with the non-blocking
    // status 1 instead, and keep 0 for --help and --version.
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let _ = error.print();
            return std::process::ExitCode::from(if error.use_stderr() { 1 } else { 0 });
        }
    };
    hookkit_runtime::typed::run_event::<PreToolUse, _>(move |input, _environment, runtime| {
        let project_root = absolute_utf8_path(
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

        let references = referenced_paths(&input, &project_root, TARGET_RESOLUTION_BUDGET)?;
        if references.resolution.paths.is_empty() {
            return Ok(PreToolUseOutput::no_op());
        }

        let rules = discover_rules(claude_home.as_deref(), &project_root)?;
        let loaded_rules = SessionState::ensure(runtime, state_root)
            .and_then(|state| state.family(FamilyId::new(STATE_FAMILY, STATE_FAMILY_VERSION)?))
            .and_then(|family| family.claims("loaded-rules"))
            .map_err(state_error)?;

        let mut additional_context = Vec::new();
        for rule in rules {
            if !references
                .resolution
                .paths
                .iter()
                .any(|path| rule_matches(&rule, path, &project_root))
            {
                continue;
            }
            let key = utf8_path_to_slash(&rule.source);
            if loaded_rules.try_claim(&key).map_err(state_error)? == ClaimResult::Claimed {
                additional_context.push(format!(
                    "# Claude Code rule: {}\n\n{}",
                    rule.source,
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
    project_root: &Utf8Path,
) -> hookkit_core::Result<Vec<Rule>> {
    let mut directories = Vec::new();
    if let Some(claude_home) = claude_home {
        directories.push(claude_home.join("rules"));
    }
    directories.push(project_root.join(".claude/rules").into_std_path_buf());

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
            let source =
                Utf8PathBuf::from_path_buf(std::fs::canonicalize(&file)?).map_err(|path| {
                    invalid_data(format!("rule path is not UTF-8: {}", path.display()))
                })?;
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

fn parse_rule(source: Utf8PathBuf, content: &str) -> hookkit_core::Result<Option<Rule>> {
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
        let Ok(pattern) = Glob::new(&utf8_path_to_slash(Utf8Path::new(&pattern))) else {
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

fn rule_matches(rule: &Rule, path: &Utf8Path, project_root: &Utf8Path) -> bool {
    let root = normalize_utf8_path(project_root);
    path.strip_prefix(root)
        .ok()
        .is_some_and(|relative| rule.patterns.is_match(utf8_path_to_slash(relative)))
}

struct ReferencedPathReport {
    resolution: ResolvedTargets,
}

fn referenced_paths(
    input: &PreToolUseInput,
    project_root: &Utf8Path,
    max_entries: usize,
) -> hookkit_core::Result<ReferencedPathReport> {
    let analyzer = ToolAccessAnalyzer::new(
        BashAnalyzer::default(),
        FileAccessAnalyzer::default()
            .with_unknown_command_fallback(UnknownCommandFallback::LiteralPathOperands),
        hookkit_tool_access::StructuredFieldAnalyzer::default(),
    );
    // The typed native input is analyzed in place; no aligned clone is needed.
    let access = analyzer.analyze_native(input);
    let mut options = TargetResolutionOptions::new(vec![project_root.to_path_buf()]);
    options.max_entries = max_entries;
    options.ignored_directory_names.clear();
    options.exact_paths = ExactPathPolicy::RetainNonexistent;
    let resolution = resolve_targets(
        access.candidates.iter().map(|candidate| &candidate.target),
        &options,
    )
    .map_err(invalid_data)?;
    Ok(ReferencedPathReport { resolution })
}

fn absolute_utf8_path(path: &Path) -> std::io::Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(resolve_path(std::env::current_dir()?, path)).map_err(|path| {
        std::io::Error::new(
            ErrorKind::InvalidData,
            format!("project root is not UTF-8: {}", path.display()),
        )
    })
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
    use std::sync::{Arc, Barrier};

    static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

    fn test_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "hookkit-{label}-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn pre_tool_input_at_cwd(
        cwd: &str,
        tool_name: &str,
        tool_input: serde_json::Value,
    ) -> PreToolUseInput {
        serde_json::from_value(serde_json::json!({
            "session_id": "session-1",
            "transcript_path": null,
            "cwd": cwd,
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

    fn pre_tool_input(tool_name: &str, tool_input: serde_json::Value) -> PreToolUseInput {
        pre_tool_input_at_cwd("/repo", tool_name, tool_input)
    }

    #[test]
    fn parses_list_and_single_path_frontmatter() {
        let list = parse_rule(
            Utf8PathBuf::from("list.md"),
            "---\npaths:\n  - 'src/**/*.{ts,tsx}'\n---\nUse TypeScript rules.\n",
        )
        .unwrap()
        .unwrap();
        assert!(list.patterns.is_match("src/ui/app.tsx"));
        assert_eq!(list.body, "Use TypeScript rules.\n");

        let one = parse_rule(
            Utf8PathBuf::from("one.md"),
            "---\npaths: '**/*.rs'\n---\nUse Rust rules.\n",
        )
        .unwrap()
        .unwrap();
        assert!(one.patterns.is_match("crates/core/src/lib.rs"));

        let partly_invalid = parse_rule(
            Utf8PathBuf::from("mixed.md"),
            "---\npaths: ['photos [2024/**', '**/*.md']\n---\nMarkdown.\n",
        )
        .unwrap()
        .unwrap();
        assert!(partly_invalid.patterns.is_match("docs/guide.md"));
    }

    #[test]
    fn ignores_unscoped_rules() {
        assert!(
            parse_rule(Utf8PathBuf::from("plain.md"), "# Always loaded\n")
                .unwrap()
                .is_none()
        );
        assert!(
            parse_rule(
                Utf8PathBuf::from("unscoped.md"),
                "---\ntitle: general\n---\nAlways loaded.\n"
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn extracts_structured_patch_and_shell_paths() {
        // Codex sends the apply_patch text in `command`.
        let input = pre_tool_input(
            "apply_patch",
            serde_json::json!({
                "command": "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n-old\n+new\n*** Add File: tests/new.rs\n+new\n*** End Patch\n"
            }),
        );
        let paths = referenced_paths(&input, Utf8Path::new("/repo"), TARGET_RESOLUTION_BUDGET)
            .unwrap()
            .resolution
            .paths
            .iter()
            .map(utf8_path_to_slash)
            .collect::<BTreeSet<_>>();
        assert!(paths.contains("/repo/src/lib.rs"));
        assert!(paths.contains("/repo/tests/new.rs"));

        let structured = pre_tool_input(
            "mcp__docs__read_file",
            serde_json::json!({"nested": {"file_path": "README.md"}}),
        );
        assert!(
            referenced_paths(
                &structured,
                Utf8Path::new("/repo"),
                TARGET_RESOLUTION_BUDGET
            )
            .unwrap()
            .resolution
            .paths
            .contains(Utf8Path::new("/repo/README.md"))
        );

        let input = pre_tool_input(
            "Bash",
            serde_json::json!({
                "command": "cat 'src/file with spaces.rs'; sed -n '1,20p' tests/unit.rs > build/out.txt"
            }),
        );
        let paths = referenced_paths(&input, Utf8Path::new("/repo"), TARGET_RESOLUTION_BUDGET)
            .unwrap()
            .resolution
            .paths;
        assert_eq!(
            paths,
            BTreeSet::from([
                Utf8PathBuf::from("/repo/build/out.txt"),
                Utf8PathBuf::from("/repo/src/file with spaces.rs"),
                Utf8PathBuf::from("/repo/tests/unit.rs"),
            ])
        );
    }

    #[test]
    fn shell_extraction_is_exact_and_does_not_treat_search_queries_as_paths() {
        let input = pre_tool_input(
            "Bash",
            serde_json::json!({"command": "rg 'src/not-a-target.rs' src"}),
        );
        let paths = referenced_paths(&input, Utf8Path::new("/repo"), TARGET_RESOLUTION_BUDGET)
            .unwrap()
            .resolution
            .paths;
        assert_eq!(paths, BTreeSet::from([Utf8PathBuf::from("/repo/src")]));

        let alias = pre_tool_input(
            "exec_command",
            serde_json::json!({"command": "cat src/main.rs"}),
        );
        assert!(
            referenced_paths(&alias, Utf8Path::new("/repo"), TARGET_RESOLUTION_BUDGET)
                .unwrap()
                .resolution
                .paths
                .is_empty()
        );
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
        let key = utf8_path_to_slash(Utf8Path::new("/repo/.claude/rules/rust.md"));
        assert_eq!(loaded_rules.try_claim(&key).unwrap(), ClaimResult::Claimed);
        assert_eq!(
            loaded_rules.try_claim(&key).unwrap(),
            ClaimResult::AlreadyClaimed
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn concurrent_rule_claim_has_exactly_one_injector() {
        let directory = test_dir("rules-concurrent-claim");
        let loaded_rules = Arc::new(
            SessionState::open(
                hookkit_core::HarnessId::CODEX,
                hookkit_session_state::SessionIdentity::Session("session-1".into()),
                StateRoot::new(&directory),
            )
            .and_then(|state| state.family(FamilyId::new(STATE_FAMILY, STATE_FAMILY_VERSION)?))
            .and_then(|family| family.claims("loaded-rules"))
            .unwrap(),
        );
        let barrier = Arc::new(Barrier::new(8));
        let handles = (0..8)
            .map(|_| {
                let loaded_rules = Arc::clone(&loaded_rules);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    loaded_rules
                        .try_claim("/repo/.claude/rules/rust.md")
                        .unwrap()
                })
            })
            .collect::<Vec<_>>();
        let injectors = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(|result| *result == ClaimResult::Claimed)
            .count();

        assert_eq!(injectors, 1);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn matches_project_relative_paths_only() {
        let rule = parse_rule(
            Utf8PathBuf::from("rust.md"),
            "---\npaths: 'src/**/*.rs'\n---\nRust.\n",
        )
        .unwrap()
        .unwrap();
        assert!(rule_matches(
            &rule,
            Utf8Path::new("/repo/src/lib.rs"),
            Utf8Path::new("/repo")
        ));
        assert!(!rule_matches(
            &rule,
            Utf8Path::new("/other/src/lib.rs"),
            Utf8Path::new("/repo")
        ));
    }

    #[test]
    fn resolves_relative_project_roots_before_matching() {
        let root =
            absolute_utf8_path(Path::new("examples/codex-claude-rules/fixture-project")).unwrap();
        let rule = parse_rule(
            Utf8PathBuf::from("rust.md"),
            "---\npaths: 'src/**/*.rs'\n---\nRust.\n",
        )
        .unwrap()
        .unwrap();
        assert!(rule_matches(&rule, &root.join("src/lib.rs"), &root));
    }

    #[test]
    fn directory_search_materialization_activates_descendant_rule() {
        let directory = test_dir("rules-directory-search");
        let root = Utf8PathBuf::from_path_buf(directory.clone()).unwrap();
        std::fs::create_dir_all(root.join("src/nested")).unwrap();
        std::fs::write(root.join("src/nested/lib.rs"), "pub fn example() {}\n").unwrap();
        let input = pre_tool_input_at_cwd(
            root.as_str(),
            "Bash",
            serde_json::json!({"command": "rg needle src"}),
        );
        let references = referenced_paths(&input, &root, TARGET_RESOLUTION_BUDGET).unwrap();
        let rule = parse_rule(
            Utf8PathBuf::from("rust.md"),
            "---\npaths: 'src/**/*.rs'\n---\nRust.\n",
        )
        .unwrap()
        .unwrap();

        assert!(
            references
                .resolution
                .paths
                .iter()
                .any(|path| rule_matches(&rule, path, &root))
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unknown_literal_fallback_and_truncation_follow_public_policies() {
        let unknown = pre_tool_input(
            "Bash",
            serde_json::json!({"command": "mystery path/to/input.rs"}),
        );
        let recovered =
            referenced_paths(&unknown, Utf8Path::new("/repo"), TARGET_RESOLUTION_BUDGET).unwrap();
        assert!(
            recovered
                .resolution
                .paths
                .contains(Utf8Path::new("/repo/path/to/input.rs"))
        );

        let truncated = referenced_paths(&unknown, Utf8Path::new("/repo"), 0).unwrap();
        assert!(truncated.resolution.paths.is_empty());
        assert!(truncated.resolution.budget_exhausted);
        assert!(!truncated.resolution.unresolved.is_empty());
    }

    #[test]
    fn native_cwd_does_not_get_rewritten_to_project_root() {
        let input = pre_tool_input_at_cwd(
            "/native/cwd",
            "mcp__filesystem__read_file",
            serde_json::json!({"path": "src/lib.rs"}),
        );
        let references =
            referenced_paths(&input, Utf8Path::new("/project"), TARGET_RESOLUTION_BUDGET).unwrap();
        assert_eq!(
            references.resolution.paths,
            BTreeSet::from([Utf8PathBuf::from("/native/cwd/src/lib.rs")])
        );
        let rule = parse_rule(
            Utf8PathBuf::from("rust.md"),
            "---\npaths: 'src/**/*.rs'\n---\nRust.\n",
        )
        .unwrap()
        .unwrap();
        assert!(!references.resolution.paths.iter().any(|path| rule_matches(
            &rule,
            path,
            Utf8Path::new("/project")
        )));
    }
}
