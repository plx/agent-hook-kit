//! Lossless cross-harness wrappers for semantically aligned lifecycle events.
//!
//! Each family wraps the complete native input, command environment, and
//! output of every harness that supports it. Nothing is flattened into a
//! common wire shape: a handler can always match an arm and reach every
//! native field. The accessors and output helpers here are conveniences for
//! the parts the harnesses genuinely share, and their documentation records
//! where the harnesses still differ.
//!
//! # Choosing a pre-tool response
//!
//! [`PreToolUseOutput`] offers one helper per intent. They are not
//! interchangeable, because the harnesses give "allow" and "no answer"
//! different meanings:
//!
//! | Helper | Claude Code | Codex | Antigravity |
//! | :- | :- | :- | :- |
//! | [`PreToolUseOutput::pass_through`] | empty output: the normal permission flow decides | empty stdout: the normal approval flow decides | `{"decision":"ask"}`: prompts unless an "Always Allow" grant covers the call |
//! | [`PreToolUseOutput::allow`] | `permissionDecision: "allow"`: auto-approves and skips the permission prompt | no equivalent; lowers to empty stdout, so the normal approval flow decides | `{"decision":"allow"}`: auto-approves and bypasses Ask presets |
//! | [`PreToolUseOutput::deny`] | `permissionDecision: "deny"`; the reason is shown to Claude | `permissionDecision: "deny"` | `{"decision":"deny"}` with the reason |
//! | [`PreToolUseOutput::rewrite`] | `updatedInput` with the caller's [`RewriteApproval`] | `allow` plus `updatedInput`: replaces the input; approval policy still runs | unsupported |
//!
//! A guard that only objects to some calls must answer every other call with
//! [`PreToolUseOutput::pass_through`]. Answering with
//! [`PreToolUseOutput::allow`] turns the guard into an auto-approver on Claude
//! Code and Antigravity.
//!
//! # Roots and working directories
//!
//! The harnesses report locations differently:
//!
//! - Claude Code's input `cwd` is the agent's *current* directory. It follows
//!   `cd` in the Bash tool and moves into a worktree when Claude enters one.
//!   The stable project root where the session started is only available as
//!   `CLAUDE_PROJECT_DIR` in the command environment.
//! - Codex's input `cwd` is the turn's configured working directory.
//! - Antigravity sends `workspacePaths`, the mounted workspace roots (possibly
//!   none), and no working directory.
//!
//! `workspace_roots()` reports what the native input carries: `[cwd]` for
//! Claude Code and Codex, and `workspacePaths` for Antigravity. It matches
//! [`hookkit_core::RuntimeContext::workspace_roots`]. `project_roots()` takes
//! the command environment as well and returns the stable roots:
//! `CLAUDE_PROJECT_DIR` for Claude Code, `cwd` for Codex, and `workspacePaths`
//! for Antigravity. Use `project_roots()` for configuration discovery and
//! project-relative matching, and `cwd()` to resolve relative operands.
//!
//! `project_roots()` names the checkout the session started in, not
//! necessarily the one the agent is working in. After Claude Code enters a
//! git worktree, `CLAUDE_PROJECT_DIR` stays at the original checkout while
//! `cwd` and the edited files move into the worktree. A hook that acts on the
//! files the agent is editing (a formatter, a build) should locate them from
//! the edited path or `cwd()` (for example, its enclosing checkout) rather
//! than run in `project_roots()`, which would touch the user's main checkout
//! instead.
//!
//! # Reasons
//!
//! Every deny and block helper rejects a reason that is empty after trimming,
//! for every harness. Codex treats a blank reason as an invalid decision and
//! lets the action proceed while Claude Code honors it, so accepting one would
//! make the same handler block on one harness and not the other.
//!
//! # Harness selection
//!
//! Constructors take the open [`HarnessId`] used throughout HookKit. An
//! identity with no adapter for the family, including the `claude` CLI alias
//! and Antigravity for the Claude Code/Codex-only families, fails with
//! [`HookkitError::UnsupportedHarness`].

use hookkit_core::{
    BuiltinHarness, EventId, EventSpec, HarnessId, HookkitError, Utf8Path, Utf8PathBuf,
};
use std::borrow::Cow;

/// Returns the built-in harness selected by `harness`, or the
/// [`HookkitError::UnsupportedHarness`] error for `family`.
fn select(harness: &HarnessId, family: &'static str) -> hookkit_core::Result<BuiltinHarness> {
    BuiltinHarness::from_id(harness).ok_or_else(|| unsupported(harness, family))
}

fn unsupported(harness: &HarnessId, family: &'static str) -> HookkitError {
    HookkitError::UnsupportedHarness {
        harness: harness.clone(),
        message: format!(
            "no aligned {family} adapter is registered for this harness \
             (expected claude-code, codex, or antigravity)"
        ),
    }
}

fn unsupported_pair(harness: &HarnessId, family: &'static str) -> HookkitError {
    HookkitError::UnsupportedHarness {
        harness: harness.clone(),
        message: format!(
            "no aligned {family} adapter is registered for this harness; \
             {family} aligns claude-code and codex only"
        ),
    }
}

/// Rejects a reason that is empty after trimming.
fn require_reason(reason: String, message: &'static str) -> hookkit_core::Result<String> {
    if reason.trim().is_empty() {
        Err(HookkitError::InvalidProcessEmission(message))
    } else {
        Ok(reason)
    }
}

macro_rules! three_harness_environment {
    ($(#[$meta:meta])* $environment:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone)]
        #[non_exhaustive]
        pub enum $environment {
            /// Claude Code's native command environment.
            Claude(hookkit_claude::ClaudeCommandEnvironment),
            /// Codex's native command environment.
            Codex(hookkit_codex::CodexCommandEnvironment),
            /// Antigravity's native command environment.
            Antigravity(hookkit_antigravity::AntigravityCommandEnvironment),
        }

        impl $environment {
            /// Returns the harness represented by this environment arm.
            pub fn harness(&self) -> HarnessId {
                match self {
                    Self::Claude(_) => HarnessId::CLAUDE_CODE,
                    Self::Codex(_) => HarnessId::CODEX,
                    Self::Antigravity(_) => HarnessId::ANTIGRAVITY,
                }
            }

            /// Returns Claude Code's `CLAUDE_PROJECT_DIR`, the project root
            /// where the session started.
            ///
            /// Unlike Claude's input `cwd`, it does not follow `cd` or a
            /// worktree switch. Codex and Antigravity export no project-root
            /// variable, so their arms return `None`.
            pub fn project_dir(&self) -> Option<&Utf8Path> {
                match self {
                    Self::Claude(environment) => Some(&environment.project_dir),
                    Self::Codex(_) | Self::Antigravity(_) => None,
                }
            }
        }
    };
}

three_harness_environment!(
    /// Lossless native command-environment arms for aligned pre-tool execution.
    PreToolUseCommandEnvironment
);

/// Borrowed view over the two native JSON representations used for tool input.
///
/// The original native input remains available through its aligned wrapper.
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

    /// Returns the locations the native input carries: `[cwd]` for Claude
    /// Code and Codex, and `workspacePaths` for Antigravity.
    ///
    /// Claude Code's `cwd` is the agent's current directory, which follows
    /// `cd` and worktree switches, so it is not a stable root. Use
    /// [`Self::project_roots`] for configuration discovery and
    /// project-relative matching. No arm allocates.
    pub fn workspace_roots(&self) -> Cow<'_, [Utf8PathBuf]> {
        match self {
            Self::Claude(input) => Cow::Borrowed(std::slice::from_ref(&input.cwd)),
            Self::Codex(input) => Cow::Borrowed(std::slice::from_ref(&input.cwd)),
            Self::Antigravity(input) => Cow::Borrowed(&input.workspace_paths),
        }
    }

    /// Returns the stable project roots for this invocation.
    ///
    /// Claude Code reports `CLAUDE_PROJECT_DIR` from `environment`, Codex its
    /// configured `cwd`, and Antigravity its `workspacePaths` (possibly
    /// empty). When `environment` belongs to another harness, which the
    /// aligned runtime never produces, this falls back to
    /// [`Self::workspace_roots`].
    ///
    /// On Claude Code this stays at the session's original checkout after the
    /// agent enters a git worktree; see the
    /// [module documentation](self#roots-and-working-directories).
    pub fn project_roots<'a>(
        &'a self,
        environment: &'a PreToolUseCommandEnvironment,
    ) -> Cow<'a, [Utf8PathBuf]> {
        match (self, environment) {
            (Self::Claude(_), PreToolUseCommandEnvironment::Claude(environment)) => {
                Cow::Borrowed(std::slice::from_ref(&environment.project_dir))
            }
            _ => self.workspace_roots(),
        }
    }

    /// Returns the native working directory when that event carries one.
    ///
    /// On Claude Code this is the agent's current directory, which follows
    /// `cd`; relative tool operands resolve against it. Antigravity supplies
    /// workspace roots rather than a working directory.
    pub fn cwd(&self) -> Option<&Utf8Path> {
        match self {
            Self::Claude(input) => Some(&input.cwd),
            Self::Codex(input) => Some(&input.cwd),
            Self::Antigravity(_) => None,
        }
    }

    /// Returns the native tool name when its value is a string.
    pub fn tool_name(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => input.field("tool_name").and_then(serde_json::Value::as_str),
            Self::Codex(input) => Some(&input.tool_name),
            Self::Antigravity(input) => Some(&input.tool_call.name),
        }
    }

    /// Borrows the complete native tool-input value without allocation.
    pub fn tool_input(&self) -> Option<ToolInputRef<'_>> {
        match self {
            Self::Claude(input) => input.field("tool_input").map(ToolInputRef::Value),
            Self::Codex(input) => Some(ToolInputRef::Value(&input.tool_input)),
            Self::Antigravity(input) => Some(ToolInputRef::Object(&input.tool_call.args)),
        }
    }

    /// Returns the native tool-call identifier (`tool_use_id`).
    ///
    /// Antigravity sends no tool-call identifier, so its arm returns `None`.
    pub fn tool_call_id(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => input
                .field("tool_use_id")
                .and_then(serde_json::Value::as_str),
            Self::Codex(input) => Some(&input.tool_use_id),
            Self::Antigravity(_) => None,
        }
    }
}

/// Lossless native pre-tool output arms. No generic serialized envelope is
/// introduced during lowering.
///
/// See the [module documentation](self#choosing-a-pre-tool-response) for
/// what each helper means on each harness.
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

/// Permission decision Claude Code receives with a rewritten tool input.
///
/// Claude Code documents no `updatedInput` form that leaves the permission
/// flow unchanged: `allow` auto-approves the rewritten call and `ask` always
/// prompts the user, while `defer` ignores `updatedInput`. The caller must
/// therefore choose. Codex has no such choice: a Codex rewrite only replaces
/// the input, and Codex's normal approval policy still runs, so this value
/// does not affect the Codex arm.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RewriteApproval {
    /// Claude Code `permissionDecision: "allow"`: the rewritten call runs
    /// without a permission prompt. Deny and ask rules are still evaluated
    /// against the rewritten input.
    ///
    /// Use it only when the hook itself is the approval, for example when
    /// the rewrite makes the call safe by construction.
    AutoApprove,
    /// Claude Code `permissionDecision: "ask"`: the user confirms the
    /// rewritten call. The reason is shown to the user, not to Claude.
    ///
    /// This is the least-privilege choice. It prompts even where Claude
    /// Code's normal policy would have run the call silently, including in
    /// auto mode.
    Ask(String),
}

impl PreToolUseOutput {
    /// Builds the selected harness's "no objection" response, which leaves
    /// the call to the harness's normal permission flow.
    ///
    /// - Claude Code: empty output (`{}`), so its permission flow decides.
    /// - Codex: empty stdout, so its approval flow decides.
    /// - Antigravity: `{"decision":"ask"}`. Antigravity requires a decision on
    ///   every response and documents no pass-through value, so `ask` is the
    ///   closest neutral answer: it respects "Always Allow" settings and cached
    ///   grants, and prompts only where none covers the call. It can still
    ///   prompt for a call that Antigravity's default policy would have run
    ///   without asking. That is the deliberate least-privilege trade-off;
    ///   [`Self::allow`] would instead auto-approve every call.
    ///
    /// Use this, not [`Self::allow`], for every call a guard does not object
    /// to.
    pub fn pass_through(harness: &HarnessId) -> hookkit_core::Result<Self> {
        match select(harness, "PreToolUse")? {
            BuiltinHarness::ClaudeCode => Ok(Self::Claude(
                hookkit_claude::catalog::PreToolUseOutput::no_op(),
            )),
            BuiltinHarness::Codex => Ok(Self::Codex(
                hookkit_codex::protocol::PreToolUseOutput::no_op(),
            )),
            BuiltinHarness::Antigravity => Ok(Self::Antigravity(
                hookkit_antigravity::PreToolUseOutput::ask(),
            )),
            _ => Err(unsupported(harness, "PreToolUse")),
        }
    }

    /// Builds the selected harness's explicit auto-approval.
    ///
    /// This is not a "no objection" answer:
    ///
    /// - Claude Code: `permissionDecision: "allow"` skips the permission
    ///   prompt. Only deny and ask rules, and the actions no permission mode
    ///   auto-approves, are still enforced.
    /// - Antigravity: `{"decision":"allow"}` auto-approves the call and
    ///   bypasses the prompts that Ask presets would show.
    /// - Codex: cannot express an explicit approval. It rejects
    ///   `permissionDecision: "allow"` without `updatedInput`, so this helper
    ///   lowers to empty stdout and Codex's normal approval flow still runs.
    ///   [`Self::explicit_allow_supported`] reports this downgrade.
    ///
    /// A hook with no objection should return [`Self::pass_through`].
    pub fn allow(harness: &HarnessId) -> hookkit_core::Result<Self> {
        match select(harness, "PreToolUse")? {
            BuiltinHarness::ClaudeCode => Ok(Self::Claude(
                hookkit_claude::catalog::PreToolUseOutput::allow(),
            )),
            BuiltinHarness::Codex => Ok(Self::Codex(
                hookkit_codex::protocol::PreToolUseOutput::no_op(),
            )),
            BuiltinHarness::Antigravity => Ok(Self::Antigravity(
                hookkit_antigravity::PreToolUseOutput::allow(),
            )),
            _ => Err(unsupported(harness, "PreToolUse")),
        }
    }

    /// Reports whether [`Self::allow`] is an explicit auto-approval on
    /// `harness`.
    ///
    /// Returns `true` for Claude Code and Antigravity. Returns `false` for
    /// Codex, where [`Self::allow`] lowers to "no decision" and Codex's normal
    /// approval flow still runs, and for harnesses with no aligned adapter.
    pub fn explicit_allow_supported(harness: &HarnessId) -> bool {
        matches!(
            BuiltinHarness::from_id(harness),
            Some(BuiltinHarness::ClaudeCode | BuiltinHarness::Antigravity)
        )
    }

    /// Builds the selected harness's native deny output with a reason.
    ///
    /// Claude Code shows the reason to Claude, Codex reports it as the
    /// denial reason, and Antigravity sends it as `reason`. A reason that is
    /// empty after trimming is rejected for every harness, because Codex
    /// would otherwise ignore the denial and run the tool.
    pub fn deny(harness: &HarnessId, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        let reason = require_reason(
            reason.into(),
            "aligned PreToolUse deny reason must be non-empty after trimming",
        )?;
        match select(harness, "PreToolUse")? {
            BuiltinHarness::ClaudeCode => Ok(Self::Claude(
                hookkit_claude::catalog::PreToolUseOutput::deny(reason),
            )),
            BuiltinHarness::Codex => Ok(Self::Codex(
                hookkit_codex::protocol::PreToolUseOutput::deny(reason),
            )),
            BuiltinHarness::Antigravity => Ok(Self::Antigravity(
                hookkit_antigravity::PreToolUseOutput::deny().with_reason(reason),
            )),
            _ => Err(unsupported(harness, "PreToolUse")),
        }
    }

    /// Builds a Claude Code or Codex response that replaces the complete
    /// tool-input object.
    ///
    /// The harnesses treat permission differently here:
    ///
    /// - Claude Code sends `updatedInput` with the decision `approval`
    ///   selects. [`RewriteApproval::AutoApprove`] runs the rewritten call
    ///   without a prompt; [`RewriteApproval::Ask`] asks the user to confirm
    ///   it. Claude Code documents no rewrite that defers to its normal
    ///   permission flow.
    /// - Codex sends `permissionDecision: "allow"` with `updatedInput`, which
    ///   Codex treats as an input replacement only: its sandbox and approval
    ///   policy still apply to the rewritten call. `approval` is ignored.
    ///
    /// Input replacement is not part of the three-harness floor: Antigravity's
    /// pre-tool output cannot replace tool input, so it fails with
    /// [`HookkitError::UnsupportedHarness`].
    pub fn rewrite(
        harness: &HarnessId,
        updated_input: serde_json::Map<String, serde_json::Value>,
        approval: RewriteApproval,
    ) -> hookkit_core::Result<Self> {
        match select(harness, "PreToolUse")? {
            BuiltinHarness::ClaudeCode => {
                let decision = match approval {
                    RewriteApproval::AutoApprove => {
                        hookkit_claude::catalog::PreToolUseOutput::allow()
                    }
                    RewriteApproval::Ask(reason) => {
                        hookkit_claude::catalog::PreToolUseOutput::ask(reason)
                    }
                };
                decision.with_updated_input(updated_input).map(Self::Claude)
            }
            BuiltinHarness::Codex => Ok(Self::Codex(
                hookkit_codex::protocol::PreToolUseOutput::rewrite(updated_input),
            )),
            BuiltinHarness::Antigravity => Err(HookkitError::UnsupportedHarness {
                harness: harness.clone(),
                message: "Antigravity pre-tool output cannot replace tool input".into(),
            }),
            _ => Err(unsupported(harness, "PreToolUse")),
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

three_harness_environment!(
    /// Lossless native command-environment arms for aligned post-tool execution.
    PostToolUseCommandEnvironment
);

/// Lossless aligned input: every arm retains the complete native value.
///
/// All supported harnesses use the native event name `PostToolUse`.
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

    /// Returns the locations the native input carries: `[cwd]` for Claude
    /// Code and Codex, and `workspacePaths` for Antigravity.
    ///
    /// See [`PreToolUseInput::workspace_roots`] for why Claude Code's entry
    /// is not a stable root; prefer [`Self::project_roots`].
    pub fn workspace_roots(&self) -> Cow<'_, [Utf8PathBuf]> {
        match self {
            Self::Claude(input) => Cow::Borrowed(std::slice::from_ref(&input.cwd)),
            Self::Codex(input) => Cow::Borrowed(std::slice::from_ref(&input.cwd)),
            Self::Antigravity(input) => Cow::Borrowed(&input.workspace_paths),
        }
    }

    /// Returns the stable project roots for this invocation: Claude Code's
    /// `CLAUDE_PROJECT_DIR`, Codex's `cwd`, or Antigravity's `workspacePaths`.
    ///
    /// These name the checkout the session started in. After Claude Code
    /// enters a git worktree, the file a tool just edited lies in the
    /// worktree instead, so a post-edit formatter must not run here blindly;
    /// see the [module documentation](self#roots-and-working-directories) and
    /// [`PreToolUseInput::project_roots`].
    pub fn project_roots<'a>(
        &'a self,
        environment: &'a PostToolUseCommandEnvironment,
    ) -> Cow<'a, [Utf8PathBuf]> {
        match (self, environment) {
            (Self::Claude(_), PostToolUseCommandEnvironment::Claude(environment)) => {
                Cow::Borrowed(std::slice::from_ref(&environment.project_dir))
            }
            _ => self.workspace_roots(),
        }
    }

    /// Returns the native working directory when that event carries one.
    ///
    /// On Claude Code this is the agent's current directory, which follows
    /// `cd`. Antigravity supplies workspace roots rather than a working
    /// directory.
    pub fn cwd(&self) -> Option<&Utf8Path> {
        match self {
            Self::Claude(input) => Some(&input.cwd),
            Self::Codex(input) => Some(&input.cwd),
            Self::Antigravity(_) => None,
        }
    }

    /// Returns the native tool name.
    ///
    /// Antigravity returns `None` when its payload omits `toolCall`, as the
    /// IDE reference example does.
    pub fn tool_name(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => Some(&input.tool_name),
            Self::Codex(input) => Some(&input.tool_name),
            Self::Antigravity(input) => input.tool_call.as_ref().map(|call| call.name.as_str()),
        }
    }

    /// Borrows the complete native tool-input value without allocation.
    ///
    /// Antigravity returns `None` when its payload omits `toolCall`.
    pub fn tool_input(&self) -> Option<ToolInputRef<'_>> {
        match self {
            Self::Claude(input) => Some(ToolInputRef::Value(&input.tool_input)),
            Self::Codex(input) => Some(ToolInputRef::Value(&input.tool_input)),
            Self::Antigravity(input) => input
                .tool_call
                .as_ref()
                .map(|call| ToolInputRef::Object(&call.args)),
        }
    }

    /// Returns the native tool-call identifier (`tool_use_id`).
    ///
    /// Antigravity sends no tool-call identifier, so its arm returns `None`.
    pub fn tool_call_id(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => Some(&input.tool_use_id),
            Self::Codex(input) => Some(&input.tool_use_id),
            Self::Antigravity(_) => None,
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

impl PostToolUseOutput {
    /// Builds the selected harness's native post-tool no-op response.
    pub fn no_op(harness: &HarnessId) -> hookkit_core::Result<Self> {
        match select(harness, "PostToolUse")? {
            BuiltinHarness::ClaudeCode => Ok(Self::Claude(
                hookkit_claude::protocol::PostToolUseOutput::no_op(),
            )),
            BuiltinHarness::Codex => Ok(Self::Codex(
                hookkit_codex::protocol::PostToolUseOutput::no_op(),
            )),
            BuiltinHarness::Antigravity => Ok(Self::Antigravity(
                hookkit_antigravity::PostToolUseOutput::default(),
            )),
            _ => Err(unsupported(harness, "PostToolUse")),
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

three_harness_environment!(
    /// Lossless native command-environment arms for aligned turn completion.
    TurnCompletionCommandEnvironment
);

/// The event at which the agent's execution attempts to stop.
///
/// All supported harnesses use the native event name `Stop`, but they fire
/// it under different conditions:
///
/// - Claude Code fires `Stop` when the main agent finishes responding. It
///   does not fire on a user interrupt, and an API error fires `StopFailure`
///   instead.
/// - Codex fires `Stop` when a turn completes. An interrupted turn dispatches
///   `Interrupt`, not `Stop`.
/// - Antigravity fires `Stop` whenever the execution loop terminates,
///   including `terminationReason` `error` and `max_steps_exceeded`, and
///   while background work is still running (`fullyIdle: false`).
///
/// A hook that asks the agent to continue should consult
/// [`Self::stop_hook_active`] on Claude Code and Codex, and
/// [`Self::termination_reason`] and [`Self::fully_idle`] on Antigravity,
/// before re-entering a loop the harness is trying to end.
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

    /// Returns the locations the native input carries: `[cwd]` for Claude
    /// Code and Codex, and `workspacePaths` for Antigravity.
    ///
    /// See [`PreToolUseInput::workspace_roots`] for why Claude Code's entry
    /// is not a stable root; prefer [`Self::project_roots`].
    pub fn workspace_roots(&self) -> Cow<'_, [Utf8PathBuf]> {
        match self {
            Self::Claude(input) => Cow::Borrowed(std::slice::from_ref(&input.cwd)),
            Self::Codex(input) => Cow::Borrowed(std::slice::from_ref(&input.cwd)),
            Self::Antigravity(input) => Cow::Borrowed(&input.workspace_paths),
        }
    }

    /// Returns the stable project roots for this invocation: Claude Code's
    /// `CLAUDE_PROJECT_DIR`, Codex's `cwd`, or Antigravity's `workspacePaths`.
    ///
    /// See [`PreToolUseInput::project_roots`].
    pub fn project_roots<'a>(
        &'a self,
        environment: &'a TurnCompletionCommandEnvironment,
    ) -> Cow<'a, [Utf8PathBuf]> {
        match (self, environment) {
            (Self::Claude(_), TurnCompletionCommandEnvironment::Claude(environment)) => {
                Cow::Borrowed(std::slice::from_ref(&environment.project_dir))
            }
            _ => self.workspace_roots(),
        }
    }

    /// Returns the native working directory when that event carries one.
    pub fn cwd(&self) -> Option<&Utf8Path> {
        match self {
            Self::Claude(input) => Some(&input.cwd),
            Self::Codex(input) => Some(&input.cwd),
            Self::Antigravity(_) => None,
        }
    }

    /// Returns Claude Code's and Codex's `stop_hook_active` loop guard: `true`
    /// when the agent is already continuing because a stop hook blocked an
    /// earlier stop.
    ///
    /// Antigravity sends no loop guard, so its arm returns `None`.
    pub fn stop_hook_active(&self) -> Option<bool> {
        match self {
            Self::Claude(input) => input
                .field("stop_hook_active")
                .and_then(serde_json::Value::as_bool),
            Self::Codex(input) => input.stop_hook_active(),
            Self::Antigravity(_) => None,
        }
    }

    /// Returns the agent's final message when the native input carries one.
    pub fn last_assistant_message(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => input
                .field("last_assistant_message")
                .and_then(serde_json::Value::as_str),
            Self::Codex(input) => input.last_assistant_message(),
            Self::Antigravity(_) => None,
        }
    }

    /// Returns Antigravity's `terminationReason` wire string, such as
    /// `model_stop`, `max_steps_exceeded`, or `error`.
    ///
    /// Claude Code and Codex fire their turn-completion event only for a
    /// normal completion, so their arms return `None`.
    pub fn termination_reason(&self) -> Option<&str> {
        match self {
            Self::Antigravity(input) => Some(input.termination_reason.as_str()),
            Self::Claude(_) | Self::Codex(_) => None,
        }
    }

    /// Returns Antigravity's `fullyIdle`: `false` while background commands
    /// or asynchronous tasks are still running.
    ///
    /// Claude Code and Codex send no equivalent, so their arms return `None`.
    pub fn fully_idle(&self) -> Option<bool> {
        match self {
            Self::Antigravity(input) => Some(input.fully_idle),
            Self::Claude(_) | Self::Codex(_) => None,
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
    /// Builds the selected harness's native response that permits turn
    /// completion without adding a message.
    ///
    /// Claude Code receives `{}`, Codex empty stdout, and Antigravity
    /// `{"decision":"stop"}`.
    pub fn allow(harness: &HarnessId) -> hookkit_core::Result<Self> {
        match select(harness, "TurnCompletion")? {
            BuiltinHarness::ClaudeCode => {
                Ok(Self::Claude(hookkit_claude::catalog::StopOutput::no_op()))
            }
            BuiltinHarness::Codex => Ok(Self::Codex(hookkit_codex::catalog::StopOutput::no_op())),
            BuiltinHarness::Antigravity => Ok(Self::Antigravity(
                hookkit_antigravity::StopOutput::allow_stop(),
            )),
            _ => Err(unsupported(harness, "TurnCompletion")),
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

            /// Returns Claude Code's `CLAUDE_PROJECT_DIR`, the project root
            /// where the session started; `None` for Codex, which exports no
            /// project-root variable.
            pub fn project_dir(&self) -> Option<&Utf8Path> {
                match self {
                    Self::Claude(environment) => Some(&environment.project_dir),
                    Self::Codex(_) => None,
                }
            }
        }

        $(#[$input_meta])*
        #[derive(Debug, Clone)]
        #[non_exhaustive]
        // Claude's typed inputs are larger than Codex's catalog envelopes.
        #[allow(clippy::large_enum_variant)]
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
            ///
            /// On Claude Code this is the agent's current directory, which
            /// follows `cd` and worktree switches.
            pub fn cwd(&self) -> &Utf8Path {
                match self {
                    Self::Claude(input) => &input.cwd,
                    Self::Codex(input) => &input.cwd,
                }
            }

            /// Returns the location the native input carries: its `cwd`.
            ///
            /// Claude Code's `cwd` follows `cd`, so prefer
            /// `project_roots` for configuration discovery.
            pub fn workspace_roots(&self) -> Cow<'_, [Utf8PathBuf]> {
                match self {
                    Self::Claude(input) => Cow::Borrowed(std::slice::from_ref(&input.cwd)),
                    Self::Codex(input) => Cow::Borrowed(std::slice::from_ref(&input.cwd)),
                }
            }

            /// Returns the stable project roots for this invocation: Claude
            /// Code's `CLAUDE_PROJECT_DIR`, or Codex's `cwd`.
            pub fn project_roots<'a>(
                &'a self,
                environment: &'a $environment,
            ) -> Cow<'a, [Utf8PathBuf]> {
                match (self, environment) {
                    (Self::Claude(_), $environment::Claude(environment)) => {
                        Cow::Borrowed(std::slice::from_ref(&environment.project_dir))
                    }
                    _ => self.workspace_roots(),
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

/// Selects the Claude Code or Codex arm for a pair family.
fn select_pair(harness: &HarnessId, family: &'static str) -> hookkit_core::Result<PairHarness> {
    match BuiltinHarness::from_id(harness) {
        Some(BuiltinHarness::ClaudeCode) => Ok(PairHarness::Claude),
        Some(BuiltinHarness::Codex) => Ok(PairHarness::Codex),
        _ => Err(unsupported_pair(harness, family)),
    }
}

/// The harnesses a Claude Code/Codex pair family lowers to.
#[derive(Clone, Copy)]
enum PairHarness {
    Claude,
    Codex,
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
    /// Builds the selected harness's "no answer" response, which leaves the
    /// permission dialog to the user.
    ///
    /// Claude Code receives `{}` and Codex empty stdout.
    pub fn no_op(harness: &HarnessId) -> hookkit_core::Result<Self> {
        match select_pair(harness, "PermissionRequest")? {
            PairHarness::Claude => Ok(Self::Claude(
                hookkit_claude::catalog::PermissionRequestOutput::no_op(),
            )),
            PairHarness::Codex => Ok(Self::Codex(
                hookkit_codex::catalog::PermissionRequestOutput::no_op(),
            )),
        }
    }

    /// Builds the selected harness's native approval, which answers the
    /// permission dialog on the user's behalf.
    ///
    /// This grants the permission without showing the dialog. A hook that
    /// only observes permission requests should return [`Self::no_op`].
    pub fn allow(harness: &HarnessId) -> hookkit_core::Result<Self> {
        match select_pair(harness, "PermissionRequest")? {
            PairHarness::Claude => Ok(Self::Claude(
                hookkit_claude::catalog::PermissionRequestOutput::allow(),
            )),
            PairHarness::Codex => Ok(Self::Codex(
                hookkit_codex::catalog::PermissionRequestOutput::allow(),
            )),
        }
    }

    /// Builds the selected harness's native denial response.
    ///
    /// A reason that is empty after trimming is rejected, as for every
    /// aligned deny helper.
    pub fn deny(harness: &HarnessId, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        let reason = require_reason(
            reason.into(),
            "aligned PermissionRequest deny reason must be non-empty after trimming",
        )?;
        match select_pair(harness, "PermissionRequest")? {
            PairHarness::Claude => Ok(Self::Claude(
                hookkit_claude::catalog::PermissionRequestOutput::deny(reason),
            )),
            PairHarness::Codex => Ok(Self::Codex(
                hookkit_codex::catalog::PermissionRequestOutput::deny(reason),
            )),
        }
    }
}

impl PermissionRequestInput {
    /// Returns the harness-native tool name.
    pub fn tool_name(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => input.field("tool_name").and_then(serde_json::Value::as_str),
            Self::Codex(input) => input.tool_name(),
        }
    }

    /// Borrows the complete native tool-input value without allocation.
    pub fn tool_input(&self) -> Option<ToolInputRef<'_>> {
        match self {
            Self::Claude(input) => input.field("tool_input").map(ToolInputRef::Value),
            Self::Codex(input) => input.tool_input().map(ToolInputRef::Value),
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
        match select_pair(harness, "PreCompact")? {
            PairHarness::Claude => Ok(Self::Claude(
                hookkit_claude::catalog::PreCompactOutput::no_op(),
            )),
            PairHarness::Codex => Ok(Self::Codex(
                hookkit_codex::catalog::PreCompactOutput::no_op(),
            )),
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
        match select_pair(harness, "PostCompact")? {
            PairHarness::Claude => Ok(Self::Claude(
                hookkit_claude::catalog::PostCompactOutput::no_op(),
            )),
            PairHarness::Codex => Ok(Self::Codex(
                hookkit_codex::catalog::PostCompactOutput::no_op(),
            )),
        }
    }

    /// Builds a response that shows `message` to the user where the harness
    /// can deliver it.
    ///
    /// - Codex shows the top-level `systemMessage` as a warning.
    /// - Claude Code discards a `PostCompact` hook's `systemMessage`
    ///   (claude-code/docs-2026-09-29-r1) and has no other user notice for a
    ///   successful hook, so the Claude arm is the no-op and `message` is
    ///   dropped. [`Self::system_notice_delivered`] reports this.
    pub fn with_system_notice(
        harness: &HarnessId,
        message: impl Into<String>,
    ) -> hookkit_core::Result<Self> {
        match select_pair(harness, "PostCompact")? {
            PairHarness::Claude => Ok(Self::Claude(
                hookkit_claude::catalog::PostCompactOutput::no_op(),
            )),
            PairHarness::Codex => hookkit_codex::catalog::PostCompactOutput::no_op()
                .with_system_message(message)
                .map(Self::Codex),
        }
    }

    /// Reports whether [`Self::with_system_notice`] reaches the user on
    /// `harness`: `true` for Codex, `false` for Claude Code (which discards
    /// it) and for harnesses with no aligned adapter.
    pub fn system_notice_delivered(harness: &HarnessId) -> bool {
        matches!(select_pair(harness, "PostCompact"), Ok(PairHarness::Codex))
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
        match select_pair(harness, "SessionStart")? {
            PairHarness::Claude => Ok(Self::Claude(
                hookkit_claude::protocol::SessionStartOutput::no_op(),
            )),
            PairHarness::Codex => Ok(Self::Codex(
                hookkit_codex::catalog::SessionStartOutput::no_op(),
            )),
        }
    }

    /// Builds the selected harness's structured agent-context response
    /// (`hookSpecificOutput.additionalContext`).
    pub fn with_context(
        harness: &HarnessId,
        context: impl Into<String>,
    ) -> hookkit_core::Result<Self> {
        let context = context.into();
        match select_pair(harness, "SessionStart")? {
            PairHarness::Claude => Ok(Self::Claude(
                hookkit_claude::protocol::SessionStartOutput::with_context(context),
            )),
            PairHarness::Codex => Ok(Self::Codex(
                hookkit_codex::catalog::SessionStartOutput::with_context(context),
            )),
        }
    }
}

impl SessionStartInput {
    /// Returns the native session-start source as its wire spelling.
    pub fn source(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => Some(input.source.as_str()),
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
        match select_pair(harness, "SessionEnd")? {
            PairHarness::Claude => Ok(Self::Claude(
                hookkit_claude::catalog::SessionEndOutput::no_op(),
            )),
            PairHarness::Codex => Ok(Self::Codex(
                hookkit_codex::catalog::SessionEndOutput::no_op(),
            )),
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
        match select_pair(harness, "SubagentStart")? {
            PairHarness::Claude => Ok(Self::Claude(
                hookkit_claude::catalog::SubagentStartOutput::no_op(),
            )),
            PairHarness::Codex => Ok(Self::Codex(
                hookkit_codex::catalog::SubagentStartOutput::no_op(),
            )),
        }
    }

    /// Builds the selected harness's structured agent-context response
    /// (`hookSpecificOutput.additionalContext`).
    pub fn with_context(
        harness: &HarnessId,
        context: impl Into<String>,
    ) -> hookkit_core::Result<Self> {
        let context = context.into();
        match select_pair(harness, "SubagentStart")? {
            PairHarness::Claude => Ok(Self::Claude(
                hookkit_claude::catalog::SubagentStartOutput::with_context(context),
            )),
            PairHarness::Codex => Ok(Self::Codex(
                hookkit_codex::catalog::SubagentStartOutput::with_context(context),
            )),
        }
    }
}

impl SubagentStartInput {
    /// Returns the native subagent identifier when it is a string.
    pub fn agent_id(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => input.field("agent_id").and_then(serde_json::Value::as_str),
            Self::Codex(input) => input.agent_id(),
        }
    }

    /// Returns the native subagent type when it is a string.
    pub fn agent_type(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => input
                .field("agent_type")
                .and_then(serde_json::Value::as_str),
            Self::Codex(input) => input.agent_type(),
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
        match select_pair(harness, "SubagentStop")? {
            PairHarness::Claude => Ok(Self::Claude(
                hookkit_claude::catalog::SubagentStopOutput::no_op(),
            )),
            PairHarness::Codex => Ok(Self::Codex(
                hookkit_codex::catalog::SubagentStopOutput::no_op(),
            )),
        }
    }

    /// Builds the selected harness's native block response, which keeps the
    /// subagent working with `reason` as its next instruction.
    ///
    /// A reason that is empty after trimming is rejected for every harness:
    /// Codex treats a blank block reason as invalid and lets the subagent
    /// stop. Check [`SubagentStopInput::stop_hook_active`] before blocking
    /// again.
    pub fn block(harness: &HarnessId, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        let reason = require_reason(
            reason.into(),
            "aligned SubagentStop block reason must be non-empty after trimming",
        )?;
        match select_pair(harness, "SubagentStop")? {
            PairHarness::Claude => Ok(Self::Claude(
                hookkit_claude::catalog::SubagentStopOutput::block(reason),
            )),
            PairHarness::Codex => Ok(Self::Codex(
                hookkit_codex::catalog::SubagentStopOutput::block(reason),
            )),
        }
    }
}

impl SubagentStopInput {
    /// Returns the native subagent identifier when it is a string.
    pub fn agent_id(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => input.field("agent_id").and_then(serde_json::Value::as_str),
            Self::Codex(input) => input.agent_id(),
        }
    }

    /// Returns the native subagent type when it is a string.
    pub fn agent_type(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => input
                .field("agent_type")
                .and_then(serde_json::Value::as_str),
            Self::Codex(input) => input.agent_type(),
        }
    }

    /// Returns the `stop_hook_active` loop guard: `true` when the subagent is
    /// already continuing because a stop hook blocked an earlier stop.
    pub fn stop_hook_active(&self) -> Option<bool> {
        match self {
            Self::Claude(input) => input
                .field("stop_hook_active")
                .and_then(serde_json::Value::as_bool),
            Self::Codex(input) => input.stop_hook_active(),
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
        match select_pair(harness, "UserPromptSubmit")? {
            PairHarness::Claude => Ok(Self::Claude(
                hookkit_claude::catalog::UserPromptSubmitOutput::no_op(),
            )),
            PairHarness::Codex => Ok(Self::Codex(
                hookkit_codex::catalog::UserPromptSubmitOutput::no_op(),
            )),
        }
    }

    /// Builds the selected harness's structured agent-context response
    /// (`hookSpecificOutput.additionalContext`).
    ///
    /// Both harnesses use the JSON channel, so context that itself looks like
    /// JSON (for example text starting with `[` or `{`) is delivered intact
    /// rather than being parsed as a malformed response.
    pub fn with_context(
        harness: &HarnessId,
        context: impl Into<String>,
    ) -> hookkit_core::Result<Self> {
        let context = context.into();
        match select_pair(harness, "UserPromptSubmit")? {
            PairHarness::Claude => Ok(Self::Claude(
                hookkit_claude::catalog::UserPromptSubmitOutput::with_context(context),
            )),
            PairHarness::Codex => Ok(Self::Codex(
                hookkit_codex::catalog::UserPromptSubmitOutput::with_context(context),
            )),
        }
    }

    /// Blocks prompt submission with a reason using each harness's exit-2
    /// path.
    ///
    /// The audiences differ. Claude Code erases the prompt and shows the
    /// reason to the user only; it is not added to Claude's context
    /// (claude-code/docs-2026-09-29-r1). Codex rejects the prompt and reports
    /// the reason as the blocking reason. A reason that is empty after
    /// trimming is rejected for every harness: Codex ignores a blank exit-2
    /// reason and submits the prompt.
    pub fn block(harness: &HarnessId, reason: impl Into<String>) -> hookkit_core::Result<Self> {
        let reason = require_reason(
            reason.into(),
            "aligned UserPromptSubmit block reason must be non-empty after trimming",
        )?;
        match select_pair(harness, "UserPromptSubmit")? {
            PairHarness::Claude => Ok(Self::Claude(
                hookkit_claude::catalog::UserPromptSubmitOutput::blocking_error(reason),
            )),
            PairHarness::Codex => Ok(Self::Codex(
                hookkit_codex::catalog::UserPromptSubmitOutput::blocking_error(reason),
            )),
        }
    }
}

impl UserPromptSubmitInput {
    /// Returns the submitted user prompt.
    pub fn prompt(&self) -> Option<&str> {
        match self {
            Self::Claude(input) => input.field("prompt").and_then(serde_json::Value::as_str),
            Self::Codex(input) => input.prompt(),
        }
    }
}

#[cfg(test)]
mod tests;
