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

/// Exact wire event identity, intentionally separate from aligned categories.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventId(Cow<'static, str>);

impl EventId {
    pub const fn builtin(value: &'static str) -> Self {
        Self(Cow::Borrowed(value))
    }

    pub fn new(value: impl Into<String>) -> crate::Result<Self> {
        let value = value.into();
        if value.is_empty() {
            return Err(crate::HookkitError::InvalidIdentity("empty event id"));
        }
        Ok(Self(Cow::Owned(value)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Declared wire ancestry. It is documentation/reuse metadata, never permission
/// to fall back to the parent parser after a dialect parse fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialectLineage {
    pub dialect: HarnessId,
    pub parent: HarnessId,
    pub snapshot: &'static str,
}
