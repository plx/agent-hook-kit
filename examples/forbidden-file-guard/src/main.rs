use clap::{Parser, ValueEnum};
use globset::{Glob, GlobSet, GlobSetBuilder};
use hookkit_common::{PreToolUseInput, PreToolUseOutput};
use hookkit_core::{
    HarnessId, Utf8Path, Utf8PathBuf, expand_utf8_home, normalize_utf8_path, resolve_utf8_path,
    utf8_path_to_slash,
};
use hookkit_shell::{
    ANTIGRAVITY_RUN_COMMAND_PROFILE, BashAnalyzer, CLAUDE_BASH_PROFILE, CODEX_BASH_PROFILE,
    FileAccessAnalyzer, GEMINI_RUN_SHELL_COMMAND_PROFILE, UnknownCommandFallback,
};
use hookkit_tool_access::{
    AccessCandidate, AccessProvenance, AccessSource, AccessTarget, ExactPathPolicy,
    TargetResolutionOptions, ToolAccessAnalyzer, ToolCallObservation, observe_pre_tool,
    resolve_targets,
};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::PathBuf;

const PROJECT_CONFIG: &str = ".agent-hook-kit/forbidden-files.yaml";
const TARGET_RESOLUTION_BUDGET: usize = 100_000;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Harness {
    Claude,
    Codex,
    Gemini,
    Antigravity,
}

impl Harness {
    fn id(self) -> HarnessId {
        match self {
            Self::Claude => HarnessId::CLAUDE_CODE,
            Self::Codex => HarnessId::CODEX,
            Self::Gemini => HarnessId::GEMINI_CLI,
            Self::Antigravity => HarnessId::ANTIGRAVITY,
        }
    }
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
    /// Preferred replacement for `block_shell_commands`.
    access_policy: Option<AccessPolicy>,
}

struct Policy {
    patterns: GlobSet,
    access_policy: AccessPolicy,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum AccessPolicy {
    /// Match recovered candidates and allow analysis/resolver gaps.
    #[default]
    InspectKnown,
    /// Match candidates and deny when either analysis or resolution is incomplete.
    DenyUnresolved,
    /// Deny exact native shell calls; inspect non-shell calls like `InspectKnown`.
    DenyAllShell,
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let harness = cli.harness.id();
    hookkit_runtime::aligned::run_aligned_event::<hookkit_runtime::aligned::PreToolUse, _>(
        harness.clone(),
        move |input, _environment, _context| {
            let reason = deny_reason(&cli.config_paths, &input, TARGET_RESOLUTION_BUDGET);
            match reason {
                Some(reason) => PreToolUseOutput::deny(&harness, reason),
                None => PreToolUseOutput::allow(&harness),
            }
        },
    )
}

/// Load the active policy and return a deny reason, or `None` to allow. A policy
/// that fails to load is itself a deny so a broken configuration never silently
/// opens the boundary.
fn deny_reason(
    config_paths: &[PathBuf],
    input: &PreToolUseInput,
    max_entries: usize,
) -> Option<String> {
    let roots = input.workspace_roots();
    let policy = match load_policy(config_paths, &roots) {
        Ok(policy) => policy,
        Err(error) => {
            return Some(format!(
                "Forbidden-file policy could not be loaded: {error}"
            ));
        }
    };
    evaluate(&policy, input, &roots, max_entries)
}

fn load_policy(config_paths: &[PathBuf], roots: &[Utf8PathBuf]) -> hookkit_core::Result<Policy> {
    let explicit_paths = !config_paths.is_empty();
    let paths = if config_paths.is_empty() {
        default_config_paths(roots)
    } else {
        config_paths.to_vec()
    };

    let mut builder = GlobSetBuilder::new();
    let mut access_policy = AccessPolicy::InspectKnown;
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
        if config.block_shell_commands {
            access_policy = access_policy.max(AccessPolicy::DenyAllShell);
        }
        if let Some(configured) = config.access_policy {
            access_policy = access_policy.max(configured);
        }
        for pattern in config.patterns {
            if pattern.trim().is_empty() {
                continue;
            }
            builder.add(Glob::new(&expand_pattern_home(&pattern)).map_err(invalid_data)?);
        }
    }

    Ok(Policy {
        patterns: builder.build().map_err(invalid_data)?,
        access_policy,
    })
}

fn default_config_paths(roots: &[Utf8PathBuf]) -> Vec<PathBuf> {
    let mut paths = BTreeSet::new();
    if let Some(home) = dirs::home_dir() {
        paths.insert(home.join(PROJECT_CONFIG));
    }
    for root in roots {
        for ancestor in root.as_std_path().ancestors() {
            paths.insert(ancestor.join(PROJECT_CONFIG));
        }
    }
    paths.into_iter().collect()
}

/// Analyze one aligned native call, materialize its scoped targets, and apply
/// the caller-selected uncertainty posture.
fn evaluate(
    policy: &Policy,
    input: &PreToolUseInput,
    roots: &[Utf8PathBuf],
    max_entries: usize,
) -> Option<String> {
    let shell_call = is_exact_native_shell(input);
    if policy.access_policy == AccessPolicy::DenyAllShell && shell_call {
        return Some(blocked_shell_reason(
            input.tool_name().unwrap_or("<unknown>"),
        ));
    }

    let analyzer = ToolAccessAnalyzer::new(
        BashAnalyzer::default(),
        FileAccessAnalyzer::default()
            .with_unknown_command_fallback(UnknownCommandFallback::LiteralPathOperands),
        hookkit_tool_access::StructuredFieldAnalyzer::default(),
    );
    let report = analyzer.analyze_pre_tool(input);
    if let Some(reason) = report
        .candidates
        .iter()
        .find_map(|candidate| forbidden_candidate(policy, candidate, roots))
    {
        return Some(reason);
    }

    let mut options = TargetResolutionOptions::new(roots.to_vec());
    options.max_entries = max_entries;
    options.ignored_directory_names.clear();
    options.exact_paths = ExactPathPolicy::RetainNonexistent;
    let resolution = match resolve_targets(
        report.candidates.iter().map(|candidate| &candidate.target),
        &options,
    ) {
        Ok(resolution) => resolution,
        Err(error) => {
            return Some(format!(
                "Forbidden-file policy could not resolve an observed target: {error}"
            ));
        }
    };
    if let Some(path) = resolution
        .paths
        .iter()
        .find(|path| matches_concrete_policy(policy, path, roots))
    {
        return Some(forbidden_path_reason(path));
    }

    if policy.access_policy == AccessPolicy::DenyUnresolved
        && (!report.is_complete() || !resolution.is_complete())
    {
        return Some(format!(
            "Forbidden-file policy denies unresolved access analysis ({} analysis gap(s), {} resolution gap(s))",
            report.gaps.len(),
            resolution.unresolved.len()
        ));
    }
    None
}

fn blocked_shell_reason(tool_name: &str) -> String {
    format!(
        "Forbidden-file policy blocks shell tool `{tool_name}` because shell file access cannot be inspected exhaustively"
    )
}

fn forbidden_candidate(
    policy: &Policy,
    candidate: &AccessCandidate,
    roots: &[Utf8PathBuf],
) -> Option<String> {
    let AccessTarget::Path { expression, .. } = &candidate.target else {
        return None;
    };
    if matches_raw_policy(policy, &expression.raw)
        || expression
            .resolved
            .as_deref()
            .is_some_and(|path| matches_concrete_policy(policy, path, roots))
    {
        let shown = expression
            .resolved
            .as_deref()
            .map_or(expression.raw.as_str(), Utf8Path::as_str);
        Some(format!(
            "Forbidden-file policy denies {} access to `{shown}`",
            provenance_label(&candidate.provenance)
        ))
    } else {
        None
    }
}

fn provenance_label(provenance: &AccessProvenance) -> &'static str {
    match provenance.source() {
        AccessSource::Structured => "structured-tool",
        AccessSource::Patch => "patch",
        AccessSource::Shell => "shell",
        AccessSource::Custom => "custom",
        _ => "unknown",
    }
}

fn matches_raw_policy(policy: &Policy, raw: &str) -> bool {
    let path = Utf8Path::new(raw);
    let expanded = expand_candidate_home(path);
    [utf8_path_to_slash(path), utf8_path_to_slash(expanded)]
        .iter()
        .any(|form| policy.patterns.is_match(form))
}

fn matches_concrete_policy(policy: &Policy, path: &Utf8Path, roots: &[Utf8PathBuf]) -> bool {
    let expanded = expand_candidate_home(path);
    let mut forms = BTreeSet::from([utf8_path_to_slash(path), utf8_path_to_slash(&expanded)]);
    for root in roots {
        let absolute = resolve_utf8_path(root, &expanded);
        let normalized_root = normalize_utf8_path(root);
        insert_path_forms(&mut forms, &absolute, &normalized_root);
        if let Some(canonical) = canonicalize_utf8(&absolute) {
            let canonical_root = canonicalize_utf8(&normalized_root).unwrap_or(normalized_root);
            insert_path_forms(&mut forms, &canonical, &canonical_root);
        }
    }
    forms.iter().any(|form| policy.patterns.is_match(form))
}

fn canonicalize_utf8(path: &Utf8Path) -> Option<Utf8PathBuf> {
    std::fs::canonicalize(path.as_std_path())
        .ok()
        .and_then(|path| Utf8PathBuf::from_path_buf(path).ok())
}

fn insert_path_forms(forms: &mut BTreeSet<String>, path: &Utf8Path, root: &Utf8Path) {
    forms.insert(utf8_path_to_slash(path));
    if let Ok(relative) = path.strip_prefix(root) {
        forms.insert(utf8_path_to_slash(relative));
    }
}

fn forbidden_path_reason(path: &Utf8Path) -> String {
    format!("Forbidden-file policy denies access to `{path}`")
}

fn is_exact_native_shell(input: &PreToolUseInput) -> bool {
    let ToolCallObservation::Call(call) = observe_pre_tool(input) else {
        return false;
    };
    (call.harness() == &HarnessId::CLAUDE_CODE && call.tool_name == CLAUDE_BASH_PROFILE.tool_name())
        || (call.harness() == &HarnessId::CODEX && call.tool_name == CODEX_BASH_PROFILE.tool_name())
        || (call.harness() == &HarnessId::GEMINI_CLI
            && call.tool_name == GEMINI_RUN_SHELL_COMMAND_PROFILE.tool_name())
        || (call.harness() == &HarnessId::ANTIGRAVITY
            && call.tool_name == ANTIGRAVITY_RUN_COMMAND_PROFILE.tool_name())
}

fn expand_pattern_home(pattern: &str) -> String {
    let path = Utf8Path::new(pattern);
    dirs::home_dir()
        .and_then(|home| Utf8PathBuf::from_path_buf(home).ok())
        .map_or_else(
            || utf8_path_to_slash(path),
            |home| utf8_path_to_slash(expand_utf8_home(path, home)),
        )
}

fn expand_candidate_home(path: &Utf8Path) -> Utf8PathBuf {
    dirs::home_dir()
        .and_then(|home| Utf8PathBuf::from_path_buf(home).ok())
        .map_or_else(|| path.to_path_buf(), |home| expand_utf8_home(path, home))
}

fn invalid_data(error: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hookkit_core::{EventId, HarnessId};
    use hookkit_tool_access::{JsonRef, ToolCallRef, ToolPhase};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

    fn test_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "hookkit-{label}-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn policy(patterns: &[&str], access_policy: AccessPolicy) -> Policy {
        let mut builder = GlobSetBuilder::new();
        for pattern in patterns {
            builder.add(Glob::new(pattern).unwrap());
        }
        Policy {
            patterns: builder.build().unwrap(),
            access_policy,
        }
    }

    fn codex_input_at_cwd(
        cwd: &str,
        tool_name: &str,
        tool_input: serde_json::Value,
    ) -> PreToolUseInput {
        PreToolUseInput::Codex(
            serde_json::from_value(serde_json::json!({
                "session_id": "session",
                "transcript_path": null,
                "cwd": cwd,
                "hook_event_name": "PreToolUse",
                "model": "gpt-test",
                "turn_id": "turn",
                "permission_mode": "default",
                "tool_name": tool_name,
                "tool_use_id": "call",
                "tool_input": tool_input
            }))
            .unwrap(),
        )
    }

    fn codex_input(tool_name: &str, tool_input: serde_json::Value) -> PreToolUseInput {
        codex_input_at_cwd("/repo", tool_name, tool_input)
    }

    fn evaluate_codex(policy: &Policy, input: &PreToolUseInput) -> Option<String> {
        evaluate(
            policy,
            input,
            &[Utf8PathBuf::from("/repo")],
            TARGET_RESOLUTION_BUDGET,
        )
    }

    #[test]
    fn blocks_structured_reads_writes_and_patch_roles() {
        let guard = policy(
            &[".env", "**/.env", "secrets/**"],
            AccessPolicy::InspectKnown,
        );
        for (tool, input) in [
            ("read_file", serde_json::json!({"path": "/repo/.env"})),
            (
                "write_file",
                serde_json::json!({"file_path": "services/api/.env"}),
            ),
        ] {
            assert!(evaluate_codex(&guard, &codex_input(tool, input)).is_some());
        }

        let patch = serde_json::json!({
            "patch": "*** Add File: secrets/new.txt\n+new\n*** Update File: secrets/current.txt\n*** Delete File: secrets/old.txt\n*** Update File: secrets/from.txt\n*** Move to: secrets/to.txt\n"
        });
        assert!(evaluate_codex(&guard, &codex_input("apply_patch", patch)).is_some());
    }

    #[test]
    fn inspects_shell_candidates_and_supports_three_uncertainty_postures() {
        let roots = [Utf8PathBuf::from("/repo")];
        let inspect = policy(&[".env", "**/.env"], AccessPolicy::InspectKnown);
        for command in [
            "cat .env",
            "test -f .env && cat .env",
            "echo secret > config/.env",
        ] {
            let input = codex_input("Bash", serde_json::json!({"command": command}));
            assert!(
                evaluate(&inspect, &input, &roots, TARGET_RESOLUTION_BUDGET).is_some(),
                "expected `{command}` to be denied"
            );
        }

        let dynamic = codex_input("Bash", serde_json::json!({"command": "cat \"$SECRET\""}));
        assert!(evaluate(&inspect, &dynamic, &roots, TARGET_RESOLUTION_BUDGET).is_none());
        let deny_unresolved = policy(&["**/.env"], AccessPolicy::DenyUnresolved);
        assert!(evaluate(&deny_unresolved, &dynamic, &roots, TARGET_RESOLUTION_BUDGET).is_some());

        let deny_all = policy(&[], AccessPolicy::DenyAllShell);
        let safe = codex_input("Bash", serde_json::json!({"command": "echo safe"}));
        assert!(evaluate(&deny_all, &safe, &roots, TARGET_RESOLUTION_BUDGET).is_some());
    }

    #[test]
    fn shell_patch_and_recursive_remove_materialize_forbidden_descendants() {
        let directory = test_dir("forbidden-descendants");
        let root = Utf8PathBuf::from_path_buf(directory.clone()).unwrap();
        std::fs::create_dir_all(root.join("secrets/nested")).unwrap();
        std::fs::write(root.join("secrets/nested/token.txt"), "token").unwrap();
        let guard = policy(&["secrets/**"], AccessPolicy::InspectKnown);

        let remove = codex_input_at_cwd(
            root.as_str(),
            "Bash",
            serde_json::json!({"command": "rm -rf secrets"}),
        );
        assert!(
            evaluate(
                &guard,
                &remove,
                std::slice::from_ref(&root),
                TARGET_RESOLUTION_BUDGET
            )
            .is_some()
        );

        let patch = codex_input_at_cwd(
            root.as_str(),
            "Bash",
            serde_json::json!({
                "command": "apply_patch <<'PATCH'\n*** Add File: secrets/new.txt\n+new\nPATCH\n"
            }),
        );
        assert!(
            evaluate(
                &guard,
                &patch,
                std::slice::from_ref(&root),
                TARGET_RESOLUTION_BUDGET
            )
            .is_some()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unknown_literal_fallback_and_truncation_are_explicit_policy_inputs() {
        let directory = test_dir("forbidden-truncation");
        let root = Utf8PathBuf::from_path_buf(directory.clone()).unwrap();
        std::fs::create_dir_all(root.join("secrets")).unwrap();
        std::fs::write(root.join("secrets/token.txt"), "token").unwrap();
        let inspect = policy(&["secrets/**"], AccessPolicy::InspectKnown);

        let unknown = codex_input_at_cwd(
            root.as_str(),
            "Bash",
            serde_json::json!({"command": "mystery secrets/token.txt"}),
        );
        assert!(evaluate(&inspect, &unknown, std::slice::from_ref(&root), 100).is_some());

        let scoped = codex_input_at_cwd(
            root.as_str(),
            "Bash",
            serde_json::json!({"command": "rm -rf secrets"}),
        );
        assert!(evaluate(&inspect, &scoped, std::slice::from_ref(&root), 0).is_none());
        let deny_unresolved = policy(&["secrets/**"], AccessPolicy::DenyUnresolved);
        assert!(evaluate(&deny_unresolved, &scoped, std::slice::from_ref(&root), 0).is_some());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn malformed_shell_is_denied_when_unresolved_access_is_denied() {
        let guard = policy(&["**/.env"], AccessPolicy::DenyUnresolved);
        let malformed = codex_input("Bash", serde_json::json!({"not_command": "cat .env"}));
        assert!(evaluate_codex(&guard, &malformed).is_some());
    }

    #[test]
    fn exact_native_shell_names_and_explicit_aliases_are_distinct() {
        let native = codex_input("Bash", serde_json::json!({"command": "echo safe"}));
        let guessed = codex_input(
            "exec_command",
            serde_json::json!({"command": "cat secrets/token.txt"}),
        );
        assert!(is_exact_native_shell(&native));
        assert!(!is_exact_native_shell(&guessed));

        let alias_input = serde_json::json!({"request": {"command": "cat secrets/token.txt"}});
        let alias = ToolCallRef::new(
            EventId::builtin(HarnessId::CODEX, "PreToolUse"),
            ToolPhase::Pre,
            "project_shell",
            JsonRef::Value(&alias_input),
            Some(Utf8Path::new("/repo")),
            vec![Utf8PathBuf::from("/repo")],
        );
        let profile =
            hookkit_shell::ShellToolProfile::new("project_shell", "/request/command").unwrap();
        let report = ToolAccessAnalyzer::default()
            .with_shell_profile(profile)
            .analyze_call(&alias);
        assert!(
            report
                .candidates
                .iter()
                .any(|candidate| matches!(candidate.provenance, AccessProvenance::Shell { .. }))
        );
    }

    #[test]
    fn native_cwd_is_distinct_from_policy_roots() {
        let guard = policy(&["/project/src/**"], AccessPolicy::InspectKnown);
        let input = codex_input_at_cwd(
            "/native/cwd",
            "read_file",
            serde_json::json!({"path": "src/lib.rs"}),
        );
        assert!(
            evaluate(
                &guard,
                &input,
                &[Utf8PathBuf::from("/project")],
                TARGET_RESOLUTION_BUDGET
            )
            .is_none()
        );
    }

    #[test]
    fn antigravity_command_line_shell_is_inspected() {
        let input = PreToolUseInput::Antigravity(
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
            .unwrap(),
        );
        let guard = policy(&["**/.env"], AccessPolicy::InspectKnown);
        assert!(evaluate_codex(&guard, &input).is_some());
    }

    #[test]
    fn explicit_configs_merge_and_legacy_shell_blocking_maps_to_deny_all() {
        let directory = test_dir("forbidden-config");
        std::fs::create_dir_all(&directory).unwrap();
        let first = directory.join("first.yaml");
        let second = directory.join("second.yaml");
        std::fs::write(
            &first,
            "patterns: ['.env']\naccess_policy: deny_unresolved\n",
        )
        .unwrap();
        std::fs::write(
            &second,
            "patterns: ['**/*.pem']\nblock_shell_commands: true\n",
        )
        .unwrap();

        let guard = load_policy(&[first, second], &[Utf8PathBuf::from("/repo")]).unwrap();
        assert_eq!(guard.patterns.len(), 2);
        assert_eq!(guard.access_policy, AccessPolicy::DenyAllShell);
        assert!(guard.patterns.is_match("keys/private.pem"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn missing_explicit_config_is_an_error() {
        let missing = test_dir("missing-forbidden-config").join("policy.yaml");
        assert!(load_policy(&[missing], &[Utf8PathBuf::from("/repo")]).is_err());
    }

    #[test]
    fn parent_traversal_is_normalized_against_native_cwd() {
        let guard = policy(&[".env"], AccessPolicy::InspectKnown);
        let input = codex_input("read_file", serde_json::json!({"path": "../../repo/.env"}));
        assert!(evaluate_codex(&guard, &input).is_some());
    }

    #[cfg(unix)]
    #[test]
    fn canonical_candidate_forms_detect_symlinked_forbidden_paths() {
        let directory = test_dir("forbidden-symlink");
        let root = Utf8PathBuf::from_path_buf(directory.clone()).unwrap();
        std::fs::create_dir_all(root.join("real")).unwrap();
        std::fs::write(root.join("real/.env"), "secret").unwrap();
        std::os::unix::fs::symlink(root.join("real/.env"), root.join("alias")).unwrap();
        let guard = policy(&["real/.env"], AccessPolicy::InspectKnown);
        let input = codex_input_at_cwd(
            root.as_str(),
            "read_file",
            serde_json::json!({"path": "alias"}),
        );
        assert!(
            evaluate(
                &guard,
                &input,
                std::slice::from_ref(&root),
                TARGET_RESOLUTION_BUDGET
            )
            .is_some()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
