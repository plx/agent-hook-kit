use hookkit_common::TurnCompletionOutput;
use hookkit_core::{HarnessId, HookkitError};
use hookkit_pkl_config::schema as pkl;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HarnessCapabilities {
    allowed_user: Option<&'static str>,
    allowed_agent: Option<&'static str>,
    blocked_user: Option<&'static str>,
    blocked_agent: Option<&'static str>,
    warning_fallback: Option<&'static str>,
}

/// The exact native Stop audience capability matrix. An absent
/// channel means that using a syntactically valid native field would not
/// faithfully deliver that audience at this completion state.
fn capabilities(harness: &HarnessId) -> Option<HarnessCapabilities> {
    match harness.as_str() {
        "claude-code" => Some(HarnessCapabilities {
            allowed_user: Some("systemMessage"),
            allowed_agent: Some("hookSpecificOutput.additionalContext"),
            blocked_user: Some("systemMessage"),
            blocked_agent: Some("reason+hookSpecificOutput.additionalContext"),
            warning_fallback: None,
        }),
        "codex" => Some(HarnessCapabilities {
            allowed_user: Some("systemMessage"),
            allowed_agent: None,
            blocked_user: Some("systemMessage"),
            blocked_agent: Some("reason"),
            warning_fallback: None,
        }),
        "antigravity" => Some(HarnessCapabilities {
            allowed_user: None,
            allowed_agent: None,
            blocked_user: None,
            blocked_agent: Some("reason"),
            warning_fallback: Some("reason"),
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

pub(crate) fn plan_stop_lowering(
    harness: &HarnessId,
    blocked: bool,
    user: Option<&str>,
    agent: Option<&str>,
    policy: pkl::LoweringPolicy,
) -> hookkit_core::Result<StopLoweringPlan> {
    let capabilities = capabilities(harness).ok_or_else(|| {
        invalid_data(format!("turn-completion runner does not support {harness}"))
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
    let agent_lowering = lower_audience(
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
    let strict_error = (!unsupported.is_empty()).then(|| {
        format!(
            "strict deferred Stop lowering cannot represent {} for {harness} while completion is {}",
            unsupported.join(" and "),
            if blocked { "blocked" } else { "allowed" }
        )
    });
    let mut diagnostic_reason = None;
    if !warnings.is_empty() {
        let warning_text = warnings.join("\n");
        if user_channel.is_some() {
            append_message(&mut native_user, warning_text);
        } else if agent_channel.is_some() {
            append_message(&mut native_agent, warning_text);
        } else if capabilities.warning_fallback.is_some() {
            diagnostic_reason = Some(warning_text);
        }
    }

    let metadata = StopLoweringMetadata {
        policy: policy_name(policy),
        blocked,
        user: user_lowering,
        agent: agent_lowering,
        warnings,
        strict_error,
    };
    if metadata.strict_error.is_some() {
        return Ok(StopLoweringPlan {
            metadata,
            output: None,
        });
    }
    let output = build_native_output(
        harness,
        blocked,
        native_user,
        native_agent,
        diagnostic_reason,
    )?;
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
    diagnostic_reason: Option<String>,
) -> hookkit_core::Result<TurnCompletionOutput> {
    match harness.as_str() {
        "claude-code" => {
            let native = if blocked {
                match agent {
                    Some(agent) => hookkit_claude::catalog::StopOutput::block_with_context(
                        agent.clone(),
                        agent,
                    ),
                    None => hookkit_claude::catalog::StopOutput::block(""),
                }
            } else {
                match agent {
                    Some(agent) => hookkit_claude::catalog::StopOutput::with_context(agent),
                    None => hookkit_claude::catalog::StopOutput::no_op(),
                }
            };
            Ok(TurnCompletionOutput::Claude(match user {
                Some(user) => native.with_system_message(user)?,
                None => native,
            }))
        }
        "codex" => {
            let native = if blocked {
                hookkit_codex::catalog::StopOutput::block(agent.unwrap_or_default())
            } else {
                hookkit_codex::catalog::StopOutput::no_op()
            };
            Ok(TurnCompletionOutput::Codex(match user {
                Some(user) => native.with_system_message(user)?,
                None => native,
            }))
        }
        "antigravity" => Ok(TurnCompletionOutput::Antigravity(
            hookkit_antigravity::StopOutput {
                decision: if blocked { "continue" } else { "stop" }.into(),
                reason: if blocked { agent } else { diagnostic_reason },
            },
        )),
        _ => Err(invalid_data(format!(
            "turn-completion runner does not support {harness}"
        ))),
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

    #[test]
    fn capability_matrix_matches_exact_native_stop_surfaces() {
        let claude = capabilities(&HarnessId::CLAUDE_CODE).unwrap();
        assert!(claude.allowed_user.is_some());
        assert!(claude.allowed_agent.is_some());
        assert!(claude.blocked_user.is_some());
        assert!(claude.blocked_agent.is_some());

        let codex = capabilities(&HarnessId::CODEX).unwrap();
        assert!(codex.allowed_user.is_some());
        assert!(codex.allowed_agent.is_none());
        assert!(codex.blocked_user.is_some());
        assert!(codex.blocked_agent.is_some());

        let antigravity = capabilities(&HarnessId::ANTIGRAVITY).unwrap();
        assert!(antigravity.allowed_user.is_none());
        assert!(antigravity.allowed_agent.is_none());
        assert!(antigravity.blocked_user.is_none());
        assert_eq!(antigravity.blocked_agent, Some("reason"));
    }

    #[test]
    fn strict_rejects_an_allowed_codex_agent_message() {
        let plan = plan_stop_lowering(
            &HarnessId::CODEX,
            false,
            Some("user"),
            Some("agent"),
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
            pkl::LoweringPolicy::BestEffortWithWarnings,
        )
        .unwrap();
        assert_eq!(warned.metadata.agent.status, "omitted");
        assert_eq!(warned.metadata.warnings.len(), 1);
        assert!(warned.finish().is_ok());
    }

    #[test]
    fn empty_agent_message_needs_no_capability() {
        let plan = plan_stop_lowering(
            &HarnessId::CODEX,
            true,
            Some("user"),
            None,
            pkl::LoweringPolicy::Strict,
        )
        .unwrap();
        assert_eq!(plan.metadata.agent.status, "empty");
        assert!(plan.finish().is_ok());
    }
}
