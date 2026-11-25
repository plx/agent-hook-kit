//! Native harness adapters. Tool names and field locations are intentionally
//! exact; aliases belong in an explicit [`ShellToolProfile`](crate::ShellToolProfile).

use hookkit_core::{EventId, HarnessId};

#[cfg(feature = "claude")]
use crate::call::ShellToolCallError;
use crate::call::{ShellToolCallExt, ShellToolCallMatch, ShellToolProfile, ToolPhase};

#[cfg(feature = "claude")]
/// Exact Claude Code `Bash` tool profile.
///
/// The hook event `cwd` is the command's working directory. The dialect is
/// [`ShellDialect::Unknown`](crate::ShellDialect::Unknown) because the Bash
/// tool runs the user's shell (zsh by default on macOS).
///
/// In a Code Mode session (Claude Code 2.1.323 or later) a shell command that
/// Claude's code runs can arrive prefixed with `cd '<dir>' && `. The analyzer
/// does not resolve relative operands after such a `cd` against the hook
/// `cwd`; it reports them as unresolved
/// ([`UnresolvedFileAccessReason::WorkingDirectoryMayHaveChanged`](crate::UnresolvedFileAccessReason::WorkingDirectoryMayHaveChanged)). A Bash matcher also fires for shell commands Claude
/// starts with the Monitor tool (Claude Code v2.1.314 or later), delivered as
/// a Bash-shaped call with no matching `PostToolUse`; see
/// [`CLAUDE_MONITOR_PROFILE`] for the Monitor tool's own calls.
///
/// The same profile matches the injected command a `PreSkillShellCommand`
/// hook receives (Claude Code v2.1.332 or later) when its `tool_name` is
/// `Bash`: a shell command that a loading skill runs through the Bash tool,
/// in the same working directory, without firing `PreToolUse`.
pub const CLAUDE_BASH_PROFILE: ShellToolProfile =
    ShellToolProfile::builtin("Bash", "/command", None, true);
#[cfg(feature = "claude")]
/// Exact Claude Code `Monitor` tool profile (Claude Code v2.1.314 or later).
///
/// A Monitor call watches the output of a shell command it starts
/// (`tool_input.command`, run in the user's shell like `Bash`) or a
/// WebSocket (`tool_input.ws`). Only the command form is a shell call; a
/// WebSocket watch reports [`ShellToolCallMatch::NotShell`]. A hook that
/// rewrites a Monitor call must keep its `description` and `timeout_ms`,
/// because `updatedInput` replaces the whole input.
pub const CLAUDE_MONITOR_PROFILE: ShellToolProfile =
    ShellToolProfile::builtin("Monitor", "/command", None, true);
#[cfg(feature = "codex")]
/// Exact Codex `Bash` tool profile.
///
/// Codex's `exec_command` accepts a `workdir` argument that it joins onto the
/// step's environment cwd, and its hook payloads carry only `command`. Since
/// Codex 0.166.0 the hook `cwd` of a local command is that resolved workdir,
/// but a command in a remote execution environment still reports the host's
/// step directory, and the payload does not say which case applies, so
/// matched calls report
/// [`ShellCwdOrigin::UnverifiedFallback`](crate::ShellCwdOrigin::UnverifiedFallback).
/// A `/workdir` value is resolved against the hook `cwd`; real Codex payloads
/// never carry one, and if one appeared alongside a 0.166.0 workdir `cwd` it
/// would be applied twice. The dialect is unknown: Codex runs the user's
/// shell, which is zsh by default on macOS and PowerShell on Windows.
pub const CODEX_BASH_PROFILE: ShellToolProfile =
    ShellToolProfile::builtin("Bash", "/command", Some("/workdir"), false);
#[cfg(feature = "antigravity")]
/// Exact Antigravity `run_command` tool profile, including its optional cwd.
pub const ANTIGRAVITY_RUN_COMMAND_PROFILE: ShellToolProfile =
    ShellToolProfile::builtin("run_command", "/CommandLine", Some("/Cwd"), true);

/// The Claude Code profile for `tool_name`: `Bash`, or `Monitor` when it runs
/// a command. A Monitor WebSocket watch (`ws`, no `command`) runs no shell
/// command.
#[cfg(feature = "claude")]
fn claude_shell_profile(
    tool_name: &str,
    tool_input: Option<&serde_json::Value>,
) -> Option<ShellToolProfile> {
    if tool_name == CLAUDE_BASH_PROFILE.tool_name() {
        Some(CLAUDE_BASH_PROFILE)
    } else if tool_name == CLAUDE_MONITOR_PROFILE.tool_name()
        && tool_input.is_some_and(|input| input.get("command").is_some())
    {
        Some(CLAUDE_MONITOR_PROFILE)
    } else {
        None
    }
}

/// The Claude Code event that runs before an injected command, a shell
/// command that a loading skill is about to run (Claude Code v2.1.332 or
/// later).
#[cfg(feature = "claude")]
const PRE_SKILL_SHELL_COMMAND: &str = "PreSkillShellCommand";

/// Claude Code events whose input carries a shell command in `tool_name` and
/// `tool_input`, with their phase and the field that carries the response.
#[cfg(feature = "claude")]
fn claude_tool_event(name: &str) -> Option<(&'static str, ToolPhase, Option<&'static str>)> {
    match name {
        "PreToolUse" => Some(("PreToolUse", ToolPhase::Pre, None)),
        // The permission prompt precedes execution.
        "PermissionRequest" => Some(("PermissionRequest", ToolPhase::Pre, None)),
        // Auto mode denied the call, so it never executed.
        "PermissionDenied" => Some(("PermissionDenied", ToolPhase::Pre, None)),
        "PostToolUse" => Some(("PostToolUse", ToolPhase::Post, Some("tool_response"))),
        "PostToolUseFailure" => Some(("PostToolUseFailure", ToolPhase::Post, None)),
        // The injected command has not run yet. It is not a tool call:
        // `PreToolUse` does not fire for it, no post-tool event follows it,
        // and its `tool_use_id` names no tool call in the transcript.
        PRE_SKILL_SHELL_COMMAND => Some((PRE_SKILL_SHELL_COMMAND, ToolPhase::Pre, None)),
        _ => None,
    }
}

#[cfg(feature = "claude")]
impl ShellToolCallExt for hookkit_claude::catalog::CatalogInput {
    /// Matches the `Bash` tool, and the `Monitor` tool when it runs a
    /// command, on `PreToolUse`, `PermissionRequest`, `PermissionDenied`,
    /// `PostToolUse`, and `PostToolUseFailure`.
    ///
    /// On `PreSkillShellCommand` (Claude Code v2.1.332 or later) it matches
    /// the injected command a loading skill is about to run, as a
    /// [`ToolPhase::Pre`] call, when `tool_name` is `Bash`; `PowerShell`
    /// reports [`ShellToolCallMatch::NotShell`], as a `PowerShell` tool call
    /// does. Claude Code runs the command in the session shell's current
    /// directory, which the hook `cwd` reports. The command is not a tool
    /// call: `PreToolUse` does not fire for it, no post-tool event follows
    /// it, and its `tool_use_id` names no tool call in the transcript, so do
    /// not correlate it with tool events.
    ///
    /// Other events return [`ShellToolCallMatch::UnsupportedEvent`].
    fn shell_tool_call(&self) -> ShellToolCallMatch<'_> {
        let Some((event_name, phase, response_field)) = claude_tool_event(&self.hook_event_name)
        else {
            return ShellToolCallMatch::UnsupportedEvent;
        };
        let Some(tool_name) = self.field("tool_name").and_then(serde_json::Value::as_str) else {
            return ShellToolCallMatch::NotShell;
        };
        let profile = if event_name == PRE_SKILL_SHELL_COMMAND {
            // `tool_name` names the shell tool that would run the injected
            // command, `Bash` or `PowerShell`.
            (tool_name == CLAUDE_BASH_PROFILE.tool_name()).then_some(CLAUDE_BASH_PROFILE)
        } else {
            claude_shell_profile(tool_name, self.field("tool_input"))
        };
        let Some(profile) = profile else {
            return ShellToolCallMatch::NotShell;
        };
        let event = EventId::builtin(HarnessId::CLAUDE_CODE, event_name);
        let Some(tool_input) = self.field("tool_input") else {
            return ShellToolCallMatch::Malformed(ShellToolCallError::missing_command(
                event,
                tool_name,
                profile.command_pointer(),
            ));
        };
        profile.extract_from_value(
            event,
            phase,
            tool_name,
            tool_input,
            Some(self.cwd.as_path()),
            response_field.and_then(|field| self.field(field)),
        )
    }
}

#[cfg(feature = "claude")]
impl ShellToolCallExt for hookkit_claude::protocol::PostToolUseInput {
    /// Matches the `Bash` tool, and the `Monitor` tool when it runs a
    /// command.
    fn shell_tool_call(&self) -> ShellToolCallMatch<'_> {
        let Some(profile) = claude_shell_profile(&self.tool_name, Some(&self.tool_input)) else {
            return ShellToolCallMatch::NotShell;
        };
        profile.extract_from_value(
            EventId::builtin(HarnessId::CLAUDE_CODE, "PostToolUse"),
            ToolPhase::Post,
            &self.tool_name,
            &self.tool_input,
            Some(self.cwd.as_path()),
            Some(&self.tool_response),
        )
    }
}

#[cfg(feature = "codex")]
impl ShellToolCallExt for hookkit_codex::protocol::PreToolUseInput {
    fn shell_tool_call(&self) -> ShellToolCallMatch<'_> {
        CODEX_BASH_PROFILE.extract_from_value(
            EventId::builtin(HarnessId::CODEX, "PreToolUse"),
            ToolPhase::Pre,
            &self.tool_name,
            &self.tool_input,
            Some(self.cwd.as_path()),
            None,
        )
    }
}

#[cfg(feature = "codex")]
impl ShellToolCallExt for hookkit_codex::protocol::PostToolUseInput {
    fn shell_tool_call(&self) -> ShellToolCallMatch<'_> {
        CODEX_BASH_PROFILE.extract_from_value(
            EventId::builtin(HarnessId::CODEX, "PostToolUse"),
            ToolPhase::Post,
            &self.tool_name,
            &self.tool_input,
            Some(self.cwd.as_path()),
            Some(&self.tool_response),
        )
    }
}

#[cfg(feature = "antigravity")]
impl ShellToolCallExt for hookkit_antigravity::PreToolUseInput {
    fn shell_tool_call(&self) -> ShellToolCallMatch<'_> {
        ANTIGRAVITY_RUN_COMMAND_PROFILE.extract_from_object(
            EventId::builtin(HarnessId::ANTIGRAVITY, "PreToolUse"),
            ToolPhase::Pre,
            &self.tool_call.name,
            &self.tool_call.args,
            self.workspace_paths
                .first()
                .map(hookkit_core::Utf8PathBuf::as_path),
            None,
        )
    }
}

#[cfg(feature = "antigravity")]
impl ShellToolCallExt for hookkit_antigravity::PostToolUseInput {
    fn shell_tool_call(&self) -> ShellToolCallMatch<'_> {
        // Payloads without `toolCall` (the IDE reference shape) cannot say
        // whether a shell command ran.
        let Some(tool_call) = &self.tool_call else {
            return ShellToolCallMatch::UnsupportedEvent;
        };
        ANTIGRAVITY_RUN_COMMAND_PROFILE.extract_from_object(
            EventId::builtin(HarnessId::ANTIGRAVITY, "PostToolUse"),
            ToolPhase::Post,
            &tool_call.name,
            &tool_call.args,
            self.workspace_paths
                .first()
                .map(hookkit_core::Utf8PathBuf::as_path),
            None,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(any(feature = "codex", feature = "antigravity"))]
    use crate::call::ShellCwdOrigin;

    #[cfg(feature = "claude")]
    fn claude_catalog(event: &str) -> hookkit_claude::catalog::CatalogInput {
        serde_json::from_value(serde_json::json!({
            "session_id": "session",
            "transcript_path": "/tmp/transcript.jsonl",
            "cwd": "/repo",
            "hook_event_name": event,
            "tool_name": "Bash",
            "tool_input": {"command": "cargo test"},
            "tool_response": {"stdout": "ok"}
        }))
        .unwrap()
    }

    #[cfg(feature = "claude")]
    #[test]
    fn extracts_claude_catalog_pre_tool_use() {
        let input = claude_catalog("PreToolUse");

        let ShellToolCallMatch::Matched(call) = input.shell_tool_call() else {
            panic!("expected a shell call");
        };
        assert_eq!(call.command, "cargo test");
        assert_eq!(call.cwd, Some(hookkit_core::Utf8Path::new("/repo")));
    }

    #[cfg(feature = "claude")]
    #[test]
    fn claude_catalog_matches_every_tool_event_and_flags_others() {
        for (event, phase, has_response) in [
            ("PermissionRequest", ToolPhase::Pre, false),
            ("PermissionDenied", ToolPhase::Pre, false),
            ("PostToolUse", ToolPhase::Post, true),
            ("PostToolUseFailure", ToolPhase::Post, false),
        ] {
            let input = claude_catalog(event);
            let ShellToolCallMatch::Matched(call) = input.shell_tool_call() else {
                panic!("expected a shell call for {event}");
            };
            assert_eq!(call.phase, phase, "{event}");
            assert_eq!(call.event.name(), event);
            assert_eq!(call.response.is_some(), has_response, "{event}");
        }
        assert!(matches!(
            claude_catalog("Stop").shell_tool_call(),
            ShellToolCallMatch::UnsupportedEvent
        ));
    }

    /// One input fixture list (`positive` or `negative`) of the selected
    /// Claude Code snapshot's `PreSkillShellCommand` contract, as
    /// `(id, value)` pairs.
    #[cfg(feature = "claude")]
    fn pre_skill_shell_command_fixtures(kind: &str) -> Vec<(String, serde_json::Value)> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../contracts/harnesses/claude-code/snapshots")
            .join(hookkit_claude::SNAPSHOT.as_str())
            .join("events/pre-skill-shell-command/fixtures.yaml");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let document: serde_json::Value = serde_yaml_ng::from_str(&text)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        document["input"][kind]
            .as_array()
            .unwrap_or_else(|| panic!("{} has no {kind} inputs", path.display()))
            .iter()
            .map(|fixture| {
                (
                    fixture["id"].as_str().unwrap().to_owned(),
                    fixture["value"].clone(),
                )
            })
            .collect()
    }

    #[cfg(feature = "claude")]
    fn parse_pre_skill_shell_command(
        value: &serde_json::Value,
    ) -> hookkit_core::Result<hookkit_claude::catalog::CatalogInput> {
        use hookkit_core::EventSpec;
        let invocation =
            hookkit_core::RawInvocation::parse(serde_json::to_vec(value).unwrap()).unwrap();
        hookkit_claude::catalog::PreSkillShellCommand::parse(&invocation)
    }

    #[cfg(feature = "claude")]
    #[test]
    fn claude_pre_skill_shell_command_positive_fixtures_yield_the_injected_command() {
        let fixtures = pre_skill_shell_command_fixtures("positive");
        assert!(fixtures.len() >= 5, "{fixtures:?}");
        let mut bash = 0;
        for (id, value) in &fixtures {
            let input = parse_pre_skill_shell_command(value)
                .unwrap_or_else(|error| panic!("{id}: {error}"));
            let injected = input.injected_command().unwrap();
            match (input.tool_name(), input.shell_tool_call()) {
                (Some("Bash"), ShellToolCallMatch::Matched(call)) => {
                    bash += 1;
                    assert_eq!(call.command, injected, "{id}");
                    assert_eq!(call.tool_name, "Bash", "{id}");
                    assert_eq!(call.phase, ToolPhase::Pre, "{id}");
                    assert_eq!(call.harness, HarnessId::CLAUDE_CODE, "{id}");
                    assert_eq!(
                        call.event,
                        EventId::builtin(HarnessId::CLAUDE_CODE, "PreSkillShellCommand"),
                        "{id}"
                    );
                    // Claude Code runs the command in the session shell's
                    // current directory, which the hook `cwd` reports.
                    assert_eq!(call.cwd, Some(input.cwd.as_path()), "{id}");
                    assert_eq!(call.cwd_origin, crate::ShellCwdOrigin::Fallback, "{id}");
                    assert_eq!(call.dialect, crate::ShellDialect::Unknown, "{id}");
                    // The command has not run, so there is no response.
                    assert!(call.response.is_none(), "{id}");
                }
                // The Bash analyzer cannot parse PowerShell, so the PowerShell
                // tool matches no bundled profile here either.
                (Some("PowerShell"), ShellToolCallMatch::NotShell) => {}
                (tool, other) => panic!("{id}: tool {tool:?} gave {other:?}"),
            }
        }
        assert!(bash >= 4, "{fixtures:?}");
    }

    #[cfg(feature = "claude")]
    #[test]
    fn claude_pre_skill_shell_command_negatives_check_only_the_shell_fields() {
        // The exact parser rejects every negative input. The adapter reads
        // only the event name and the shell fields, so the negatives that
        // break those report why no command was extracted, and the others
        // (contract fields the adapter does not read) still extract it.
        for (id, value) in pre_skill_shell_command_fixtures("negative") {
            assert!(parse_pre_skill_shell_command(&value).is_err(), "{id}");
            let input: hookkit_claude::catalog::CatalogInput =
                serde_json::from_value(value).unwrap_or_else(|error| panic!("{id}: {error}"));
            let result = input.shell_tool_call();
            match id.as_str() {
                "wrong-discriminator" => {
                    assert!(
                        matches!(result, ShellToolCallMatch::UnsupportedEvent),
                        "{id}: {result:?}"
                    );
                }
                "missing-tool_name" => {
                    assert!(
                        matches!(result, ShellToolCallMatch::NotShell),
                        "{id}: {result:?}"
                    );
                }
                "missing-command" | "tool-input-string" => {
                    let ShellToolCallMatch::Malformed(error) = result else {
                        panic!("{id}: {result:?}");
                    };
                    assert_eq!(
                        error.kind,
                        crate::ShellToolCallErrorKind::MissingCommand,
                        "{id}"
                    );
                    assert_eq!(error.event.name(), "PreSkillShellCommand", "{id}");
                    assert_eq!(error.pointer, "/command", "{id}");
                }
                "missing-command-name" | "missing-invoked-by" | "invoked-by-number" => {
                    assert!(
                        matches!(
                            result,
                            ShellToolCallMatch::Matched(ref call)
                                if call.command == "git log -1 --format=%H"
                        ),
                        "{id}: {result:?}"
                    );
                }
                other => panic!("negative fixture `{other}` has no expected shell match"),
            }
        }
    }

    #[cfg(feature = "claude")]
    fn claude_pre_skill_shell_command(
        tool_name: &str,
        tool_input: serde_json::Value,
    ) -> hookkit_claude::catalog::CatalogInput {
        serde_json::from_value(serde_json::json!({
            "session_id": "session",
            "transcript_path": "/tmp/transcript.jsonl",
            "cwd": "/repo",
            "hook_event_name": "PreSkillShellCommand",
            "tool_name": tool_name,
            "tool_input": tool_input,
            "tool_use_id": "skillsh_1",
            "command_name": "release-notes",
            "invoked_by": "user"
        }))
        .unwrap()
    }

    #[cfg(feature = "claude")]
    #[test]
    fn claude_pre_skill_shell_command_edges() {
        // Only Bash and PowerShell run injected commands; the event reports
        // no Monitor call even when the input has a command.
        for tool in ["Monitor", "bash", "Shell", ""] {
            let input = claude_pre_skill_shell_command(tool, serde_json::json!({"command": "ls"}));
            assert!(
                matches!(input.shell_tool_call(), ShellToolCallMatch::NotShell),
                "{tool}"
            );
        }
        let not_string =
            claude_pre_skill_shell_command("Bash", serde_json::json!({"command": ["ls"]}));
        assert!(matches!(
            not_string.shell_tool_call(),
            ShellToolCallMatch::Malformed(crate::ShellToolCallError {
                kind: crate::ShellToolCallErrorKind::CommandNotString,
                ..
            })
        ));
        // An injected command takes the event's `tool_input` as a whole and
        // has no working-directory field of its own.
        let input = claude_pre_skill_shell_command(
            "Bash",
            serde_json::json!({"command": "cat .env", "cwd": "/elsewhere"}),
        );
        let ShellToolCallMatch::Matched(call) = input.shell_tool_call() else {
            panic!("expected a shell call");
        };
        assert_eq!(call.cwd, Some(hookkit_core::Utf8Path::new("/repo")));
        let report = crate::FileAccessAnalyzer::default().infer(
            &crate::BashAnalyzer::default().analyze(call.command),
            crate::FileInferenceContext::for_call(&call),
        );
        assert!(
            report.candidates.iter().any(|candidate| matches!(
                &candidate.target,
                crate::FileTarget::Path { expression, .. }
                    if expression.resolved.as_deref()
                        == Some(hookkit_core::Utf8Path::new("/repo/.env"))
            )),
            "{report:?}"
        );
        // A typed `PreToolUse` with the same Bash command matches the same
        // way, so one policy serves both events.
        let mut as_tool_call = input.clone();
        as_tool_call.hook_event_name = "PreToolUse".into();
        let ShellToolCallMatch::Matched(tool_call) = as_tool_call.shell_tool_call() else {
            panic!("expected a shell call");
        };
        assert_eq!(tool_call.command, call.command);
        assert_eq!(tool_call.cwd, call.cwd);
        assert_eq!(tool_call.event.name(), "PreToolUse");
    }

    #[cfg(feature = "claude")]
    fn claude_tool_call(
        tool_name: &str,
        tool_input: serde_json::Value,
    ) -> hookkit_claude::catalog::CatalogInput {
        serde_json::from_value(serde_json::json!({
            "session_id": "session",
            "transcript_path": "/tmp/transcript.jsonl",
            "cwd": "/repo",
            "hook_event_name": "PreToolUse",
            "tool_name": tool_name,
            "tool_input": tool_input,
            "tool_use_id": "toolu_1"
        }))
        .unwrap()
    }

    #[cfg(feature = "claude")]
    #[test]
    fn claude_monitor_commands_are_shell_calls_and_websocket_watches_are_not() {
        let command = claude_tool_call(
            "Monitor",
            serde_json::json!({
                "description": "watch app log",
                "timeout_ms": 300000,
                "command": "tail -f app.log"
            }),
        );
        let ShellToolCallMatch::Matched(call) = command.shell_tool_call() else {
            panic!("expected a shell call");
        };
        assert_eq!(call.tool_name, "Monitor");
        assert_eq!(call.command, "tail -f app.log");
        assert_eq!(call.cwd, Some(hookkit_core::Utf8Path::new("/repo")));

        let websocket = claude_tool_call(
            "Monitor",
            serde_json::json!({
                "description": "ws",
                "timeout_ms": 60000,
                "ws": {"url": "wss://example.test/feed"}
            }),
        );
        assert!(matches!(
            websocket.shell_tool_call(),
            ShellToolCallMatch::NotShell
        ));

        let malformed = claude_tool_call(
            "Monitor",
            serde_json::json!({"description": "x", "timeout_ms": 1, "command": 7}),
        );
        assert!(matches!(
            malformed.shell_tool_call(),
            ShellToolCallMatch::Malformed(_)
        ));
    }

    /// A hook whose matcher includes `Monitor`, or that has no matcher,
    /// receives the Monitor call's own input on every tool event, so the
    /// catalog adapter on each event and the typed `PostToolUse` adapter
    /// match a Monitor command, and none matches a WebSocket watch.
    #[cfg(feature = "claude")]
    #[test]
    fn claude_monitor_commands_match_on_every_tool_event_and_the_typed_post_tool_use() {
        let command = serde_json::json!({
            "description": "watch app log",
            "timeout_ms": 300000,
            "command": "tail -f app.log"
        });
        let websocket = serde_json::json!({
            "description": "ws",
            "timeout_ms": 60000,
            "ws": {"url": "wss://example.test/feed"}
        });
        let catalog = |event: &str, tool_input: &serde_json::Value| {
            let mut value = serde_json::json!({
                "session_id": "session",
                "transcript_path": "/tmp/transcript.jsonl",
                "cwd": "/repo",
                "hook_event_name": event,
                "tool_name": "Monitor",
                "tool_input": tool_input,
                "tool_use_id": "toolu_1"
            });
            match event {
                "PostToolUse" => value["tool_response"] = serde_json::json!({"stdout": ""}),
                "PostToolUseFailure" => value["error"] = serde_json::json!("Exit code 1"),
                _ => {}
            }
            serde_json::from_value::<hookkit_claude::catalog::CatalogInput>(value).unwrap()
        };
        for (event, phase, has_response) in [
            ("PermissionRequest", ToolPhase::Pre, false),
            ("PermissionDenied", ToolPhase::Pre, false),
            ("PostToolUse", ToolPhase::Post, true),
            ("PostToolUseFailure", ToolPhase::Post, false),
        ] {
            let input = catalog(event, &command);
            let ShellToolCallMatch::Matched(call) = input.shell_tool_call() else {
                panic!("expected a shell call for {event}");
            };
            assert_eq!(call.tool_name, "Monitor", "{event}");
            assert_eq!(call.command, "tail -f app.log", "{event}");
            assert_eq!(call.phase, phase, "{event}");
            assert_eq!(call.response.is_some(), has_response, "{event}");
            assert!(
                matches!(
                    catalog(event, &websocket).shell_tool_call(),
                    ShellToolCallMatch::NotShell
                ),
                "{event}"
            );
        }

        let typed = |tool_input: &serde_json::Value| {
            serde_json::from_value::<hookkit_claude::protocol::PostToolUseInput>(
                serde_json::json!({
                    "session_id": "session",
                    "transcript_path": "/tmp/transcript.jsonl",
                    "cwd": "/repo",
                    "hook_event_name": "PostToolUse",
                    "tool_name": "Monitor",
                    "tool_input": tool_input,
                    "tool_use_id": "toolu_1",
                    "tool_response": {"stdout": ""}
                }),
            )
            .unwrap()
        };
        let input = typed(&command);
        let ShellToolCallMatch::Matched(call) = input.shell_tool_call() else {
            panic!("expected a shell call for the typed PostToolUse");
        };
        assert_eq!(call.tool_name, "Monitor");
        assert_eq!(call.command, "tail -f app.log");
        assert_eq!(call.phase, ToolPhase::Post);
        assert!(call.response.is_some());
        assert!(matches!(
            typed(&websocket).shell_tool_call(),
            ShellToolCallMatch::NotShell
        ));
    }

    #[cfg(feature = "claude")]
    #[test]
    fn claude_powershell_calls_do_not_match_the_bash_profile() {
        let input = claude_tool_call(
            "PowerShell",
            serde_json::json!({"command": "Get-ChildItem -Recurse"}),
        );
        assert!(matches!(
            input.shell_tool_call(),
            ShellToolCallMatch::NotShell
        ));
    }

    #[cfg(feature = "claude")]
    #[test]
    fn claude_code_mode_cd_prefix_leaves_relative_operands_unresolved() {
        // Code Mode prefixes a shell command from Claude's code with
        // `cd '<dir>' && `, so a relative operand is not under the hook cwd.
        let input = claude_tool_call(
            "Bash",
            serde_json::json!({"command": "cd 'app' && rm -f out.txt"}),
        );
        let ShellToolCallMatch::Matched(call) = input.shell_tool_call() else {
            panic!("expected a shell call");
        };
        let report = crate::FileAccessAnalyzer::default().infer(
            &crate::BashAnalyzer::default().analyze(call.command),
            crate::FileInferenceContext::for_call(&call),
        );
        assert!(
            report.candidates.iter().all(|candidate| !matches!(
                &candidate.target,
                crate::FileTarget::Path { expression, .. }
                    if expression.resolved.as_deref()
                        == Some(hookkit_core::Utf8Path::new("/repo/out.txt"))
            )),
            "{report:?}"
        );
        assert!(
            report.unresolved.iter().any(|gap| gap.reason
                == crate::UnresolvedFileAccessReason::WorkingDirectoryMayHaveChanged),
            "{report:?}"
        );
    }

    #[cfg(feature = "codex")]
    fn codex_pre_tool_use(
        tool_input: serde_json::Value,
    ) -> hookkit_codex::protocol::PreToolUseInput {
        serde_json::from_value(serde_json::json!({
            "session_id": "session",
            "transcript_path": "/tmp/transcript.jsonl",
            "cwd": "/repo",
            "hook_event_name": "PreToolUse",
            "model": "gpt-5",
            "turn_id": "turn",
            "permission_mode": "default",
            "tool_name": "Bash",
            "tool_use_id": "tool",
            "tool_input": tool_input
        }))
        .unwrap()
    }

    #[cfg(feature = "codex")]
    #[test]
    fn extracts_codex_pre_tool_use_and_rejects_other_tools() {
        let mut input = codex_pre_tool_use(serde_json::json!({"command": "cargo test"}));

        let ShellToolCallMatch::Matched(call) = input.shell_tool_call() else {
            panic!("expected a shell call");
        };
        // A remote-environment command runs elsewhere than the reported cwd.
        assert_eq!(call.cwd, Some(hookkit_core::Utf8Path::new("/repo")));
        assert_eq!(call.cwd_origin, ShellCwdOrigin::UnverifiedFallback);
        input.tool_name = "Read".into();
        assert!(matches!(
            input.shell_tool_call(),
            ShellToolCallMatch::NotShell
        ));
    }

    #[cfg(feature = "codex")]
    #[test]
    fn codex_resolved_workdir_cwd_anchors_relative_operands() {
        // Since Codex 0.166.0 a local command run with `workdir` reports the
        // resolved directory as the hook cwd.
        let mut input = codex_pre_tool_use(serde_json::json!({"command": "cat a.txt"}));
        input.cwd = "/repo/sub".into();
        let ShellToolCallMatch::Matched(call) = input.shell_tool_call() else {
            panic!("expected a shell call");
        };
        assert_eq!(call.cwd, Some(hookkit_core::Utf8Path::new("/repo/sub")));
        // Remote commands still report the host cwd with no marker.
        assert_eq!(call.cwd_origin, ShellCwdOrigin::UnverifiedFallback);
        let report = crate::FileAccessAnalyzer::default().infer(
            &crate::BashAnalyzer::default().analyze(call.command),
            crate::FileInferenceContext::for_call(&call),
        );
        assert!(
            matches!(
                &report.candidates[0].target,
                crate::FileTarget::Path { expression, .. }
                    if expression.resolved.as_deref()
                        == Some(hookkit_core::Utf8Path::new("/repo/sub/a.txt"))
            ),
            "{report:?}"
        );
    }

    #[cfg(feature = "codex")]
    #[test]
    fn codex_workdir_is_resolved_against_the_hook_cwd() {
        let relative = codex_pre_tool_use(serde_json::json!({
            "command": "cat id_rsa",
            "workdir": "nested/../keys"
        }));
        let ShellToolCallMatch::Matched(call) = relative.shell_tool_call() else {
            panic!("expected a shell call");
        };
        assert_eq!(call.cwd, None);
        assert_eq!(call.cwd_origin, ShellCwdOrigin::ToolInputRelativeToFallback);
        assert_eq!(
            call.effective_cwd(),
            Some(hookkit_core::Utf8Path::new("/repo/keys"))
        );

        let absolute = codex_pre_tool_use(serde_json::json!({
            "command": "cat id_rsa",
            "workdir": "/Users/me/.ssh"
        }));
        let ShellToolCallMatch::Matched(call) = absolute.shell_tool_call() else {
            panic!("expected a shell call");
        };
        assert_eq!(
            call.cwd,
            Some(hookkit_core::Utf8Path::new("/Users/me/.ssh"))
        );
        assert_eq!(call.cwd_origin, ShellCwdOrigin::ToolInput);
    }

    #[cfg(feature = "antigravity")]
    fn antigravity_args(args: serde_json::Value) -> hookkit_antigravity::PreToolUseInput {
        serde_json::from_value(serde_json::json!({
            "conversationId": "conversation",
            "workspacePaths": ["/repo"],
            "transcriptPath": "/tmp/transcript.jsonl",
            "artifactDirectoryPath": "/tmp/artifacts",
            "toolCall": {
                "name": "run_command",
                "args": args
            },
            "stepIdx": 1
        }))
        .unwrap()
    }

    #[cfg(feature = "antigravity")]
    #[test]
    fn antigravity_command_cwd_overrides_workspace_fallback() {
        let input =
            antigravity_args(serde_json::json!({"CommandLine": "pwd", "Cwd": "/repo/subdir"}));

        let ShellToolCallMatch::Matched(call) = input.shell_tool_call() else {
            panic!("expected a shell call");
        };
        assert_eq!(call.cwd, Some(hookkit_core::Utf8Path::new("/repo/subdir")));
        assert_eq!(call.cwd_origin, ShellCwdOrigin::ToolInput);
    }

    #[cfg(feature = "antigravity")]
    #[test]
    fn antigravity_relative_cwd_is_never_used_as_an_absolute_base() {
        for (cwd, expected) in [(".", "/repo"), ("sub", "/repo/sub")] {
            let input =
                antigravity_args(serde_json::json!({"CommandLine": "cat .env", "Cwd": cwd}));
            let ShellToolCallMatch::Matched(call) = input.shell_tool_call() else {
                panic!("expected a shell call");
            };
            assert_eq!(call.cwd, None, "{cwd}");
            assert_eq!(
                call.effective_cwd(),
                Some(hookkit_core::Utf8Path::new(expected)),
                "{cwd}"
            );
            let report = crate::FileAccessAnalyzer::default().infer(
                &crate::BashAnalyzer::default().analyze(call.command),
                crate::FileInferenceContext::for_call(&call),
            );
            assert!(
                matches!(
                    &report.candidates[0].target,
                    crate::FileTarget::Path { expression, .. }
                        if expression.resolved.as_deref()
                            == Some(hookkit_core::Utf8Path::new(&format!("{expected}/.env")))
                ),
                "{cwd}: {report:?}"
            );
        }
        let empty = antigravity_args(serde_json::json!({"CommandLine": "pwd", "Cwd": ""}));
        let ShellToolCallMatch::Matched(call) = empty.shell_tool_call() else {
            panic!("expected a shell call");
        };
        assert_eq!(call.cwd, Some(hookkit_core::Utf8Path::new("/repo")));
        assert_eq!(call.cwd_origin, ShellCwdOrigin::Fallback);
    }

    #[cfg(feature = "antigravity")]
    #[test]
    fn extracts_antigravity_post_tool_command() {
        let input: hookkit_antigravity::PostToolUseInput =
            serde_json::from_value(serde_json::json!({
                "conversationId": "conversation",
                "workspacePaths": ["/repo"],
                "transcriptPath": "/tmp/transcript.jsonl",
                "artifactDirectoryPath": "/tmp/artifacts",
                "toolCall": {
                    "name": "run_command",
                    "args": {"CommandLine": "pwd", "Cwd": "/repo/subdir"}
                },
                "stepIdx": 1
            }))
            .unwrap();

        let ShellToolCallMatch::Matched(call) = input.shell_tool_call() else {
            panic!("expected a shell call");
        };
        assert_eq!(call.phase, ToolPhase::Post);
        assert_eq!(call.cwd, Some(hookkit_core::Utf8Path::new("/repo/subdir")));
    }
}
