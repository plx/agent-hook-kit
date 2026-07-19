use hookkit_core::{EventId, EventSpec, HarnessId, HookkitError, Utf8Path};
use std::borrow::Cow;

/// Lossless native command-environment arms for aligned pre-tool execution.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum PreToolUseCommandEnvironment {
    Claude(hookkit_claude::ClaudeCommandEnvironment),
    Codex(hookkit_codex::CodexCommandEnvironment),
    Gemini(hookkit_gemini::GeminiCommandEnvironment),
    Antigravity(hookkit_antigravity::AntigravityCommandEnvironment),
}

impl PreToolUseCommandEnvironment {
    pub fn harness(&self) -> HarnessId {
        match self {
            Self::Claude(_) => HarnessId::CLAUDE_CODE,
            Self::Codex(_) => HarnessId::CODEX,
            Self::Gemini(_) => HarnessId::GEMINI_CLI,
            Self::Antigravity(_) => HarnessId::ANTIGRAVITY,
        }
    }
}

/// Borrowed view over the two native JSON representations used for tool input.
///
/// The original native input remains available through [`PreToolUseInput`].
/// This view does not clone or flatten a JSON object into a common wire shape.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum ToolInputRef<'a> {
    Value(&'a serde_json::Value),
    Object(&'a serde_json::Map<String, serde_json::Value>),
}

impl<'a> ToolInputRef<'a> {
    pub fn get(self, key: &str) -> Option<&'a serde_json::Value> {
        match self {
            Self::Value(value) => value.get(key),
            Self::Object(object) => object.get(key),
        }
    }

    pub fn as_object(self) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
        match self {
            Self::Value(value) => value.as_object(),
            Self::Object(object) => Some(object),
        }
    }
}

/// Lossless aligned pre-tool input.
///
/// Native event names remain distinct: Claude Code and Codex use
/// `PreToolUse`, Gemini CLI uses `BeforeTool`, and Antigravity uses
/// `PreToolUse`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum PreToolUseInput {
    Claude(hookkit_claude::catalog::CatalogInput),
    Codex(hookkit_codex::protocol::PreToolUseInput),
    Gemini(hookkit_gemini::protocol::BeforeToolInput),
    Antigravity(hookkit_antigravity::PreToolUseInput),
}

impl PreToolUseInput {
    pub fn harness(&self) -> HarnessId {
        match self {
            Self::Claude(_) => HarnessId::CLAUDE_CODE,
            Self::Codex(_) => HarnessId::CODEX,
            Self::Gemini(_) => HarnessId::GEMINI_CLI,
            Self::Antigravity(_) => HarnessId::ANTIGRAVITY,
        }
    }

    pub fn event_id(&self) -> EventId {
        match self {
            Self::Claude(_) => hookkit_claude::catalog::PreToolUse::EVENT,
            Self::Codex(_) => hookkit_codex::protocol::PreToolUse::EVENT,
            Self::Gemini(_) => hookkit_gemini::protocol::BeforeTool::EVENT,
            Self::Antigravity(_) => hookkit_antigravity::PreToolUse::EVENT,
        }
    }

    pub fn workspace_roots(&self) -> Cow<'_, [hookkit_core::Utf8PathBuf]> {
        match self {
            Self::Claude(input) => Cow::Owned(vec![input.cwd.clone()]),
            Self::Codex(input) => Cow::Owned(vec![input.cwd.clone()]),
            Self::Gemini(input) => Cow::Owned(vec![input.cwd.clone()]),
            Self::Antigravity(input) => Cow::Borrowed(&input.workspace_paths),
        }
    }

    /// Return the native working directory when that event carries one.
    ///
    /// Antigravity supplies workspace roots rather than a single native cwd.
    pub fn cwd(&self) -> Option<&Utf8Path> {
        match self {
            Self::Claude(input) => Some(&input.cwd),
            Self::Codex(input) => Some(&input.cwd),
            Self::Gemini(input) => Some(&input.cwd),
            Self::Antigravity(_) => None,
        }
    }

    /// Return the native tool name when its value is a string.
    pub fn tool_name(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => input.field("tool_name").and_then(serde_json::Value::as_str),
            Self::Codex(input) => Some(&input.tool_name),
            Self::Gemini(input) => Some(&input.tool_name),
            Self::Antigravity(input) => Some(&input.tool_call.name),
        }
    }

    /// Borrow the complete native tool-input value without allocation.
    pub fn tool_input(&self) -> Option<ToolInputRef<'_>> {
        match self {
            Self::Claude(input) => input.field("tool_input").map(ToolInputRef::Value),
            Self::Codex(input) => Some(ToolInputRef::Value(&input.tool_input)),
            Self::Gemini(input) => Some(ToolInputRef::Object(&input.tool_input)),
            Self::Antigravity(input) => Some(ToolInputRef::Object(&input.tool_call.args)),
        }
    }
}

/// Lossless native pre-tool output arms. No generic serialized envelope is
/// introduced during lowering.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum PreToolUseOutput {
    Claude(hookkit_claude::catalog::PreToolUseOutput),
    Codex(hookkit_codex::protocol::PreToolUseOutput),
    Gemini(hookkit_gemini::protocol::BeforeToolOutput),
    Antigravity(hookkit_antigravity::PreToolUseOutput),
}

impl PreToolUseOutput {
    /// Build the selected harness's native explicit-allow output.
    pub fn allow(harness: &HarnessId) -> hookkit_core::Result<Self> {
        match harness.as_str() {
            "claude-code" => Ok(Self::Claude(
                hookkit_claude::catalog::PreToolUseOutput::decide(
                    hookkit_claude::catalog::PreToolPermissionDecision::Allow,
                    None,
                    None,
                    None,
                ),
            )),
            "codex" => Ok(Self::Codex(
                hookkit_codex::protocol::PreToolUseOutput::allow(),
            )),
            "gemini-cli" => Ok(Self::Gemini(
                hookkit_gemini::protocol::BeforeToolOutput::allow(),
            )),
            "antigravity" => Ok(Self::Antigravity(hookkit_antigravity::PreToolUseOutput {
                decision: hookkit_antigravity::ToolDecision::Allow,
                reason: None,
                permission_overrides: Vec::new(),
            })),
            _ => Err(unsupported_pre_tool_harness(harness)),
        }
    }

    /// Build the selected harness's native deny output with a reason.
    pub fn deny(harness: &HarnessId, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        let reason = reason.into();
        match harness.as_str() {
            "claude-code" => Ok(Self::Claude(
                hookkit_claude::catalog::PreToolUseOutput::decide(
                    hookkit_claude::catalog::PreToolPermissionDecision::Deny,
                    Some(reason),
                    None,
                    None,
                ),
            )),
            "codex" => Ok(Self::Codex(
                hookkit_codex::protocol::PreToolUseOutput::deny(Some(reason)),
            )),
            "gemini-cli" => Ok(Self::Gemini(
                hookkit_gemini::protocol::BeforeToolOutput::deny(reason),
            )),
            "antigravity" => Ok(Self::Antigravity(hookkit_antigravity::PreToolUseOutput {
                decision: hookkit_antigravity::ToolDecision::Deny,
                reason: Some(reason),
                permission_overrides: Vec::new(),
            })),
            _ => Err(unsupported_pre_tool_harness(harness)),
        }
    }

    pub fn harness(&self) -> HarnessId {
        match self {
            Self::Claude(_) => HarnessId::CLAUDE_CODE,
            Self::Codex(_) => HarnessId::CODEX,
            Self::Gemini(_) => HarnessId::GEMINI_CLI,
            Self::Antigravity(_) => HarnessId::ANTIGRAVITY,
        }
    }

    pub fn event_id(&self) -> EventId {
        match self {
            Self::Claude(_) => hookkit_claude::catalog::PreToolUse::EVENT,
            Self::Codex(_) => hookkit_codex::protocol::PreToolUse::EVENT,
            Self::Gemini(_) => hookkit_gemini::protocol::BeforeTool::EVENT,
            Self::Antigravity(_) => hookkit_antigravity::PreToolUse::EVENT,
        }
    }
}

fn unsupported_pre_tool_harness(harness: &HarnessId) -> HookkitError {
    HookkitError::UnrecognizedEvent {
        harness: harness.clone(),
        message: "no aligned PreToolUse adapter is registered".into(),
    }
}

/// Lossless native command-environment arms for aligned post-tool execution.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum PostToolUseCommandEnvironment {
    Claude(hookkit_claude::ClaudeCommandEnvironment),
    Codex(hookkit_codex::CodexCommandEnvironment),
    Gemini(hookkit_gemini::GeminiCommandEnvironment),
    Antigravity(hookkit_antigravity::AntigravityCommandEnvironment),
}

impl PostToolUseCommandEnvironment {
    pub fn harness(&self) -> HarnessId {
        match self {
            Self::Claude(_) => HarnessId::CLAUDE_CODE,
            Self::Codex(_) => HarnessId::CODEX,
            Self::Gemini(_) => HarnessId::GEMINI_CLI,
            Self::Antigravity(_) => HarnessId::ANTIGRAVITY,
        }
    }
}

/// Lossless aligned input: every arm retains the complete native value.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum PostToolUseInput {
    Claude(hookkit_claude::protocol::PostToolUseInput),
    Codex(hookkit_codex::protocol::PostToolUseInput),
    Gemini(hookkit_gemini::protocol::AfterToolInput),
    Antigravity(hookkit_antigravity::PostToolUseInput),
}

impl PostToolUseInput {
    pub fn harness(&self) -> HarnessId {
        match self {
            Self::Claude(_) => HarnessId::CLAUDE_CODE,
            Self::Codex(_) => HarnessId::CODEX,
            Self::Gemini(_) => HarnessId::GEMINI_CLI,
            Self::Antigravity(_) => HarnessId::ANTIGRAVITY,
        }
    }

    pub fn event_id(&self) -> EventId {
        match self {
            Self::Claude(_) => hookkit_claude::protocol::PostToolUse::EVENT,
            Self::Codex(_) => hookkit_codex::protocol::PostToolUse::EVENT,
            Self::Gemini(_) => hookkit_gemini::protocol::AfterTool::EVENT,
            Self::Antigravity(_) => hookkit_antigravity::PostToolUse::EVENT,
        }
    }

    pub fn workspace_roots(&self) -> Cow<'_, [hookkit_core::Utf8PathBuf]> {
        match self {
            Self::Claude(input) => Cow::Owned(vec![input.cwd.clone()]),
            Self::Codex(input) => Cow::Owned(vec![input.cwd.clone()]),
            Self::Gemini(input) => Cow::Owned(vec![input.cwd.clone()]),
            Self::Antigravity(input) => Cow::Borrowed(&input.workspace_paths),
        }
    }
}

/// Lossless native output arms. No generic envelope participates in lowering.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum PostToolUseOutput {
    Claude(hookkit_claude::protocol::PostToolUseOutput),
    Codex(hookkit_codex::protocol::PostToolUseOutput),
    Gemini(hookkit_gemini::protocol::AfterToolOutput),
    Antigravity(hookkit_antigravity::PostToolUseOutput),
}

/// Lossless native command-environment arms for aligned turn completion.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum TurnCompletionCommandEnvironment {
    Claude(hookkit_claude::ClaudeCommandEnvironment),
    Codex(hookkit_codex::CodexCommandEnvironment),
    Gemini(hookkit_gemini::GeminiCommandEnvironment),
    Antigravity(hookkit_antigravity::AntigravityCommandEnvironment),
}

impl TurnCompletionCommandEnvironment {
    pub fn harness(&self) -> HarnessId {
        match self {
            Self::Claude(_) => HarnessId::CLAUDE_CODE,
            Self::Codex(_) => HarnessId::CODEX,
            Self::Gemini(_) => HarnessId::GEMINI_CLI,
            Self::Antigravity(_) => HarnessId::ANTIGRAVITY,
        }
    }
}

/// The event at which one agent turn is about to complete.
///
/// Native names and values remain intact: Claude Code and Codex use `Stop`,
/// Gemini CLI uses `AfterAgent`, and Antigravity uses `Stop`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum TurnCompletionInput {
    Claude(hookkit_claude::catalog::CatalogInput),
    Codex(hookkit_codex::catalog::CatalogInput),
    Gemini(hookkit_gemini::catalog::CatalogInput),
    Antigravity(hookkit_antigravity::StopInput),
}

impl TurnCompletionInput {
    pub fn harness(&self) -> HarnessId {
        match self {
            Self::Claude(_) => HarnessId::CLAUDE_CODE,
            Self::Codex(_) => HarnessId::CODEX,
            Self::Gemini(_) => HarnessId::GEMINI_CLI,
            Self::Antigravity(_) => HarnessId::ANTIGRAVITY,
        }
    }

    pub fn event_id(&self) -> EventId {
        match self {
            Self::Claude(_) => hookkit_claude::catalog::Stop::EVENT,
            Self::Codex(_) => hookkit_codex::catalog::Stop::EVENT,
            Self::Gemini(_) => hookkit_gemini::catalog::AfterAgent::EVENT,
            Self::Antigravity(_) => hookkit_antigravity::Stop::EVENT,
        }
    }

    pub fn workspace_roots(&self) -> Cow<'_, [hookkit_core::Utf8PathBuf]> {
        match self {
            Self::Claude(input) => Cow::Owned(vec![input.cwd.clone()]),
            Self::Codex(input) => Cow::Owned(vec![input.cwd.clone()]),
            Self::Gemini(input) => Cow::Owned(vec![input.cwd.clone()]),
            Self::Antigravity(input) => Cow::Borrowed(&input.workspace_paths),
        }
    }
}

/// Lossless native turn-completion outputs.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum TurnCompletionOutput {
    Claude(hookkit_claude::catalog::StopOutput),
    Codex(hookkit_codex::catalog::StopOutput),
    Gemini(hookkit_gemini::catalog::AfterAgentOutput),
    Antigravity(hookkit_antigravity::StopOutput),
}

impl TurnCompletionOutput {
    pub fn harness(&self) -> HarnessId {
        match self {
            Self::Claude(_) => HarnessId::CLAUDE_CODE,
            Self::Codex(_) => HarnessId::CODEX,
            Self::Gemini(_) => HarnessId::GEMINI_CLI,
            Self::Antigravity(_) => HarnessId::ANTIGRAVITY,
        }
    }

    pub fn event_id(&self) -> EventId {
        match self {
            Self::Claude(_) => hookkit_claude::catalog::Stop::EVENT,
            Self::Codex(_) => hookkit_codex::catalog::Stop::EVENT,
            Self::Gemini(_) => hookkit_gemini::catalog::AfterAgent::EVENT,
            Self::Antigravity(_) => hookkit_antigravity::Stop::EVENT,
        }
    }
}

impl PostToolUseOutput {
    pub fn harness(&self) -> HarnessId {
        match self {
            Self::Claude(_) => HarnessId::CLAUDE_CODE,
            Self::Codex(_) => HarnessId::CODEX,
            Self::Gemini(_) => HarnessId::GEMINI_CLI,
            Self::Antigravity(_) => HarnessId::ANTIGRAVITY,
        }
    }

    pub fn event_id(&self) -> EventId {
        match self {
            Self::Claude(_) => hookkit_claude::protocol::PostToolUse::EVENT,
            Self::Codex(_) => hookkit_codex::protocol::PostToolUse::EVENT,
            Self::Gemini(_) => hookkit_gemini::protocol::AfterTool::EVENT,
            Self::Antigravity(_) => hookkit_antigravity::PostToolUse::EVENT,
        }
    }
}
