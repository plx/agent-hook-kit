use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "hookkit-rules-integration-{}-{}",
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

fn run(
    project: &Path,
    claude_home: &Path,
    state: &Path,
    input: &serde_json::Value,
) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_codex-claude-rules"))
        .args([
            format!("--project-root={}", project.display()),
            format!("--claude-home={}", claude_home.display()),
            format!("--state-dir={}", state.display()),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(input).unwrap())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn codex_stdin_injects_once_then_emits_native_no_op() {
    let temporary = TempDirectory::new();
    let project = temporary.0.join("project");
    let claude_home = temporary.0.join("empty-claude-home");
    let state = temporary.0.join("state");
    std::fs::create_dir_all(project.join(".claude/rules")).unwrap();
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::create_dir_all(&claude_home).unwrap();
    std::fs::write(
        project.join(".claude/rules/rust.md"),
        "---\npaths: 'src/**/*.rs'\n---\nUse the Rust rule.\n",
    )
    .unwrap();
    std::fs::write(project.join("src/lib.rs"), "pub fn example() {}\n").unwrap();
    let input = serde_json::json!({
        "session_id": "rules-session",
        "transcript_path": null,
        "cwd": project.to_str().unwrap(),
        "hook_event_name": "PreToolUse",
        "model": "gpt-test",
        "turn_id": "turn",
        "permission_mode": "default",
        "tool_name": "Bash",
        "tool_use_id": "call",
        "tool_input": {"command": "rg needle src"}
    });

    let first = run(&project, &claude_home, &state, &input);
    assert!(first.status.success(), "{:?}", first.stderr);
    let value: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert!(
        value["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("Use the Rust rule.")
    );

    let repeated = run(&project, &claude_home, &state, &input);
    assert!(repeated.status.success(), "{:?}", repeated.stderr);
    assert!(repeated.stdout.is_empty());
}
