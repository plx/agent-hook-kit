use clap::{Parser, ValueEnum};
use hookkit_common::{PostToolUseInput, PostToolUseOutput};
use hookkit_core::{HarnessId, RuntimeContext};
use hookkit_session_state::{
    EntityId, EntityMode, FamilyId, ModifiedFileEvent, ModifiedFiles, SessionState, StateRoot,
};
use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Harness {
    Claude,
    Codex,
    Gemini,
}

impl Harness {
    fn id(self) -> HarnessId {
        match self {
            Self::Claude => HarnessId::CLAUDE_CODE,
            Self::Codex => HarnessId::CODEX,
            Self::Gemini => HarnessId::GEMINI_CLI,
        }
    }
}

#[derive(Debug, Parser)]
#[command(about = "Accumulate files that appear modified during one agent session")]
struct Cli {
    /// Hook harness whose native post-tool contract should be used.
    #[arg(long, value_enum)]
    harness: Harness,

    /// Override the root directory for per-session tracker state.
    #[arg(long)]
    state_dir: Option<PathBuf>,
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    hookkit_runtime::aligned::run_aligned_event::<hookkit_runtime::aligned::PostToolUse, _>(
        cli.harness.id(),
        move |input, _environment, context| {
            handle_post_tool(input, context, cli.state_dir.as_deref())
        },
    )
}

fn handle_post_tool(
    input: PostToolUseInput,
    context: &RuntimeContext<'_>,
    state_dir: Option<&Path>,
) -> hookkit_core::Result<PostToolUseOutput> {
    let (tool_name, tool_input) = match &input {
        PostToolUseInput::Claude(input) => (input.tool_name.as_str(), &input.tool_input),
        PostToolUseInput::Codex(input) => (input.tool_name.as_str(), &input.tool_input),
        PostToolUseInput::Gemini(input) => {
            let value = serde_json::Value::Object(input.tool_input.clone());
            let paths = modified_paths(&input.tool_name, &value, first_workspace_root(context)?);
            record_paths(context, state_dir, &paths)?;
            return native_no_op(context.harness());
        }
        PostToolUseInput::Antigravity(_) => {
            return Err(std::io::Error::new(
                ErrorKind::Unsupported,
                "Antigravity PostToolUse omits the tool call, so modified paths cannot be tracked",
            )
            .into());
        }
        _ => {
            return Err(std::io::Error::new(
                ErrorKind::Unsupported,
                "unknown aligned PostToolUse input arm",
            )
            .into());
        }
    };

    let paths = modified_paths(tool_name, tool_input, first_workspace_root(context)?);
    record_paths(context, state_dir, &paths)?;
    native_no_op(context.harness())
}

fn first_workspace_root<'a>(context: &'a RuntimeContext<'_>) -> hookkit_core::Result<&'a Path> {
    context
        .workspace_roots()
        .first()
        .map(|path| path.as_std_path())
        .ok_or_else(|| std::io::Error::other("missing workspace root").into())
}

fn modified_paths(tool_name: &str, input: &serde_json::Value, cwd: &Path) -> Vec<PathBuf> {
    let mut raw = Vec::new();
    if is_structured_writer(tool_name) {
        collect_path_fields(input, None, &mut raw);
    }
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
        collect_shell_modified_paths(command, &mut raw);
    }

    let mut seen = BTreeSet::new();
    raw.into_iter()
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
        .map(|path| {
            normalize_path(if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            })
        })
        .filter(|path| seen.insert(slash_path(path)))
        .collect()
}

fn is_structured_writer(tool_name: &str) -> bool {
    let name = tool_name.to_ascii_lowercase();
    [
        "write", "edit", "replace", "patch", "save", "create", "delete", "remove", "move", "rename",
    ]
    .iter()
    .any(|verb| name.contains(verb))
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

fn collect_shell_modified_paths(command: &str, out: &mut Vec<String>) {
    let tokens = shell_tokens(command);
    let tokens = tokens
        .iter()
        .map(|token| token.trim_matches(shell_punctuation))
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();

    for (index, token) in tokens.iter().enumerate() {
        if !matches!(*token, ">" | ">>") {
            continue;
        }
        let Some(path) = tokens
            .get(index + 1)
            .filter(|path| !is_shell_operator(path))
        else {
            continue;
        };
        out.push((*path).to_string());
    }

    for segment in tokens.split(|token| matches!(*token, ";" | "&&" | "||" | "|")) {
        collect_mutating_command_segment(segment, out);
    }
}

fn shell_tokens(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut characters = command.chars().peekable();

    while let Some(character) = characters.next() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }

        if let Some(delimiter) = quote {
            match character {
                character if character == delimiter => quote = None,
                '\\' if delimiter != '\'' => escaped = true,
                _ => current.push(character),
            }
            continue;
        }

        match character {
            '\\' => escaped = true,
            '\'' | '"' | '`' => quote = Some(character),
            '\n' | '\r' => {
                push_shell_word(&mut tokens, &mut current);
                tokens.push(";".to_string());
            }
            character if character.is_whitespace() => {
                push_shell_word(&mut tokens, &mut current);
            }
            ';' | '|' | '&' | '<' | '>' => {
                push_shell_word(&mut tokens, &mut current);
                let mut operator = character.to_string();
                if matches!(character, '|' | '&' | '<' | '>')
                    && characters.peek() == Some(&character)
                {
                    operator.push(characters.next().expect("peeked character must exist"));
                }
                tokens.push(operator);
            }
            _ => current.push(character),
        }
    }

    if escaped {
        current.push('\\');
    }
    push_shell_word(&mut tokens, &mut current);
    tokens
}

fn push_shell_word(tokens: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        tokens.push(std::mem::take(current));
    }
}

fn is_shell_operator(token: &&str) -> bool {
    matches!(
        *token,
        ";" | "|" | "||" | "&" | "&&" | "<" | "<<" | ">" | ">>"
    )
}

fn collect_mutating_command_segment(segment: &[&str], out: &mut Vec<String>) {
    let Some((program, args)) = segment.split_first() else {
        return;
    };
    let program = Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(program);
    let candidates = args
        .iter()
        .copied()
        .filter(|arg| !arg.starts_with('-') && !arg.starts_with('>'))
        .collect::<Vec<_>>();

    match program {
        "touch" | "rm" | "unlink" | "truncate" | "mv" => {
            out.extend(candidates.into_iter().map(str::to_owned));
        }
        "cp" | "install" => {
            if let Some(destination) = candidates.last() {
                out.push((*destination).to_string());
            }
        }
        "tee" => out.extend(candidates.into_iter().map(str::to_owned)),
        "sed" if args.iter().any(|arg| arg.starts_with("-i")) => {
            if let Some(path) = candidates.iter().rev().find(|path| looks_like_path(path)) {
                out.push((*path).to_string());
            }
        }
        _ => {}
    }
}

fn shell_punctuation(character: char) -> bool {
    "'\"`(){}[]".contains(character)
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

fn record_paths(
    context: &RuntimeContext<'_>,
    state_dir: Option<&Path>,
    paths: &[PathBuf],
) -> hookkit_core::Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let state_root = state_dir.map(StateRoot::new).unwrap_or_default();
    let journal = SessionState::ensure(context, state_root)
        .and_then(|state| state.family(FamilyId::new("agent-hook-kit.modified-files", 1)?))
        .and_then(|family| family.session_scope())
        .and_then(|scope| {
            scope.entity::<ModifiedFiles>(EntityId::new("dirty-files", 1)?, EntityMode::Windowed)
        })
        .map_err(state_error)?;
    let invocation = String::from_utf8_lossy(context.raw().bytes());

    for path in paths {
        let path = slash_path(path);
        let observation = ModifiedFileEvent {
            path: hookkit_core::Utf8PathBuf::from(path.clone()),
            event: Some(context.event().name().to_string()),
            tool_call_id: context.tool_call_id().map(ToString::to_string),
        };
        journal
            .append(&format!("{invocation}\0{path}"), &observation)
            .map_err(state_error)?;
    }
    Ok(())
}

fn native_no_op(harness: &HarnessId) -> hookkit_core::Result<PostToolUseOutput> {
    match harness.as_str() {
        "claude-code" => Ok(PostToolUseOutput::Claude(
            hookkit_claude::protocol::PostToolUseOutput::no_op(),
        )),
        "codex" => Ok(PostToolUseOutput::Codex(
            hookkit_codex::protocol::PostToolUseOutput::no_op(),
        )),
        "gemini-cli" => Ok(PostToolUseOutput::Gemini(
            hookkit_gemini::protocol::AfterToolOutput::no_op(),
        )),
        _ => Err(std::io::Error::new(
            ErrorKind::Unsupported,
            format!("unsupported tracker harness {harness}"),
        )
        .into()),
    }
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
    path.to_string_lossy().replace('\\', "/")
}

fn state_error(error: hookkit_session_state::StateError) -> std::io::Error {
    std::io::Error::other(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hookkit_core::{
        ContractId, DISABLED_DIAGNOSTICS, EventId, NativeContext, RawInvocation,
        ResolutionProvenance, SnapshotId,
    };
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
    fn distinguishes_readers_from_structured_writers() {
        assert!(
            modified_paths(
                "read_file",
                &serde_json::json!({"path": "src/lib.rs"}),
                Path::new("/repo")
            )
            .is_empty()
        );
        assert_eq!(
            modified_paths(
                "write_file",
                &serde_json::json!({"path": "src/lib.rs"}),
                Path::new("/repo")
            ),
            vec![PathBuf::from("/repo/src/lib.rs")]
        );
    }

    #[test]
    fn finds_patch_and_direct_shell_modifications() {
        assert_eq!(
            modified_paths(
                "apply_patch",
                &serde_json::json!({"patch": "*** Update File: src/lib.rs\n"}),
                Path::new("/repo")
            ),
            vec![PathBuf::from("/repo/src/lib.rs")]
        );

        let paths = modified_paths(
            "Bash",
            &serde_json::json!({"command": "touch tmp/new.txt Makefile && rm old.txt"}),
            Path::new("/repo"),
        );
        assert_eq!(
            paths.into_iter().collect::<BTreeSet<_>>(),
            BTreeSet::from([
                PathBuf::from("/repo/Makefile"),
                PathBuf::from("/repo/old.txt"),
                PathBuf::from("/repo/tmp/new.txt")
            ])
        );

        let paths = modified_paths(
            "Bash",
            &serde_json::json!({"command": "echo hi > out.txt; touch src/new.rs"}),
            Path::new("/repo"),
        );
        assert_eq!(
            paths.into_iter().collect::<BTreeSet<_>>(),
            BTreeSet::from([
                PathBuf::from("/repo/out.txt"),
                PathBuf::from("/repo/src/new.rs")
            ])
        );
    }

    #[test]
    fn aggregates_repeated_observations_into_a_file_set() {
        let directory = test_dir("modified-state");
        let invocation =
            RawInvocation::parse(serde_json::to_vec(&serde_json::json!({})).unwrap()).unwrap();
        let context = RuntimeContext::new(
            HarnessId::CODEX,
            SnapshotId::builtin("test"),
            EventId::builtin(HarnessId::CODEX, "PostToolUse"),
            ContractId::builtin("test"),
            ResolutionProvenance::TypedStatic,
            &invocation,
            NativeContext {
                session_id: hookkit_core::SessionId::new("session-1").ok(),
                workspace_roots: vec![hookkit_core::Utf8PathBuf::from("/repo")],
                ..NativeContext::default()
            },
            &DISABLED_DIAGNOSTICS,
        )
        .unwrap();
        let paths = vec![PathBuf::from("/repo/a.rs"), PathBuf::from("/repo/b.rs")];
        record_paths(&context, Some(&directory), &paths).unwrap();
        record_paths(&context, Some(&directory), &paths).unwrap();

        let journal = SessionState::open(
            HarnessId::CODEX,
            hookkit_session_state::SessionIdentity::Session("session-1".into()),
            StateRoot::new(&directory),
        )
        .and_then(|state| state.family(FamilyId::new("agent-hook-kit.modified-files", 1)?))
        .and_then(|family| family.session_scope())
        .and_then(|scope| {
            scope.entity::<ModifiedFiles>(EntityId::new("dirty-files", 1)?, EntityMode::Windowed)
        })
        .unwrap();
        journal
            .with_entity(|view| {
                assert_eq!(view.state().paths().len(), 2);
                assert_eq!(
                    view.state()
                        .paths()
                        .iter()
                        .map(|path| path.as_str())
                        .collect::<BTreeSet<_>>(),
                    BTreeSet::from(["/repo/a.rs", "/repo/b.rs"])
                );
                Ok(hookkit_session_state::EntityOutcome::retain(()))
            })
            .unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }
}
