//! Output validation — checks outputs before emission.

use hookkit_claude::output::OutputEnvelope as ClaudeEnvelope;
use hookkit_codex::output::OutputEnvelope as CodexEnvelope;
use hookkit_core::{Harness, HookEventKey, HookkitError};
use hookkit_gemini::output::OutputEnvelope as GeminiEnvelope;

/// Validate a Claude output envelope.
pub fn validate_claude(envelope: &ClaudeEnvelope) -> Result<(), HookkitError> {
    // Block requires a reason
    if envelope.decision.as_deref() == Some("block") && envelope.reason.is_none() {
        return Err(HookkitError::InvalidOutputCombination {
            harness: Harness::Claude,
            event: HookEventKey::Other("unknown".to_string()),
            message: "decision 'block' requires a reason".to_string(),
        });
    }

    // Cannot have both top-level decision and hookSpecificOutput.permissionDecision
    if envelope.decision.is_some() {
        if let Some(hso) = &envelope.hook_specific_output {
            if hso.get("permissionDecision").is_some() {
                return Err(HookkitError::InvalidOutputCombination {
                    harness: Harness::Claude,
                    event: HookEventKey::PreToolUse,
                    message: "cannot set both top-level decision and hookSpecificOutput.permissionDecision".to_string(),
                });
            }
        }
    }

    Ok(())
}

/// Validate a Codex output envelope.
pub fn validate_codex(envelope: &CodexEnvelope) -> Result<(), HookkitError> {
    // Deny requires a reason
    if envelope.decision.as_deref() == Some("deny") && envelope.reason.is_none() {
        return Err(HookkitError::InvalidOutputCombination {
            harness: Harness::Codex,
            event: HookEventKey::PreToolUse,
            message: "decision 'deny' requires a reason".to_string(),
        });
    }

    Ok(())
}

/// Validate a Gemini output envelope.
pub fn validate_gemini(envelope: &GeminiEnvelope) -> Result<(), HookkitError> {
    // Deny requires a reason
    if envelope.decision.as_deref() == Some("deny") && envelope.reason.is_none() {
        return Err(HookkitError::InvalidOutputCombination {
            harness: Harness::Gemini,
            event: HookEventKey::Other("unknown".to_string()),
            message: "decision 'deny' requires a reason".to_string(),
        });
    }

    // Retry requires a reason
    if envelope.decision.as_deref() == Some("retry") && envelope.reason.is_none() {
        return Err(HookkitError::InvalidOutputCombination {
            harness: Harness::Gemini,
            event: HookEventKey::Other("unknown".to_string()),
            message: "decision 'retry' requires a reason".to_string(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_block_without_reason_fails() {
        let env = ClaudeEnvelope {
            decision: Some("block".to_string()),
            reason: None,
            ..Default::default()
        };
        assert!(validate_claude(&env).is_err());
    }

    #[test]
    fn claude_block_with_reason_ok() {
        let env = ClaudeEnvelope::block("test");
        assert!(validate_claude(&env).is_ok());
    }

    #[test]
    fn claude_conflicting_decision_and_permission() {
        let env = ClaudeEnvelope {
            decision: Some("allow".to_string()),
            hook_specific_output: Some(serde_json::json!({
                "permissionDecision": {"decision": "deny", "reason": "test"}
            })),
            ..Default::default()
        };
        assert!(validate_claude(&env).is_err());
    }

    #[test]
    fn codex_deny_without_reason_fails() {
        let env = CodexEnvelope {
            decision: Some("deny".to_string()),
            reason: None,
            ..Default::default()
        };
        assert!(validate_codex(&env).is_err());
    }

    #[test]
    fn codex_deny_with_reason_ok() {
        let env = CodexEnvelope::deny("test");
        assert!(validate_codex(&env).is_ok());
    }

    #[test]
    fn gemini_deny_without_reason_fails() {
        let env = GeminiEnvelope {
            decision: Some("deny".to_string()),
            reason: None,
            ..Default::default()
        };
        assert!(validate_gemini(&env).is_err());
    }

    #[test]
    fn gemini_retry_without_reason_fails() {
        let env = GeminiEnvelope {
            decision: Some("retry".to_string()),
            reason: None,
            ..Default::default()
        };
        assert!(validate_gemini(&env).is_err());
    }

    #[test]
    fn gemini_retry_with_reason_ok() {
        let env = GeminiEnvelope::retry("test");
        assert!(validate_gemini(&env).is_ok());
    }
}
