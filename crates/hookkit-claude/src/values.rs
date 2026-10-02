//! Forward-compatible value types for harness-sent Claude Code input fields.
//!
//! Every enum here models a value Claude Code *sends*. A value the selected
//! snapshot does not document parses as `Unknown(String)`, retaining the raw
//! wire spelling, so a newer Claude Code release cannot turn a hook into a
//! parse failure (exit 1, which Claude treats as a non-blocking error and
//! therefore as a silent fail-open). Strict validation of documented values
//! belongs to the contract schemas and the conformance suite, not to the
//! runtime parser.
//!
//! Parsing never produces `Unknown` for a documented spelling. A caller that
//! constructs `Unknown("plan".into())` by hand gets a value that does not
//! compare equal to [`PermissionMode::Plan`]; use `From<&str>` to normalize.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use std::fmt;

macro_rules! wire_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $(
                $(#[$variant_meta:meta])*
                $variant:ident = $wire:literal,
            )*
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        #[non_exhaustive]
        pub enum $name {
            $(
                $(#[$variant_meta])*
                $variant,
            )*
            /// A value the selected snapshot does not document, retained
            /// verbatim.
            Unknown(String),
        }

        impl $name {
            /// Wire spellings documented by the selected snapshot.
            pub const DOCUMENTED: &'static [&'static str] = &[$($wire),*];

            /// Returns the exact native wire spelling.
            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$variant => $wire,)*
                    Self::Unknown(value) => value,
                }
            }

            /// Reports whether the value is documented by the selected
            /// snapshot.
            pub fn is_documented(&self) -> bool {
                !matches!(self, Self::Unknown(_))
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                match value {
                    $($wire => Self::$variant,)*
                    other => Self::Unknown(other.to_owned()),
                }
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                match value.as_str() {
                    $($wire => Self::$variant,)*
                    _ => Self::Unknown(value),
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                String::deserialize(deserializer).map(Self::from)
            }
        }
    };
}

wire_enum! {
    /// Native cause of a Claude Code `SessionStart` event.
    pub enum SessionSource {
        /// A newly started session.
        Startup = "startup",
        /// A previously persisted session was resumed.
        Resume = "resume",
        /// The current conversation context was cleared.
        Clear = "clear",
        /// The current conversation context was compacted.
        Compact = "compact",
        /// A new session was forked from an existing session.
        Fork = "fork",
    }
}

wire_enum! {
    /// Claude Code permission policy active for a hook event.
    pub enum PermissionMode {
        /// The default interactive permission policy. Claude Code's `manual`
        /// mode also arrives as `default`.
        Default = "default",
        /// Restricts the agent to planning behavior.
        Plan = "plan",
        /// Automatically accepts file-edit operations.
        AcceptEdits = "acceptEdits",
        /// Lets Claude's automatic classifier choose permissions.
        Auto = "auto",
        /// Does not prompt for otherwise disallowed operations.
        DontAsk = "dontAsk",
        /// Bypasses normal permission checks.
        BypassPermissions = "bypassPermissions",
    }
}

wire_enum! {
    /// Claude effort level carried in `effort.level`.
    pub enum EffortLevel {
        /// Low effort.
        Low = "low",
        /// Medium effort.
        Medium = "medium",
        /// High effort.
        High = "high",
        /// Extra-high effort.
        Xhigh = "xhigh",
        /// Maximum effort.
        Max = "max",
    }
}

wire_enum! {
    /// Configuration source of the MCP server that owns a tool.
    ///
    /// The vocabulary is open upstream. Claude Code documents that an
    /// unrecognized source must be treated as an unrecognized *configured*
    /// source, never as [`Self::Sdk`]; [`Self::is_sdk`] follows that rule.
    pub enum McpServerSource {
        /// A server supplied by an Agent SDK host.
        Sdk = "sdk",
        /// A server contributed by a plugin.
        Plugin = "plugin",
        /// A server from user settings.
        User = "user",
        /// A server from project settings.
        Project = "project",
        /// A server from local settings.
        Local = "local",
        /// A dynamically registered server.
        Dynamic = "dynamic",
        /// A server from managed settings.
        Managed = "managed",
        /// A server from enterprise configuration.
        Enterprise = "enterprise",
        /// A claude.ai connector.
        Claudeai = "claudeai",
        /// A server defined by an agent.
        Agent = "agent",
    }
}

impl McpServerSource {
    /// Reports whether the server was supplied by an Agent SDK host.
    ///
    /// Unknown sources are configured sources, so they return `false`.
    pub fn is_sdk(&self) -> bool {
        matches!(self, Self::Sdk)
    }
}

wire_enum! {
    /// Origin of a Claude Code model switch.
    ///
    /// `PreModelSwitch` documents `command`, `picker`, and `sdk`;
    /// `PostModelSwitch` additionally documents `auto` and `resume`.
    pub enum ModelSwitchSource {
        /// `/model <name>`, the `/config` Model setting, or enabling fast mode.
        Command = "command",
        /// A model picker.
        Picker = "picker",
        /// An Agent SDK or Remote Control `set_model` or
        /// `apply_flag_settings` request.
        Sdk = "sdk",
        /// An automatic fallback or other change Claude Code made on its own.
        /// Sent only to `PostModelSwitch`.
        Auto = "auto",
        /// The model restored when a session resumes. Sent only to
        /// `PostModelSwitch`.
        Resume = "resume",
    }
}

wire_enum! {
    /// Prompt-cache lifetime Claude Code requests for the session.
    pub enum CacheTtl {
        /// Five-minute cache lifetime.
        FiveMinutes = "5m",
        /// One-hour cache lifetime.
        OneHour = "1h",
    }
}

wire_enum! {
    /// How Claude Code priced a model switch's estimated cache write.
    pub enum ModelSwitchPricing {
        /// The organization's configured rates.
        Configured = "configured",
        /// List price from Claude Code's catalog.
        Catalog = "catalog",
        /// A default rate assumed for an unpriced model.
        Default = "default",
    }
}

wire_enum! {
    /// Reason a Claude Code session ended.
    pub enum SessionEndReason {
        /// The conversation was cleared.
        Clear = "clear",
        /// The session ended because another session was resumed.
        Resume = "resume",
        /// The user logged out.
        Logout = "logout",
        /// The user exited at the prompt.
        PromptInputExit = "prompt_input_exit",
        /// Any other reason.
        Other = "other",
        /// Bypass-permissions mode was disabled. Removed in Claude Code
        /// v2.1.234; only older releases send it.
        BypassPermissionsDisabled = "bypass_permissions_disabled",
    }
}

wire_enum! {
    /// Kind of Claude Code notification. The vocabulary is open upstream.
    pub enum NotificationType {
        /// A permission prompt is waiting.
        PermissionPrompt = "permission_prompt",
        /// The prompt has been idle.
        IdlePrompt = "idle_prompt",
        /// Authentication succeeded.
        AuthSuccess = "auth_success",
        /// An MCP elicitation dialog is open.
        ElicitationDialog = "elicitation_dialog",
        /// An MCP URL-mode elicitation dialog is open.
        ElicitationUrlDialog = "elicitation_url_dialog",
        /// An MCP elicitation completed.
        ElicitationComplete = "elicitation_complete",
        /// An MCP elicitation received a response.
        ElicitationResponse = "elicitation_response",
        /// An agent needs input.
        AgentNeedsInput = "agent_needs_input",
        /// An agent completed.
        AgentCompleted = "agent_completed",
        /// A quota auto-resume fired (Claude Code v2.1.234 or later).
        QuotaAutoResumeFired = "quota_auto_resume_fired",
        /// A quota auto-resume became stale (Claude Code v2.1.234 or later).
        QuotaAutoResumeStale = "quota_auto_resume_stale",
        /// Quota auto-resume was disabled (Claude Code v2.1.234 or later).
        QuotaAutoResumeDisabled = "quota_auto_resume_disabled",
    }
}

wire_enum! {
    /// API failure that ended a Claude Code turn.
    pub enum StopFailureError {
        /// The request was rate limited.
        RateLimit = "rate_limit",
        /// The API was overloaded.
        Overloaded = "overloaded",
        /// Authentication failed.
        AuthenticationFailed = "authentication_failed",
        /// The organization is not allowed to use OAuth.
        OauthOrgNotAllowed = "oauth_org_not_allowed",
        /// The account is on hold.
        AccountOnHold = "account_on_hold",
        /// A billing problem blocked the request.
        BillingError = "billing_error",
        /// The request was invalid.
        InvalidRequest = "invalid_request",
        /// The model was not found.
        ModelNotFound = "model_not_found",
        /// The server failed.
        ServerError = "server_error",
        /// The response hit its output-token limit.
        MaxOutputTokens = "max_output_tokens",
        /// Cloud credentials failed to load (Claude Code v2.1.267 or later).
        CloudCredentialError = "cloud_credential_error",
        /// Verification is required. Typed by the Agent SDK but not
        /// described by the hooks reference.
        VerificationRequired = "verification_required",
        /// An unclassified failure.
        UnknownError = "unknown",
    }
}

wire_enum! {
    /// What triggered a Claude Code compaction.
    pub enum CompactTrigger {
        /// A manual `/compact`.
        Manual = "manual",
        /// Automatic compaction at the auto-compact window.
        Auto = "auto",
    }
}

/// Largest integer JavaScript represents exactly (`Number.MAX_SAFE_INTEGER`).
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

/// Serializes an `f64` input field the way Claude Code spells it on the wire.
///
/// Claude Code writes hook input with JavaScript's `JSON.stringify`, which
/// never gives an integral number a fractional part. Writing such a value as
/// an integer therefore reproduces the received JSON (`12`, not `12.0`).
pub(crate) fn serialize_js_number<S: Serializer>(
    value: &f64,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    if value.fract() == 0.0 && value.abs() <= MAX_SAFE_INTEGER {
        // Integral and within the exactly representable range, so the cast
        // is lossless.
        serializer.serialize_i64(*value as i64)
    } else {
        serializer.serialize_f64(*value)
    }
}

/// [`serialize_js_number`] for an optional field; `None` is skipped by the
/// field's `skip_serializing_if`.
pub(crate) fn serialize_optional_js_number<S: Serializer>(
    value: &Option<f64>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(value) => serialize_js_number(value, serializer),
        None => serializer.serialize_none(),
    }
}

/// Nested effort object carried by Claude Code hook inputs.
///
/// Unknown keys are retained in [`Self::extra`] rather than rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Effort {
    /// Requested effort level.
    pub level: EffortLevel,
    /// Unknown keys retained for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Effort {
    /// Creates an effort object with no extra keys.
    pub fn new(level: EffortLevel) -> Self {
        Self {
            level,
            extra: BTreeMap::new(),
        }
    }
}

/// MCP server that owns a tool, sent with tool events for MCP tools
/// (Claude Code v2.1.274 or later).
///
/// Unknown keys are retained in [`Self::extra`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct McpServer {
    /// Configured server name.
    pub name: String,
    /// Configuration source of the server.
    pub source: McpServerSource,
    /// Unknown keys retained for forward compatibility.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl McpServer {
    /// Creates an MCP server descriptor with no extra keys.
    pub fn new(name: impl Into<String>, source: McpServerSource) -> Self {
        Self {
            name: name.into(),
            source,
            extra: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_values_parse_to_named_variants() {
        let mode: PermissionMode = serde_json::from_str(r#""acceptEdits""#).unwrap();
        assert_eq!(mode, PermissionMode::AcceptEdits);
        assert_eq!(mode.as_str(), "acceptEdits");
        assert!(mode.is_documented());
        assert_eq!(serde_json::to_string(&mode).unwrap(), r#""acceptEdits""#);
    }

    #[test]
    fn undocumented_values_round_trip_verbatim() {
        for raw in [r#""manual""#, r#""newMode""#] {
            let mode: PermissionMode = serde_json::from_str(raw).unwrap();
            assert!(!mode.is_documented());
            assert_eq!(serde_json::to_string(&mode).unwrap(), raw);
        }
        let level: EffortLevel = serde_json::from_str(r#""ultra""#).unwrap();
        assert_eq!(level, EffortLevel::Unknown("ultra".into()));
        assert_eq!(level.to_string(), "ultra");
        let source: SessionSource = serde_json::from_str(r#""branch""#).unwrap();
        assert_eq!(source.as_str(), "branch");
    }

    #[test]
    fn effort_retains_unknown_keys() {
        let effort: Effort =
            serde_json::from_value(serde_json::json!({"level": "high", "source": "x"})).unwrap();
        assert_eq!(effort.level, EffortLevel::High);
        assert_eq!(effort.extra["source"], "x");
        assert_eq!(
            serde_json::to_value(&effort).unwrap(),
            serde_json::json!({"level": "high", "source": "x"})
        );
    }

    #[test]
    fn unknown_mcp_source_is_a_configured_source() {
        let server: McpServer =
            serde_json::from_value(serde_json::json!({"name": "db", "source": "team-registry"}))
                .unwrap();
        assert!(!server.source.is_sdk());
        assert!(!server.source.is_documented());
        assert!(McpServerSource::from("sdk").is_sdk());
    }

    #[test]
    fn non_string_values_are_rejected() {
        assert!(serde_json::from_str::<SessionSource>("1").is_err());
    }

    #[test]
    fn integral_numbers_serialize_with_the_javascript_spelling() {
        #[derive(Serialize)]
        struct Probe {
            #[serde(serialize_with = "serialize_js_number")]
            value: f64,
        }
        let render = |value| serde_json::to_string(&Probe { value }).unwrap();
        assert_eq!(render(12.0), r#"{"value":12}"#);
        assert_eq!(render(0.0), r#"{"value":0}"#);
        assert_eq!(render(-3.0), r#"{"value":-3}"#);
        assert_eq!(render(1.1396), r#"{"value":1.1396}"#);
        assert_eq!(render(1e300), r#"{"value":1e+300}"#);
    }
}
