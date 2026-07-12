use hookkit_core::HarnessId;

/// Lossless aligned input: every arm retains the complete native value.
#[derive(Debug, Clone)]
pub enum PostToolUseInput {
    Claude(hookkit_claude::input::PostToolUse),
    Codex(hookkit_codex::input::PostToolUse),
    Gemini(hookkit_gemini::input::AfterTool),
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

    pub fn workspace_roots(&self) -> Option<&[hookkit_core::Utf8PathBuf]> {
        match self {
            Self::Antigravity(input) => Some(&input.workspace_paths),
            _ => None,
        }
    }
}

/// Lossless native output arms. No generic envelope participates in lowering.
#[derive(Debug, Clone)]
pub enum PostToolUseOutput {
    Claude(hookkit_claude::ClaudeHookOutput),
    Codex(hookkit_codex::CodexHookOutput),
    Gemini(hookkit_gemini::GeminiHookOutput),
    Antigravity(hookkit_antigravity::PostToolUseOutput),
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
}
