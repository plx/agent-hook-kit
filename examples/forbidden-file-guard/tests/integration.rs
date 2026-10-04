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

/// Runs the guard with `args`, feeding `stdin`. On Claude Code the baseline
/// environment names `project_dir` as `CLAUDE_PROJECT_DIR`. `home` replaces
/// `$HOME` so default discovery never reads the developer's own policy.
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
        command
            .env("CLAUDECODE", "1")
            .env("CLAUDE_CODE_CHILD_SESSION", "1")
            .env("CLAUDE_CODE_SESSION_ID", "session")
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
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("PreToolUse failed"),
                "{harness}"
            );
        }
    }
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
