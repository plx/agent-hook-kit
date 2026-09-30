use crate::util::unsupported_harness;
use hookkit_common::TurnCompletionOutput;
use hookkit_core::{BuiltinHarness, HarnessId, HookkitError};
use hookkit_pkl_config::schema as pkl;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HarnessCapabilities {
    allowed_user: Option<&'static str>,
    allowed_agent: Option<&'static str>,
    blocked_user: Option<&'static str>,
    blocked_agent: Option<&'static str>,
}

/// The exact native Stop audience capability matrix. An absent channel means
/// that using a syntactically valid native field would not faithfully deliver
/// that audience at this completion state.
///
/// Claude's Stop `hookSpecificOutput.additionalContext` continues the
/// conversation through the same loop protection as `decision: "block"`, so
/// it is not an allowed-completion agent channel. Antigravity injects Stop
/// `reason` only when `decision` is `"continue"`, so an allowed Antigravity
/// stop has no channel at all.
fn capabilities(harness: &HarnessId) -> Option<HarnessCapabilities> {
    match BuiltinHarness::from_id(harness)? {
        BuiltinHarness::ClaudeCode => Some(HarnessCapabilities {
            allowed_user: Some("systemMessage"),
            allowed_agent: None,
            blocked_user: Some("systemMessage"),
            blocked_agent: Some("reason"),
        }),
        BuiltinHarness::Codex => Some(HarnessCapabilities {
            allowed_user: Some("systemMessage"),
            allowed_agent: None,
            blocked_user: Some("systemMessage"),
            blocked_agent: Some("reason"),
        }),
        BuiltinHarness::Antigravity => Some(HarnessCapabilities {
            allowed_user: None,
            allowed_agent: None,
            blocked_user: None,
            blocked_agent: Some("reason"),
        }),
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AudienceLowering {
    pub status: &'static str,
    pub native_channel: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StopLoweringMetadata {
    pub policy: &'static str,
    pub blocked: bool,
    pub user: AudienceLowering,
    pub agent: AudienceLowering,
    pub warnings: Vec<String>,
    /// Whether the omission warnings reached a native channel. Allowed
    /// Antigravity stops have none, so their warnings live only here.
    pub warnings_delivered: bool,
    pub strict_error: Option<String>,
}

pub(crate) struct StopLoweringPlan {
    pub metadata: StopLoweringMetadata,
    output: Option<TurnCompletionOutput>,
}

impl StopLoweringPlan {
    pub(crate) fn finish(self) -> hookkit_core::Result<TurnCompletionOutput> {
        match (self.output, self.metadata.strict_error) {
            (Some(output), None) => Ok(output),
            (None, Some(message)) => Err(invalid_data(message)),
            _ => Err(invalid_data(
                "invalid deferred Stop lowering plan".to_owned(),
            )),
        }
    }
}

/// Plan the exact native Stop response for one rendered deferred result.
///
/// A blocked completion always carries a non-empty agent reason: Codex treats
/// `decision: "block"` with a blank reason as an invalid hook result (and lets
/// the turn end), Claude documents the reason as required, and Antigravity
/// would re-enter its loop with no explanation. When the rendered agent
/// message is empty (for example an empty template), `fallback_reason` is
/// used and the agent audience is recorded as `synthesized`.
pub(crate) fn plan_stop_lowering(
    harness: &HarnessId,
    blocked: bool,
    user: Option<&str>,
    agent: Option<&str>,
    fallback_reason: &str,
    policy: pkl::LoweringPolicy,
) -> hookkit_core::Result<StopLoweringPlan> {
    let capabilities = capabilities(harness).ok_or_else(|| {
        unsupported_harness(
            harness,
            "the turn-completion runner has no Stop lowering for this harness",
        )
    })?;
    let user_channel = if blocked {
        capabilities.blocked_user
    } else {
        capabilities.allowed_user
    };
    let agent_channel = if blocked {
        capabilities.blocked_agent
    } else {
        capabilities.allowed_agent
    };
    let mut native_user = None;
    let mut native_agent = None;
    let mut unsupported = Vec::new();
    let mut warnings = Vec::new();
    let user_lowering = lower_audience(
        "user",
        user,
        user_channel,
        policy,
        harness,
        blocked,
        &mut native_user,
        &mut unsupported,
        &mut warnings,
    );
    let mut agent_lowering = lower_audience(
        "agent",
        agent,
        agent_channel,
        policy,
        harness,
        blocked,
        &mut native_agent,
        &mut unsupported,
        &mut warnings,
    );
    if blocked && native_agent.is_none() {
        native_agent = Some(fallback_reason.to_owned());
        agent_lowering = AudienceLowering {
            status: "synthesized",
            native_channel: agent_channel,
        };
    }
    let strict_error = (!unsupported.is_empty()).then(|| {
        format!(
            "strict deferred Stop lowering cannot represent {} for {harness} while completion is {}",
            unsupported.join(" and "),
            if blocked { "blocked" } else { "allowed" }
        )
    });
    let mut warnings_delivered = warnings.is_empty();
    if !warnings.is_empty() {
        let warning_text = warnings.join("\n");
        if user_channel.is_some() {
            append_message(&mut native_user, warning_text);
            warnings_delivered = true;
        } else if agent_channel.is_some() {
            append_message(&mut native_agent, warning_text);
            warnings_delivered = true;
        }
    }

    let metadata = StopLoweringMetadata {
        policy: policy_name(policy),
        blocked,
        user: user_lowering,
        agent: agent_lowering,
        warnings,
        warnings_delivered,
        strict_error,
    };
    if metadata.strict_error.is_some() {
        return Ok(StopLoweringPlan {
            metadata,
            output: None,
        });
    }
    let output = build_native_output(harness, blocked, native_user, native_agent)?;
    Ok(StopLoweringPlan {
        metadata,
        output: Some(output),
    })
}

#[allow(clippy::too_many_arguments)]
fn lower_audience(
    audience: &'static str,
    message: Option<&str>,
    native_channel: Option<&'static str>,
    policy: pkl::LoweringPolicy,
    harness: &HarnessId,
    blocked: bool,
    native_message: &mut Option<String>,
    unsupported: &mut Vec<&'static str>,
    warnings: &mut Vec<String>,
) -> AudienceLowering {
    let Some(message) = message.filter(|message| !message.trim().is_empty()) else {
        return AudienceLowering {
            status: "empty",
            native_channel,
        };
    };
    if native_channel.is_some() {
        *native_message = Some(message.to_owned());
        return AudienceLowering {
            status: "emitted",
            native_channel,
        };
    }
    match policy {
        pkl::LoweringPolicy::Strict => unsupported.push(audience),
        pkl::LoweringPolicy::BestEffort => {}
        pkl::LoweringPolicy::BestEffortWithWarnings => warnings.push(format!(
            "hookkit: omitted {audience} deferred Stop message because {harness} has no faithful {audience} channel while completion is {}",
            if blocked { "blocked" } else { "allowed" }
        )),
    }
    AudienceLowering {
        status: if matches!(policy, pkl::LoweringPolicy::Strict) {
            "unrepresentable"
        } else {
            "omitted"
        },
        native_channel: None,
    }
}

fn append_message(target: &mut Option<String>, addition: String) {
    match target {
        Some(target) if !target.is_empty() => {
            target.push('\n');
            target.push_str(&addition);
        }
        Some(target) => *target = addition,
        None => *target = Some(addition),
    }
}

fn build_native_output(
    harness: &HarnessId,
    blocked: bool,
    user: Option<String>,
    agent: Option<String>,
) -> hookkit_core::Result<TurnCompletionOutput> {
    let reason = || {
        agent
            .clone()
            .filter(|reason| !reason.trim().is_empty())
            .ok_or_else(|| invalid_data("a blocked deferred Stop needs a reason".to_owned()))
    };
    match BuiltinHarness::from_id(harness) {
        Some(BuiltinHarness::ClaudeCode) => {
            let native = if blocked {
                hookkit_claude::catalog::StopOutput::block(reason()?)
            } else {
                hookkit_claude::catalog::StopOutput::no_op()
            };
            Ok(TurnCompletionOutput::Claude(match user {
                Some(user) => native.with_system_message(user)?,
                None => native,
            }))
        }
        Some(BuiltinHarness::Codex) => {
            let native = if blocked {
                hookkit_codex::catalog::StopOutput::block(reason()?)
            } else {
                hookkit_codex::catalog::StopOutput::no_op()
            };
            Ok(TurnCompletionOutput::Codex(match user {
                Some(user) => native.with_system_message(user)?,
                None => native,
            }))
        }
        Some(BuiltinHarness::Antigravity) => Ok(TurnCompletionOutput::Antigravity(if blocked {
            hookkit_antigravity::StopOutput::continue_with(reason()?)
        } else {
            hookkit_antigravity::StopOutput::allow_stop()
        })),
        _ => Err(unsupported_harness(
            harness,
            "the turn-completion runner has no Stop lowering for this harness",
        )),
    }
}

fn policy_name(policy: pkl::LoweringPolicy) -> &'static str {
    match policy {
        pkl::LoweringPolicy::Strict => "strict",
        pkl::LoweringPolicy::BestEffort => "best-effort",
        pkl::LoweringPolicy::BestEffortWithWarnings => "best-effort-with-warnings",
    }
}

fn invalid_data(message: String) -> HookkitError {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hookkit_core::EventSpec as _;

    const FALLBACK: &str = "Deferred checks need attention; see /state/run/summary.json.";

    fn stdout_json(output: TurnCompletionOutput) -> serde_json::Value {
        let emission = match output {
            TurnCompletionOutput::Claude(native) => {
                hookkit_claude::catalog::Stop::emit(native).unwrap()
            }
            TurnCompletionOutput::Codex(native) => {
                hookkit_codex::catalog::Stop::emit(native).unwrap()
            }
            TurnCompletionOutput::Antigravity(native) => {
                hookkit_antigravity::Stop::emit(native).unwrap()
            }
            _ => panic!("unexpected harness"),
        };
        assert_eq!(emission.exit_code(), 0);
        serde_json::from_slice(emission.stdout()).unwrap()
    }

    #[test]
    fn capability_matrix_matches_exact_native_stop_surfaces() {
        let claude = capabilities(&HarnessId::CLAUDE_CODE).unwrap();
        assert!(claude.allowed_user.is_some());
        assert!(
            claude.allowed_agent.is_none(),
            "Claude Stop additionalContext continues the turn"
        );
        assert!(claude.blocked_user.is_some());
        assert_eq!(claude.blocked_agent, Some("reason"));

        let codex = capabilities(&HarnessId::CODEX).unwrap();
        assert!(codex.allowed_user.is_some());
        assert!(codex.allowed_agent.is_none());
        assert!(codex.blocked_user.is_some());
        assert_eq!(codex.blocked_agent, Some("reason"));

        let antigravity = capabilities(&HarnessId::ANTIGRAVITY).unwrap();
        assert!(antigravity.allowed_user.is_none());
        assert!(antigravity.allowed_agent.is_none());
        assert!(antigravity.blocked_user.is_none());
        assert_eq!(antigravity.blocked_agent, Some("reason"));
    }

    #[test]
    fn allowed_claude_stop_never_emits_a_continuing_agent_channel() {
        let plan = plan_stop_lowering(
            &HarnessId::CLAUDE_CODE,
            false,
            Some("Auto-fixed 1 file: a.rs"),
            Some("Auto-fixed 1 file; re-read changed files."),
            FALLBACK,
            pkl::LoweringPolicy::BestEffortWithWarnings,
        )
        .unwrap();
        assert_eq!(plan.metadata.agent.status, "omitted");
        let json = stdout_json(plan.finish().unwrap());
        assert!(json.get("hookSpecificOutput").is_none(), "{json}");
        assert!(json.get("decision").is_none(), "{json}");
        let system = json["systemMessage"].as_str().unwrap();
        assert!(system.starts_with("Auto-fixed 1 file: a.rs"));
        assert!(system.contains("omitted agent"));
    }

    #[test]
    fn blocked_claude_stop_sends_the_agent_text_once() {
        let plan = plan_stop_lowering(
            &HarnessId::CLAUDE_CODE,
            true,
            Some("user"),
            Some("fix a.rs"),
            FALLBACK,
            pkl::LoweringPolicy::BestEffortWithWarnings,
        )
        .unwrap();
        let json = stdout_json(plan.finish().unwrap());
        assert_eq!(json["decision"], "block");
        assert_eq!(json["reason"], "fix a.rs");
        assert!(json.get("hookSpecificOutput").is_none(), "{json}");
    }

    #[test]
    fn blocked_stops_never_emit_a_blank_reason() {
        for harness in [
            HarnessId::CLAUDE_CODE,
            HarnessId::CODEX,
            HarnessId::ANTIGRAVITY,
        ] {
            for agent in [None, Some(""), Some("  \n")] {
                let plan = plan_stop_lowering(
                    &harness,
                    true,
                    None,
                    agent,
                    FALLBACK,
                    pkl::LoweringPolicy::Strict,
                )
                .unwrap();
                assert_eq!(plan.metadata.agent.status, "synthesized");
                let json = stdout_json(plan.finish().unwrap());
                assert_eq!(json["reason"], FALLBACK, "{harness}: {json}");
            }
        }
    }

    #[test]
    fn allowed_antigravity_stop_carries_no_reason_and_records_undelivered_warnings() {
        let plan = plan_stop_lowering(
            &HarnessId::ANTIGRAVITY,
            false,
            Some("Checked 1 clean file"),
            None,
            FALLBACK,
            pkl::LoweringPolicy::BestEffortWithWarnings,
        )
        .unwrap();
        assert_eq!(plan.metadata.user.status, "omitted");
        assert_eq!(plan.metadata.warnings.len(), 1);
        assert!(!plan.metadata.warnings_delivered);
        let json = stdout_json(plan.finish().unwrap());
        assert_eq!(json, serde_json::json!({"decision": "stop"}));
    }

    #[test]
    fn blocked_antigravity_stop_carries_warnings_in_its_continue_reason() {
        let plan = plan_stop_lowering(
            &HarnessId::ANTIGRAVITY,
            true,
            Some("user text"),
            Some("fix a.rs"),
            FALLBACK,
            pkl::LoweringPolicy::BestEffortWithWarnings,
        )
        .unwrap();
        assert!(plan.metadata.warnings_delivered);
        let json = stdout_json(plan.finish().unwrap());
        assert_eq!(json["decision"], "continue");
        let reason = json["reason"].as_str().unwrap();
        assert!(reason.starts_with("fix a.rs\n"));
        assert!(reason.contains("omitted user"));
    }

    #[test]
    fn strict_rejects_an_allowed_codex_agent_message() {
        let plan = plan_stop_lowering(
            &HarnessId::CODEX,
            false,
            Some("user"),
            Some("agent"),
            FALLBACK,
            pkl::LoweringPolicy::Strict,
        )
        .unwrap();
        assert_eq!(plan.metadata.user.status, "emitted");
        assert_eq!(plan.metadata.agent.status, "unrepresentable");
        assert!(plan.metadata.strict_error.is_some());
        assert!(plan.finish().is_err());
    }

    #[test]
    fn best_effort_omits_without_blocking_and_warning_policy_records_loss() {
        let omitted = plan_stop_lowering(
            &HarnessId::CODEX,
            false,
            Some("user"),
            Some("agent"),
            FALLBACK,
            pkl::LoweringPolicy::BestEffort,
        )
        .unwrap();
        assert_eq!(omitted.metadata.agent.status, "omitted");
        assert!(omitted.metadata.warnings.is_empty());
        assert!(omitted.finish().is_ok());

        let warned = plan_stop_lowering(
            &HarnessId::CODEX,
            false,
            Some("user"),
            Some("agent"),
            FALLBACK,
            pkl::LoweringPolicy::BestEffortWithWarnings,
        )
        .unwrap();
        assert_eq!(warned.metadata.agent.status, "omitted");
        assert_eq!(warned.metadata.warnings.len(), 1);
        assert!(warned.metadata.warnings_delivered);
        assert!(warned.finish().is_ok());
    }
}
