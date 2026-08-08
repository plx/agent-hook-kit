//! Lossless cross-harness wrappers for semantically aligned lifecycle events.

use hookkit_core::{EventId, EventSpec, HarnessId, HookkitError, Utf8Path};
use std::borrow::Cow;

/// Lossless native command-environment arms for aligned pre-tool execution.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum PreToolUseCommandEnvironment {
    /// Claude Code's native command environment.
    Claude(hookkit_claude::ClaudeCommandEnvironment),
    /// Codex's native command environment.
    Codex(hookkit_codex::CodexCommandEnvironment),
    /// Antigravity's native command environment.
    Antigravity(hookkit_antigravity::AntigravityCommandEnvironment),
}

impl PreToolUseCommandEnvironment {
    /// Returns the harness represented by this environment arm.
    pub fn harness(&self) -> HarnessId {
        match self {
            Self::Claude(_) => HarnessId::CLAUDE_CODE,
            Self::Codex(_) => HarnessId::CODEX,
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
    /// An arbitrary native JSON value, which may or may not be an object.
    Value(&'a serde_json::Value),
    /// A native JSON object guaranteed by the harness contract.
    Object(&'a serde_json::Map<String, serde_json::Value>),
}

impl<'a> ToolInputRef<'a> {
    /// Returns an object member, or `None` when a value arm is not an object.
    pub fn get(self, key: &str) -> Option<&'a serde_json::Value> {
        match self {
            Self::Value(value) => value.get(key),
            Self::Object(object) => object.get(key),
        }
    }

    /// Returns the input as an object when its native representation permits.
    pub fn as_object(self) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
        match self {
            Self::Value(value) => value.as_object(),
            Self::Object(object) => Some(object),
        }
    }
}

/// Lossless aligned pre-tool input.
///
/// All supported harnesses use the native event name `PreToolUse`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum PreToolUseInput {
    /// Claude Code's native `PreToolUse` input.
    Claude(hookkit_claude::catalog::CatalogInput),
    /// Codex's native `PreToolUse` input.
    Codex(hookkit_codex::protocol::PreToolUseInput),
    /// Antigravity's native `PreToolUse` input.
    Antigravity(hookkit_antigravity::PreToolUseInput),
}

impl PreToolUseInput {
    /// Returns the harness represented by this input arm.
    pub fn harness(&self) -> HarnessId {
        match self {
            Self::Claude(_) => HarnessId::CLAUDE_CODE,
            Self::Codex(_) => HarnessId::CODEX,
            Self::Antigravity(_) => HarnessId::ANTIGRAVITY,
        }
    }

    /// Returns the exact native event represented by this input arm.
    pub fn event_id(&self) -> EventId {
        match self {
            Self::Claude(_) => hookkit_claude::catalog::PreToolUse::EVENT,
            Self::Codex(_) => hookkit_codex::protocol::PreToolUse::EVENT,
            Self::Antigravity(_) => hookkit_antigravity::PreToolUse::EVENT,
        }
    }

    /// Returns workspace roots explicitly supplied by the native input.
    ///
    /// Single-working-directory harnesses require a one-element allocation;
    /// Antigravity's native root slice is borrowed.
    pub fn workspace_roots(&self) -> Cow<'_, [hookkit_core::Utf8PathBuf]> {
        match self {
            Self::Claude(input) => Cow::Owned(vec![input.cwd.clone()]),
            Self::Codex(input) => Cow::Owned(vec![input.cwd.clone()]),
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
            Self::Antigravity(_) => None,
        }
    }

    /// Return the native tool name when its value is a string.
    pub fn tool_name(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => input.field("tool_name").and_then(serde_json::Value::as_str),
            Self::Codex(input) => Some(&input.tool_name),
            Self::Antigravity(input) => Some(&input.tool_call.name),
        }
    }

    /// Borrow the complete native tool-input value without allocation.
    pub fn tool_input(&self) -> Option<ToolInputRef<'_>> {
        match self {
            Self::Claude(input) => input.field("tool_input").map(ToolInputRef::Value),
            Self::Codex(input) => Some(ToolInputRef::Value(&input.tool_input)),
            Self::Antigravity(input) => Some(ToolInputRef::Object(&input.tool_call.args)),
        }
    }
}

/// Lossless native pre-tool output arms. No generic serialized envelope is
/// introduced during lowering.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum PreToolUseOutput {
    /// Claude Code's native `PreToolUse` output.
    Claude(hookkit_claude::catalog::PreToolUseOutput),
    /// Codex's native `PreToolUse` output.
    Codex(hookkit_codex::protocol::PreToolUseOutput),
    /// Antigravity's native `PreToolUse` output.
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
                hookkit_codex::protocol::PreToolUseOutput::no_op(),
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
                hookkit_codex::protocol::PreToolUseOutput::deny(reason),
            )),
            "antigravity" => Ok(Self::Antigravity(hookkit_antigravity::PreToolUseOutput {
                decision: hookkit_antigravity::ToolDecision::Deny,
                reason: Some(reason),
                permission_overrides: Vec::new(),
            })),
            _ => Err(unsupported_pre_tool_harness(harness)),
        }
    }

    /// Build a Claude Code or Codex native allow response that replaces the
    /// complete tool-input object.
    ///
    /// Input replacement is not part of the universal three-harness portable
    /// floor: Antigravity's current pre-tool output has no equivalent field.
    /// Callers must therefore restrict this helper to the Claude/Codex tier.
    pub fn rewrite(
        harness: &HarnessId,
        updated_input: serde_json::Map<String, serde_json::Value>,
    ) -> hookkit_core::Result<Self> {
        match harness.as_str() {
            "claude-code" => Ok(Self::Claude(
                hookkit_claude::catalog::PreToolUseOutput::decide(
                    hookkit_claude::catalog::PreToolPermissionDecision::Allow,
                    None,
                    Some(updated_input),
                    None,
                ),
            )),
            "codex" => Ok(Self::Codex(
                hookkit_codex::protocol::PreToolUseOutput::rewrite(updated_input),
            )),
            _ => Err(unsupported_pre_tool_harness(harness)),
        }
    }

    /// Returns the harness represented by this output arm.
    pub fn harness(&self) -> HarnessId {
        match self {
            Self::Claude(_) => HarnessId::CLAUDE_CODE,
            Self::Codex(_) => HarnessId::CODEX,
            Self::Antigravity(_) => HarnessId::ANTIGRAVITY,
        }
    }

    /// Returns the exact native event represented by this output arm.
    pub fn event_id(&self) -> EventId {
        match self {
            Self::Claude(_) => hookkit_claude::catalog::PreToolUse::EVENT,
            Self::Codex(_) => hookkit_codex::protocol::PreToolUse::EVENT,
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
    /// Claude Code's native command environment.
    Claude(hookkit_claude::ClaudeCommandEnvironment),
    /// Codex's native command environment.
    Codex(hookkit_codex::CodexCommandEnvironment),
    /// Antigravity's native command environment.
    Antigravity(hookkit_antigravity::AntigravityCommandEnvironment),
}

impl PostToolUseCommandEnvironment {
    /// Returns the harness represented by this environment arm.
    pub fn harness(&self) -> HarnessId {
        match self {
            Self::Claude(_) => HarnessId::CLAUDE_CODE,
            Self::Codex(_) => HarnessId::CODEX,
            Self::Antigravity(_) => HarnessId::ANTIGRAVITY,
        }
    }
}

/// Lossless aligned input: every arm retains the complete native value.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum PostToolUseInput {
    /// Claude Code's native `PostToolUse` input.
    Claude(hookkit_claude::protocol::PostToolUseInput),
    /// Codex's native `PostToolUse` input.
    Codex(hookkit_codex::protocol::PostToolUseInput),
    /// Antigravity's native `PostToolUse` input.
    Antigravity(hookkit_antigravity::PostToolUseInput),
}

impl PostToolUseInput {
    /// Returns the harness represented by this input arm.
    pub fn harness(&self) -> HarnessId {
        match self {
            Self::Claude(_) => HarnessId::CLAUDE_CODE,
            Self::Codex(_) => HarnessId::CODEX,
            Self::Antigravity(_) => HarnessId::ANTIGRAVITY,
        }
    }

    /// Returns the exact native event represented by this input arm.
    pub fn event_id(&self) -> EventId {
        match self {
            Self::Claude(_) => hookkit_claude::protocol::PostToolUse::EVENT,
            Self::Codex(_) => hookkit_codex::protocol::PostToolUse::EVENT,
            Self::Antigravity(_) => hookkit_antigravity::PostToolUse::EVENT,
        }
    }

    /// Returns workspace roots explicitly supplied by the native input.
    pub fn workspace_roots(&self) -> Cow<'_, [hookkit_core::Utf8PathBuf]> {
        match self {
            Self::Claude(input) => Cow::Owned(vec![input.cwd.clone()]),
            Self::Codex(input) => Cow::Owned(vec![input.cwd.clone()]),
            Self::Antigravity(input) => Cow::Borrowed(&input.workspace_paths),
        }
    }
}

/// Lossless native output arms. No generic envelope participates in lowering.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum PostToolUseOutput {
    /// Claude Code's native `PostToolUse` output.
    Claude(hookkit_claude::protocol::PostToolUseOutput),
    /// Codex's native `PostToolUse` output.
    Codex(hookkit_codex::protocol::PostToolUseOutput),
    /// Antigravity's native `PostToolUse` output.
    Antigravity(hookkit_antigravity::PostToolUseOutput),
}

/// Lossless native command-environment arms for aligned turn completion.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum TurnCompletionCommandEnvironment {
    /// Claude Code's native command environment.
    Claude(hookkit_claude::ClaudeCommandEnvironment),
    /// Codex's native command environment.
    Codex(hookkit_codex::CodexCommandEnvironment),
    /// Antigravity's native command environment.
    Antigravity(hookkit_antigravity::AntigravityCommandEnvironment),
}

impl TurnCompletionCommandEnvironment {
    /// Returns the harness represented by this environment arm.
    pub fn harness(&self) -> HarnessId {
        match self {
            Self::Claude(_) => HarnessId::CLAUDE_CODE,
            Self::Codex(_) => HarnessId::CODEX,
            Self::Antigravity(_) => HarnessId::ANTIGRAVITY,
        }
    }
}

/// The event at which one agent turn is about to complete.
///
/// All supported harnesses use the native event name `Stop`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum TurnCompletionInput {
    /// Claude Code's native `Stop` input.
    Claude(hookkit_claude::catalog::CatalogInput),
    /// Codex's native `Stop` input.
    Codex(hookkit_codex::catalog::CatalogInput),
    /// Antigravity's native `Stop` input.
    Antigravity(hookkit_antigravity::StopInput),
}

impl TurnCompletionInput {
    /// Returns the harness represented by this input arm.
    pub fn harness(&self) -> HarnessId {
        match self {
            Self::Claude(_) => HarnessId::CLAUDE_CODE,
            Self::Codex(_) => HarnessId::CODEX,
            Self::Antigravity(_) => HarnessId::ANTIGRAVITY,
        }
    }

    /// Returns the exact native event represented by this input arm.
    pub fn event_id(&self) -> EventId {
        match self {
            Self::Claude(_) => hookkit_claude::catalog::Stop::EVENT,
            Self::Codex(_) => hookkit_codex::catalog::Stop::EVENT,
            Self::Antigravity(_) => hookkit_antigravity::Stop::EVENT,
        }
    }

    /// Returns workspace roots explicitly supplied by the native input.
    pub fn workspace_roots(&self) -> Cow<'_, [hookkit_core::Utf8PathBuf]> {
        match self {
            Self::Claude(input) => Cow::Owned(vec![input.cwd.clone()]),
            Self::Codex(input) => Cow::Owned(vec![input.cwd.clone()]),
            Self::Antigravity(input) => Cow::Borrowed(&input.workspace_paths),
        }
    }
}

/// Lossless native turn-completion outputs.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum TurnCompletionOutput {
    /// Claude Code's native `Stop` output.
    Claude(hookkit_claude::catalog::StopOutput),
    /// Codex's native `Stop` output.
    Codex(hookkit_codex::catalog::StopOutput),
    /// Antigravity's native `Stop` output.
    Antigravity(hookkit_antigravity::StopOutput),
}

impl TurnCompletionOutput {
    /// Build the selected harness's native response that permits turn
    /// completion without adding a message.
    pub fn allow(harness: &HarnessId) -> hookkit_core::Result<Self> {
        match harness.as_str() {
            "claude-code" => Ok(Self::Claude(hookkit_claude::catalog::StopOutput::no_op())),
            "codex" => Ok(Self::Codex(hookkit_codex::catalog::StopOutput::no_op())),
            "antigravity" => Ok(Self::Antigravity(hookkit_antigravity::StopOutput {
                decision: String::from("stop"),
                reason: None,
            })),
            _ => Err(HookkitError::UnrecognizedEvent {
                harness: harness.clone(),
                message: "no aligned turn-completion adapter is registered".into(),
            }),
        }
    }

    /// Returns the harness represented by this output arm.
    pub fn harness(&self) -> HarnessId {
        match self {
            Self::Claude(_) => HarnessId::CLAUDE_CODE,
            Self::Codex(_) => HarnessId::CODEX,
            Self::Antigravity(_) => HarnessId::ANTIGRAVITY,
        }
    }

    /// Returns the exact native event represented by this output arm.
    pub fn event_id(&self) -> EventId {
        match self {
            Self::Claude(_) => hookkit_claude::catalog::Stop::EVENT,
            Self::Codex(_) => hookkit_codex::catalog::Stop::EVENT,
            Self::Antigravity(_) => hookkit_antigravity::Stop::EVENT,
        }
    }
}

impl PostToolUseOutput {
    /// Build the selected harness's native post-tool no-op response.
    pub fn no_op(harness: &HarnessId) -> hookkit_core::Result<Self> {
        match harness.as_str() {
            "claude-code" => Ok(Self::Claude(
                hookkit_claude::protocol::PostToolUseOutput::no_op(),
            )),
            "codex" => Ok(Self::Codex(
                hookkit_codex::protocol::PostToolUseOutput::no_op(),
            )),
            "antigravity" => Ok(Self::Antigravity(
                hookkit_antigravity::PostToolUseOutput::default(),
            )),
            _ => Err(HookkitError::UnrecognizedEvent {
                harness: harness.clone(),
                message: "no aligned post-tool adapter is registered".into(),
            }),
        }
    }

    /// Returns the harness represented by this output arm.
    pub fn harness(&self) -> HarnessId {
        match self {
            Self::Claude(_) => HarnessId::CLAUDE_CODE,
            Self::Codex(_) => HarnessId::CODEX,
            Self::Antigravity(_) => HarnessId::ANTIGRAVITY,
        }
    }

    /// Returns the exact native event represented by this output arm.
    pub fn event_id(&self) -> EventId {
        match self {
            Self::Claude(_) => hookkit_claude::protocol::PostToolUse::EVENT,
            Self::Codex(_) => hookkit_codex::protocol::PostToolUse::EVENT,
            Self::Antigravity(_) => hookkit_antigravity::PostToolUse::EVENT,
        }
    }
}

macro_rules! claude_codex_alignment {
    (
        $(#[$environment_meta:meta])*
        $environment:ident,
        $(#[$input_meta:meta])*
        $input:ident,
        $(#[$output_meta:meta])*
        $output:ident,
        $family:literal,
        claude($claude_event:ty, $claude_input:ty, $claude_output:ty),
        codex($codex_event:ty, $codex_input:ty, $codex_output:ty)
    ) => {
        $(#[$environment_meta])*
        #[derive(Debug, Clone)]
        #[non_exhaustive]
        pub enum $environment {
            /// Claude Code's native command environment.
            Claude(hookkit_claude::ClaudeCommandEnvironment),
            /// Codex's native command environment.
            Codex(hookkit_codex::CodexCommandEnvironment),
        }

        impl $environment {
            /// Returns the harness represented by this environment arm.
            pub fn harness(&self) -> HarnessId {
                match self {
                    Self::Claude(_) => HarnessId::CLAUDE_CODE,
                    Self::Codex(_) => HarnessId::CODEX,
                }
            }
        }

        $(#[$input_meta])*
        #[derive(Debug, Clone)]
        #[non_exhaustive]
        pub enum $input {
            /// Claude Code's complete native input.
            Claude($claude_input),
            /// Codex's complete native input.
            Codex($codex_input),
        }

        impl $input {
            /// Returns the harness represented by this input arm.
            pub fn harness(&self) -> HarnessId {
                match self {
                    Self::Claude(_) => HarnessId::CLAUDE_CODE,
                    Self::Codex(_) => HarnessId::CODEX,
                }
            }

            /// Returns the exact native event represented by this input arm.
            pub fn event_id(&self) -> EventId {
                match self {
                    Self::Claude(_) => <$claude_event as EventSpec>::EVENT,
                    Self::Codex(_) => <$codex_event as EventSpec>::EVENT,
                }
            }

            /// Returns the native session identifier.
            pub fn session_id(&self) -> &str {
                match self {
                    Self::Claude(input) => &input.session_id,
                    Self::Codex(input) => &input.session_id,
                }
            }

            /// Returns the exact native working directory.
            pub fn cwd(&self) -> &Utf8Path {
                match self {
                    Self::Claude(input) => &input.cwd,
                    Self::Codex(input) => &input.cwd,
                }
            }

            /// Returns the workspace root supplied by the native input.
            pub fn workspace_roots(&self) -> Cow<'_, [hookkit_core::Utf8PathBuf]> {
                match self {
                    Self::Claude(input) => Cow::Borrowed(std::slice::from_ref(&input.cwd)),
                    Self::Codex(input) => Cow::Borrowed(std::slice::from_ref(&input.cwd)),
                }
            }
        }

        $(#[$output_meta])*
        #[derive(Debug, Clone)]
        #[non_exhaustive]
        pub enum $output {
            /// Claude Code's complete native output.
            Claude($claude_output),
            /// Codex's complete native output.
            Codex($codex_output),
        }

        impl $output {
            /// Returns the harness represented by this output arm.
            pub fn harness(&self) -> HarnessId {
                match self {
                    Self::Claude(_) => HarnessId::CLAUDE_CODE,
                    Self::Codex(_) => HarnessId::CODEX,
                }
            }

            /// Returns the exact native event represented by this output arm.
            pub fn event_id(&self) -> EventId {
                match self {
                    Self::Claude(_) => <$claude_event as EventSpec>::EVENT,
                    Self::Codex(_) => <$codex_event as EventSpec>::EVENT,
                }
            }
        }
    };
}

claude_codex_alignment!(
    /// Lossless command-environment arms for aligned permission requests.
    PermissionRequestCommandEnvironment,
    /// Lossless aligned permission-request input.
    PermissionRequestInput,
    /// Lossless native permission-request output arms.
    PermissionRequestOutput,
    "PermissionRequest",
    claude(
        hookkit_claude::catalog::PermissionRequest,
        hookkit_claude::catalog::CatalogInput,
        hookkit_claude::catalog::PermissionRequestOutput
    ),
    codex(
        hookkit_codex::catalog::PermissionRequest,
        hookkit_codex::catalog::CatalogInput,
        hookkit_codex::catalog::PermissionRequestOutput
    )
);

impl PermissionRequestOutput {
    /// Builds the selected harness's native approval response.
    pub fn allow(harness: &HarnessId) -> hookkit_core::Result<Self> {
        match harness.as_str() {
            "claude-code" => Ok(Self::Claude(
                hookkit_claude::catalog::PermissionRequestOutput::decide(
                    hookkit_claude::catalog::PermissionRequestBehavior::Allow,
                    None,
                    None,
                ),
            )),
            "codex" => Ok(Self::Codex(
                hookkit_codex::catalog::PermissionRequestOutput::allow(),
            )),
            _ => Err(unsupported_pair_harness(harness, "PermissionRequest")),
        }
    }

    /// Builds the selected harness's native denial response.
    pub fn deny(harness: &HarnessId, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        let reason = reason.into();
        match harness.as_str() {
            "claude-code" => Ok(Self::Claude(
                hookkit_claude::catalog::PermissionRequestOutput::decide(
                    hookkit_claude::catalog::PermissionRequestBehavior::Deny,
                    Some(reason),
                    None,
                ),
            )),
            "codex" => Ok(Self::Codex(
                hookkit_codex::catalog::PermissionRequestOutput::deny(reason),
            )),
            _ => Err(unsupported_pair_harness(harness, "PermissionRequest")),
        }
    }
}

impl PermissionRequestInput {
    /// Returns the harness-native tool name.
    pub fn tool_name(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => input.field("tool_name").and_then(serde_json::Value::as_str),
            Self::Codex(input) => input.field("tool_name").and_then(serde_json::Value::as_str),
        }
    }

    /// Returns the complete native tool-input value.
    pub fn tool_input(&self) -> Option<&serde_json::Value> {
        match self {
            Self::Claude(input) => input.field("tool_input"),
            Self::Codex(input) => input.field("tool_input"),
        }
    }
}

claude_codex_alignment!(
    /// Lossless command-environment arms for aligned pre-compaction hooks.
    PreCompactCommandEnvironment,
    /// Lossless aligned pre-compaction input.
    PreCompactInput,
    /// Lossless native pre-compaction output arms.
    PreCompactOutput,
    "PreCompact",
    claude(
        hookkit_claude::catalog::PreCompact,
        hookkit_claude::catalog::CatalogInput,
        hookkit_claude::catalog::PreCompactOutput
    ),
    codex(
        hookkit_codex::catalog::PreCompact,
        hookkit_codex::catalog::CatalogInput,
        hookkit_codex::catalog::PreCompactOutput
    )
);

impl PreCompactOutput {
    /// Builds the selected harness's native observer/no-op response.
    ///
    /// Deliberately no portable block helper is provided: Claude's
    /// compaction block and Codex's broader execution stop are not equivalent.
    pub fn no_op(harness: &HarnessId) -> hookkit_core::Result<Self> {
        match harness.as_str() {
            "claude-code" => Ok(Self::Claude(
                hookkit_claude::catalog::PreCompactOutput::no_op(),
            )),
            "codex" => Ok(Self::Codex(
                hookkit_codex::catalog::PreCompactOutput::no_op(),
            )),
            _ => Err(unsupported_pair_harness(harness, "PreCompact")),
        }
    }
}

impl PreCompactInput {
    /// Returns the native compaction trigger when it is a string.
    pub fn trigger(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => input.field("trigger").and_then(serde_json::Value::as_str),
            Self::Codex(input) => input.field("trigger").and_then(serde_json::Value::as_str),
        }
    }
}

claude_codex_alignment!(
    /// Lossless command-environment arms for aligned post-compaction hooks.
    PostCompactCommandEnvironment,
    /// Lossless aligned post-compaction input.
    PostCompactInput,
    /// Lossless native post-compaction output arms.
    PostCompactOutput,
    "PostCompact",
    claude(
        hookkit_claude::catalog::PostCompact,
        hookkit_claude::catalog::CatalogInput,
        hookkit_claude::catalog::PostCompactOutput
    ),
    codex(
        hookkit_codex::catalog::PostCompact,
        hookkit_codex::catalog::CatalogInput,
        hookkit_codex::catalog::PostCompactOutput
    )
);

impl PostCompactOutput {
    /// Builds the selected harness's native observer/no-op response.
    pub fn no_op(harness: &HarnessId) -> hookkit_core::Result<Self> {
        match harness.as_str() {
            "claude-code" => Ok(Self::Claude(
                hookkit_claude::catalog::PostCompactOutput::no_op(),
            )),
            "codex" => Ok(Self::Codex(
                hookkit_codex::catalog::PostCompactOutput::no_op(),
            )),
            _ => Err(unsupported_pair_harness(harness, "PostCompact")),
        }
    }

    /// Builds the selected harness's native optional system-notice response.
    pub fn with_system_notice(
        harness: &HarnessId,
        message: impl Into<String>,
    ) -> hookkit_core::Result<Self> {
        let message = message.into();
        match harness.as_str() {
            "claude-code" => Ok(Self::Claude(
                hookkit_claude::catalog::PostCompactOutput::with_system_message(message),
            )),
            "codex" => hookkit_codex::catalog::PostCompactOutput::no_op()
                .with_system_message(message)
                .map(Self::Codex),
            _ => Err(unsupported_pair_harness(harness, "PostCompact")),
        }
    }
}

impl PostCompactInput {
    /// Returns the native compaction trigger when it is a string.
    pub fn trigger(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => input.field("trigger").and_then(serde_json::Value::as_str),
            Self::Codex(input) => input.field("trigger").and_then(serde_json::Value::as_str),
        }
    }
}

claude_codex_alignment!(
    /// Lossless command-environment arms for aligned session-start hooks.
    SessionStartCommandEnvironment,
    /// Lossless aligned session-start input.
    SessionStartInput,
    /// Lossless native session-start output arms.
    SessionStartOutput,
    "SessionStart",
    claude(
        hookkit_claude::protocol::SessionStart,
        hookkit_claude::protocol::SessionStartInput,
        hookkit_claude::protocol::SessionStartOutput
    ),
    codex(
        hookkit_codex::catalog::SessionStart,
        hookkit_codex::catalog::CatalogInput,
        hookkit_codex::catalog::SessionStartOutput
    )
);

impl SessionStartOutput {
    /// Builds the selected harness's native observer/no-op response.
    pub fn no_op(harness: &HarnessId) -> hookkit_core::Result<Self> {
        match harness.as_str() {
            "claude-code" => Ok(Self::Claude(
                hookkit_claude::protocol::SessionStartOutput::no_op(),
            )),
            "codex" => Ok(Self::Codex(
                hookkit_codex::catalog::SessionStartOutput::no_op(),
            )),
            _ => Err(unsupported_pair_harness(harness, "SessionStart")),
        }
    }

    /// Builds the selected harness's native agent-context response.
    pub fn with_context(
        harness: &HarnessId,
        context: impl Into<String>,
    ) -> hookkit_core::Result<Self> {
        let context = context.into();
        match harness.as_str() {
            "claude-code" => Ok(Self::Claude(
                hookkit_claude::protocol::SessionStartOutput::with_context(context),
            )),
            "codex" => Ok(Self::Codex(
                hookkit_codex::catalog::SessionStartOutput::with_context(context),
            )),
            _ => Err(unsupported_pair_harness(harness, "SessionStart")),
        }
    }
}

impl SessionStartInput {
    /// Returns the native session-start source as its wire spelling.
    pub fn source(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => Some(match input.source {
                hookkit_claude::protocol::SessionSource::Startup => "startup",
                hookkit_claude::protocol::SessionSource::Resume => "resume",
                hookkit_claude::protocol::SessionSource::Fork => "fork",
                hookkit_claude::protocol::SessionSource::Clear => "clear",
                hookkit_claude::protocol::SessionSource::Compact => "compact",
            }),
            Self::Codex(input) => input.field("source").and_then(serde_json::Value::as_str),
        }
    }
}

claude_codex_alignment!(
    /// Lossless command-environment arms for aligned session-end hooks.
    SessionEndCommandEnvironment,
    /// Lossless aligned session-end input.
    SessionEndInput,
    /// Lossless native session-end output arms.
    SessionEndOutput,
    "SessionEnd",
    claude(
        hookkit_claude::catalog::SessionEnd,
        hookkit_claude::catalog::CatalogInput,
        hookkit_claude::catalog::SessionEndOutput
    ),
    codex(
        hookkit_codex::catalog::SessionEnd,
        hookkit_codex::catalog::CatalogInput,
        hookkit_codex::catalog::SessionEndOutput
    )
);

impl SessionEndOutput {
    /// Builds the selected harness's native observer/no-op response.
    ///
    /// Codex ignores session-end output and therefore emits no stdout.
    pub fn no_op(harness: &HarnessId) -> hookkit_core::Result<Self> {
        match harness.as_str() {
            "claude-code" => Ok(Self::Claude(
                hookkit_claude::catalog::SessionEndOutput::no_op(),
            )),
            "codex" => Ok(Self::Codex(
                hookkit_codex::catalog::SessionEndOutput::no_op(),
            )),
            _ => Err(unsupported_pair_harness(harness, "SessionEnd")),
        }
    }
}

impl SessionEndInput {
    /// Returns the native session-end reason when it is a string.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => input.field("reason").and_then(serde_json::Value::as_str),
            Self::Codex(input) => input.field("reason").and_then(serde_json::Value::as_str),
        }
    }
}

claude_codex_alignment!(
    /// Lossless command-environment arms for aligned subagent-start hooks.
    SubagentStartCommandEnvironment,
    /// Lossless aligned subagent-start input.
    SubagentStartInput,
    /// Lossless native subagent-start output arms.
    SubagentStartOutput,
    "SubagentStart",
    claude(
        hookkit_claude::catalog::SubagentStart,
        hookkit_claude::catalog::CatalogInput,
        hookkit_claude::catalog::SubagentStartOutput
    ),
    codex(
        hookkit_codex::catalog::SubagentStart,
        hookkit_codex::catalog::CatalogInput,
        hookkit_codex::catalog::SubagentStartOutput
    )
);

impl SubagentStartOutput {
    /// Builds the selected harness's native observer/no-op response.
    pub fn no_op(harness: &HarnessId) -> hookkit_core::Result<Self> {
        match harness.as_str() {
            "claude-code" => Ok(Self::Claude(
                hookkit_claude::catalog::SubagentStartOutput::no_op(),
            )),
            "codex" => Ok(Self::Codex(
                hookkit_codex::catalog::SubagentStartOutput::no_op(),
            )),
            _ => Err(unsupported_pair_harness(harness, "SubagentStart")),
        }
    }

    /// Builds the selected harness's native agent-context response.
    pub fn with_context(
        harness: &HarnessId,
        context: impl Into<String>,
    ) -> hookkit_core::Result<Self> {
        let context = context.into();
        match harness.as_str() {
            "claude-code" => Ok(Self::Claude(
                hookkit_claude::catalog::SubagentStartOutput::with_context(context),
            )),
            "codex" => Ok(Self::Codex(
                hookkit_codex::catalog::SubagentStartOutput::with_context(context),
            )),
            _ => Err(unsupported_pair_harness(harness, "SubagentStart")),
        }
    }
}

impl SubagentStartInput {
    /// Returns the native subagent identifier when it is a string.
    pub fn agent_id(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => input.field("agent_id").and_then(serde_json::Value::as_str),
            Self::Codex(input) => input.field("agent_id").and_then(serde_json::Value::as_str),
        }
    }

    /// Returns the native subagent type when it is a string.
    pub fn agent_type(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => input
                .field("agent_type")
                .and_then(serde_json::Value::as_str),
            Self::Codex(input) => input
                .field("agent_type")
                .and_then(serde_json::Value::as_str),
        }
    }
}

claude_codex_alignment!(
    /// Lossless command-environment arms for aligned subagent-stop hooks.
    SubagentStopCommandEnvironment,
    /// Lossless aligned subagent-stop input.
    SubagentStopInput,
    /// Lossless native subagent-stop output arms.
    SubagentStopOutput,
    "SubagentStop",
    claude(
        hookkit_claude::catalog::SubagentStop,
        hookkit_claude::catalog::CatalogInput,
        hookkit_claude::catalog::SubagentStopOutput
    ),
    codex(
        hookkit_codex::catalog::SubagentStop,
        hookkit_codex::catalog::CatalogInput,
        hookkit_codex::catalog::SubagentStopOutput
    )
);

impl SubagentStopOutput {
    /// Builds the selected harness's native observer/no-op response.
    pub fn no_op(harness: &HarnessId) -> hookkit_core::Result<Self> {
        match harness.as_str() {
            "claude-code" => Ok(Self::Claude(
                hookkit_claude::catalog::SubagentStopOutput::no_op(),
            )),
            "codex" => Ok(Self::Codex(
                hookkit_codex::catalog::SubagentStopOutput::no_op(),
            )),
            _ => Err(unsupported_pair_harness(harness, "SubagentStop")),
        }
    }

    /// Builds the selected harness's native block response with a reason.
    pub fn block(harness: &HarnessId, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        let reason = reason.into();
        match harness.as_str() {
            "claude-code" => Ok(Self::Claude(
                hookkit_claude::catalog::SubagentStopOutput::block(reason),
            )),
            "codex" => Ok(Self::Codex(
                hookkit_codex::catalog::SubagentStopOutput::block(reason),
            )),
            _ => Err(unsupported_pair_harness(harness, "SubagentStop")),
        }
    }
}

impl SubagentStopInput {
    /// Returns the native subagent identifier when it is a string.
    pub fn agent_id(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => input.field("agent_id").and_then(serde_json::Value::as_str),
            Self::Codex(input) => input.field("agent_id").and_then(serde_json::Value::as_str),
        }
    }

    /// Returns the native subagent type when it is a string.
    pub fn agent_type(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => input
                .field("agent_type")
                .and_then(serde_json::Value::as_str),
            Self::Codex(input) => input
                .field("agent_type")
                .and_then(serde_json::Value::as_str),
        }
    }
}

claude_codex_alignment!(
    /// Lossless command-environment arms for aligned user-prompt hooks.
    UserPromptSubmitCommandEnvironment,
    /// Lossless aligned user-prompt-submit input.
    UserPromptSubmitInput,
    /// Lossless native user-prompt-submit output arms.
    UserPromptSubmitOutput,
    "UserPromptSubmit",
    claude(
        hookkit_claude::catalog::UserPromptSubmit,
        hookkit_claude::catalog::CatalogInput,
        hookkit_claude::catalog::UserPromptSubmitOutput
    ),
    codex(
        hookkit_codex::catalog::UserPromptSubmit,
        hookkit_codex::catalog::CatalogInput,
        hookkit_codex::catalog::UserPromptSubmitOutput
    )
);

impl UserPromptSubmitOutput {
    /// Builds the selected harness's native observer/no-op response.
    pub fn no_op(harness: &HarnessId) -> hookkit_core::Result<Self> {
        match harness.as_str() {
            "claude-code" => Ok(Self::Claude(
                hookkit_claude::catalog::UserPromptSubmitOutput::no_op(),
            )),
            "codex" => Ok(Self::Codex(
                hookkit_codex::catalog::UserPromptSubmitOutput::no_op(),
            )),
            _ => Err(unsupported_pair_harness(harness, "UserPromptSubmit")),
        }
    }

    /// Builds the selected harness's native agent-context response.
    pub fn with_context(
        harness: &HarnessId,
        context: impl Into<String>,
    ) -> hookkit_core::Result<Self> {
        let context = context.into();
        match harness.as_str() {
            "claude-code" => Ok(Self::Claude(
                hookkit_claude::catalog::UserPromptSubmitOutput::with_context(context),
            )),
            "codex" => Ok(Self::Codex(
                hookkit_codex::catalog::UserPromptSubmitOutput::text_context(context),
            )),
            _ => Err(unsupported_pair_harness(harness, "UserPromptSubmit")),
        }
    }

    /// Blocks prompt submission with a reason using each harness's code-2
    /// feedback path.
    pub fn block(harness: &HarnessId, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        let reason = reason.into();
        match harness.as_str() {
            "claude-code" => Ok(Self::Claude(
                hookkit_claude::catalog::UserPromptSubmitOutput::blocking_error(reason),
            )),
            "codex" => Ok(Self::Codex(
                hookkit_codex::catalog::UserPromptSubmitOutput::blocking_error(reason),
            )),
            _ => Err(unsupported_pair_harness(harness, "UserPromptSubmit")),
        }
    }
}

impl UserPromptSubmitInput {
    /// Returns the submitted user prompt.
    pub fn prompt(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => input.field("prompt").and_then(serde_json::Value::as_str),
            Self::Codex(input) => input.field("prompt").and_then(serde_json::Value::as_str),
        }
    }
}

fn unsupported_pair_harness(harness: &HarnessId, family: &'static str) -> HookkitError {
    HookkitError::UnrecognizedEvent {
        harness: harness.clone(),
        message: format!("no aligned {family} adapter is registered"),
    }
}
