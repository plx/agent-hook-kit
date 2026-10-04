use clap::{Parser, ValueEnum};
use globset::{Glob, GlobSet, GlobSetBuilder};
use hookkit_common::{PreToolUseCommandEnvironment, PreToolUseInput, PreToolUseOutput};
use hookkit_core::{
    EventId, HarnessId, Utf8Path, Utf8PathBuf, expand_utf8_home, normalize_utf8_path,
    resolve_utf8_path, utf8_path_to_slash,
};
use hookkit_shell::{
    ANTIGRAVITY_RUN_COMMAND_PROFILE, BashAnalyzer, CLAUDE_BASH_PROFILE, CODEX_BASH_PROFILE,
    FileAccessAnalyzer, UnknownCommandFallback,
};
use hookkit_tool_access::{
    AccessCandidate, AccessProvenance, AccessSource, AccessTarget, ExactPathPolicy, PathBase,
    PathExpression, TargetResolutionOptions, ToolAccessAnalyzer, ToolAccessReport,
    ToolCallObservation, observe_pre_tool, resolve_targets,
};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::PathBuf;

const PROJECT_CONFIG: &str = ".agent-hook-kit/forbidden-files.yaml";
const TARGET_RESOLUTION_BUDGET: usize = 100_000;

/// Claude Code tools that run shell commands the analyzer cannot parse:
/// `PowerShell`, the primary shell on Windows wherever it is enabled, and
/// `Monitor`, whose watch commands Claude Code reviews the same way as `Bash`
/// commands.
const CLAUDE_UNPARSED_SHELL_TOOLS: &[&str] = &["PowerShell", "Monitor"];

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Harness {
    /// Claude Code; `claude-code` is the canonical HookKit identity.
    #[value(alias = "claude-code")]
    Claude,
    Codex,
    Antigravity,
}

impl Harness {
    fn id(self) -> HarnessId {
        match self {
            Self::Claude => HarnessId::CLAUDE_CODE,
            Self::Codex => HarnessId::CODEX,
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
    /// Every root-relative pattern prefixed with `**/`, for operands whose
    /// working directory the payload does not report; see
    /// [`session_relative_operands`].
    floating: GlobSet,
    posture: Posture,
}

/// One configured uncertainty posture. Each value turns on one independent
/// strictness flag; see [`Posture`].
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AccessPolicy {
    /// Match recovered candidates and allow analysis/resolver gaps.
    #[default]
    InspectKnown,
    /// Match candidates and deny when either analysis or resolution is incomplete.
    DenyUnresolved,
    /// Deny native shell calls (see [`is_native_shell`]); inspect non-shell
    /// calls like `InspectKnown`.
    DenyAllShell,
}

/// The effective uncertainty posture: independent strictness flags OR-ed
/// across every setting in every loaded configuration file.
///
/// `deny_unresolved` and `deny_all_shell` constrain different calls (the first
/// non-shell gaps, the second every shell call), so neither subsumes the
/// other. Combining them, within one file or across the home and project
/// layers, keeps both: a later or weaker layer can never relax an earlier,
/// stricter one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Posture {
    /// Deny every native shell call.
    deny_shell: bool,
    /// Deny any call whose access analysis or target resolution is incomplete.
    deny_unresolved: bool,
}

impl Posture {
    fn include(&mut self, policy: AccessPolicy) {
        match policy {
            AccessPolicy::InspectKnown => {}
            AccessPolicy::DenyUnresolved => self.deny_unresolved = true,
            AccessPolicy::DenyAllShell => self.deny_shell = true,
        }
    }
}

fn main() -> std::process::ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        // `--help` and `--version` print to stdout and succeed.
        Err(error) if !error.use_stderr() => {
            let _ = error.print();
            return std::process::ExitCode::SUCCESS;
        }
        Err(error) => return usage_error(&error),
    };
    let harness = cli.harness.id();
    // A guard must not let a call through because it failed to decide: every
    // parse, environment, handler, or emission failure becomes a native deny.
    hookkit_runtime::aligned::run_aligned_event_with_options::<
        hookkit_runtime::aligned::PreToolUse,
        _,
    >(
        harness.clone(),
        hookkit_runtime::RunOptions::new().fail_closed(),
        move |input, environment, _context| {
            let reason = deny_reason(
                &cli.config_paths,
                &input,
                environment,
                TARGET_RESOLUTION_BUDGET,
            );
            match reason {
                Some(reason) => PreToolUseOutput::deny(&harness, reason),
                // No objection: leave the call to the harness's normal
                // permission flow. An explicit allow would auto-approve it.
                None => PreToolUseOutput::pass_through(&harness),
            }
        },
    )
}

/// Blocks the call when the command line is invalid.
///
/// A guard that cannot read its own configuration cannot decide, so it fails
/// closed like any other failure: exit 2 with the usage error as the reason,
/// which Claude Code and Codex treat as a block, or a `deny` decision when
/// `--harness` names Antigravity. Exiting 1, a non-blocking hook error, would
/// let every call through on a typo such as `--confg`.
fn usage_error(error: &clap::Error) -> std::process::ExitCode {
    let Some(harness) = requested_harness(std::env::args_os().skip(1)) else {
        // The harness is unknown, so no native deny can be chosen; exit 2
        // blocks on Claude Code and Codex.
        eprintln!("{}", usage_diagnostic(None, error));
        return std::process::ExitCode::from(2);
    };
    let harness = harness.id();
    let event = EventId::builtin(harness.clone(), "PreToolUse");
    hookkit_runtime::failure::failure_response(
        hookkit_runtime::FailurePolicy::FailClosed,
        &harness,
        Some(&event),
        &usage_diagnostic(Some(&event), error),
    )
    .emit()
}

/// The harness a `--harness` argument names, even when another argument is
/// invalid (including one that is not UTF-8).
fn requested_harness(arguments: impl IntoIterator<Item = OsString>) -> Option<Harness> {
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        let Some(argument) = argument.to_str() else {
            continue;
        };
        let value = match argument.strip_prefix("--harness") {
            Some("") => arguments.next()?.into_string().ok()?,
            Some(value) => match value.strip_prefix('=') {
                Some(value) => value.to_owned(),
                None => continue,
            },
            None if argument == "--" => return None,
            None => continue,
        };
        return Harness::from_str(&value, false).ok();
    }
    None
}

/// One stderr line naming the program, the hook, and clap's message.
fn usage_diagnostic(event: Option<&EventId>, error: &clap::Error) -> String {
    let message = error.to_string();
    let message = message.split_whitespace().collect::<Vec<_>>().join(" ");
    match event {
        Some(event) => {
            format!("hookkit: forbidden-file-guard {event} failed: invalid arguments: {message}")
        }
        None => format!("hookkit: forbidden-file-guard failed: invalid arguments: {message}"),
    }
}

/// Returns the roots used for policy discovery and root-relative matching:
/// the stable project roots (Claude Code's `CLAUDE_PROJECT_DIR`, Codex's
/// `cwd`, Antigravity's workspace paths), then the native working directory,
/// then the checkout that encloses it, each when it differs.
///
/// Claude Code's `cwd` follows `cd` and worktree switches, so on its own it
/// would miss the project configuration after `cd /tmp` and mis-anchor
/// root-relative patterns after `cd src`. Keeping it as an extra root covers
/// a worktree the agent entered, whose root `CLAUDE_PROJECT_DIR` does not
/// name, and the enclosing checkout (the nearest ancestor with a `.git`
/// directory or file) keeps covering that worktree after a `cd src` inside it.
fn policy_roots(
    input: &PreToolUseInput,
    environment: &PreToolUseCommandEnvironment,
) -> Vec<Utf8PathBuf> {
    let mut roots = input.project_roots(environment).into_owned();
    for directory in input.workspace_roots().iter() {
        let checkout = enclosing_checkout(directory);
        for root in std::iter::once(directory.as_path()).chain(checkout) {
            if !roots.iter().any(|known| known == root) {
                roots.push(root.to_path_buf());
            }
        }
    }
    roots
}

/// The nearest ancestor of `directory`, itself included, that holds a `.git`
/// directory, or the `.git` file of a linked worktree.
fn enclosing_checkout(directory: &Utf8Path) -> Option<&Utf8Path> {
    directory
        .ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
}

/// Load the active policy and return a deny reason, or `None` for no
/// objection. A policy that fails to load is itself a deny so a broken
/// configuration never silently opens the boundary.
fn deny_reason(
    config_paths: &[PathBuf],
    input: &PreToolUseInput,
    environment: &PreToolUseCommandEnvironment,
    max_entries: usize,
) -> Option<String> {
    let roots = policy_roots(input, environment);
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
    let mut floating = GlobSetBuilder::new();
    let mut posture = Posture::default();
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
            posture.include(AccessPolicy::DenyAllShell);
        }
        if let Some(configured) = config.access_policy {
            posture.include(configured);
        }
        for pattern in config.patterns {
            if pattern.trim().is_empty() {
                continue;
            }
            let pattern = expand_pattern_home(&pattern);
            builder.add(Glob::new(&pattern).map_err(invalid_data)?);
            if let Some(pattern) = floating_pattern(&pattern) {
                floating.add(Glob::new(&pattern).map_err(invalid_data)?);
            }
        }
    }

    Ok(Policy {
        patterns: builder.build().map_err(invalid_data)?,
        floating: floating.build().map_err(invalid_data)?,
        posture,
    })
}

/// `pattern` anchored anywhere (`**/pattern`) when it is relative to a policy
/// root; `None` for an absolute pattern or one that already floats.
fn floating_pattern(pattern: &str) -> Option<String> {
    (!pattern.starts_with('/') && !pattern.starts_with("**/") && pattern != "**")
        .then(|| format!("**/{}", pattern.trim_start_matches("./")))
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
    let shell_call = is_native_shell(input);
    if policy.posture.deny_shell && shell_call {
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
    let session_relative = session_relative_operands(&report);
    if let Some(expression) = session_relative
        .iter()
        .find(|expression| matches_floating_policy(policy, &expression.raw))
    {
        return Some(format!(
            "Forbidden-file policy denies access to `{}`: the tool's working directory is not reported, so it may name a forbidden path",
            expression.raw
        ));
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

    if policy.posture.deny_unresolved
        && (!report.is_complete() || !resolution.is_complete() || !session_relative.is_empty())
    {
        return Some(format!(
            "Forbidden-file policy denies unresolved access analysis ({} analysis gap(s), {} resolution gap(s), {} operand(s) relative to an unreported working directory)",
            report.gaps.len(),
            resolution.unresolved.len(),
            session_relative.len()
        ));
    }
    None
}

/// Relative shell operands the analyzer could only resolve against the hook's
/// session directory ([`PathBase::SessionCwd`]).
///
/// The command may run elsewhere: a Codex shell call's `workdir` argument
/// moves it, and the `Bash` hook payload omits it. `cat token.txt` run in
/// `secrets/` then reads `secrets/token.txt` although the operand resolves to
/// `<cwd>/token.txt`. `deny_unresolved` counts such operands as unresolved,
/// and every posture matches them against root-relative patterns anchored
/// anywhere (`**/secrets/**`), which catches a workdir above the forbidden
/// path but not one inside it.
fn session_relative_operands(report: &ToolAccessReport) -> Vec<&PathExpression> {
    report
        .candidates
        .iter()
        .filter(|candidate| candidate.provenance.source() == AccessSource::Shell)
        .filter_map(|candidate| match &candidate.target {
            AccessTarget::Path { expression, .. }
                if expression.base == PathBase::SessionCwd
                    && Utf8Path::new(&expression.raw).is_relative() =>
            {
                Some(expression)
            }
            _ => None,
        })
        .collect()
}

fn matches_floating_policy(policy: &Policy, raw: &str) -> bool {
    let path = Utf8Path::new(raw);
    [
        utf8_path_to_slash(path),
        utf8_path_to_slash(normalize_utf8_path(path)),
    ]
    .iter()
    .any(|form| policy.floating.is_match(form.trim_start_matches("./")))
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

/// Whether the call runs a shell command: an exact native shell profile
/// (Claude Code `Bash`, Codex `Bash`, Antigravity `run_command`) or a Claude
/// Code shell tool the analyzer cannot parse (see
/// [`CLAUDE_UNPARSED_SHELL_TOOLS`]).
fn is_native_shell(input: &PreToolUseInput) -> bool {
    let ToolCallObservation::Call(call) = observe_pre_tool(input) else {
        return false;
    };
    (call.harness() == &HarnessId::CLAUDE_CODE
        && (call.tool_name == CLAUDE_BASH_PROFILE.tool_name()
            || CLAUDE_UNPARSED_SHELL_TOOLS.contains(&call.tool_name)))
        || (call.harness() == &HarnessId::CODEX && call.tool_name == CODEX_BASH_PROFILE.tool_name())
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
        let mut floating = GlobSetBuilder::new();
        for pattern in patterns {
            builder.add(Glob::new(pattern).unwrap());
            if let Some(pattern) = floating_pattern(pattern) {
                floating.add(Glob::new(&pattern).unwrap());
            }
        }
        let mut posture = Posture::default();
        posture.include(access_policy);
        Policy {
            patterns: builder.build().unwrap(),
            floating: floating.build().unwrap(),
            posture,
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

    fn claude_input_at_cwd(
        cwd: &str,
        tool_name: &str,
        tool_input: serde_json::Value,
    ) -> PreToolUseInput {
        PreToolUseInput::Claude(
            serde_json::from_value(serde_json::json!({
                "session_id": "session",
                "transcript_path": "/tmp/transcript.jsonl",
                "cwd": cwd,
                "hook_event_name": "PreToolUse",
                "permission_mode": "default",
                "tool_name": tool_name,
                "tool_input": tool_input,
                "tool_use_id": "toolu_1"
            }))
            .unwrap(),
        )
    }

    fn antigravity_input(tool_name: &str, args: serde_json::Value) -> PreToolUseInput {
        PreToolUseInput::Antigravity(
            serde_json::from_value(serde_json::json!({
                "conversationId": "conversation",
                "workspacePaths": ["/repo"],
                "transcriptPath": "/tmp/transcript.jsonl",
                "artifactDirectoryPath": "/tmp/artifacts",
                "toolCall": {"name": tool_name, "args": args},
                "stepIdx": 1
            }))
            .unwrap(),
        )
    }

    /// Codex's native apply_patch hook input: the patch text in `command`.
    fn codex_patch(patch_body: &str) -> PreToolUseInput {
        codex_input(
            "apply_patch",
            serde_json::json!({
                "command": format!("*** Begin Patch\n{patch_body}*** End Patch\n")
            }),
        )
    }

    #[test]
    fn blocks_structured_reads_writes_and_patch_roles() {
        let guard = policy(
            &[".env", "**/.env", "secrets/**"],
            AccessPolicy::InspectKnown,
        );
        for (tool, input) in [
            ("Read", serde_json::json!({"file_path": "/repo/.env"})),
            (
                "Write",
                serde_json::json!({"file_path": "/repo/services/api/.env", "content": "x"}),
            ),
            (
                "Edit",
                serde_json::json!({"file_path": "/repo/.env", "old_string": "a", "new_string": "b"}),
            ),
        ] {
            let input = claude_input_at_cwd("/repo", tool, input);
            assert!(evaluate_codex(&guard, &input).is_some(), "{tool}");
        }

        for body in [
            "*** Add File: secrets/new.txt\n+new\n",
            "*** Update File: secrets/current.txt\n@@\n-old\n+new\n",
            "*** Delete File: secrets/old.txt\n",
            "*** Update File: src/from.txt\n*** Move to: secrets/to.txt\n@@\n-a\n+b\n",
        ] {
            assert!(
                evaluate_codex(&guard, &codex_patch(body)).is_some(),
                "{body}"
            );
        }
        let benign = codex_patch("*** Update File: src/lib.rs\n@@\n-old\n+new\n");
        assert!(evaluate_codex(&guard, &benign).is_none());
        let strict = policy(&["secrets/**"], AccessPolicy::DenyUnresolved);
        assert!(evaluate_codex(&strict, &benign).is_none());
    }

    #[test]
    fn claude_search_and_notebook_tools_are_inspected() {
        let directory = test_dir("forbidden-claude-search");
        let root = Utf8PathBuf::from_path_buf(directory.clone()).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(root.join("config/.env"), "KEY=secret").unwrap();
        std::fs::write(root.join("lib.rs"), "fn main() {}").unwrap();
        let guard = policy(&["**/.env"], AccessPolicy::InspectKnown);
        let evaluate_claude = |tool: &str, input: serde_json::Value| {
            evaluate(
                &guard,
                &claude_input_at_cwd(root.as_str(), tool, input),
                std::slice::from_ref(&root),
                TARGET_RESOLUTION_BUDGET,
            )
        };

        for (tool, input) in [
            (
                "Grep",
                serde_json::json!({"pattern": "KEY", "path": root.as_str(), "glob": "**/.env"}),
            ),
            (
                "Grep",
                serde_json::json!({"pattern": "KEY", "glob": ".env"}),
            ),
            (
                "Grep",
                serde_json::json!({"pattern": "KEY", "output_mode": "content"}),
            ),
            ("Glob", serde_json::json!({"pattern": "**/.env"})),
            (
                "NotebookEdit",
                serde_json::json!({"notebook_path": root.join(".env").as_str(), "new_source": "x"}),
            ),
        ] {
            assert!(evaluate_claude(tool, input.clone()).is_some(), "{input}");
        }

        // A search narrowed to other files does not touch the secret.
        assert!(
            evaluate_claude(
                "Grep",
                serde_json::json!({"pattern": "KEY", "glob": "*.rs"})
            )
            .is_none()
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn antigravity_documented_file_tools_are_inspected() {
        let guard = policy(&["**/.env"], AccessPolicy::InspectKnown);
        for (tool, args) in [
            (
                "view_file",
                serde_json::json!({"AbsolutePath": "/repo/.env"}),
            ),
            (
                "write_to_file",
                serde_json::json!({"TargetFile": "/repo/.env", "CodeContent": "x"}),
            ),
            (
                "replace_file_content",
                serde_json::json!({"TargetFile": "/repo/.env", "TargetContent": "a"}),
            ),
            (
                "multi_replace_file_content",
                serde_json::json!({"TargetFile": "/repo/.env", "ReplacementChunks": []}),
            ),
            (
                "grep_search",
                serde_json::json!({"SearchPath": "/repo/.env", "Query": "KEY"}),
            ),
        ] {
            assert!(
                evaluate_codex(&guard, &antigravity_input(tool, args)).is_some(),
                "{tool}"
            );
        }
    }

    #[test]
    fn file_free_tools_are_not_denied_as_unresolved() {
        let strict = policy(&["**/.env"], AccessPolicy::DenyUnresolved);
        for input in [
            claude_input_at_cwd(
                "/repo",
                "TodoWrite",
                serde_json::json!({"todos": [{"content": "write the file"}]}),
            ),
            claude_input_at_cwd(
                "/repo",
                "WebFetch",
                serde_json::json!({"url": "https://example.com", "prompt": "p"}),
            ),
            codex_input("update_plan", serde_json::json!({"plan": []})),
            antigravity_input("search_web", serde_json::json!({"query": "rust"})),
        ] {
            assert!(evaluate_codex(&strict, &input).is_none());
        }
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
        assert!(is_native_shell(&native));
        assert!(!is_native_shell(&guessed));

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
            "mcp__filesystem__read_file",
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
        // The second file's shell block adds to, and does not replace, the
        // first file's unresolved-access denial.
        assert_eq!(
            guard.posture,
            Posture {
                deny_shell: true,
                deny_unresolved: true
            }
        );
        assert!(guard.patterns.is_match("keys/private.pem"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn combined_postures_keep_denying_unresolved_non_shell_calls() {
        // Regression: taking the "maximum" posture let `deny_all_shell`
        // silently replace `deny_unresolved`, so an incomplete non-shell
        // analysis was allowed as soon as shell blocking was also enabled.
        let directory = test_dir("forbidden-posture");
        std::fs::create_dir_all(&directory).unwrap();
        let home = directory.join("home.yaml");
        let project = directory.join("project.yaml");
        std::fs::write(
            &home,
            "patterns: ['secrets/**']\naccess_policy: deny_unresolved\n",
        )
        .unwrap();
        std::fs::write(&project, "access_policy: deny_all_shell\n").unwrap();
        let single = directory.join("single.yaml");
        std::fs::write(
            &single,
            "patterns: ['secrets/**']\naccess_policy: deny_unresolved\nblock_shell_commands: true\n",
        )
        .unwrap();

        let incomplete = codex_input("Bash", serde_json::json!({"command": "cat \"$SECRET\""}));
        let incomplete_patch = codex_input(
            "apply_patch",
            serde_json::json!({"command": "*** Begin Patch\n*** Update File: \n*** End Patch\n"}),
        );
        let roots = [Utf8PathBuf::from("/repo")];
        for paths in [vec![home.clone(), project.clone()], vec![single.clone()]] {
            let guard = load_policy(&paths, &roots).unwrap();
            assert!(guard.posture.deny_shell && guard.posture.deny_unresolved);
            assert!(evaluate(&guard, &incomplete, &roots, TARGET_RESOLUTION_BUDGET).is_some());
            let analysis = ToolAccessAnalyzer::default().analyze_pre_tool(&incomplete_patch);
            assert!(!analysis.is_complete(), "{:?}", analysis.gaps);
            assert!(
                evaluate(&guard, &incomplete_patch, &roots, TARGET_RESOLUTION_BUDGET).is_some(),
                "an incomplete non-shell analysis must still be denied"
            );
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn claude_policy_roots_prefer_the_project_dir_and_keep_the_moving_cwd() {
        let input = claude_input_at_cwd(
            "/repo/src",
            "Read",
            serde_json::json!({"file_path": "/repo/secrets/token.txt"}),
        );
        let environment = PreToolUseCommandEnvironment::Claude(
            <hookkit_claude::ClaudeCommandEnvironment as hookkit_core::CommandEnvironmentSpec>::from_variables(
                &EventId::builtin(HarnessId::CLAUDE_CODE, "PreToolUse"),
                &hookkit_core::EnvironmentVariables::from_pairs([
                    ("CLAUDECODE", "1"),
                    ("CLAUDE_CODE_CHILD_SESSION", "1"),
                    ("CLAUDE_CODE_SESSION_ID", "session"),
                    ("CLAUDE_PROJECT_DIR", "/repo"),
                ]),
            )
            .unwrap(),
        );
        let roots = policy_roots(&input, &environment);
        assert_eq!(
            roots,
            [Utf8PathBuf::from("/repo"), Utf8PathBuf::from("/repo/src")]
        );
        // `secrets/**` is relative to the project root, not to the directory
        // the agent moved into.
        let guard = policy(&["secrets/**"], AccessPolicy::InspectKnown);
        assert!(evaluate(&guard, &input, &roots, TARGET_RESOLUTION_BUDGET).is_some());
    }

    #[test]
    fn claude_powershell_and_monitor_are_shell_tools() {
        let deny_all = policy(&["**/.env"], AccessPolicy::DenyAllShell);
        for (tool, input) in [
            (
                "PowerShell",
                serde_json::json!({"command": "Get-Content .env"}),
            ),
            (
                "PowerShell",
                serde_json::json!({"command": "Write-Output hi"}),
            ),
            ("Monitor", serde_json::json!({"command": "tail -f .env"})),
        ] {
            let input = claude_input_at_cwd("/repo", tool, input);
            assert!(is_native_shell(&input), "{tool}");
            let reason = evaluate_codex(&deny_all, &input).unwrap();
            assert!(reason.contains(&format!("shell tool `{tool}`")), "{reason}");
        }
        // The analyzer parses neither, so deny_unresolved also denies them.
        let strict = policy(&["**/.env"], AccessPolicy::DenyUnresolved);
        for tool in ["PowerShell", "Monitor"] {
            let input = claude_input_at_cwd(
                "/repo",
                tool,
                serde_json::json!({"command": "Get-Content notes.txt"}),
            );
            assert!(evaluate_codex(&strict, &input).is_some(), "{tool}");
        }
    }

    #[test]
    fn codex_shell_operands_relative_to_an_unreported_workdir_are_uncertain() {
        // Codex runs `exec_command` in its `workdir` argument, which the Bash
        // hook payload omits: `cat token.txt` may read `secrets/token.txt`.
        let hidden = codex_input("Bash", serde_json::json!({"command": "cat token.txt"}));
        let inspect = policy(&["secrets/**"], AccessPolicy::InspectKnown);
        assert!(evaluate_codex(&inspect, &hidden).is_none());
        let strict = policy(&["secrets/**"], AccessPolicy::DenyUnresolved);
        let reason = evaluate_codex(&strict, &hidden).unwrap();
        assert!(
            reason.contains("1 operand(s) relative to an unreported working directory"),
            "{reason}"
        );

        // A workdir above the forbidden directory is caught under every
        // posture by matching the pattern anywhere below it.
        for command in ["cat app/secrets/token.txt", "cat ../secrets/token.txt"] {
            let input = codex_input("Bash", serde_json::json!({"command": command}));
            let reason = evaluate_codex(&inspect, &input).unwrap_or_else(|| panic!("{command}"));
            assert!(
                reason.contains("working directory is not reported"),
                "{reason}"
            );
        }

        // Absolute operands, and tools whose directory is reported, are
        // unaffected.
        let absolute = codex_input("Bash", serde_json::json!({"command": "cat /tmp/notes.txt"}));
        assert!(evaluate_codex(&strict, &absolute).is_none());
        let claude = claude_input_at_cwd(
            "/repo",
            "Bash",
            serde_json::json!({"command": "cat notes.txt"}),
        );
        assert!(evaluate_codex(&strict, &claude).is_none());
        let patch = codex_patch("*** Update File: app/secrets.txt\n@@\n-a\n+b\n");
        assert!(evaluate_codex(&strict, &patch).is_none());
    }

    #[test]
    fn a_worktree_stays_a_policy_root_after_cd_inside_it() {
        let directory = test_dir("forbidden-worktree");
        let project = Utf8PathBuf::from_path_buf(directory.clone()).unwrap();
        let worktree = project.join(".claude/worktrees/feat");
        std::fs::create_dir_all(project.join(".git")).unwrap();
        std::fs::create_dir_all(worktree.join("src")).unwrap();
        std::fs::create_dir_all(worktree.join("secrets")).unwrap();
        // A linked worktree's `.git` is a file naming the main repository.
        std::fs::write(
            worktree.join(".git"),
            "gitdir: ../../../.git/worktrees/feat\n",
        )
        .unwrap();
        std::fs::write(worktree.join("secrets/token.txt"), "token").unwrap();

        let input = claude_input_at_cwd(
            worktree.join("src").as_str(),
            "Read",
            serde_json::json!({"file_path": worktree.join("secrets/token.txt").as_str()}),
        );
        let environment = PreToolUseCommandEnvironment::Claude(
            <hookkit_claude::ClaudeCommandEnvironment as hookkit_core::CommandEnvironmentSpec>::from_variables(
                &EventId::builtin(HarnessId::CLAUDE_CODE, "PreToolUse"),
                &hookkit_core::EnvironmentVariables::from_pairs([
                    ("CLAUDECODE", "1"),
                    ("CLAUDE_CODE_CHILD_SESSION", "1"),
                    ("CLAUDE_CODE_SESSION_ID", "session"),
                    ("CLAUDE_PROJECT_DIR", project.as_str()),
                ]),
            )
            .unwrap(),
        );
        let roots = policy_roots(&input, &environment);
        assert_eq!(
            roots,
            [project.clone(), worktree.join("src"), worktree.clone()]
        );
        let guard = policy(&["secrets/**"], AccessPolicy::InspectKnown);
        assert!(evaluate(&guard, &input, &roots, TARGET_RESOLUTION_BUDGET).is_some());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn the_requested_harness_is_found_despite_other_argument_errors() {
        let arguments =
            |text: &str| -> Vec<OsString> { text.split_whitespace().map(OsString::from).collect() };
        for (text, expected) in [
            ("--harness=codex --bogus", Some("codex")),
            ("--confg x --harness antigravity", Some("antigravity")),
            ("--harness=claude-code", Some("claude")),
            ("--harness=claude", Some("claude")),
            ("--harness=claud", None),
            ("--harnessx=codex", None),
            ("--config x", None),
            ("--harness", None),
        ] {
            let found = requested_harness(arguments(text))
                .map(|harness| harness.to_possible_value().unwrap().get_name().to_owned());
            assert_eq!(found.as_deref(), expected, "{text}");
        }

        // An argument that is not UTF-8 is what clap rejected; it must not
        // stop the scan (`std::env::args` would panic on it).
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt;
            let arguments = [
                OsString::from_vec(b"--config=\xff".to_vec()),
                OsString::from("--harness=codex"),
            ];
            assert!(matches!(requested_harness(arguments), Some(Harness::Codex)));
        }
    }

    #[test]
    fn missing_explicit_config_is_an_error() {
        let missing = test_dir("missing-forbidden-config").join("policy.yaml");
        assert!(load_policy(&[missing], &[Utf8PathBuf::from("/repo")]).is_err());
    }

    #[test]
    fn parent_traversal_is_normalized_against_native_cwd() {
        let guard = policy(&[".env"], AccessPolicy::InspectKnown);
        let input = codex_input(
            "mcp__filesystem__read_file",
            serde_json::json!({"path": "../../repo/.env"}),
        );
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
            "mcp__filesystem__read_file",
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
