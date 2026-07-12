use std::borrow::Cow;
use std::fmt;

/// Stable, open harness identity. Built-ins use borrowed constants; dialects may
/// use owned names without modifying a central enum.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HarnessId(Cow<'static, str>);

impl HarnessId {
    pub const CLAUDE_CODE: Self = Self(Cow::Borrowed("claude-code"));
    pub const CODEX: Self = Self(Cow::Borrowed("codex"));
    pub const GEMINI_CLI: Self = Self(Cow::Borrowed("gemini-cli"));
    pub const ANTIGRAVITY: Self = Self(Cow::Borrowed("antigravity"));

    pub const fn builtin(value: &'static str) -> Self {
        Self(Cow::Borrowed(value))
    }

    pub fn new(value: impl Into<String>) -> crate::Result<Self> {
        let value = value.into();
        if value.is_empty() {
            return Err(crate::HookkitError::InvalidIdentity("empty harness id"));
        }
        Ok(Self(Cow::Owned(value)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HarnessId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Convenience selection for the built-in harness adapters.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinHarness {
    ClaudeCode,
    Codex,
    GeminiCli,
    Antigravity,
}

impl BuiltinHarness {
    pub const fn id(self) -> HarnessId {
        match self {
            Self::ClaudeCode => HarnessId::CLAUDE_CODE,
            Self::Codex => HarnessId::CODEX,
            Self::GeminiCli => HarnessId::GEMINI_CLI,
            Self::Antigravity => HarnessId::ANTIGRAVITY,
        }
    }
}

/// Immutable catalog snapshot identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SnapshotId(&'static str);

impl SnapshotId {
    pub const fn builtin(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for SnapshotId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

/// Immutable event/binding contract identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContractId(&'static str);

impl ContractId {
    pub const fn builtin(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl fmt::Display for ContractId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

/// Exact wire event identity, scoped to its harness.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventId {
    harness: HarnessId,
    name: Cow<'static, str>,
}

impl EventId {
    pub const fn builtin(harness: HarnessId, name: &'static str) -> Self {
        Self {
            harness,
            name: Cow::Borrowed(name),
        }
    }

    pub fn new(harness: HarnessId, name: impl Into<String>) -> crate::Result<Self> {
        let name = name.into();
        if name.is_empty() {
            return Err(crate::HookkitError::InvalidIdentity("empty event id"));
        }
        Ok(Self {
            harness,
            name: Cow::Owned(name),
        })
    }

    pub fn harness(&self) -> &HarnessId {
        &self.harness
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.harness, self.name)
    }
}

/// A genuinely aligned lifecycle concept. It never substitutes for [`EventId`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlignedEventKind {
    SessionStart,
    PreToolUse,
    PostToolUse,
    Stop,
}

/// Declared wire ancestry. It is documentation/reuse metadata, never permission
/// to fall back to the parent parser after a dialect parse fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialectLineage {
    pub dialect: HarnessId,
    pub parent: HarnessId,
    pub snapshot: SnapshotId,
}
