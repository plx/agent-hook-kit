use std::fmt;

/// Identifies which coding-agent harness is in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Harness {
    Claude,
    Codex,
    Gemini,
}

impl fmt::Display for Harness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Harness::Claude => write!(f, "Claude"),
            Harness::Codex => write!(f, "Codex"),
            Harness::Gemini => write!(f, "Gemini"),
        }
    }
}

impl Harness {
    pub const fn id(self) -> crate::HarnessId {
        match self {
            Self::Claude => crate::HarnessId::CLAUDE_CODE,
            Self::Codex => crate::HarnessId::CODEX,
            Self::Gemini => crate::HarnessId::GEMINI_CLI,
        }
    }
}
