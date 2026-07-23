use std::borrow::Cow;
use std::fmt;

/// Stable, open harness identity. Built-ins use borrowed constants; dialects may
/// use owned names without modifying a central enum.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HarnessId(Cow<'static, str>);

impl HarnessId {
    /// Built-in identity for Claude Code.
    pub const CLAUDE_CODE: Self = Self(Cow::Borrowed("claude-code"));
    /// Built-in identity for Codex CLI.
    pub const CODEX: Self = Self(Cow::Borrowed("codex"));
    /// Built-in identity for Gemini CLI.
    pub const GEMINI_CLI: Self = Self(Cow::Borrowed("gemini-cli"));
    /// Built-in identity for Antigravity.
    pub const ANTIGRAVITY: Self = Self(Cow::Borrowed("antigravity"));

    /// Creates a borrowed identity for a statically known harness.
    ///
    /// This constructor does not validate `value`; callers defining custom
    /// static identities must preserve the non-empty invariant themselves.
    pub const fn builtin(value: &'static str) -> Self {
        Self(Cow::Borrowed(value))
    }

    /// Creates an owned harness identity, rejecting the empty string.
    pub fn new(value: impl Into<String>) -> crate::Result<Self> {
        let value = value.into();
        if value.is_empty() {
            return Err(crate::HookkitError::InvalidIdentity("empty harness id"));
        }
        Ok(Self(Cow::Owned(value)))
    }

    /// Returns the wire identity string.
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
    /// Claude Code's hook protocol.
    ClaudeCode,
    /// Codex CLI's hook protocol.
    Codex,
    /// Gemini CLI's hook protocol.
    GeminiCli,
    /// Antigravity's hook protocol.
    Antigravity,
}

impl BuiltinHarness {
    /// Returns the stable open identity for this built-in harness.
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
    /// Creates an identity for a statically defined catalog snapshot.
    pub const fn builtin(value: &'static str) -> Self {
        Self(value)
    }

    /// Returns the snapshot identity string.
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
    /// Creates an identity for a statically defined event/binding contract.
    pub const fn builtin(value: &'static str) -> Self {
        Self(value)
    }

    /// Returns the contract identity string.
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
    /// Creates a statically named event belonging to `harness`.
    ///
    /// This constructor does not validate `name`; catalog authors must supply
    /// a non-empty wire event name.
    pub const fn builtin(harness: HarnessId, name: &'static str) -> Self {
        Self {
            harness,
            name: Cow::Borrowed(name),
        }
    }

    /// Creates a dynamically named event, rejecting an empty name.
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

    /// Returns the harness that scopes this event identity.
    pub fn harness(&self) -> &HarnessId {
        &self.harness
    }

    /// Returns the harness-native event name.
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
    /// A session or conversation was initialized.
    SessionStart,
    /// A tool invocation is about to execute.
    PreToolUse,
    /// A tool invocation finished.
    PostToolUse,
    /// A turn is attempting to stop.
    Stop,
}

/// Declared wire ancestry. It is documentation/reuse metadata, never permission
/// to fall back to the parent parser after a dialect parse fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialectLineage {
    /// Identity of the derived protocol dialect.
    pub dialect: HarnessId,
    /// Identity of the protocol from which the dialect was derived.
    pub parent: HarnessId,
    /// Exact catalog snapshot that declares this relationship.
    pub snapshot: SnapshotId,
}
