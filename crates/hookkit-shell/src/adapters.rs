//! Native harness adapters. Tool names and field locations are intentionally
//! exact; aliases belong in an explicit [`ShellToolProfile`](crate::ShellToolProfile).

use hookkit_core::{EventId, HarnessId};

#[cfg(feature = "gemini")]
use crate::call::JsonRef;
#[cfg(feature = "claude")]
use crate::call::ShellToolCallError;
use crate::call::{ShellToolCallExt, ShellToolCallMatch, ShellToolProfile, ToolPhase};

#[cfg(feature = "claude")]
pub const CLAUDE_BASH_PROFILE: ShellToolProfile =
    ShellToolProfile::builtin("Bash", "/command", None);
#[cfg(feature = "codex")]
pub const CODEX_BASH_PROFILE: ShellToolProfile =
    ShellToolProfile::builtin("Bash", "/command", None);
#[cfg(feature = "gemini")]
pub const GEMINI_RUN_SHELL_COMMAND_PROFILE: ShellToolProfile =
    ShellToolProfile::builtin("run_shell_command", "/command", None);
#[cfg(feature = "antigravity")]
pub const ANTIGRAVITY_RUN_COMMAND_PROFILE: ShellToolProfile =
    ShellToolProfile::builtin("run_command", "/CommandLine", Some("/Cwd"));

#[cfg(feature = "claude")]
impl ShellToolCallExt for hookkit_claude::catalog::CatalogInput {
    fn shell_tool_call(&self) -> ShellToolCallMatch<'_> {
        if self.hook_event_name != "PreToolUse" {
            return ShellToolCallMatch::NotShell;
        }
        let Some(tool_name) = self.field("tool_name").and_then(serde_json::Value::as_str) else {
            return ShellToolCallMatch::NotShell;
        };
        if tool_name != CLAUDE_BASH_PROFILE.tool_name() {
            return ShellToolCallMatch::NotShell;
        }
        let event = EventId::builtin(HarnessId::CLAUDE_CODE, "PreToolUse");
        let Some(tool_input) = self.field("tool_input") else {
            return ShellToolCallMatch::Malformed(ShellToolCallError::missing_command(
                event,
                tool_name,
                CLAUDE_BASH_PROFILE.command_pointer(),
            ));
        };
        CLAUDE_BASH_PROFILE.extract_from_value(
            event,
            ToolPhase::Pre,
            tool_name,
            tool_input,
            Some(self.cwd.as_path()),
            None,
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

#[cfg(feature = "gemini")]
impl ShellToolCallExt for hookkit_gemini::protocol::BeforeToolInput {
    fn shell_tool_call(&self) -> ShellToolCallMatch<'_> {
        GEMINI_RUN_SHELL_COMMAND_PROFILE.extract_from_object(
            EventId::builtin(HarnessId::GEMINI_CLI, "BeforeTool"),
            ToolPhase::Pre,
            &self.tool_name,
            &self.tool_input,
            Some(self.cwd.as_path()),
            None,
        )
    }
}

#[cfg(feature = "gemini")]
impl ShellToolCallExt for hookkit_gemini::protocol::AfterToolInput {
    fn shell_tool_call(&self) -> ShellToolCallMatch<'_> {
        GEMINI_RUN_SHELL_COMMAND_PROFILE.extract_from_object(
            EventId::builtin(HarnessId::GEMINI_CLI, "AfterTool"),
            ToolPhase::Post,
            &self.tool_name,
            &self.tool_input,
            Some(self.cwd.as_path()),
            Some(JsonRef::Object(&self.tool_response)),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "claude")]
    #[test]
    fn extracts_claude_catalog_pre_tool_use() {
        let input: hookkit_claude::catalog::CatalogInput =
            serde_json::from_value(serde_json::json!({
                "session_id": "session",
                "transcript_path": "/tmp/transcript.jsonl",
                "cwd": "/repo",
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_input": {"command": "cargo test"}
            }))
            .unwrap();

        let ShellToolCallMatch::Matched(call) = input.shell_tool_call() else {
            panic!("expected a shell call");
        };
        assert_eq!(call.command, "cargo test");
        assert_eq!(call.cwd, Some(hookkit_core::Utf8Path::new("/repo")));
    }

    #[cfg(feature = "codex")]
    #[test]
    fn extracts_codex_pre_tool_use_and_rejects_other_tools() {
        let mut input: hookkit_codex::protocol::PreToolUseInput =
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
                "tool_input": {"command": "cargo test"}
            }))
            .unwrap();

        assert!(matches!(
            input.shell_tool_call(),
            ShellToolCallMatch::Matched(_)
        ));
        input.tool_name = "Read".into();
        assert!(matches!(
            input.shell_tool_call(),
            ShellToolCallMatch::NotShell
        ));
    }

    #[cfg(feature = "gemini")]
    #[test]
    fn extracts_gemini_after_tool_response() {
        let input: hookkit_gemini::protocol::AfterToolInput =
            serde_json::from_value(serde_json::json!({
                "session_id": "session",
                "transcript_path": "/tmp/transcript.jsonl",
                "cwd": "/repo",
                "hook_event_name": "AfterTool",
                "timestamp": "2026-07-15T00:00:00Z",
                "tool_name": "run_shell_command",
                "tool_input": {"command": "pwd"},
                "tool_response": {"output": "/repo"}
            }))
            .unwrap();

        let ShellToolCallMatch::Matched(call) = input.shell_tool_call() else {
            panic!("expected a shell call");
        };
        assert!(matches!(call.response, Some(JsonRef::Object(_))));
    }

    #[cfg(feature = "antigravity")]
    #[test]
    fn antigravity_command_cwd_overrides_workspace_fallback() {
        let input: hookkit_antigravity::PreToolUseInput =
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
        assert_eq!(call.cwd, Some(hookkit_core::Utf8Path::new("/repo/subdir")));
    }
}
