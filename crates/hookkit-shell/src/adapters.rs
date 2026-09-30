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
pub const CLAUDE_BASH_PROFILE: ShellToolProfile =
    ShellToolProfile::builtin("Bash", "/command", None, true);
#[cfg(feature = "codex")]
/// Exact Codex `Bash` tool profile.
///
/// Codex's `exec_command` accepts a `workdir` argument that it joins onto the
/// step's environment cwd, but its hook payloads carry only `command`, so the
/// hook `cwd` is only the default working directory and matched calls report
/// [`ShellCwdOrigin::UnverifiedFallback`](crate::ShellCwdOrigin::UnverifiedFallback).
/// A `/workdir` value, if a payload ever carries one, is resolved against the
/// hook `cwd`. The dialect is unknown: Codex runs the user's shell, which is
/// zsh by default on macOS and PowerShell on Windows.
pub const CODEX_BASH_PROFILE: ShellToolProfile =
    ShellToolProfile::builtin("Bash", "/command", Some("/workdir"), false);
#[cfg(feature = "antigravity")]
/// Exact Antigravity `run_command` tool profile, including its optional cwd.
pub const ANTIGRAVITY_RUN_COMMAND_PROFILE: ShellToolProfile =
    ShellToolProfile::builtin("run_command", "/CommandLine", Some("/Cwd"), true);

/// Claude Code tool events whose input carries `tool_name` and `tool_input`.
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
        _ => None,
    }
}

#[cfg(feature = "claude")]
impl ShellToolCallExt for hookkit_claude::catalog::CatalogInput {
    /// Matches the `Bash` tool on `PreToolUse`, `PermissionRequest`,
    /// `PermissionDenied`, `PostToolUse`, and `PostToolUseFailure`; other
    /// events return [`ShellToolCallMatch::UnsupportedEvent`].
    fn shell_tool_call(&self) -> ShellToolCallMatch<'_> {
        let Some((event_name, phase, response_field)) = claude_tool_event(&self.hook_event_name)
        else {
            return ShellToolCallMatch::UnsupportedEvent;
        };
        let Some(tool_name) = self.field("tool_name").and_then(serde_json::Value::as_str) else {
            return ShellToolCallMatch::NotShell;
        };
        if tool_name != CLAUDE_BASH_PROFILE.tool_name() {
            return ShellToolCallMatch::NotShell;
        }
        let event = EventId::builtin(HarnessId::CLAUDE_CODE, event_name);
        let Some(tool_input) = self.field("tool_input") else {
            return ShellToolCallMatch::Malformed(ShellToolCallError::missing_command(
                event,
                tool_name,
                CLAUDE_BASH_PROFILE.command_pointer(),
            ));
        };
        CLAUDE_BASH_PROFILE.extract_from_value(
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
    fn shell_tool_call(&self) -> ShellToolCallMatch<'_> {
        CLAUDE_BASH_PROFILE.extract_from_value(
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
        // Codex may run the command in an unexposed `workdir`.
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
