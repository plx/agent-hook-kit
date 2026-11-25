use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "hookkit-guard-integration-{label}-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Builds each harness's documented native file-tool payload for `path`:
/// Claude `Read` (absolute `file_path`), Codex `apply_patch` (patch text in
/// `command`), and Antigravity `view_file` (PascalCase `AbsolutePath`).
fn input(harness: &str, cwd: &Path, path: &str) -> serde_json::Value {
    let absolute = cwd.join(path);
    let absolute = absolute.to_str().unwrap();
    let cwd = cwd.to_str().unwrap();
    match harness {
        "claude" => serde_json::json!({
            "session_id": "session",
            "transcript_path": "/tmp/transcript.jsonl",
            "cwd": cwd,
            "permission_mode": "default",
            "hook_event_name": "PreToolUse",
            "tool_name": "Read",
            "tool_input": {"file_path": absolute},
            "tool_use_id": "call"
        }),
        "codex" => serde_json::json!({
            "session_id": "session",
            "transcript_path": null,
            "cwd": cwd,
            "hook_event_name": "PreToolUse",
            "model": "gpt-test",
            "turn_id": "turn",
            "permission_mode": "default",
            "tool_name": "apply_patch",
            "tool_use_id": "call",
            "tool_input": {
                "command": format!(
                    "*** Begin Patch\n*** Update File: {path}\n@@\n-old\n+new\n*** End Patch\n"
                )
            }
        }),
        "antigravity" => serde_json::json!({
            "conversationId": "session",
            "workspacePaths": [cwd],
            "transcriptPath": "/tmp/transcript.jsonl",
            "artifactDirectoryPath": "/tmp/artifacts",
            "toolCall": {"name": "view_file", "args": {"AbsolutePath": absolute}},
            "stepIdx": 1
        }),
        _ => unreachable!(),
    }
}

/// Builds a Claude Code `PreSkillShellCommand` payload: an injected command
/// that a loading skill is about to run through `tool_name`.
fn injected_command(cwd: &Path, tool_name: &str, command: &str) -> serde_json::Value {
    serde_json::json!({
        "session_id": "session",
        "transcript_path": "/tmp/transcript.jsonl",
        "cwd": cwd.to_str().unwrap(),
        "permission_mode": "default",
        "hook_event_name": "PreSkillShellCommand",
        "tool_name": tool_name,
        "tool_input": {"command": command},
        "tool_use_id": "skillsh_1",
        "command_name": "release-notes",
        "invoked_by": "user"
    })
}

/// Runs the guard with `args`, feeding `stdin`. On Claude Code the baseline
/// environment names `project_dir` as `CLAUDE_PROJECT_DIR` and the payload's
/// `session_id` (default `session`) as the session. `home` replaces `$HOME`
/// so default discovery never reads the developer's own policy.
fn run_with(
    args: &[String],
    harness: &str,
    project_dir: &Path,
    home: &Path,
    stdin: &[u8],
) -> std::process::Output {
    let binary = env!("CARGO_BIN_EXE_forbidden-file-guard");
    let mut command = Command::new(binary);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    clear_hook_environment(&mut command);
    command.env("HOME", home);
    if harness == "claude" {
        let session = serde_json::from_slice::<serde_json::Value>(stdin)
            .ok()
            .and_then(|payload| payload["session_id"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "session".to_owned());
        command
            .env("CLAUDECODE", "1")
            .env("CLAUDE_CODE_CHILD_SESSION", "1")
            .env("CLAUDE_CODE_SESSION_ID", session)
            .env("CLAUDE_PROJECT_DIR", project_dir);
    }
    let mut child = command.spawn().unwrap();
    if let Err(error) = child.stdin.take().unwrap().write_all(stdin) {
        // Argument errors can exit before reading stdin. Still collect the
        // native response so callers verify its status, stdout, and stderr.
        assert_eq!(
            error.kind(),
            ErrorKind::BrokenPipe,
            "writing guard stdin: {error}"
        );
    }
    child.wait_with_output().unwrap()
}

fn run(harness: &str, config: &Path, input: &serde_json::Value) -> std::process::Output {
    let cwd = input
        .get("cwd")
        .or_else(|| input.get("workspacePaths").and_then(|roots| roots.get(0)))
        .and_then(serde_json::Value::as_str)
        .unwrap();
    run_with(
        &[
            format!("--harness={harness}"),
            format!("--config={}", config.display()),
        ],
        harness,
        Path::new(cwd),
        config.parent().unwrap(),
        &serde_json::to_vec(input).unwrap(),
    )
}

/// The `hookEventName` of a Claude Code structured response.
fn hook_event_name(output: &std::process::Output) -> String {
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    value["hookSpecificOutput"]["hookEventName"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn decision(output: &std::process::Output) -> String {
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    value["decision"]
        .as_str()
        .or_else(|| value["hookSpecificOutput"]["permissionDecision"].as_str())
        .unwrap()
        .to_owned()
}

#[test]
fn every_aligned_harness_denies_matches_and_passes_other_calls_through() {
    let temporary = TempDirectory::new("harnesses");
    let config = temporary.0.join("policy.yaml");
    std::fs::write(
        &config,
        "patterns: ['.env', '**/.env']\naccess_policy: inspect_known\n",
    )
    .unwrap();

    for harness in ["claude", "codex", "antigravity"] {
        let denied = run(harness, &config, &input(harness, &temporary.0, ".env"));
        assert!(denied.status.success(), "{harness}: {:?}", denied.stderr);
        assert_eq!(decision(&denied), "deny", "{harness}");

        // No objection must not auto-approve: Claude Code and Codex keep
        // their normal permission flow, and Antigravity, which requires a
        // decision, gets the least-privilege "ask".
        let passed = run(harness, &config, &input(harness, &temporary.0, "notes.txt"));
        assert!(passed.status.success(), "{harness}: {:?}", passed.stderr);
        match harness {
            "claude" => assert_eq!(passed.stdout, b"{}"),
            "codex" => assert!(passed.stdout.is_empty(), "{harness}"),
            _ => assert_eq!(passed.stdout, br#"{"decision":"ask"}"#),
        }
    }
}

#[test]
fn claude_injected_commands_are_denied_on_match_and_otherwise_pass_through() {
    // PreToolUse does not fire for a skill's injected commands, so the same
    // registration also guards PreSkillShellCommand.
    let temporary = TempDirectory::new("injected");
    let config = temporary.0.join("policy.yaml");
    std::fs::write(
        &config,
        "patterns: ['.env', '**/.env']\naccess_policy: inspect_known\n",
    )
    .unwrap();

    let denied = run(
        "claude",
        &config,
        &injected_command(&temporary.0, "Bash", "cat .env"),
    );
    assert!(denied.status.success(), "{:?}", denied.stderr);
    assert_eq!(hook_event_name(&denied), "PreSkillShellCommand");
    assert_eq!(decision(&denied), "deny");
    let value: serde_json::Value = serde_json::from_slice(&denied.stdout).unwrap();
    assert!(
        value["hookSpecificOutput"]["permissionDecisionReason"]
            .as_str()
            .unwrap()
            .contains(".env"),
        "{value}"
    );

    // No objection is the event's no-op: an injected command cannot be
    // approved, so it runs unless another hook blocks it.
    for (tool, command) in [
        ("Bash", "cat notes.txt"),
        ("PowerShell", "Get-Content notes.txt"),
    ] {
        let passed = run(
            "claude",
            &config,
            &injected_command(&temporary.0, tool, command),
        );
        assert!(passed.status.success(), "{tool}: {:?}", passed.stderr);
        assert_eq!(passed.stdout, b"{}", "{tool}");
    }

    // Every injected command is a shell command, PowerShell included.
    std::fs::write(&config, "access_policy: deny_all_shell\n").unwrap();
    for tool in ["Bash", "PowerShell"] {
        let denied = run(
            "claude",
            &config,
            &injected_command(&temporary.0, tool, "echo safe"),
        );
        assert!(denied.status.success(), "{tool}: {:?}", denied.stderr);
        assert_eq!(decision(&denied), "deny", "{tool}");
    }
}

#[test]
fn snapshot_pre_skill_shell_command_fixtures_follow_the_shell_posture() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../contracts/harnesses/claude-code/snapshots")
        .join(hookkit_claude::SNAPSHOT.as_str())
        .join("events/pre-skill-shell-command/fixtures.yaml");
    let document: serde_json::Value =
        serde_yaml_ng::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let positives = document["input"]["positive"].as_array().unwrap();
    assert!(!positives.is_empty(), "{}", path.display());

    let temporary = TempDirectory::new("injected-fixtures");
    let config = temporary.0.join("policy.yaml");
    for (policy, expected) in [
        (
            "patterns: ['secrets/**']\naccess_policy: inspect_known\n",
            None,
        ),
        ("access_policy: deny_all_shell\n", Some("deny")),
    ] {
        std::fs::write(&config, policy).unwrap();
        for fixture in positives {
            let id = fixture["id"].as_str().unwrap();
            let output = run("claude", &config, &fixture["value"]);
            assert!(output.status.success(), "{id}: {:?}", output.stderr);
            match expected {
                Some(expected) => {
                    assert_eq!(hook_event_name(&output), "PreSkillShellCommand", "{id}");
                    assert_eq!(decision(&output), expected, "{id}");
                }
                None => assert_eq!(output.stdout, b"{}", "{id}"),
            }
        }
    }
}

#[test]
fn claude_injected_command_policy_applies_after_the_agent_changes_directory() {
    let temporary = TempDirectory::new("injected-cd");
    let project = temporary.0.join("project");
    let home = temporary.0.join("home");
    std::fs::create_dir_all(project.join(".agent-hook-kit")).unwrap();
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::create_dir_all(project.join("secrets")).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        project.join(".agent-hook-kit/forbidden-files.yaml"),
        "patterns: ['secrets/**']\n",
    )
    .unwrap();
    let token = project.join("secrets/token.txt");
    std::fs::write(&token, "token").unwrap();

    // The injected command runs in the session shell's current directory,
    // which the hook `cwd` reports; CLAUDE_PROJECT_DIR still names the
    // project root.
    let elsewhere = temporary.0.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    for (cwd, command) in [
        (project.join("src"), "cat ../secrets/token.txt".to_owned()),
        (elsewhere, format!("cat {}", token.display())),
    ] {
        let payload = injected_command(&cwd, "Bash", &command);
        let output = run_with(
            &["--harness=claude".to_owned()],
            "claude",
            &project,
            &home,
            &serde_json::to_vec(&payload).unwrap(),
        );
        assert!(output.status.success(), "{:?}", output.stderr);
        assert_eq!(decision(&output), "deny", "cwd {}", cwd.display());
    }
}

#[test]
fn claude_discovers_project_policy_after_the_agent_changes_directory() {
    let temporary = TempDirectory::new("claude-cd");
    let project = temporary.0.join("project");
    let home = temporary.0.join("home");
    std::fs::create_dir_all(project.join(".agent-hook-kit")).unwrap();
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::create_dir_all(project.join("secrets")).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        project.join(".agent-hook-kit/forbidden-files.yaml"),
        "patterns: ['secrets/**']\n",
    )
    .unwrap();
    let token = project.join("secrets/token.txt");
    std::fs::write(&token, "token").unwrap();

    // After `cd src` (or `cd /tmp`), Claude Code reports the agent's new
    // directory as `cwd`; CLAUDE_PROJECT_DIR still names the project root.
    for cwd in [project.join("src"), temporary.0.join("elsewhere")] {
        std::fs::create_dir_all(&cwd).unwrap();
        let mut payload = input("claude", &cwd, "unused");
        payload["tool_input"] = serde_json::json!({"file_path": token.to_str().unwrap()});
        let output = run_with(
            &["--harness=claude".to_owned()],
            "claude",
            &project,
            &home,
            &serde_json::to_vec(&payload).unwrap(),
        );
        assert!(output.status.success(), "{:?}", output.stderr);
        assert_eq!(decision(&output), "deny", "cwd {}", cwd.display());
    }
}

#[test]
fn claude_worktree_policy_still_applies_after_cd_inside_the_worktree() {
    let temporary = TempDirectory::new("claude-worktree");
    let project = temporary.0.join("project");
    let worktree = project.join(".claude/worktrees/feat");
    let home = temporary.0.join("home");
    std::fs::create_dir_all(project.join(".agent-hook-kit")).unwrap();
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::create_dir_all(worktree.join("src")).unwrap();
    std::fs::create_dir_all(worktree.join("secrets")).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        project.join(".agent-hook-kit/forbidden-files.yaml"),
        "patterns: ['secrets/**']\n",
    )
    .unwrap();
    // A linked worktree's `.git` is a file naming the main repository.
    std::fs::write(
        worktree.join(".git"),
        "gitdir: ../../../.git/worktrees/feat\n",
    )
    .unwrap();
    let token = worktree.join("secrets/token.txt");
    std::fs::write(&token, "token").unwrap();

    // CLAUDE_PROJECT_DIR stays at the main checkout while `cwd` moves into
    // the worktree and then below its root.
    for cwd in [worktree.clone(), worktree.join("src")] {
        let mut read = input("claude", &cwd, "unused");
        read["tool_input"] = serde_json::json!({"file_path": token.to_str().unwrap()});
        let mut shell = input("claude", &cwd, "unused");
        shell["tool_name"] = serde_json::json!("Bash");
        shell["tool_input"] = serde_json::json!({"command": "cat ../secrets/token.txt"});
        for payload in [read, shell] {
            let output = run_with(
                &["--harness=claude".to_owned()],
                "claude",
                &project,
                &home,
                &serde_json::to_vec(&payload).unwrap(),
            );
            assert!(output.status.success(), "{:?}", output.stderr);
            if cwd == worktree && payload["tool_name"] == "Bash" {
                // From the worktree root, `../secrets` is outside it.
                continue;
            }
            assert_eq!(
                decision(&output),
                "deny",
                "cwd {}: {payload}",
                cwd.display()
            );
        }
    }
}

#[test]
fn argument_errors_block_the_call() {
    let temporary = TempDirectory::new("usage");
    let config = temporary.0.join("policy.yaml");
    std::fs::write(&config, "patterns: ['.env']\n").unwrap();
    // Exceed the stdin pipe buffer so argument rejection exercises the child
    // closing stdin before the parent finishes writing, regardless of scheduling.
    let mut stdin = b"{}".to_vec();
    stdin.resize(1024 * 1024, b' ');
    // Clap's own usage status would be 2 as well; exiting 1 instead, a
    // non-blocking hook error, would let every call through on a typo.
    for args in [
        vec!["--harness=claud".to_owned()],
        vec![],
        vec!["--harness=codex".to_owned(), "--bogus".to_owned()],
        vec![
            "--harness=claude".to_owned(),
            "--confg".to_owned(),
            "x".to_owned(),
        ],
    ] {
        let output = run_with(&args, "codex", &temporary.0, &temporary.0, &stdin);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(stderr.lines().count(), 1, "{args:?}: {stderr:?}");
        assert!(
            stderr.starts_with("hookkit: forbidden-file-guard ")
                && stderr.contains("failed: invalid arguments: error: "),
            "{args:?}: {stderr:?}"
        );
    }

    // Antigravity reads a decision rather than an exit status.
    let output = run_with(
        &["--harness=antigravity".to_owned(), "--bogus".to_owned()],
        "antigravity",
        &temporary.0,
        &temporary.0,
        &stdin,
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(decision(&output), "deny");

    let help = run_with(
        &["--help".to_owned()],
        "codex",
        &temporary.0,
        &temporary.0,
        b"",
    );
    assert_eq!(help.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&help.stdout).contains("--harness"));

    // The canonical HookKit identity is accepted as an alias.
    let payload = input("claude", &temporary.0, ".env");
    let output = run_with(
        &[
            "--harness=claude-code".to_owned(),
            format!("--config={}", config.display()),
        ],
        "claude",
        &temporary.0,
        &temporary.0,
        &serde_json::to_vec(&payload).unwrap(),
    );
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(decision(&output), "deny");
}

#[test]
fn unreadable_invocations_fail_closed_with_each_native_block() {
    let temporary = TempDirectory::new("fail-closed");
    let config = temporary.0.join("policy.yaml");
    std::fs::write(&config, "patterns: ['.env']\n").unwrap();
    for harness in ["claude", "codex", "antigravity"] {
        let output = run_with(
            &[
                format!("--harness={harness}"),
                format!("--config={}", config.display()),
            ],
            harness,
            &temporary.0,
            &temporary.0,
            b"not json",
        );
        if harness == "antigravity" {
            assert_eq!(output.status.code(), Some(0));
            assert_eq!(decision(&output), "deny");
        } else {
            // Exit 2 with a reason blocks the call on Claude Code and Codex.
            assert_eq!(output.status.code(), Some(2), "{harness}");
            assert!(output.stdout.is_empty(), "{harness}");
            // On Claude Code the guard serves PreToolUse and
            // PreSkillShellCommand, and an unreadable payload names neither.
            let hook = match harness {
                "claude" => "claude-code failed",
                _ => "PreToolUse failed",
            };
            assert!(
                String::from_utf8_lossy(&output.stderr).contains(hook),
                "{harness}: {:?}",
                output.stderr
            );
        }
    }

    // An injected command the exact parser rejects still blocks.
    let mut malformed = injected_command(&temporary.0, "Bash", "cat .env");
    malformed["tool_input"] = serde_json::json!({});
    let output = run("claude", &config, &malformed);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("claude-code/PreSkillShellCommand failed"),
        "{:?}",
        output.stderr
    );
}

#[test]
fn claude_events_the_guard_does_not_guard_are_not_blocked() {
    // A registration under an event that gates nothing reports a
    // non-blocking error instead of guessing a response.
    let temporary = TempDirectory::new("other-event");
    let config = temporary.0.join("policy.yaml");
    std::fs::write(&config, "patterns: ['.env']\n").unwrap();
    let mut payload = input("claude", &temporary.0, ".env");
    payload["hook_event_name"] = serde_json::json!("PostToolUseFailure");
    payload["error"] = serde_json::json!("failed");
    let output = run("claude", &config, &payload);
    assert_eq!(output.status.code(), Some(1), "{:?}", output.stderr);
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("guards Claude Code PreToolUse and PreSkillShellCommand"),
        "{:?}",
        output.stderr
    );
}

#[test]
fn codex_shell_heredoc_patch_is_denied_through_stdin() {
    let temporary = TempDirectory::new("heredoc");
    let config = temporary.0.join("policy.yaml");
    std::fs::write(&config, "patterns: ['secrets/**']\n").unwrap();
    let mut input = input("codex", &temporary.0, "unused");
    input["tool_name"] = serde_json::json!("Bash");
    input["tool_input"] = serde_json::json!({
        "command": "apply_patch <<'PATCH'\n*** Add File: secrets/token.txt\n+token\nPATCH\n"
    });

    let output = run("codex", &config, &input);
    assert!(output.status.success());
    assert_eq!(decision(&output), "deny");
}

#[test]
fn checked_in_antigravity_fixture_is_denied() {
    let temporary = TempDirectory::new("antigravity-fixture");
    let config = temporary.0.join("policy.yaml");
    std::fs::write(&config, "patterns: ['**/.env']\n").unwrap();
    let fixture = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/antigravity_pre_tool_use.json"),
    )
    .unwrap();
    let input: serde_json::Value = serde_json::from_slice(&fixture).unwrap();
    assert_eq!(input["toolCall"]["name"], "view_file");

    let output = run("antigravity", &config, &input);
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(decision(&output), "deny");
}

#[test]
fn checked_in_claude_pre_skill_shell_command_fixture_is_denied() {
    let temporary = TempDirectory::new("claude-fixture");
    let config = temporary.0.join("policy.yaml");
    std::fs::write(&config, "patterns: ['**/.env']\n").unwrap();
    let fixture = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/claude_pre_skill_shell_command.json"),
    )
    .unwrap();
    let input: serde_json::Value = serde_json::from_slice(&fixture).unwrap();
    assert_eq!(input["hook_event_name"], "PreSkillShellCommand");

    let output = run("claude", &config, &input);
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(hook_event_name(&output), "PreSkillShellCommand");
    assert_eq!(decision(&output), "deny");
}

fn clear_hook_environment(command: &mut Command) {
    for name in [
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
    ] {
        command.env_remove(name);
    }
}
