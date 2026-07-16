use hookkit_core::{EventId, EventSpec, HarnessId};
use std::borrow::Cow;

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
